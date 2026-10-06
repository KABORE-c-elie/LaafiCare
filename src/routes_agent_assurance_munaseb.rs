//! Routes HTTP exposant `auth_agent_assurance_munaseb` (section 12). Même
//! principe que `routes_patient.rs` : le handler désérialise, appelle le
//! service métier, traduit le résultat en réponse HTTP -- aucune logique
//! métier ici.

use axum::{Json, extract::State, http::StatusCode};
use serde::{Deserialize, Serialize};

use crate::AppState;
use laaficare_backend::auth_agent_assurance_munaseb::{self, ErreurAgentAssuranceMunaseb};
use crate::erreur_api::{JsonApi, ReponseErreur, reponse_erreur, reponse_erreur_interne};

// Codes HTTP validés avec le porteur avant d'écrire ce fichier (423 Locked
// pour le compte verrouillé -- décision actée, section 13) ; `code` ajouté
// pour le format d'erreur unique (section 12).
fn mapper_erreur(erreur: ErreurAgentAssuranceMunaseb) -> ReponseErreur {
    let (statut, code) = match &erreur {
        ErreurAgentAssuranceMunaseb::IdentifiantsInvalides => (StatusCode::UNAUTHORIZED, "identifiants_invalides"),
        ErreurAgentAssuranceMunaseb::CompteVerrouille => (StatusCode::LOCKED, "compte_verrouille"),
        // Détail dans les logs seulement, jamais dans la réponse.
        ErreurAgentAssuranceMunaseb::Interne(detail) => {
            return reponse_erreur_interne("AuthAgentAssuranceMunasebService", detail);
        }
    };
    reponse_erreur(statut, code, erreur.to_string())
}

#[derive(Deserialize)]
pub struct ConnexionRequete {
    pub email: String,
    pub mot_de_passe: String,
}

#[derive(Serialize)]
pub struct ConnexionReponse {
    pub jeton: String,
}

/// `POST /api/assurance-munaseb/connexion` -- connexion email + mot de
/// passe, JWT émis uniquement si le rôle agent_assurance_munaseb est
/// confirmé (voir auth_agent_assurance_munaseb::se_connecter).
pub async fn se_connecter(
    State(state): State<AppState>,
    JsonApi(corps): JsonApi<ConnexionRequete>,
) -> Result<(StatusCode, Json<ConnexionReponse>), ReponseErreur> {
    let jeton = auth_agent_assurance_munaseb::se_connecter(
        &state.db,
        &state.jwt,
        &corps.email,
        &corps.mot_de_passe,
    )
    .await
    .map_err(mapper_erreur)?;

    Ok((StatusCode::OK, Json(ConnexionReponse { jeton })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erreurs_de_connexion() {
        for (erreur, statut, code) in [
            (ErreurAgentAssuranceMunaseb::IdentifiantsInvalides, StatusCode::UNAUTHORIZED, "identifiants_invalides"),
            (ErreurAgentAssuranceMunaseb::CompteVerrouille, StatusCode::LOCKED, "compte_verrouille"),
            (ErreurAgentAssuranceMunaseb::Interne("détail secret".into()), StatusCode::INTERNAL_SERVER_ERROR, "erreur_interne"),
        ] {
            let (s, Json(corps)) = mapper_erreur(erreur);
            assert_eq!((s, corps.code), (statut, code));
            assert!(!corps.erreur.contains("secret"));
        }
    }

}
