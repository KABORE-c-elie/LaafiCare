//! Code de soin du patient (module médecin, étape 3 ; décision du porteur
//! du 2026-11-17 ; migration 0037).
//!
//! Le patient donne ce code au soignant pour ouvrir une prise en charge
//! (moyen préféré du modèle d'accès MD1). C'est un **TOTP à 6 chiffres**
//! (RFC 6238), avec un **secret propre à chaque patient**, que le téléphone
//! calcule **sans réseau**. Mêmes paramètres et même protection que le
//! second facteur des administrateurs (`totp.rs`, décisions Z1 à Z4) :
//! SHA-1, 6 chiffres, pas de 30 s, tolérance d'un pas ; secret de 160 bits
//! tiré par la crate ; chiffré en AES-256-GCM avec la clé
//! `TOTP_CLE_CHIFFREMENT`, nonce aléatoire, version de clé ; un code ne sert
//! qu'une fois (RFC 6238 : « MUST NOT accept the second attempt »).
//!
//! Donnée associée du chiffrement : une étiquette propre au code de soin,
//! puis l'identifiant du compte patient. Un secret recopié depuis la table
//! du second facteur, ou depuis le compte d'un autre patient, ne se
//! déchiffre pas.
//!
//! Limite des essais : faite par l'appelant (`prise_en_charge::ouvrir`),
//! par affectation et par patient (5 en 15 minutes, comme BF-01).
//!
//! L'écran du téléphone viendra plus tard (décision du porteur) : seul le
//! serveur est écrit ici.

use sqlx::types::Uuid;
use sqlx::{PgPool, Postgres, Transaction};

use crate::totp::{self, ErreurTotp};
use crate::verrouillage::{self, ErreurConnexion};

const EMETTEUR: &str = "LaafiCare";
/// Nom affiché dans une application d'authentification : aucune donnée
/// personnelle (Z8).
const NOM_COMPTE: &str = "Code de soin";
/// Étiquette de la donnée associée (séparation des usages de la clé).
const ETIQUETTE_AAD: &[u8] = b"laaficare:code_soin:";

fn aad(compte_id: Uuid) -> Vec<u8> {
    [ETIQUETTE_AAD, compte_id.as_bytes()].concat()
}

/// Ce que l'application du patient garde pour calculer ses codes. Montré
/// une seule fois ; la réponse HTTP porte `Cache-Control: no-store`.
#[derive(Debug, serde::Serialize)]
pub struct CodeSoinCree {
    pub cle_base32: String,
    pub url_otpauth: String,
    pub algorithme: &'static str,
    pub chiffres: u32,
    pub periode_secondes: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum ErreurCodeSoin {
    /// Mot de passe faux, compte verrouillé ou désactivé : jamais
    /// distingués (P1, L1).
    #[error("Mot de passe incorrect.")]
    MotDePasseIncorrect,
    #[error("erreur interne : {0}")]
    Interne(String),
}

impl From<sqlx::Error> for ErreurCodeSoin {
    fn from(erreur: sqlx::Error) -> Self {
        ErreurCodeSoin::Interne(erreur.to_string())
    }
}

impl From<ErreurTotp> for ErreurCodeSoin {
    fn from(erreur: ErreurTotp) -> Self {
        ErreurCodeSoin::Interne(erreur.to_string())
    }
}

/// Crée ou remplace le secret du code de soin du compte patient. Le mot de
/// passe est exigé et compté dans le verrouillage (même règle que
/// l'activation du second facteur, Z7) : une session volée ne suffit pas à
/// obtenir le secret. L'ancien secret est supprimé : ses codes ne servent
/// plus.
pub async fn generer(
    pool: &PgPool,
    cle: &[u8; 32],
    compte_id: Uuid,
    mot_de_passe: &str,
) -> Result<CodeSoinCree, ErreurCodeSoin> {
    verrouillage::verifier(pool, Some(compte_id), mot_de_passe)
        .await
        .map_err(|erreur| match erreur {
            ErreurConnexion::IdentifiantsInvalides => ErreurCodeSoin::MotDePasseIncorrect,
            ErreurConnexion::Interne(detail) => ErreurCodeSoin::Interne(detail),
        })?;

    // Sans `with_secret`, `build` génère un secret de 160 bits
    // (fonctionnalité `gen_secret` de totp-rs, taille de la RFC 4226).
    let code = totp::parametres()
        .with_issuer(Some(EMETTEUR))
        .with_account_name(NOM_COMPTE)
        .build()
        .map_err(|e| ErreurCodeSoin::Interne(e.to_string()))?;
    let (secret_chiffre, nonce) =
        totp::chiffrer_avec(cle, &aad(compte_id), code.secret().as_bytes())?;

    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM code_soin_patient WHERE compte_id = $1")
        .bind(compte_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO code_soin_patient (compte_id, secret_chiffre, nonce, version_cle) VALUES ($1, $2, $3, $4)",
    )
    .bind(compte_id)
    .bind(&secret_chiffre)
    .bind(&nonce)
    .bind(totp::VERSION_CLE)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    Ok(CodeSoinCree {
        url_otpauth: code
            .to_url()
            .map_err(|e| ErreurCodeSoin::Interne(e.to_string()))?,
        cle_base32: code.secret().to_base32(),
        algorithme: "SHA1",
        chiffres: 6,
        periode_secondes: 30,
    })
}

/// Vérifie le code de soin du patient, dans la transaction de l'appelant
/// (verrou sur la ligne du code : deux vérifications simultanées du même
/// code n'en acceptent qu'une). `Ok(false)` : code faux, déjà utilisé, mal
/// formé, ou patient sans code de soin.
pub(crate) async fn verifier(
    tx: &mut Transaction<'_, Postgres>,
    cle: &[u8; 32],
    patient_id: Uuid,
    code: &str,
) -> Result<bool, ErreurTotp> {
    verifier_a(tx, cle, patient_id, code, totp::maintenant()?).await
}

async fn verifier_a(
    tx: &mut Transaction<'_, Postgres>,
    cle: &[u8; 32],
    patient_id: Uuid,
    code: &str,
    instant: u64,
) -> Result<bool, ErreurTotp> {
    let code = code.trim();
    if code.len() != 6 || !code.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(false);
    }
    let ligne: Option<(Uuid, Vec<u8>, Vec<u8>, i16)> = sqlx::query_as(
        "SELECT cs.compte_id, cs.secret_chiffre, cs.nonce, cs.version_cle \
         FROM code_soin_patient cs JOIN compte c ON c.id = cs.compte_id \
         JOIN patient p ON p.utilisateur_id = c.utilisateur_id \
         WHERE p.id = $1 FOR UPDATE OF cs",
    )
    .bind(patient_id)
    .fetch_optional(&mut **tx)
    .await?;
    let Some((compte_id, secret_chiffre, nonce, version_cle)) = ligne else {
        return Ok(false);
    };
    totp::verifier_version_cle(version_cle)?;
    let secret = totp::dechiffrer_avec(cle, &aad(compte_id), &secret_chiffre, &nonce)?;
    let pas = match totp::pas_valide(&totp::totp_depuis(secret)?, code, instant) {
        Ok(pas) => pas,
        Err(ErreurTotp::Refuse) => return Ok(false),
        Err(autre) => return Err(autre),
    };
    let accepte = sqlx::query(
        "UPDATE code_soin_patient SET dernier_pas = $2 \
         WHERE compte_id = $1 AND (dernier_pas IS NULL OR dernier_pas < $2)",
    )
    .bind(compte_id)
    .bind(pas)
    .execute(&mut **tx)
    .await?
    .rows_affected()
        == 1;
    Ok(accepte)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::fixtures_medicales::patient;

    pub(crate) const CLE: [u8; 32] = [9; 32];

    /// Donne un mot de passe connu au compte patient de test.
    pub(crate) async fn mot_de_passe(pool: &PgPool, compte_id: Uuid) {
        let hachage = crate::mot_de_passe::hacher_async("Soin-2026!")
            .await
            .unwrap();
        sqlx::query("UPDATE compte SET mot_de_passe_hash = $2 WHERE id = $1")
            .bind(compte_id)
            .bind(hachage.as_str())
            .execute(pool)
            .await
            .unwrap();
    }

    /// Le code que le téléphone calculerait maintenant.
    pub(crate) fn code_actuel(cree: &CodeSoinCree) -> String {
        let secret = totp_rs::Secret::try_from_base32(&cree.cle_base32).unwrap();
        totp::parametres()
            .with_secret(secret)
            .build()
            .unwrap()
            .generate_current()
            .to_string()
    }

    /// Création avec le mot de passe ; code juste accepté une seule fois ;
    /// code faux, mal formé ; remplacement : l'ancien secret ne sert plus.
    #[sqlx::test]
    #[ignore]
    async fn code_de_soin(pool: PgPool) {
        let p = patient(&pool, "+22670380001").await;
        mot_de_passe(&pool, p.compte_id).await;
        assert!(matches!(
            generer(&pool, &CLE, p.compte_id, "faux").await,
            Err(ErreurCodeSoin::MotDePasseIncorrect)
        ));
        let cree = generer(&pool, &CLE, p.compte_id, "Soin-2026!")
            .await
            .unwrap();
        assert!(!cree.url_otpauth.contains("OUEDRAOGO"));
        let code = code_actuel(&cree);

        let mut tx = pool.begin().await.unwrap();
        assert!(
            !verifier(&mut tx, &CLE, p.patient_id, "12345")
                .await
                .unwrap()
        );
        assert!(verifier(&mut tx, &CLE, p.patient_id, &code).await.unwrap());
        assert!(
            !verifier(&mut tx, &CLE, p.patient_id, &code).await.unwrap(),
            "un code ne sert qu'une fois"
        );
        tx.commit().await.unwrap();

        // Le secret est chiffré : il n'est jamais en clair en base.
        let chiffre: Vec<u8> =
            sqlx::query_scalar("SELECT secret_chiffre FROM code_soin_patient WHERE compte_id = $1")
                .bind(p.compte_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(chiffre.len(), 36);

        // Remplacé : l'ancien secret ne donne plus de code valable.
        let nouveau = generer(&pool, &CLE, p.compte_id, "Soin-2026!")
            .await
            .unwrap();
        assert_ne!(nouveau.cle_base32, cree.cle_base32);
        let mut tx = pool.begin().await.unwrap();
        let ancien_code = code_actuel(&cree);
        let nouveau_code = code_actuel(&nouveau);
        if ancien_code != nouveau_code {
            assert!(
                !verifier(&mut tx, &CLE, p.patient_id, &ancien_code)
                    .await
                    .unwrap()
            );
        }
        // Une autre clé de chiffrement : le secret ne se déchiffre pas.
        assert!(
            verifier(&mut tx, &[1; 32], p.patient_id, &nouveau_code)
                .await
                .is_err()
        );
    }
}
