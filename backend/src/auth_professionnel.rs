//! Connexion professionnelle (étape 3.6.2, décisions P1 à P7 du
//! 2026-10-15) : email + mot de passe sur le compte `professionnel`,
//! verrouillage BF-01 (`verrouillage.rs`), email vérifié (R0), second
//! facteur (Z10), puis session (`session.rs`, jeton
//! `laaficare:professionnel_sans_affectation`, J5).
//!
//! La logique commune prend le type de compte en paramètre (P6) ; seule la
//! connexion professionnelle est publique. Celle des administrateurs
//! viendra à l'étape 3.6.4 avec sa règle propre : sans TOTP actif, pas de
//! jeton complet mais un jeton de réenrôlement (Y4).
//!
//! **Récupération du mot de passe** : par email uniquement (décisions R1 à
//! R9, étape 3.12). L'OTP ne sert qu'au compte patient (section 14).

use std::fmt;

use sqlx::PgPool;
use sqlx::types::Uuid;
use zeroize::Zeroize;

use crate::config::CleTotp;
use crate::jwt::{JwtService, SorteJeton};
use crate::session::{self, PaireJetons};
use crate::totp::{self, ErreurTotp};
use crate::verrouillage::{self, ErreurConnexion, Verification};

/// Jeton intermédiaire du second facteur (5 min) : même traitement que le
/// jeton de renouvellement (`session.rs`). `Debug` affiche `***`, ni
/// `Display` ni `Serialize` : la seule lecture est `en_clair()`, au moment
/// de construire la réponse. Effacé de la mémoire à sa libération.
pub struct JetonIntermediaire(String);

impl JetonIntermediaire {
    pub fn en_clair(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for JetonIntermediaire {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

impl Drop for JetonIntermediaire {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// Résultat d'une connexion réussie (mot de passe correct, email vérifié).
pub enum ConnexionProfessionnelle {
    /// Pas de TOTP actif : session ouverte, paire de jetons.
    Complete(PaireJetons),
    /// TOTP actif : jeton intermédiaire, à échanger contre la paire avec le
    /// code (étape 3.6.3). Aucune session n'est encore ouverte.
    SecondFacteur { jeton_intermediaire: JetonIntermediaire },
}

// Écrit à la main : `PaireJetons` n'a volontairement pas de `Debug`, et
// aucun jeton ne doit apparaître.
impl fmt::Debug for ConnexionProfessionnelle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConnexionProfessionnelle::Complete(_) => f.write_str("Complete(***)"),
            ConnexionProfessionnelle::SecondFacteur { jeton_intermediaire } => f
                .debug_struct("SecondFacteur")
                .field("jeton_intermediaire", jeton_intermediaire)
                .finish(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ErreurAuthProfessionnel {
    /// Email inconnu, mauvais mot de passe, compte verrouillé ou désactivé :
    /// jamais distingués (décisions P1 et L1), même temps de réponse.
    #[error("Email ou mot de passe incorrect.")]
    IdentifiantsInvalides,

    /// Mot de passe correct, email jamais vérifié (R0, P3) : seul le
    /// détenteur du mot de passe l'apprend.
    #[error("Votre email n'est pas encore vérifié. Ouvrez le lien que nous vous avons envoyé, ou demandez-en un nouveau.")]
    EmailNonVerifie,

    /// Seulement la nature de l'échec, un texte fixe : jamais l'email ni le
    /// mot de passe saisis, garanti par le type (`&'static str`, aucun
    /// texte construit à partir des données).
    #[error("erreur interne : {0}")]
    Interne(&'static str),
}

/// Nature d'une erreur SQLx, sans son message : un message de PostgreSQL
/// peut reprendre une valeur. Seule cette nature va aux journaux.
fn nature_sqlx(erreur: &sqlx::Error) -> String {
    match erreur {
        sqlx::Error::Database(e) => format!("base de données, code {}", e.code().as_deref().unwrap_or("inconnu")),
        sqlx::Error::PoolTimedOut => "délai d'attente de la base dépassé".to_string(),
        sqlx::Error::PoolClosed => "connexions à la base fermées".to_string(),
        sqlx::Error::Io(_) => "entrée-sortie avec la base".to_string(),
        _ => "autre erreur de la base".to_string(),
    }
}

fn interne_sqlx(etape: &'static str, erreur: &sqlx::Error) -> ErreurAuthProfessionnel {
    tracing::error!(etape, nature = %nature_sqlx(erreur), "connexion professionnelle : erreur interne");
    ErreurAuthProfessionnel::Interne(etape)
}

/// Erreur dont le détail n'est pas journalisé : il peut venir d'un module
/// qui ne garantit pas l'absence de données saisies.
fn interne(etape: &'static str) -> ErreurAuthProfessionnel {
    tracing::error!(etape, "connexion professionnelle : erreur interne");
    ErreurAuthProfessionnel::Interne(etape)
}

/// Connexion d'un professionnel (`POST /api/professionnels/connexion`).
pub async fn se_connecter(
    pool: &PgPool,
    jwt: &JwtService,
    email: &str,
    mot_de_passe: &str,
) -> Result<ConnexionProfessionnelle, ErreurAuthProfessionnel> {
    connexion_par_email(pool, jwt, "professionnel", SorteJeton::SecondFacteurProfessionnel, email, mot_de_passe).await
}

/// Logique commune (P6). `type_compte` : valeur de `compte.type_compte` ;
/// `sorte_intermediaire` : jeton remis si un TOTP est actif.
async fn connexion_par_email(
    pool: &PgPool,
    jwt: &JwtService,
    type_compte: &'static str,
    sorte_intermediaire: SorteJeton,
    email: &str,
    mot_de_passe: &str,
) -> Result<ConnexionProfessionnelle, ErreurAuthProfessionnel> {
    // Email comparé sans les espaces autour de la saisie et sans tenir
    // compte des majuscules (J6). Au plus une ligne : un seul compte par
    // type et par identité (migration 0011, `un_compte_par_type`), une
    // seule identité par email (migration 0001, index unique sur
    // `lower(email)`). Les emails enregistrés n'ont pas d'espaces autour :
    // la base l'imposera par `CHECK (email = btrim(email))` dans la
    // migration de l'étape 3.7, où naissent les comptes professionnels
    // (décision du 2026-10-15) ; aucun email existant ne la viole.
    let compte: Option<(Uuid, bool, i32)> = sqlx::query_as(
        "SELECT c.id, u.email_verifie_le IS NOT NULL, c.version_jeton \
         FROM compte c JOIN utilisateur u ON u.id = c.utilisateur_id \
         WHERE lower(u.email) = lower(btrim($1)) AND c.type_compte = $2",
    )
    .bind(email)
    .bind(type_compte)
    .fetch_optional(pool)
    .await
    .map_err(|e| interne_sqlx("recherche du compte", &e))?;

    // Verrouillage BF-01, faux hachage si le compte n'existe pas (même
    // temps de réponse), compteur remis à zéro seulement sans TOTP actif
    // (règle V1) : tout est fait par `verrouillage::verifier`.
    let verification = verrouillage::verifier(pool, compte.map(|(id, _, _)| id), mot_de_passe)
        .await
        .map_err(|erreur| match erreur {
            ErreurConnexion::IdentifiantsInvalides => ErreurAuthProfessionnel::IdentifiantsInvalides,
            ErreurConnexion::Interne(_) => interne("vérification du mot de passe"),
        })?;

    // `verifier` n'accepte qu'un compte existant : `compte` est présent.
    let (compte_id, email_verifie, version_jeton) = compte.ok_or_else(|| interne("compte vérifié introuvable"))?;

    // R0, P4 : contrôlé avant de délivrer un jeton intermédiaire.
    if !email_verifie {
        return Err(ErreurAuthProfessionnel::EmailNonVerifie);
    }

    match verification {
        Verification::SecondFacteurRequis(_) => {
            let jeton = jwt
                .emettre(sorte_intermediaire, compte_id, version_jeton, None, None)
                .map_err(|_| interne("émission du jeton intermédiaire"))?;
            Ok(ConnexionProfessionnelle::SecondFacteur { jeton_intermediaire: JetonIntermediaire(jeton) })
        }
        Verification::Complete(_) => {
            let paire = session::ouvrir(pool, compte_id, None)
                .await
                .and_then(|active| active.en_paire(jwt))
                .map_err(|_| interne("ouverture de session"))?;
            Ok(ConnexionProfessionnelle::Complete(paire))
        }
    }
}

// ---------------------------------------------------------------------
// Second temps de la connexion (étape 3.6.3, décisions F1 à F7)
// ---------------------------------------------------------------------

/// Ce que la personne saisit au second temps : un code de son application
/// d'authentification, ou un code de secours. `Debug` affiche `***` : un
/// code est un secret, même valable 30 secondes.
pub enum SaisieSecondFacteur {
    Code(String),
    CodeSecours(String),
}

impl fmt::Debug for SaisieSecondFacteur {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SaisieSecondFacteur::Code(_) => f.write_str("Code(***)"),
            SaisieSecondFacteur::CodeSecours(_) => f.write_str("CodeSecours(***)"),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ErreurSecondFacteur {
    /// Code faux, code de secours faux ou déjà utilisé, compte verrouillé,
    /// TOTP retiré entre-temps : une seule réponse (F3, même logique que
    /// L1) ; distinguer « verrouillé » renseignerait celui qui essaie des
    /// codes.
    #[error("Code incorrect.")]
    CodeInvalide,
    /// Le jeton intermédiaire a déjà servi (F5 b) : une autre requête l'a
    /// consommé. Aucune session n'est ouverte.
    #[error("jeton intermédiaire déjà utilisé")]
    JetonDejaUtilise,
    /// Seulement l'étape en échec, un texte fixe : jamais le code saisi.
    #[error("erreur interne : {0}")]
    Interne(&'static str),
}

/// Second temps de la connexion, après un jeton intermédiaire valide (que
/// l'extracteur a déjà contrôlé : sorte, compte actif, `version_jeton`,
/// email vérifié, `iat` plus récent que le dernier jeton consommé).
///
/// Ordre (décision du 2026-10-15) : 1. le code, ce qui compte l'essai dans
/// le verrouillage du compte et consomme le pas TOTP ou le code de secours
/// (`totp.rs`) ; 2. le jeton, consommé en une seule requête conditionnelle ;
/// 3. la session. Un code faux ne consomme pas le jeton : la personne peut
/// retaper son code dans les 5 minutes, dans la limite du verrouillage.
///
/// Écrite sans dépendre du type de compte (F7) : la session prend le type
/// du compte, et l'administrateur la réutilisera (étape 3.6.4).
pub async fn terminer_connexion(
    pool: &PgPool,
    jwt: &JwtService,
    cle: &CleTotp,
    compte_id: Uuid,
    iat: u64,
    saisie: &SaisieSecondFacteur,
) -> Result<PaireJetons, ErreurSecondFacteur> {
    let verification = match saisie {
        SaisieSecondFacteur::Code(code) => totp::verifier_code(pool, cle.octets(), compte_id, code).await,
        SaisieSecondFacteur::CodeSecours(code) => totp::verifier_code_secours(pool, compte_id, code).await,
    };
    verification.map_err(|erreur| match erreur {
        ErreurTotp::Refuse => ErreurSecondFacteur::CodeInvalide,
        // Le détail de `totp.rs` n'est pas journalisé : il ne garantit pas
        // l'absence de la saisie.
        ErreurTotp::Interne(_) => {
            tracing::error!(etape = "vérification du code", "second facteur : erreur interne");
            ErreurSecondFacteur::Interne("vérification du code")
        }
    })?;

    // Consommation du jeton (F5 b, migration 0018) : la colonne ne prend la
    // valeur `iat` que si elle est vide ou plus petite. Le verrou de ligne
    // de l'UPDATE sérialise deux requêtes simultanées avec le même jeton :
    // une seule réussit. On compare le `iat` du jeton à celui d'un autre
    // jeton, tous deux de l'horloge du serveur Rust : jamais à `now()` de
    // PostgreSQL, dont l'horloge peut différer.
    let iat = i64::try_from(iat).map_err(|_| ErreurSecondFacteur::JetonDejaUtilise)?;
    let consomme: Option<Uuid> = sqlx::query_scalar(
        "UPDATE compte SET second_facteur_dernier_iat = $2 \
         WHERE id = $1 AND (second_facteur_dernier_iat IS NULL OR second_facteur_dernier_iat < $2) RETURNING id",
    )
    .bind(compte_id)
    .bind(iat)
    .fetch_optional(pool)
    .await
    .map_err(|e| {
        tracing::error!(etape = "consommation du jeton", nature = %nature_sqlx(&e), "second facteur : erreur interne");
        ErreurSecondFacteur::Interne("consommation du jeton")
    })?;
    if consomme.is_none() {
        return Err(ErreurSecondFacteur::JetonDejaUtilise);
    }

    session::ouvrir(pool, compte_id, None)
        .await
        .and_then(|active| active.en_paire(jwt))
        .map_err(|_| {
            tracing::error!(etape = "ouverture de session", "second facteur : erreur interne");
            ErreurSecondFacteur::Interne("ouverture de session")
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mot_de_passe;

    // Base temporaire par test ; données fictives.

    const BON: &str = "Exemple#2026";
    const MAUVAIS: &str = "Mauvais#2026";

    fn jwt() -> JwtService {
        JwtService::new(&"a".repeat(32))
    }

    async fn identite(pool: &PgPool, telephone: &str, email: &str, verifie: bool) -> Uuid {
        let id: Uuid = sqlx::query_scalar("INSERT INTO utilisateur (telephone, email) VALUES ($1, $2) RETURNING id")
            .bind(telephone)
            .bind(email)
            .fetch_one(pool)
            .await
            .unwrap();
        if verifie {
            sqlx::query("UPDATE utilisateur SET email_verifie_le = now() WHERE id = $1")
                .bind(id)
                .execute(pool)
                .await
                .unwrap();
        }
        id
    }

    async fn compte(pool: &PgPool, utilisateur_id: Uuid, type_compte: &str) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(utilisateur_id)
        .bind(type_compte)
        .bind(mot_de_passe::hacher(BON).unwrap())
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn professionnel(pool: &PgPool, telephone: &str, email: &str, verifie: bool) -> Uuid {
        let id = identite(pool, telephone, email, verifie).await;
        compte(pool, id, "professionnel").await
    }

    async fn tentatives(pool: &PgPool, compte_id: Uuid) -> i16 {
        sqlx::query_scalar("SELECT tentatives_echouees FROM compte WHERE id = $1")
            .bind(compte_id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /// TOTP actif posé en SQL : le contenu n'est jamais déchiffré ici.
    async fn activer_totp(pool: &PgPool, compte_id: Uuid) {
        sqlx::query(
            "INSERT INTO second_facteur_totp (compte_id, secret_chiffre, nonce, version_cle, statut, date_activation) \
             VALUES ($1, decode(repeat('00', 36), 'hex'), decode(repeat('00', 12), 'hex'), 1, 'actif', now())",
        )
        .bind(compte_id)
        .execute(pool)
        .await
        .unwrap();
    }

    fn refuse(resultat: Result<ConnexionProfessionnelle, ErreurAuthProfessionnel>) -> bool {
        matches!(resultat, Err(ErreurAuthProfessionnel::IdentifiantsInvalides))
    }

    #[sqlx::test]
    #[ignore]
    async fn refus_identiques(pool: PgPool) {
        let jwt = jwt();
        let compte_id = professionnel(&pool, "+22670050001", "pro.refus@example.org", true).await;

        assert!(refuse(se_connecter(&pool, &jwt, "inconnu@example.org", BON).await), "email inconnu");
        assert!(refuse(se_connecter(&pool, &jwt, "pro.refus@example.org", MAUVAIS).await), "mauvais mot de passe");

        sqlx::query("UPDATE compte SET statut = 'desactive' WHERE id = $1").bind(compte_id).execute(&pool).await.unwrap();
        assert!(refuse(se_connecter(&pool, &jwt, "pro.refus@example.org", BON).await), "compte désactivé");
        sqlx::query("UPDATE compte SET statut = 'actif' WHERE id = $1").bind(compte_id).execute(&pool).await.unwrap();

        for _ in 0..5 {
            assert!(refuse(se_connecter(&pool, &jwt, "pro.refus@example.org", MAUVAIS).await));
        }
        assert!(refuse(se_connecter(&pool, &jwt, "pro.refus@example.org", BON).await), "compte verrouillé");
        assert_eq!(ErreurAuthProfessionnel::IdentifiantsInvalides.to_string(), "Email ou mot de passe incorrect.");
    }

    #[sqlx::test]
    #[ignore]
    async fn compte_patient_seul_refuse(pool: PgPool) {
        let id = identite(&pool, "+22670050002", "patient.seul@example.org", true).await;
        compte(&pool, id, "patient").await;
        assert!(refuse(se_connecter(&pool, &jwt(), "patient.seul@example.org", BON).await));
    }

    #[sqlx::test]
    #[ignore]
    async fn connexion_complete_et_email_normalise(pool: PgPool) {
        let jwt = jwt();
        let compte_id = professionnel(&pool, "+22670050003", "pro.complet@example.org", true).await;

        let connexion = se_connecter(&pool, &jwt, "  Pro.Complet@Example.ORG ", BON).await.unwrap();
        let ConnexionProfessionnelle::Complete(paire) = connexion else {
            panic!("connexion complète attendue");
        };
        let claims = jwt.verifier(&paire.jeton_acces, SorteJeton::ProfessionnelSansAffectation).unwrap();
        assert_eq!((claims.sub, claims.aff), (compte_id, None));
        assert_eq!(paire.jeton_renouvellement.en_clair().len(), 64);

        // Session de 12 h (professionnel), désignée par le `sid` du jeton.
        let douze_heures: bool = sqlx::query_scalar(
            "SELECT expire_le - creee_le = interval '12 hours' FROM session WHERE id = $1 AND compte_id = $2",
        )
        .bind(claims.sid)
        .bind(compte_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(douze_heures);
    }

    #[sqlx::test]
    #[ignore]
    async fn email_non_verifie_sans_totp_remet_le_compteur_a_zero(pool: PgPool) {
        let jwt = jwt();
        let compte_id = professionnel(&pool, "+22670050004", "pro.nonverifie@example.org", false).await;
        for _ in 0..2 {
            assert!(refuse(se_connecter(&pool, &jwt, "pro.nonverifie@example.org", MAUVAIS).await));
        }
        assert_eq!(tentatives(&pool, compte_id).await, 2);

        assert!(matches!(
            se_connecter(&pool, &jwt, "pro.nonverifie@example.org", BON).await,
            Err(ErreurAuthProfessionnel::EmailNonVerifie)
        ));
        assert_eq!(tentatives(&pool, compte_id).await, 0, "mot de passe correct, pas de TOTP");
        let sessions: i64 = sqlx::query_scalar("SELECT count(*) FROM session WHERE compte_id = $1")
            .bind(compte_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(sessions, 0, "aucune session pour un email non vérifié");
    }

    #[sqlx::test]
    #[ignore]
    async fn email_non_verifie_avec_totp_ne_remet_pas_le_compteur_a_zero(pool: PgPool) {
        let compte_id = professionnel(&pool, "+22670050005", "pro.totp.nonverifie@example.org", false).await;
        activer_totp(&pool, compte_id).await;
        assert!(matches!(
            se_connecter(&pool, &jwt(), "pro.totp.nonverifie@example.org", BON).await,
            Err(ErreurAuthProfessionnel::EmailNonVerifie)
        ));
        assert_eq!(tentatives(&pool, compte_id).await, 1, "règle V1 : pas de remise à zéro avec un TOTP actif");
    }

    #[sqlx::test]
    #[ignore]
    async fn totp_actif_jeton_intermediaire_sans_session(pool: PgPool) {
        let jwt = jwt();
        let compte_id = professionnel(&pool, "+22670050006", "pro.totp@example.org", true).await;
        activer_totp(&pool, compte_id).await;

        let connexion = se_connecter(&pool, &jwt, "pro.totp@example.org", BON).await.unwrap();
        let affichage = format!("{connexion:?}");
        let ConnexionProfessionnelle::SecondFacteur { jeton_intermediaire } = &connexion else {
            panic!("second facteur attendu");
        };
        let claims = jwt.verifier(jeton_intermediaire.en_clair(), SorteJeton::SecondFacteurProfessionnel).unwrap();
        assert_eq!((claims.sub, claims.sid, claims.aff), (compte_id, None, None));
        assert_eq!(claims.exp - claims.iat, 5 * 60);

        // Le jeton n'apparaît jamais dans `Debug`.
        assert!(!affichage.contains(jeton_intermediaire.en_clair()), "{affichage}");
        assert!(affichage.contains("***"), "{affichage}");
        assert_eq!(format!("{jeton_intermediaire:?}"), "***");

        let sessions: i64 = sqlx::query_scalar("SELECT count(*) FROM session WHERE compte_id = $1")
            .bind(compte_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(sessions, 0, "aucune session avant le code TOTP");
    }

    /// Point 2 du porteur : une erreur interne provoquée (connexions à la
    /// base fermées) ne contient ni l'email ni le mot de passe saisis.
    #[sqlx::test]
    #[ignore]
    async fn erreur_interne_sans_donnee_saisie(pool: PgPool) {
        let email = "secret.saisi@example.org";
        let mot_de_passe = "MotDePasse#Secret1";
        pool.close().await;

        let erreur = se_connecter(&pool, &jwt(), email, mot_de_passe).await.unwrap_err();
        assert!(matches!(erreur, ErreurAuthProfessionnel::Interne(_)), "{erreur:?}");
        for texte in [erreur.to_string(), format!("{erreur:?}")] {
            assert!(!texte.contains(email) && !texte.contains("secret.saisi"), "{texte}");
            assert!(!texte.contains(mot_de_passe), "{texte}");
        }
    }

    // --- Second temps (étape 3.6.3) ---------------------------------

    fn cle() -> CleTotp {
        CleTotp::depuis_octets([7; 32])
    }

    fn maintenant() -> u64 {
        jsonwebtoken::get_current_timestamp()
    }

    /// Professionnel avec un vrai TOTP actif : activation par `totp.rs`,
    /// confirmée avec le code de l'instant présent. Renvoie le compte, un
    /// code valable pour le pas suivant (accepté grâce à la tolérance d'un
    /// pas, Z3, et jamais utilisé), et les codes de secours.
    async fn professionnel_avec_totp(pool: &PgPool, telephone: &str, email: &str) -> (Uuid, String, Vec<String>) {
        let compte_id = professionnel(pool, telephone, email, true).await;
        let activation = totp::commencer_activation(pool, cle().octets(), compte_id, BON).await.unwrap();
        let generateur = totp_rs::Builder::new()
            .with_secret(totp_rs::Secret::try_from_base32(&activation.cle_base32).unwrap())
            .build()
            .unwrap();
        let codes_secours = totp::confirmer_activation(pool, cle().octets(), compte_id, &generateur.generate(maintenant()).to_string())
            .await
            .unwrap();
        let code_suivant = generateur.generate(maintenant() + 30).to_string();
        (compte_id, code_suivant, codes_secours)
    }

    /// Jeton intermédiaire réel, obtenu par la connexion : renvoie son `iat`.
    async fn iat_du_jeton_intermediaire(pool: &PgPool, jwt: &JwtService, email: &str) -> u64 {
        let ConnexionProfessionnelle::SecondFacteur { jeton_intermediaire } =
            se_connecter(pool, jwt, email, BON).await.unwrap()
        else {
            panic!("second facteur attendu");
        };
        jwt.verifier(jeton_intermediaire.en_clair(), SorteJeton::SecondFacteurProfessionnel).unwrap().iat
    }

    async fn dernier_iat(pool: &PgPool, compte_id: Uuid) -> Option<i64> {
        sqlx::query_scalar("SELECT second_facteur_dernier_iat FROM compte WHERE id = $1")
            .bind(compte_id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    fn code_faux(code: &str) -> String {
        if code == "000000" { "000001".into() } else { "000000".into() }
    }

    #[sqlx::test]
    #[ignore]
    async fn bon_code_totp_ouvre_la_session(pool: PgPool) {
        let jwt = jwt();
        let (compte_id, code, _) = professionnel_avec_totp(&pool, "+22670090001", "sf.totp@example.org").await;
        let iat = iat_du_jeton_intermediaire(&pool, &jwt, "sf.totp@example.org").await;

        let paire = terminer_connexion(&pool, &jwt, &cle(), compte_id, iat, &SaisieSecondFacteur::Code(code))
            .await
            .unwrap();
        let claims = jwt.verifier(&paire.jeton_acces, SorteJeton::ProfessionnelSansAffectation).unwrap();
        assert_eq!(claims.sub, compte_id);
        assert_eq!(tentatives(&pool, compte_id).await, 0, "compteur remis à zéro après le succès");
        assert_eq!(dernier_iat(&pool, compte_id).await, Some(iat as i64), "jeton consommé");
    }

    /// Usage unique (F5 b) : après un succès, le même jeton (même `iat`)
    /// est refusé, même avec un code neuf ; un jeton émis plus tard (`iat`
    /// plus grand) est accepté. Les `iat` sont donnés directement : deux
    /// connexions réelles dans la même seconde auraient le même.
    #[sqlx::test]
    #[ignore]
    async fn jeton_a_usage_unique_et_jeton_plus_recent_accepte(pool: PgPool) {
        let jwt = jwt();
        let (compte_id, code, secours) = professionnel_avec_totp(&pool, "+22670090002", "sf.unique@example.org").await;
        let iat = iat_du_jeton_intermediaire(&pool, &jwt, "sf.unique@example.org").await;

        terminer_connexion(&pool, &jwt, &cle(), compte_id, iat, &SaisieSecondFacteur::Code(code)).await.unwrap();
        assert!(matches!(
            terminer_connexion(&pool, &jwt, &cle(), compte_id, iat, &SaisieSecondFacteur::CodeSecours(secours[0].clone())).await,
            Err(ErreurSecondFacteur::JetonDejaUtilise)
        ));

        terminer_connexion(&pool, &jwt, &cle(), compte_id, iat + 1, &SaisieSecondFacteur::CodeSecours(secours[1].clone()))
            .await
            .unwrap();
        assert_eq!(dernier_iat(&pool, compte_id).await, Some(iat as i64 + 1));
    }

    #[sqlx::test]
    #[ignore]
    async fn code_de_secours_ouvre_la_session_une_seule_fois(pool: PgPool) {
        let jwt = jwt();
        let (compte_id, _, secours) = professionnel_avec_totp(&pool, "+22670090003", "sf.secours@example.org").await;
        let iat = iat_du_jeton_intermediaire(&pool, &jwt, "sf.secours@example.org").await;
        let saisie = SaisieSecondFacteur::CodeSecours(secours[0].clone());

        terminer_connexion(&pool, &jwt, &cle(), compte_id, iat, &saisie).await.unwrap();
        // Le même code de secours, avec un jeton plus récent : refusé.
        assert!(matches!(
            terminer_connexion(&pool, &jwt, &cle(), compte_id, iat + 1, &saisie).await,
            Err(ErreurSecondFacteur::CodeInvalide)
        ));
    }

    /// Un code faux ne consomme pas le jeton : le bon code passe ensuite
    /// avec le même jeton.
    #[sqlx::test]
    #[ignore]
    async fn code_faux_compte_l_essai_sans_consommer_le_jeton(pool: PgPool) {
        let jwt = jwt();
        let (compte_id, code, _) = professionnel_avec_totp(&pool, "+22670090004", "sf.faux@example.org").await;
        let iat = iat_du_jeton_intermediaire(&pool, &jwt, "sf.faux@example.org").await;
        let avant = tentatives(&pool, compte_id).await;

        assert!(matches!(
            terminer_connexion(&pool, &jwt, &cle(), compte_id, iat, &SaisieSecondFacteur::Code(code_faux(&code))).await,
            Err(ErreurSecondFacteur::CodeInvalide)
        ));
        assert!(matches!(
            terminer_connexion(&pool, &jwt, &cle(), compte_id, iat, &SaisieSecondFacteur::CodeSecours("AAAAA-AAAAA".into()))
                .await,
            Err(ErreurSecondFacteur::CodeInvalide)
        ));
        assert_eq!(tentatives(&pool, compte_id).await, avant + 2, "chaque essai compte");
        assert_eq!(dernier_iat(&pool, compte_id).await, None, "jeton pas consommé");

        terminer_connexion(&pool, &jwt, &cle(), compte_id, iat, &SaisieSecondFacteur::Code(code)).await.unwrap();
    }

    /// Mots de passe et codes faux mêlés comptent dans le même verrouillage
    /// (BF-01, 5 essais). Obtenir le jeton intermédiaire compte aussi : le
    /// mot de passe correct ne remet pas le compteur à zéro avec un TOTP
    /// actif (V1). 2 mots de passe faux + 1 correct + 1 code faux + 1 code
    /// de secours faux = 5 : le compte est verrouillé.
    #[sqlx::test]
    #[ignore]
    async fn mots_de_passe_et_codes_faux_meles_verrouillent(pool: PgPool) {
        let jwt = jwt();
        let (compte_id, code, secours) = professionnel_avec_totp(&pool, "+22670090005", "sf.meles@example.org").await;
        for _ in 0..2 {
            assert!(refuse(se_connecter(&pool, &jwt, "sf.meles@example.org", MAUVAIS).await));
        }
        let iat = iat_du_jeton_intermediaire(&pool, &jwt, "sf.meles@example.org").await;
        assert!(matches!(
            terminer_connexion(&pool, &jwt, &cle(), compte_id, iat, &SaisieSecondFacteur::Code(code_faux(&code))).await,
            Err(ErreurSecondFacteur::CodeInvalide)
        ));
        assert!(matches!(
            terminer_connexion(&pool, &jwt, &cle(), compte_id, iat, &SaisieSecondFacteur::CodeSecours("AAAAA-AAAAA".into()))
                .await,
            Err(ErreurSecondFacteur::CodeInvalide)
        ));

        // Verrouillé : le bon mot de passe et le bon code sont refusés.
        assert!(refuse(se_connecter(&pool, &jwt, "sf.meles@example.org", BON).await));
        assert!(matches!(
            terminer_connexion(&pool, &jwt, &cle(), compte_id, iat, &SaisieSecondFacteur::Code(code)).await,
            Err(ErreurSecondFacteur::CodeInvalide)
        ));
        assert!(matches!(
            terminer_connexion(&pool, &jwt, &cle(), compte_id, iat, &SaisieSecondFacteur::CodeSecours(secours[0].clone()))
                .await,
            Err(ErreurSecondFacteur::CodeInvalide)
        ));
        let verrouille: bool =
            sqlx::query_scalar("SELECT verrouille_jusqua IS NOT NULL AND verrouille_jusqua > now() FROM compte WHERE id = $1")
                .bind(compte_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(verrouille);
    }

    /// Deux requêtes simultanées avec le même jeton et deux codes de
    /// secours valides : une seule ouvre une session.
    #[sqlx::test]
    #[ignore]
    async fn deux_requetes_simultanees_une_seule_reussit(pool: PgPool) {
        let jwt = jwt();
        let (compte_id, _, secours) = professionnel_avec_totp(&pool, "+22670090006", "sf.simultane@example.org").await;
        let iat = iat_du_jeton_intermediaire(&pool, &jwt, "sf.simultane@example.org").await;

        let mut taches = tokio::task::JoinSet::new();
        for code in [secours[0].clone(), secours[1].clone()] {
            let (pool, jwt) = (pool.clone(), jwt.clone());
            taches.spawn(async move {
                terminer_connexion(&pool, &jwt, &cle(), compte_id, iat, &SaisieSecondFacteur::CodeSecours(code))
                    .await
                    .is_ok()
            });
        }
        let mut reussites = 0;
        while let Some(resultat) = taches.join_next().await {
            if resultat.unwrap() {
                reussites += 1;
            }
        }
        assert_eq!(reussites, 1);
        let sessions: i64 = sqlx::query_scalar("SELECT count(*) FROM session WHERE compte_id = $1")
            .bind(compte_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(sessions, 1);
    }

    /// Une erreur interne provoquée ne contient jamais le code saisi.
    #[sqlx::test]
    #[ignore]
    async fn erreur_interne_du_second_facteur_sans_code_saisi(pool: PgPool) {
        pool.close().await;
        let code = "ABCDE-FGHIJ";
        // `PaireJetons` n'a pas de `Debug` (volontairement) : pas de
        // `unwrap_err`.
        let Err(erreur) =
            terminer_connexion(&pool, &jwt(), &cle(), Uuid::from_u128(1), 10, &SaisieSecondFacteur::CodeSecours(code.into()))
                .await
        else {
            panic!("erreur attendue");
        };
        assert!(matches!(erreur, ErreurSecondFacteur::Interne(_)), "{erreur:?}");
        for texte in [erreur.to_string(), format!("{erreur:?}")] {
            assert!(!texte.contains(code) && !texte.contains("ABCDE"), "{texte}");
        }
        assert_eq!(format!("{:?}", SaisieSecondFacteur::CodeSecours(code.into())), "CodeSecours(***)");
    }
}
