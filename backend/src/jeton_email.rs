//! Jetons envoyés par email (étape 3.6.5, décisions B1 et B3, migration
//! 0020) : vérification de l'adresse d'un compte professionnel ou
//! administrateur (R0). La réinitialisation du mot de passe (3.12)
//! réutilisera la table avec son propre objet.
//!
//! Même forme que le jeton de renouvellement (`session.rs`) : 32 octets
//! aléatoires en hexadécimal, seule l'empreinte SHA-256 en base (au-delà de
//! 112 bits, NIST SP 800-63B rév. 4 §3.1.2 admet un hachage simple). Le
//! jeton est lié à l'adresse à laquelle il a été envoyé : il ne vérifie
//! qu'elle.

use std::fmt;

use sqlx::types::Uuid;
use sqlx::{PgConnection, PgPool};
use zeroize::Zeroize;

use crate::session;

/// Durée de validité d'un jeton de vérification de l'email. Source : NIST
/// SP 800-63A rév. 4 §3.8, « 24 hours when sent to a validated email
/// address » au plus (choix A du 2026-10-15).
pub const DUREE_VERIFICATION_HEURES: i32 = 24;

/// Jeton en clair, le temps de l'envoyer. Même traitement que
/// `JetonRenouvellement` : `Debug` affiche `***`, ni `Display` ni
/// `Serialize`, effacé de la mémoire à sa libération.
pub struct JetonEmail(String);

impl JetonEmail {
    pub fn en_clair(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for JetonEmail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

impl Drop for JetonEmail {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// Jeton émis, et l'adresse à laquelle l'envoyer.
#[derive(Debug)]
pub struct VerificationEmise {
    pub jeton: JetonEmail,
    pub email: String,
}

/// Émet un jeton de vérification pour l'email actuel du compte, dans la
/// transaction de l'appelant. `None` si le compte n'a pas d'email. Les
/// jetons de vérification encore inutilisés du compte sont d'abord annulés
/// (décision du porteur, 2026-10-18) ; l'index unique de la migration 0020
/// refuserait sinon le nouveau.
///
/// Le compte est verrouillé en premier (règle de la section 12 : le compte
/// avant le reste), pour que deux émissions simultanées se suivent au lieu
/// de se heurter à l'index unique.
pub async fn emettre_verification(
    conn: &mut PgConnection,
    compte_id: Uuid,
) -> Result<Option<VerificationEmise>, sqlx::Error> {
    sqlx::query("SELECT id FROM compte WHERE id = $1 FOR UPDATE")
        .bind(compte_id)
        .fetch_one(&mut *conn)
        .await?;
    sqlx::query(
        "UPDATE jeton_email SET annule_le = now() \
         WHERE compte_id = $1 AND objet = 'verification_email' AND utilise_le IS NULL AND annule_le IS NULL",
    )
    .bind(compte_id)
    .execute(&mut *conn)
    .await?;

    let (octets, texte) = session::octets_aleatoires_hexadecimaux();
    let jeton = JetonEmail(texte);
    // Expiration calculée par PostgreSQL (une seule horloge, comme la
    // vérification qui compare à `now()`).
    let email: Option<String> = sqlx::query_scalar(
        "INSERT INTO jeton_email (compte_id, compte_type, objet, empreinte, email_vise, expire_le) \
         SELECT c.id, c.type_compte, 'verification_email', $2, u.email, now() + make_interval(hours => $3) \
         FROM compte c JOIN utilisateur u ON u.id = c.utilisateur_id \
         WHERE c.id = $1 AND u.email IS NOT NULL \
         RETURNING email_vise",
    )
    .bind(compte_id)
    .bind(session::empreinte(&octets))
    .bind(DUREE_VERIFICATION_HEURES)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(email.map(|email| VerificationEmise { jeton, email }))
}

/// Utilise un jeton de vérification : en une seule requête, il est marqué
/// utilisé et l'email de l'identité est déclaré vérifié. Renvoie le compte,
/// ou `None` dans tous les cas de refus, sans les distinguer : jeton mal
/// formé, inconnu, déjà utilisé, annulé, expiré, ou email changé depuis
/// l'envoi (l'adresse visée doit être l'email actuel, aux majuscules près).
///
/// Requête unique avec une instruction de modification dans `WITH` (doc
/// PostgreSQL 18, 7.8.4 « Data-Modifying Statements in WITH ») : les deux
/// mises à jour réussissent ou échouent ensemble, et deux utilisations
/// simultanées du même jeton n'en font réussir qu'une (verrou de ligne de
/// l'UPDATE, puis condition `utilise_le IS NULL` réévaluée).
///
/// `compte` : si présent, seul un jeton de ce compte est accepté (les
/// outils locaux savent pour quel compte ils attendent le jeton ; un jeton
/// d'un autre compte reste alors inutilisé). La route de la 3.7 ne reçoit
/// que le jeton et passe `None`.
pub async fn verifier_email(pool: &PgPool, jeton: &str, compte: Option<Uuid>) -> Result<Option<Uuid>, sqlx::Error> {
    let Some(octets) = session::decoder(jeton) else {
        return Ok(None);
    };
    sqlx::query_scalar(
        "WITH jeton AS ( \
             UPDATE jeton_email j SET utilise_le = now() \
             FROM compte c JOIN utilisateur u ON u.id = c.utilisateur_id \
             WHERE j.empreinte = $1 AND j.objet = 'verification_email' \
               AND j.utilise_le IS NULL AND j.annule_le IS NULL AND j.expire_le > now() \
               AND c.id = j.compte_id AND lower(u.email) = lower(j.email_vise) \
               AND ($2::uuid IS NULL OR j.compte_id = $2) \
             RETURNING j.compte_id, u.id AS utilisateur_id \
         ) \
         UPDATE utilisateur SET email_verifie_le = now() \
         FROM jeton WHERE utilisateur.id = jeton.utilisateur_id \
         RETURNING jeton.compte_id",
    )
    .bind(session::empreinte(&octets))
    .bind(compte)
    .fetch_optional(pool)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    // Base temporaire par test ; données fictives.

    async fn compte(pool: &PgPool, telephone: &str, email: &str, type_compte: &str) -> Uuid {
        let utilisateur_id: Uuid =
            sqlx::query_scalar("INSERT INTO utilisateur (telephone, email) VALUES ($1, $2) RETURNING id")
                .bind(telephone)
                .bind(email)
                .fetch_one(pool)
                .await
                .unwrap();
        sqlx::query_scalar(
            "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, $2, 'x') RETURNING id",
        )
        .bind(utilisateur_id)
        .bind(type_compte)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn emettre(pool: &PgPool, compte_id: Uuid) -> VerificationEmise {
        let mut tx = pool.begin().await.unwrap();
        let emise = emettre_verification(&mut tx, compte_id).await.unwrap().unwrap();
        tx.commit().await.unwrap();
        emise
    }

    async fn email_verifie(pool: &PgPool, compte_id: Uuid) -> bool {
        sqlx::query_scalar(
            "SELECT u.email_verifie_le IS NOT NULL FROM compte c JOIN utilisateur u ON u.id = c.utilisateur_id \
             WHERE c.id = $1",
        )
        .bind(compte_id)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[sqlx::test]
    #[ignore]
    async fn verification_a_usage_unique(pool: PgPool) {
        let compte_id = compte(&pool, "+22670130001", "jeton.unique@example.org", "administrateur_laaficare").await;
        let emise = emettre(&pool, compte_id).await;
        assert_eq!(emise.email, "jeton.unique@example.org");
        assert_eq!(emise.jeton.en_clair().len(), 64);
        assert_eq!(format!("{:?}", emise.jeton), "***");
        assert!(!email_verifie(&pool, compte_id).await);

        assert_eq!(verifier_email(&pool, emise.jeton.en_clair(), None).await.unwrap(), Some(compte_id));
        assert!(email_verifie(&pool, compte_id).await);
        assert_eq!(verifier_email(&pool, emise.jeton.en_clair(), None).await.unwrap(), None, "déjà utilisé");
    }

    /// Un nouveau jeton annule le précédent, qui ne vérifie plus rien.
    #[sqlx::test]
    #[ignore]
    async fn nouveau_jeton_annule_le_precedent(pool: PgPool) {
        let compte_id = compte(&pool, "+22670130002", "jeton.annule@example.org", "professionnel").await;
        let ancien = emettre(&pool, compte_id).await;
        let nouveau = emettre(&pool, compte_id).await;
        let annules: i64 = sqlx::query_scalar("SELECT count(*) FROM jeton_email WHERE compte_id = $1 AND annule_le IS NOT NULL")
            .bind(compte_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(annules, 1);
        assert_eq!(verifier_email(&pool, ancien.jeton.en_clair(), None).await.unwrap(), None);
        assert_eq!(verifier_email(&pool, nouveau.jeton.en_clair(), None).await.unwrap(), Some(compte_id));
    }

    /// R0 : un jeton envoyé à l'ancienne adresse ne vérifie pas la nouvelle.
    #[sqlx::test]
    #[ignore]
    async fn email_change_depuis_l_envoi_refuse(pool: PgPool) {
        let compte_id = compte(&pool, "+22670130003", "ancienne@example.org", "professionnel").await;
        let emise = emettre(&pool, compte_id).await;
        sqlx::query("UPDATE utilisateur SET email = 'nouvelle@example.org' FROM compte c WHERE c.id = $1 AND utilisateur.id = c.utilisateur_id")
            .bind(compte_id)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(verifier_email(&pool, emise.jeton.en_clair(), None).await.unwrap(), None);
        assert!(!email_verifie(&pool, compte_id).await);
    }

    /// Les majuscules de l'adresse ne comptent pas (J6).
    #[sqlx::test]
    #[ignore]
    async fn majuscules_de_l_adresse_ignorees(pool: PgPool) {
        let compte_id = compte(&pool, "+22670130004", "Majuscules@Example.org", "professionnel").await;
        let emise = emettre(&pool, compte_id).await;
        assert_eq!(verifier_email(&pool, emise.jeton.en_clair(), None).await.unwrap(), Some(compte_id));
    }

    #[sqlx::test]
    #[ignore]
    async fn jeton_expire_ou_mal_forme_refuse(pool: PgPool) {
        let compte_id = compte(&pool, "+22670130005", "jeton.expire@example.org", "professionnel").await;
        let emise = emettre(&pool, compte_id).await;
        sqlx::query(
            "UPDATE jeton_email SET date_creation = now() - interval '2 days', expire_le = now() - interval '1 day' \
             WHERE compte_id = $1",
        )
        .bind(compte_id)
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(verifier_email(&pool, emise.jeton.en_clair(), None).await.unwrap(), None, "expiré");
        for faux in ["", "abc", &"z".repeat(64), &"0".repeat(64)] {
            assert_eq!(verifier_email(&pool, faux, None).await.unwrap(), None, "{faux}");
        }
    }

    /// Limité à un compte : le jeton d'un autre compte est refusé et reste
    /// utilisable par son compte.
    #[sqlx::test]
    #[ignore]
    async fn jeton_d_un_autre_compte_refuse(pool: PgPool) {
        let premier = compte(&pool, "+22670130007", "premier@example.org", "professionnel").await;
        let second = compte(&pool, "+22670130008", "second@example.org", "professionnel").await;
        let emise = emettre(&pool, second).await;
        assert_eq!(verifier_email(&pool, emise.jeton.en_clair(), Some(premier)).await.unwrap(), None);
        assert!(!email_verifie(&pool, premier).await && !email_verifie(&pool, second).await);
        assert_eq!(verifier_email(&pool, emise.jeton.en_clair(), Some(second)).await.unwrap(), Some(second));
    }

    #[sqlx::test]
    #[ignore]
    async fn compte_sans_email_aucun_jeton(pool: PgPool) {
        let utilisateur_id: Uuid = sqlx::query_scalar("INSERT INTO utilisateur (telephone) VALUES ('+22670130006') RETURNING id")
            .fetch_one(&pool)
            .await
            .unwrap();
        let compte_id: Uuid = sqlx::query_scalar(
            "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'professionnel', 'x') RETURNING id",
        )
        .bind(utilisateur_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let mut tx = pool.begin().await.unwrap();
        assert!(emettre_verification(&mut tx, compte_id).await.unwrap().is_none());
    }
}
