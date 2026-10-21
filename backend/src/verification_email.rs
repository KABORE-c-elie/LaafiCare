//! Vérification de l'email d'un compte professionnel par lien (R0, choix A
//! du 2026-10-15 ; étape 3.7, décisions E5 à E7 du 2026-10-20).
//!
//! - Le lien ouvre la page `/verifier-email` de l'application web ; **ouvrir
//!   le lien ne vérifie rien**, la page envoie le jeton quand la personne
//!   confirme (RFC 9110 §9.2.1 : un GET ne doit pas changer l'état, les
//!   filtres de messagerie ouvrent les liens tout seuls).
//! - Le jeton est placé après `#`, jamais en paramètre `?jeton=` : le
//!   fragment n'est jamais envoyé au serveur (RFC 3986 §3.5) ni dans
//!   l'en-tête Referer (W3C *Referrer Policy* §8.4, étape 5) ; OWASP
//!   (*Email Validation and Verification Cheat Sheet*) : ne jamais
//!   journaliser un jeton ni une URL complète de vérification.
//! - Le jeton lui-même : `jeton_email.rs` (32 octets, empreinte seule en
//!   base, 24 h, usage unique, lié à l'adresse visée).

use sqlx::PgPool;

use crate::auth_professionnel::{self, ErreurAuthProfessionnel};
use crate::email::EmailSender;
use crate::jeton_email::{self, Renvoi, VerificationEmise};

/// Construit le lien et envoie l'email. Un échec d'envoi est journalisé
/// sans le jeton ni l'adresse, et n'annule rien : la personne demande un
/// nouveau lien par la route de renvoi.
pub fn envoyer_lien(email_sender: &dyn EmailSender, url_application_web: &str, emise: &VerificationEmise) {
    let corps = format!(
        "Bonjour,\n\n\
         Pour vérifier votre adresse email LaafiCare, ouvrez ce lien puis confirmez sur la page qui s'affiche :\n\n\
         {url_application_web}/verifier-email#jeton={}\n\n\
         Ce lien est valable {} heures et ne sert qu'une fois.\n\
         Si vous n'êtes pas à l'origine de cette demande, ignorez ce message.",
        emise.jeton.en_clair(),
        jeton_email::DUREE_VERIFICATION_HEURES,
    );
    if email_sender
        .envoyer(&emise.email, "LaafiCare : vérification de votre adresse email", &corps)
        .is_err()
    {
        tracing::error!("échec d'envoi de l'email de vérification");
    }
}

/// `POST /api/professionnels/email/verifier` : vrai si le jeton a vérifié
/// une adresse. Faux dans tous les cas de refus, jamais distingués (E6).
pub async fn verifier(pool: &PgPool, jeton: &str) -> Result<bool, sqlx::Error> {
    // `None` : la route n'est liée à aucun compte, seul le jeton compte.
    Ok(jeton_email::verifier_email(pool, jeton.trim(), None).await?.is_some())
}

#[derive(Debug, thiserror::Error)]
pub enum ErreurRenvoi {
    /// Mêmes refus et même message que la connexion (P1, L1).
    #[error("Email ou mot de passe incorrect.")]
    IdentifiantsInvalides,
    /// Un lien a été envoyé il y a moins d'une minute (E7).
    #[error("Un lien vient d'être envoyé. Patientez avant d'en demander un autre.")]
    TropTot { attendre_secondes: i64 },
    #[error("erreur interne : {0}")]
    Interne(&'static str),
}

/// `POST /api/professionnels/email/renvoyer` (E7) : email et mot de passe
/// professionnels, comptés dans le verrouillage comme une connexion (même
/// fonction, `verifier_identifiants`). Rien n'est révélé sans le bon mot de
/// passe. Email déjà vérifié : rien n'est envoyé, même réponse qu'un envoi.
pub async fn renvoyer(
    pool: &PgPool,
    email_sender: &dyn EmailSender,
    url_application_web: &str,
    email: &str,
    mot_de_passe: &str,
) -> Result<(), ErreurRenvoi> {
    match auth_professionnel::verifier_identifiants(pool, "professionnel", email, mot_de_passe).await {
        Ok(_) => return Ok(()),
        Err(ErreurAuthProfessionnel::EmailNonVerifie) => {}
        Err(ErreurAuthProfessionnel::IdentifiantsInvalides) => return Err(ErreurRenvoi::IdentifiantsInvalides),
        Err(ErreurAuthProfessionnel::Interne(etape)) => return Err(ErreurRenvoi::Interne(etape)),
    }

    // Mot de passe correct, email non vérifié : le compte existe. Même
    // recherche que `verifier_identifiants` (J6).
    let interne = |etape: &'static str, e: sqlx::Error| {
        tracing::error!(etape, nature = %auth_professionnel::nature_sqlx(&e), "renvoi du lien : erreur interne");
        ErreurRenvoi::Interne(etape)
    };
    let mut tx = pool.begin().await.map_err(|e| interne("transaction", e))?;
    let compte_id: sqlx::types::Uuid = sqlx::query_scalar(
        "SELECT c.id FROM compte c JOIN utilisateur u ON u.id = c.utilisateur_id \
         WHERE lower(u.email) = lower(btrim($1)) AND c.type_compte = 'professionnel'",
    )
    .bind(email)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| interne("recherche du compte", e))?;
    let renvoi = jeton_email::renvoyer_verification(&mut tx, compte_id)
        .await
        .map_err(|e| interne("émission du jeton", e))?;
    tx.commit().await.map_err(|e| interne("validation de la transaction", e))?;

    match renvoi {
        Some(Renvoi::Emis(emise)) => {
            // Après le commit, comme à la création du compte.
            envoyer_lien(email_sender, url_application_web, &emise);
            Ok(())
        }
        Some(Renvoi::TropTot { attendre_secondes }) => Err(ErreurRenvoi::TropTot { attendre_secondes }),
        None => Err(ErreurRenvoi::Interne("compte sans email")),
    }
}

#[cfg(test)]
mod tests {
    use sqlx::types::Uuid;

    use super::*;
    use crate::email::{EmailsRecus, jeton_du_lien};
    use crate::mot_de_passe;

    // Base temporaire par test ; données fictives.

    const BON: &str = "Exemple#2026";

    async fn professionnel(pool: &PgPool, telephone: &str, email: &str) -> Uuid {
        let utilisateur_id: Uuid = sqlx::query_scalar("INSERT INTO utilisateur (telephone, email) VALUES ($1, $2) RETURNING id")
            .bind(telephone)
            .bind(email)
            .fetch_one(pool)
            .await
            .unwrap();
        sqlx::query_scalar(
            "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'professionnel', $2) RETURNING id",
        )
        .bind(utilisateur_id)
        .bind(mot_de_passe::hacher(BON).unwrap())
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[sqlx::test]
    #[ignore]
    async fn renvoi_puis_verification(pool: PgPool) {
        let emails = EmailsRecus::default();
        let compte_id = professionnel(&pool, "+22670140001", "renvoi.route@example.org").await;

        assert!(matches!(
            renvoyer(&pool, &emails, "http://localhost:4200", "renvoi.route@example.org", "Mauvais#2026").await,
            Err(ErreurRenvoi::IdentifiantsInvalides)
        ));
        assert!(emails.0.lock().unwrap().is_empty(), "rien sans le bon mot de passe");

        renvoyer(&pool, &emails, "http://localhost:4200", " Renvoi.Route@Example.org ", BON).await.unwrap();
        let (destinataire, corps) = emails.0.lock().unwrap()[0].clone();
        assert_eq!(destinataire, "renvoi.route@example.org");
        assert!(corps.contains("http://localhost:4200/verifier-email#jeton="), "{corps}");

        // Second renvoi dans la minute : refusé, rien n'est envoyé.
        assert!(matches!(
            renvoyer(&pool, &emails, "http://localhost:4200", "renvoi.route@example.org", BON).await,
            Err(ErreurRenvoi::TropTot { .. })
        ));
        assert_eq!(emails.0.lock().unwrap().len(), 1);

        let jeton = jeton_du_lien(&corps);
        assert!(verifier(&pool, &jeton).await.unwrap());
        assert!(!verifier(&pool, &jeton).await.unwrap(), "usage unique");
        let verifie: bool = sqlx::query_scalar(
            "SELECT u.email_verifie_le IS NOT NULL FROM compte c JOIN utilisateur u ON u.id = c.utilisateur_id WHERE c.id = $1",
        )
        .bind(compte_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(verifie);

        // Email vérifié : la même réponse, sans aucun envoi.
        renvoyer(&pool, &emails, "http://localhost:4200", "renvoi.route@example.org", BON).await.unwrap();
        assert_eq!(emails.0.lock().unwrap().len(), 1);
    }

    /// Email inconnu : même réponse qu'un mot de passe faux (P1, L1).
    #[sqlx::test]
    #[ignore]
    async fn email_inconnu_meme_reponse_que_mot_de_passe_faux(pool: PgPool) {
        let emails = EmailsRecus::default();
        professionnel(&pool, "+22670140002", "connu@example.org").await;
        let inconnu = renvoyer(&pool, &emails, "http://localhost:4200", "inconnu@example.org", BON).await.unwrap_err();
        let faux = renvoyer(&pool, &emails, "http://localhost:4200", "connu@example.org", "Mauvais#2026").await.unwrap_err();
        assert!(matches!(inconnu, ErreurRenvoi::IdentifiantsInvalides));
        assert!(matches!(faux, ErreurRenvoi::IdentifiantsInvalides));
        assert_eq!(inconnu.to_string(), faux.to_string());
        assert!(emails.0.lock().unwrap().is_empty());
    }

    /// BF-01 : 5 mots de passe faux verrouillent le compte ; le bon mot de
    /// passe est ensuite refusé de la même façon, et rien n'est envoyé.
    #[sqlx::test]
    #[ignore]
    async fn mots_de_passe_faux_verrouillent_le_compte(pool: PgPool) {
        let emails = EmailsRecus::default();
        let compte_id = professionnel(&pool, "+22670140003", "verrou.renvoi@example.org").await;
        for _ in 0..5 {
            assert!(matches!(
                renvoyer(&pool, &emails, "http://localhost:4200", "verrou.renvoi@example.org", "Mauvais#2026").await,
                Err(ErreurRenvoi::IdentifiantsInvalides)
            ));
        }
        let verrouille: bool =
            sqlx::query_scalar("SELECT verrouille_jusqua IS NOT NULL AND verrouille_jusqua > now() FROM compte WHERE id = $1")
                .bind(compte_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(verrouille);
        assert!(matches!(
            renvoyer(&pool, &emails, "http://localhost:4200", "verrou.renvoi@example.org", BON).await,
            Err(ErreurRenvoi::IdentifiantsInvalides)
        ));
        assert!(emails.0.lock().unwrap().is_empty());
    }

    #[sqlx::test]
    #[ignore]
    async fn jeton_inconnu_ou_mal_forme_refuse(pool: PgPool) {
        for faux in ["", "abc", &"0".repeat(64), &"g".repeat(64)] {
            assert!(!verifier(&pool, faux).await.unwrap(), "{faux}");
        }
    }
}
