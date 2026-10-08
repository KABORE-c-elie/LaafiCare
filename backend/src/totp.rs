//! Second facteur TOTP, compatible Google Authenticator (section 14 du
//! CLAUDE.md, décisions Z1 à Z10, Y1, X1 à X4). Aucune route ici : elles
//! arrivent à l'étape 3.6, avec le jeton intermédiaire (Z10).
//!
//! Sources : RFC 6238 ; NIST SP 800-63B rév. 4 (§3.1.2 codes de secours,
//! §3.1.4 OTP, §3.2.2 limitation des essais) ; code source des crates
//! `totp-rs` 6.0.0 et `aes-gcm` 0.11.1 / `aead` 0.6.1 (lu dans le registre
//! Cargo le 2026-10-06).
//!
//! Chaque essai de code, ou de code de secours, passe par le compteur de
//! `verrouillage` (5 essais / 15 min, BF-01), compté AVANT la vérification ;
//! seul un code correct le remet à zéro (X1).

use std::time::{SystemTime, UNIX_EPOCH};

use aes_gcm::aead::{Aead, Generate, Nonce, Payload};
use aes_gcm::{Aes256Gcm, KeyInit};
use rand::random_range;
use sqlx::PgPool;
use sqlx::types::Uuid;
use totp_rs::{Algorithm, Builder, Secret, Totp};
use zeroize::Zeroizing;

use crate::mot_de_passe;
use crate::verrouillage::{self, ErreurConnexion};

/// Clé avec laquelle les nouveaux secrets sont chiffrés (colonne
/// `version_cle`, Z4) : prépare un futur changement de clé.
pub const VERSION_CLE: i16 = 1;

/// Z8 : émetteur affiché dans l'application d'authentification. Le nom du
/// compte n'est que « Patient », « Professionnel » ou « Administrateur » :
/// aucune donnée personnelle dans l'URL, qui peut finir dans une sauvegarde
/// en ligne du téléphone.
const EMETTEUR: &str = "LaafiCare";

/// Z6 : 10 codes de 10 caractères en base 32 (environ 50 bits chacun).
pub const NOMBRE_CODES_SECOURS: usize = 10;
const LONGUEUR_CODE_SECOURS: usize = 10;
/// Alphabet base 32 de la RFC 4648 : sans 0, 1, 8 ni 9, des chiffres qui se
/// confondent avec des lettres à la lecture.
const ALPHABET_BASE32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

#[derive(Debug, thiserror::Error)]
pub enum ErreurTotp {
    /// Mauvais code ou mauvais mot de passe, code déjà utilisé, compte
    /// verrouillé ou désactivé, aucun TOTP à vérifier : jamais distingués,
    /// comme pour la connexion (décisions P1 et L1).
    #[error("code ou mot de passe invalide")]
    Refuse,

    #[error("erreur interne : {0}")]
    Interne(String),
}

impl From<sqlx::Error> for ErreurTotp {
    fn from(erreur: sqlx::Error) -> Self {
        ErreurTotp::Interne(erreur.to_string())
    }
}

fn interne(erreur: impl std::fmt::Display) -> ErreurTotp {
    ErreurTotp::Interne(erreur.to_string())
}

// ---------------------------------------------------------------------
// Chiffrement du secret (Z4)
// ---------------------------------------------------------------------

/// AES-256-GCM avec un nonce aléatoire de 96 bits par chiffrement (doc
/// `aes-gcm` : « MUST be unique per message »), tiré par `getrandom`.
/// L'identifiant du compte est la donnée associée : un secret recopié sur
/// un autre compte ne se déchiffre pas (doc `aead::Aead::encrypt` : la même
/// AAD doit être fournie au déchiffrement, sinon il échoue). L'étiquette de
/// 16 octets est ajoutée à la fin du texte chiffré (« postfix tag »,
/// implémentation par défaut de `Aead::encrypt`) : 20 + 16 = 36 octets, le
/// CHECK de la migration 0013.
fn chiffrer_secret(cle: &[u8; 32], compte_id: Uuid, secret: &[u8]) -> Result<(Vec<u8>, Vec<u8>), ErreurTotp> {
    let chiffreur = Aes256Gcm::new_from_slice(cle).map_err(interne)?;
    let nonce = Nonce::<Aes256Gcm>::try_generate().map_err(interne)?;
    let chiffre = chiffreur
        .encrypt(&nonce, Payload { msg: secret, aad: compte_id.as_bytes() })
        .map_err(interne)?;
    Ok((chiffre, nonce.to_vec()))
}

/// Un échec ici (clé changée, ligne recopiée d'un autre compte, donnée
/// altérée) est une erreur du serveur, jamais une erreur de saisie.
///
/// Le texte clair n'est jamais passé à `Secret::from(Vec<u8>)` : celle-ci
/// appelle `into_boxed_slice`, qui réalloue quand la capacité dépasse la
/// longueur — c'est le cas ici, `decrypt` ayant retiré l'étiquette de
/// 16 octets — et l'ancien bloc serait libéré sans être effacé. Doc
/// `zeroize` 1.9.0 : son effacement d'un `Vec` « cannot guarantee copies of
/// the data were not previously made by buffer reallocation ». Le clair
/// reste donc dans un `Zeroizing` (toute la capacité effacée à la sortie,
/// erreur comprise), et `Secret` reçoit une copie allouée à la taille exacte
/// (`Box::from(&[u8])`), qu'il efface lui-même (fonctionnalité zeroize de
/// totp-rs).
fn dechiffrer_secret(cle: &[u8; 32], compte_id: Uuid, chiffre: &[u8], nonce: &[u8]) -> Result<Secret, ErreurTotp> {
    let chiffreur = Aes256Gcm::new_from_slice(cle).map_err(interne)?;
    let nonce = Nonce::<Aes256Gcm>::try_from(nonce).map_err(interne)?;
    let clair = Zeroizing::new(
        chiffreur
            .decrypt(&nonce, Payload { msg: chiffre, aad: compte_id.as_bytes() })
            .map_err(interne)?,
    );
    Ok(Secret::new(Box::from(clair.as_slice())))
}

fn verifier_version_cle(version_cle: i16) -> Result<(), ErreurTotp> {
    if version_cle != VERSION_CLE {
        return Err(ErreurTotp::Interne(format!("version de clé TOTP inconnue : {version_cle}")));
    }
    Ok(())
}

// ---------------------------------------------------------------------
// Paramètres TOTP
// ---------------------------------------------------------------------

/// Valeurs écrites explicitement, bien qu'elles soient celles par défaut de
/// `Builder::new` : SHA-1, 6 chiffres, pas de 30 s (défauts de la RFC 6238 ;
/// doc `totp-rs` : certaines applications reviennent silencieusement à
/// SHA-1). Tolérance d'un pas avant et après (Z3, NIST : dérive d'horloge
/// dans les deux sens), soit environ 90 s.
fn parametres() -> Builder {
    Builder::new()
        .with_algorithm(Algorithm::SHA1)
        .with_digits(6)
        .with_step_duration(30)
        .with_skew(1)
}

fn totp_depuis(secret: Secret) -> Result<Totp, ErreurTotp> {
    parametres().with_secret(secret).build().map_err(interne)
}

fn maintenant() -> Result<u64, ErreurTotp> {
    // `check_current` de la crate panique si l'horloge est avant 1970 : on
    // lit l'heure ici pour transformer ce cas en erreur.
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duree| duree.as_secs())
        .map_err(interne)
}

/// Vérifie le code et renvoie le pas de temps qui l'a validé (doc
/// `Totp::check` : « return the matched step »). La crate ne refuse pas un
/// code déjà utilisé (« it is the caller's responsibility ») : c'est le rôle
/// de `dernier_pas`.
fn pas_valide(totp: &Totp, code: &str, instant: u64) -> Result<i64, ErreurTotp> {
    let pas = totp.check(code, instant).ok_or(ErreurTotp::Refuse)?;
    i64::try_from(pas).map_err(interne)
}

/// Compte l'essai (X1) ; un compte verrouillé, inexistant ou désactivé est
/// refusé sans rien vérifier.
async fn compter_essai(pool: &PgPool, compte_id: Uuid) -> Result<(), ErreurTotp> {
    match verrouillage::compter_tentative(pool, compte_id).await? {
        Some(tentative) if tentative.statut == "actif" => Ok(()),
        _ => Err(ErreurTotp::Refuse),
    }
}

// ---------------------------------------------------------------------
// Activation (Z7, X3)
// ---------------------------------------------------------------------

/// Ce que la personne saisit dans son application d'authentification. Le QR
/// code est dessiné par Angular ou Flutter à partir de l'URL (Z2).
pub struct ActivationEnCours {
    pub url_otpauth: String,
    pub cle_base32: String,
}

/// Z7 : le mot de passe est exigé, compté comme un essai. Une demande déjà
/// en attente est remplacée (X3 : QR perdu, application fermée). Le TOTP
/// actif éventuel reste valable jusqu'à la confirmation du nouveau.
pub async fn commencer_activation(
    pool: &PgPool,
    cle: &[u8; 32],
    compte_id: Uuid,
    mot_de_passe_saisi: &str,
) -> Result<ActivationEnCours, ErreurTotp> {
    verrouillage::verifier(pool, Some(compte_id), mot_de_passe_saisi)
        .await
        .map_err(|erreur| match erreur {
            ErreurConnexion::IdentifiantsInvalides => ErreurTotp::Refuse,
            ErreurConnexion::Interne(detail) => ErreurTotp::Interne(detail),
        })?;

    let type_compte: String = sqlx::query_scalar("SELECT type_compte FROM compte WHERE id = $1")
        .bind(compte_id)
        .fetch_one(pool)
        .await?;
    let nom_compte = match type_compte.as_str() {
        "patient" => "Patient",
        "professionnel" => "Professionnel",
        "administrateur_laaficare" => "Administrateur",
        autre => return Err(ErreurTotp::Interne(format!("type de compte inconnu : {autre}"))),
    };

    // Sans `with_secret`, `build` génère le secret (fonctionnalité
    // `gen_secret`) : 160 bits, taille recommandée par la RFC 4226 (doc du
    // champ `secret` de `Totp`).
    let totp = parametres()
        .with_issuer(Some(EMETTEUR))
        .with_account_name(nom_compte)
        .build()
        .map_err(interne)?;
    let (secret_chiffre, nonce) = chiffrer_secret(cle, compte_id, totp.secret().as_bytes())?;

    let mut transaction = pool.begin().await?;
    sqlx::query("DELETE FROM second_facteur_totp WHERE compte_id = $1 AND statut = 'en_attente'")
        .bind(compte_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query(
        "INSERT INTO second_facteur_totp (compte_id, secret_chiffre, nonce, version_cle) VALUES ($1, $2, $3, $4)",
    )
    .bind(compte_id)
    .bind(&secret_chiffre)
    .bind(&nonce)
    .bind(VERSION_CLE)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;

    Ok(ActivationEnCours {
        url_otpauth: totp.to_url().map_err(interne)?,
        cle_base32: totp.secret().to_base32(),
    })
}

/// Confirme l'activation avec un premier code correct (Z7). Renvoie les
/// codes de secours en clair : c'est la seule fois qu'ils sont visibles
/// (Z6).
pub async fn confirmer_activation(
    pool: &PgPool,
    cle: &[u8; 32],
    compte_id: Uuid,
    code: &str,
) -> Result<Vec<String>, ErreurTotp> {
    confirmer_activation_a(pool, cle, compte_id, code, maintenant()?).await
}

async fn confirmer_activation_a(
    pool: &PgPool,
    cle: &[u8; 32],
    compte_id: Uuid,
    code: &str,
    instant: u64,
) -> Result<Vec<String>, ErreurTotp> {
    compter_essai(pool, compte_id).await?;

    let en_attente: Option<(Uuid, Vec<u8>, Vec<u8>, i16)> = sqlx::query_as(
        "SELECT id, secret_chiffre, nonce, version_cle FROM second_facteur_totp \
         WHERE compte_id = $1 AND statut = 'en_attente'",
    )
    .bind(compte_id)
    .fetch_optional(pool)
    .await?;
    let Some((id, secret_chiffre, nonce, version_cle)) = en_attente else {
        return Err(ErreurTotp::Refuse);
    };
    verifier_version_cle(version_cle)?;
    let totp = totp_depuis(dechiffrer_secret(cle, compte_id, &secret_chiffre, &nonce)?)?;
    let pas = pas_valide(&totp, code, instant)?;

    // Hachés avant la transaction : 10 hachages Argon2id ne doivent pas
    // garder des verrous ouverts.
    let (codes, hachages) = generer_codes_secours()?;

    let mut transaction = pool.begin().await?;
    // La demande est verrouillée : deux confirmations simultanées ne
    // peuvent pas toutes deux réussir (la seconde attend, puis ne la trouve
    // plus en attente).
    let toujours_en_attente: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM second_facteur_totp WHERE id = $1 AND statut = 'en_attente' FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *transaction)
    .await?;
    if toujours_en_attente.is_none() {
        return Err(ErreurTotp::Refuse);
    }
    // Y1 : l'ancien TOTP actif est supprimé, ses codes de secours avec lui
    // (ON DELETE CASCADE). Avant l'activation du nouveau, sinon l'index
    // `second_facteur_totp_un_actif` la refuse.
    sqlx::query("DELETE FROM second_facteur_totp WHERE compte_id = $1 AND statut = 'actif'")
        .bind(compte_id)
        .execute(&mut *transaction)
        .await?;
    // `dernier_pas` dès l'activation : le code de confirmation ne pourra
    // pas resservir à la connexion suivante.
    sqlx::query(
        "UPDATE second_facteur_totp SET statut = 'actif', date_activation = now(), dernier_pas = $2 WHERE id = $1",
    )
    .bind(id)
    .bind(pas)
    .execute(&mut *transaction)
    .await?;
    for hachage in &hachages {
        sqlx::query("INSERT INTO code_secours (second_facteur_id, hash) VALUES ($1, $2)")
            .bind(id)
            .bind(hachage)
            .execute(&mut *transaction)
            .await?;
    }
    transaction.commit().await?;

    verrouillage::remettre_a_zero(pool, compte_id).await?;
    Ok(codes)
}

// ---------------------------------------------------------------------
// Vérification d'un code
// ---------------------------------------------------------------------

/// Vérifie un code du TOTP actif. Un code n'est accepté qu'une fois (RFC
/// 6238 « MUST NOT accept the second attempt », NIST « SHALL accept a given
/// OTP only once ») : son pas doit être strictement plus grand que le
/// dernier accepté, vérifié et enregistré en une seule requête atomique.
pub async fn verifier_code(pool: &PgPool, cle: &[u8; 32], compte_id: Uuid, code: &str) -> Result<(), ErreurTotp> {
    verifier_code_a(pool, cle, compte_id, code, maintenant()?).await
}

async fn verifier_code_a(
    pool: &PgPool,
    cle: &[u8; 32],
    compte_id: Uuid,
    code: &str,
    instant: u64,
) -> Result<(), ErreurTotp> {
    compter_essai(pool, compte_id).await?;

    let actif: Option<(Uuid, Vec<u8>, Vec<u8>, i16)> = sqlx::query_as(
        "SELECT id, secret_chiffre, nonce, version_cle FROM second_facteur_totp \
         WHERE compte_id = $1 AND statut = 'actif'",
    )
    .bind(compte_id)
    .fetch_optional(pool)
    .await?;
    let Some((id, secret_chiffre, nonce, version_cle)) = actif else {
        return Err(ErreurTotp::Refuse);
    };
    verifier_version_cle(version_cle)?;
    let totp = totp_depuis(dechiffrer_secret(cle, compte_id, &secret_chiffre, &nonce)?)?;
    let pas = pas_valide(&totp, code, instant)?;

    let accepte = sqlx::query(
        "UPDATE second_facteur_totp SET dernier_pas = $2 \
         WHERE id = $1 AND statut = 'actif' AND (dernier_pas IS NULL OR dernier_pas < $2)",
    )
    .bind(id)
    .bind(pas)
    .execute(pool)
    .await?
    .rows_affected()
        == 1;
    if !accepte {
        return Err(ErreurTotp::Refuse);
    }

    verrouillage::remettre_a_zero(pool, compte_id).await?;
    Ok(())
}

// ---------------------------------------------------------------------
// Codes de secours (Z6, X2)
// ---------------------------------------------------------------------

/// 10 codes tirés par `rand::random_range` (`ThreadRng`, générateur
/// cryptographique selon la doc de rand 0.10, comme dans `otp.rs`). Affichés
/// `XXXXX-XXXXX` ; hachés sans le tiret, en Argon2id (NIST §3.1.2 : sous
/// 112 bits, hachage de mot de passe salé exigé).
fn generer_codes_secours() -> Result<(Vec<String>, Vec<String>), ErreurTotp> {
    let mut codes = Vec::with_capacity(NOMBRE_CODES_SECOURS);
    let mut hachages = Vec::with_capacity(NOMBRE_CODES_SECOURS);
    for _ in 0..NOMBRE_CODES_SECOURS {
        let brut: String = (0..LONGUEUR_CODE_SECOURS)
            .map(|_| ALPHABET_BASE32[random_range(0..ALPHABET_BASE32.len())] as char)
            .collect();
        hachages.push(mot_de_passe::hacher(&brut).map_err(interne)?);
        codes.push(format!("{}-{}", &brut[..5], &brut[5..]));
    }
    Ok((codes, hachages))
}

/// Accepte la saisie avec ou sans tiret ni espaces, en minuscules ou en
/// majuscules. `None` si le résultat ne peut pas être un code.
fn normaliser_code_secours(saisie: &str) -> Option<String> {
    let code: String = saisie
        .chars()
        .filter(|c| *c != '-' && !c.is_whitespace())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    let valide = code.len() == LONGUEUR_CODE_SECOURS && code.bytes().all(|b| ALPHABET_BASE32.contains(&b));
    valide.then_some(code)
}

/// X2 : le sel étant dans chaque hachage, le code saisi est comparé à tous
/// les codes non utilisés (au plus 10 vérifications Argon2id), sans
/// s'arrêter au premier trouvé. Usage unique garanti par la base : la
/// condition `utilise_le IS NULL` de la mise à jour.
pub async fn verifier_code_secours(pool: &PgPool, compte_id: Uuid, saisie: &str) -> Result<(), ErreurTotp> {
    compter_essai(pool, compte_id).await?;
    let Some(code) = normaliser_code_secours(saisie) else {
        return Err(ErreurTotp::Refuse);
    };

    let disponibles: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT c.id, c.hash FROM code_secours c \
         JOIN second_facteur_totp t ON t.id = c.second_facteur_id \
         WHERE t.compte_id = $1 AND t.statut = 'actif' AND c.utilise_le IS NULL",
    )
    .bind(compte_id)
    .fetch_all(pool)
    .await?;

    let mut trouve = None;
    for (id, hachage) in &disponibles {
        if mot_de_passe::verifier(&code, hachage).map_err(interne)? {
            trouve = Some(*id);
        }
    }
    let Some(id) = trouve else {
        return Err(ErreurTotp::Refuse);
    };

    let utilise = sqlx::query("UPDATE code_secours SET utilise_le = now() WHERE id = $1 AND utilise_le IS NULL")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected()
        == 1;
    if !utilise {
        return Err(ErreurTotp::Refuse);
    }

    verrouillage::remettre_a_zero(pool, compte_id).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLE: [u8; 32] = [7; 32];
    const MOT_DE_PASSE: &str = "Bon-mot-de-passe-1";
    /// Instant fixe : les tests ne dépendent pas de l'horloge.
    const INSTANT: u64 = 1_800_000_000;

    #[test]
    fn secret_chiffre_de_36_octets_et_aller_retour() {
        let compte = Uuid::from_u128(1);
        let secret = Secret::generate();
        let (chiffre, nonce) = chiffrer_secret(&CLE, compte, secret.as_bytes()).unwrap();
        // Confirme le CHECK de la migration 0013 : 20 octets + étiquette de 16.
        assert_eq!(secret.as_bytes().len(), 20);
        assert_eq!(chiffre.len(), 36);
        assert_eq!(nonce.len(), 12);
        assert_eq!(dechiffrer_secret(&CLE, compte, &chiffre, &nonce).unwrap().as_bytes(), secret.as_bytes());

        // Deux chiffrements du même secret : nonces et textes différents.
        let (chiffre_bis, nonce_bis) = chiffrer_secret(&CLE, compte, secret.as_bytes()).unwrap();
        assert_ne!(nonce, nonce_bis);
        assert_ne!(chiffre, chiffre_bis);
    }

    #[test]
    fn dechiffrement_refuse_avec_autre_compte_ou_autre_cle() {
        let compte = Uuid::from_u128(1);
        let (chiffre, nonce) = chiffrer_secret(&CLE, compte, Secret::generate().as_bytes()).unwrap();
        assert!(matches!(
            dechiffrer_secret(&CLE, Uuid::from_u128(2), &chiffre, &nonce),
            Err(ErreurTotp::Interne(_))
        ));
        assert!(matches!(
            dechiffrer_secret(&[8; 32], compte, &chiffre, &nonce),
            Err(ErreurTotp::Interne(_))
        ));
    }

    /// RFC 6238, annexe B : secret ASCII « 12345678901234567890 », SHA-1.
    /// Les valeurs de la RFC ont 8 chiffres ; sur 6 chiffres, ce sont leurs
    /// 6 derniers (même troncature, modulo 10⁶).
    #[test]
    fn vecteurs_de_la_rfc_6238() {
        let totp = totp_depuis(Secret::from(b"12345678901234567890".to_vec())).unwrap();
        assert_eq!(pas_valide(&totp, "287082", 59).unwrap(), 1); // 94287082
        assert_eq!(pas_valide(&totp, "081804", 1_111_111_109).unwrap(), 37_037_036); // 07081804, zéro initial
        assert_eq!(pas_valide(&totp, "005924", 1_234_567_890).unwrap(), 41_152_263); // 89005924
        assert!(matches!(pas_valide(&totp, "287083", 59), Err(ErreurTotp::Refuse)));
        // Tolérance d'un pas (Z3) : accepté 30 s après, refusé 60 s après.
        assert_eq!(pas_valide(&totp, "287082", 89).unwrap(), 1);
        assert!(matches!(pas_valide(&totp, "287082", 119), Err(ErreurTotp::Refuse)));
    }

    #[test]
    fn url_sans_donnee_personnelle() {
        let totp = parametres()
            .with_issuer(Some(EMETTEUR))
            .with_account_name("Patient")
            .build()
            .unwrap();
        let url = totp.to_url().unwrap();
        assert!(url.starts_with("otpauth://totp/LaafiCare:Patient?secret="), "{url}");
        assert!(url.contains("issuer=LaafiCare"), "{url}");
    }

    #[test]
    fn codes_de_secours_formats_et_saisie() {
        let (codes, hachages) = generer_codes_secours().unwrap();
        assert_eq!(codes.len(), NOMBRE_CODES_SECOURS);
        assert_eq!(hachages.len(), NOMBRE_CODES_SECOURS);
        for code in &codes {
            assert_eq!(code.len(), 11, "{code}");
            assert_eq!(&code[5..6], "-", "{code}");
        }
        let mut distincts = codes.clone();
        distincts.sort();
        distincts.dedup();
        assert_eq!(distincts.len(), NOMBRE_CODES_SECOURS);

        assert_eq!(normaliser_code_secours("abcde-fgh23").unwrap(), "ABCDEFGH23");
        assert_eq!(normaliser_code_secours(" ABCDE FGH23 ").unwrap(), "ABCDEFGH23");
        // Trop court, trop long, hors de l'alphabet (0, 1, 8 et 9 n'y sont pas).
        for saisie in ["ABCDE-FGH2", "ABCDE-FGH234", "ABCDE-FGH01"] {
            assert!(normaliser_code_secours(saisie).is_none(), "{saisie}");
        }
    }

    // --- en base : base temporaire par test, jamais la base de
    // développement (section 12 du CLAUDE.md) ---

    async fn compte_patient(pool: &PgPool, telephone: &str) -> Uuid {
        let utilisateur_id: Uuid =
            sqlx::query_scalar("INSERT INTO utilisateur (nom, prenom, telephone) VALUES ('Test', 'Totp', $1) RETURNING id")
                .bind(telephone)
                .fetch_one(pool)
                .await
                .unwrap();
        sqlx::query_scalar(
            "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'patient', $2) RETURNING id",
        )
        .bind(utilisateur_id)
        .bind(mot_de_passe::hacher(MOT_DE_PASSE).unwrap())
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// Ce que ferait l'application d'authentification : relire la clé en
    /// base 32 et calculer le code d'un instant donné.
    fn code_a(activation: &ActivationEnCours, instant: u64) -> String {
        totp_depuis(Secret::try_from_base32(&activation.cle_base32).unwrap())
            .unwrap()
            .generate(instant)
            .to_string()
    }

    async fn activer(pool: &PgPool, compte: Uuid, instant: u64) -> (ActivationEnCours, Vec<String>) {
        let activation = commencer_activation(pool, &CLE, compte, MOT_DE_PASSE).await.unwrap();
        let codes = confirmer_activation_a(pool, &CLE, compte, &code_a(&activation, instant), instant)
            .await
            .unwrap();
        (activation, codes)
    }

    fn refuse<T>(resultat: Result<T, ErreurTotp>) -> bool {
        matches!(resultat, Err(ErreurTotp::Refuse))
    }

    async fn tentatives(pool: &PgPool, compte: Uuid) -> i16 {
        sqlx::query_scalar("SELECT tentatives_echouees FROM compte WHERE id = $1")
            .bind(compte)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[sqlx::test]
    #[ignore]
    async fn activation_puis_code_utilisable_une_seule_fois(pool: PgPool) {
        let compte = compte_patient(&pool, "+22670300001").await;

        // Z7 : mauvais mot de passe, aucune demande créée.
        assert!(refuse(commencer_activation(&pool, &CLE, compte, "Mauvais-mot-de-passe-2").await));

        let activation = commencer_activation(&pool, &CLE, compte, MOT_DE_PASSE).await.unwrap();
        assert!(activation.url_otpauth.starts_with("otpauth://totp/LaafiCare:Patient?"));

        // Tant que l'activation n'est pas confirmée, aucun code n'est accepté.
        let code = code_a(&activation, INSTANT);
        assert!(refuse(verifier_code_a(&pool, &CLE, compte, &code, INSTANT).await));

        let codes = confirmer_activation_a(&pool, &CLE, compte, &code, INSTANT).await.unwrap();
        assert_eq!(codes.len(), NOMBRE_CODES_SECOURS);
        assert_eq!(tentatives(&pool, compte).await, 0, "compteur remis à zéro après le bon code");

        // Le code de confirmation ne resert pas, même dans la même fenêtre.
        assert!(refuse(verifier_code_a(&pool, &CLE, compte, &code, INSTANT).await));

        // Code du pas suivant : accepté une fois, puis refusé.
        let suivant = code_a(&activation, INSTANT + 30);
        verifier_code_a(&pool, &CLE, compte, &suivant, INSTANT + 30).await.unwrap();
        assert!(refuse(verifier_code_a(&pool, &CLE, compte, &suivant, INSTANT + 30).await));

        // Un code plus ancien que le dernier accepté est refusé, même dans
        // la tolérance d'un pas.
        let ancien = code_a(&activation, INSTANT + 60);
        verifier_code_a(&pool, &CLE, compte, &code_a(&activation, INSTANT + 90), INSTANT + 90)
            .await
            .unwrap();
        assert!(refuse(verifier_code_a(&pool, &CLE, compte, &ancien, INSTANT + 90).await));
    }

    #[sqlx::test]
    #[ignore]
    async fn cinq_mauvais_codes_verrouillent_le_compte(pool: PgPool) {
        let compte = compte_patient(&pool, "+22670300002").await;
        let (activation, codes) = activer(&pool, compte, INSTANT).await;

        let instant = INSTANT + 300;
        for _ in 0..5 {
            assert!(refuse(verifier_code_a(&pool, &CLE, compte, "000000", instant).await));
        }
        // Verrouillé : le bon code et un bon code de secours sont refusés.
        assert!(refuse(verifier_code_a(&pool, &CLE, compte, &code_a(&activation, instant), instant).await));
        assert!(refuse(verifier_code_secours(&pool, compte, &codes[0]).await));
    }

    #[sqlx::test]
    #[ignore]
    async fn code_de_secours_a_usage_unique(pool: PgPool) {
        let compte = compte_patient(&pool, "+22670300003").await;
        let (_, codes) = activer(&pool, compte, INSTANT).await;

        // Saisie en minuscules, sans tiret : acceptée.
        let saisie = codes[0].replace('-', "").to_lowercase();
        verifier_code_secours(&pool, compte, &saisie).await.unwrap();
        assert!(refuse(verifier_code_secours(&pool, compte, &codes[0]).await));
        verifier_code_secours(&pool, compte, &codes[1]).await.unwrap();
        assert!(refuse(verifier_code_secours(&pool, compte, "AAAAA-AAAAA").await));
    }

    #[sqlx::test]
    #[ignore]
    async fn remplacement_l_ancien_reste_valable_jusqu_a_la_confirmation(pool: PgPool) {
        let compte = compte_patient(&pool, "+22670300004").await;
        let (ancien, anciens_codes) = activer(&pool, compte, INSTANT).await;

        // X3 : deux demandes de suite, seule la dernière reste en attente.
        let abandonnee = commencer_activation(&pool, &CLE, compte, MOT_DE_PASSE).await.unwrap();
        let nouveau = commencer_activation(&pool, &CLE, compte, MOT_DE_PASSE).await.unwrap();
        let instant = INSTANT + 300;
        assert!(refuse(
            confirmer_activation_a(&pool, &CLE, compte, &code_a(&abandonnee, instant), instant).await
        ));

        // Pendant le remplacement, l'ancien TOTP fonctionne toujours.
        verifier_code_a(&pool, &CLE, compte, &code_a(&ancien, instant), instant).await.unwrap();

        let nouveaux_codes = confirmer_activation_a(&pool, &CLE, compte, &code_a(&nouveau, instant + 30), instant + 30)
            .await
            .unwrap();

        // Y1 : l'ancien secret et ses codes de secours ont disparu.
        let instant = instant + 60;
        assert!(refuse(verifier_code_a(&pool, &CLE, compte, &code_a(&ancien, instant), instant).await));
        assert!(refuse(verifier_code_secours(&pool, compte, &anciens_codes[0]).await));
        verifier_code_secours(&pool, compte, &nouveaux_codes[0]).await.unwrap();
        let (totp, codes): (i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM second_facteur_totp), (SELECT count(*) FROM code_secours)",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!((totp, codes), (1, NOMBRE_CODES_SECOURS as i64));
    }
}
