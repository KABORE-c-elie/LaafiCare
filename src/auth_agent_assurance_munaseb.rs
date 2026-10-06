//! `AuthAgentAssuranceMunasebService` (section 12 du CLAUDE.md) : construit
//! sur `AuthProfessionnelService::se_connecter` (identité seule, sans rôle
//! -- voir la doc de module de `auth_professionnel.rs`) pour confirmer
//! l'appartenance au rôle `agent_assurance_munaseb` et émettre le JWT
//! correspondant. C'est exactement le découplage annoncé quand
//! `auth_professionnel.rs` a été écrit : ce module est le premier « futur
//! service spécifique au rôle » qui appelle `JwtService::emettre`.
//!
//! Récupération de mot de passe (OTP, ou plus tard lien email) : reste
//! générique au niveau `utilisateur`
//! (`auth_professionnel::demander_otp_recuperation` /
//! `reinitialiser_via_otp`) -- rien à ajouter ici, un rôle ne change pas la
//! façon de prouver le contrôle d'une identité.
//!
//! Création de compte : hors scope de ce fichier. Aucun flux de
//! self-inscription pour ce rôle en V1 (à la différence du Patient) --
//! provisionnement encore à définir (probablement administratif, futur
//! rôle Administrateur). Les tests créent donc la ligne `utilisateur` +
//! `agent_assurance_munaseb` directement en SQL, comme le fait déjà
//! `auth_professionnel.rs`.

use sqlx::PgPool;
use sqlx::types::Uuid;

use crate::auth_professionnel::{self, ErreurAuthProfessionnel};
use crate::jwt::JwtService;

/// Rôle inscrit dans le JWT (`Claims::role`, `jwt.rs`).
pub const ROLE: &str = "agent_assurance_munaseb";

#[derive(Debug, thiserror::Error)]
pub enum ErreurAgentAssuranceMunaseb {
    /// Couvre email inconnu, mot de passe incorrect, ET identité valide
    /// mais sans rôle `agent_assurance_munaseb` -- décision actée avec le
    /// porteur (2026-09-23) : ne jamais laisser transparaître qu'un
    /// identifiant/mot de passe valide existe pour un autre rôle.
    #[error("Email ou mot de passe incorrect.")]
    IdentifiantsInvalides,

    #[error("compte verrouillé, réessayer plus tard")]
    CompteVerrouille,

    #[error("erreur interne : {0}")]
    Interne(String),
}

impl From<sqlx::Error> for ErreurAgentAssuranceMunaseb {
    fn from(erreur: sqlx::Error) -> Self {
        ErreurAgentAssuranceMunaseb::Interne(erreur.to_string())
    }
}

impl From<jsonwebtoken::errors::Error> for ErreurAgentAssuranceMunaseb {
    fn from(erreur: jsonwebtoken::errors::Error) -> Self {
        ErreurAgentAssuranceMunaseb::Interne(erreur.to_string())
    }
}

/// Connexion email + mot de passe. Vérifie l'identité via
/// `AuthProfessionnelService::se_connecter` (verrouillage BF-01 déjà géré
/// là), puis confirme l'appartenance au rôle avant d'émettre le JWT.
pub async fn se_connecter(
    pool: &PgPool,
    jwt: &JwtService,
    email: &str,
    mot_de_passe: &str,
) -> Result<String, ErreurAgentAssuranceMunaseb> {
    let utilisateur_id = auth_professionnel::se_connecter(pool, email, mot_de_passe)
        .await
        .map_err(|erreur| match erreur {
            ErreurAuthProfessionnel::IdentifiantsInvalides => {
                ErreurAgentAssuranceMunaseb::IdentifiantsInvalides
            }
            ErreurAuthProfessionnel::CompteVerrouille => {
                ErreurAgentAssuranceMunaseb::CompteVerrouille
            }
            // CodeInvalide / MotDePasseTropCourt appartiennent à la
            // récupération, pas à se_connecter -- non atteignables ici,
            // mais gardés dans un bras générique pour rester exhaustif sans
            // dépendre de la liste exacte des variantes de l'autre module.
            autre => ErreurAgentAssuranceMunaseb::Interne(autre.to_string()),
        })?;

    let est_agent: Option<(Uuid,)> =
        sqlx::query_as("SELECT id FROM agent_assurance_munaseb WHERE utilisateur_id = $1")
            .bind(utilisateur_id)
            .fetch_optional(pool)
            .await?;

    if est_agent.is_none() {
        return Err(ErreurAgentAssuranceMunaseb::IdentifiantsInvalides);
    }

    let jeton = jwt.emettre(&utilisateur_id.to_string(), ROLE)?;
    Ok(jeton)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Base temporaire par test, jamais la base de développement : voir
    // « Tests et environnement », section 12 du CLAUDE.md.
    #[sqlx::test]
    #[ignore]
    async fn se_connecter_confirme_le_role_en_base(pool: PgPool) {
        let jwt = JwtService::new(&"a".repeat(32));

        let email_agent = "test.agent.munaseb@example.org";
        let email_sans_role = "test.sans.role@example.org";
        let mot_de_passe = "motdepasseagent123";
        let hash = crate::mot_de_passe::hacher(mot_de_passe).unwrap();

        let utilisateur_agent_id: Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (nom, prenom, telephone, email, mot_de_passe_hash) \
             VALUES ('Agent', 'Test', '+22670003003', $1, $2) RETURNING id",
        )
        .bind(email_agent)
        .bind(&hash)
        .fetch_one(&pool)
        .await
        .unwrap();

        sqlx::query("INSERT INTO agent_assurance_munaseb (utilisateur_id) VALUES ($1)")
            .bind(utilisateur_agent_id)
            .execute(&pool)
            .await
            .unwrap();

        // Identité valide mais sans ligne agent_assurance_munaseb.
        sqlx::query(
            "INSERT INTO utilisateur (nom, prenom, telephone, email, mot_de_passe_hash) \
             VALUES ('SansRole', 'Test', '+22670004004', $1, $2)",
        )
        .bind(email_sans_role)
        .bind(&hash)
        .execute(&pool)
        .await
        .unwrap();

        // --- succès : identifiants corrects + rôle présent ---
        let jeton = se_connecter(&pool, &jwt, email_agent, mot_de_passe).await.unwrap();
        let claims = jwt.verifier(&jeton).unwrap();
        assert_eq!(claims.sub, utilisateur_agent_id.to_string());
        assert_eq!(claims.role, ROLE);

        // --- mot de passe incorrect ---
        assert!(matches!(
            se_connecter(&pool, &jwt, email_agent, "mauvais-mot-de-passe").await,
            Err(ErreurAgentAssuranceMunaseb::IdentifiantsInvalides)
        ));

        // --- identité valide, mais pas le rôle : même erreur que mot de
        // passe incorrect (décision actée, pas de fuite d'information) ---
        assert!(matches!(
            se_connecter(&pool, &jwt, email_sans_role, mot_de_passe).await,
            Err(ErreurAgentAssuranceMunaseb::IdentifiantsInvalides)
        ));
    }
}
