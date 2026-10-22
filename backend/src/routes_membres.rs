//! Routes des membres d'une structure (étape 3.11 réduite, décisions DS1 à
//! DS9 du 2026-10-21 ; contrat dans docs/api.md). Jeton professionnel en
//! exercice sur une affectation `responsable` ; la structure vient de
//! l'affectation, jamais de l'adresse, comme pour les invitations (I1). Les
//! routes `/api/professionnels/affectations` restent réservées aux
//! affectations de la personne connectée (DS1).

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use sqlx::types::Uuid;

use crate::AppState;
use crate::erreur_api::{CheminApi, ErreurApi, reponse_erreur, reponse_erreur_interne};
use crate::extracteur_jwt::ProfessionnelEnExercice;
use crate::routes_invitation::responsable;
use laaficare_backend::membres::{self, ErreurDesactivation};

fn reponse_erreur_desactivation(erreur: ErreurDesactivation) -> Response {
    let (statut, code) = match &erreur {
        // Inexistante ou d'une autre structure : jamais distinguées (DS1).
        ErreurDesactivation::AffectationInconnue => (StatusCode::NOT_FOUND, "affectation_inconnue"),
        ErreurDesactivation::TransitionInvalide { statut_actuel } => {
            let mut corps = ErreurApi::simple("transition_invalide", erreur.to_string());
            corps.statut_actuel = Some(statut_actuel.clone());
            return (StatusCode::CONFLICT, Json(corps)).into_response();
        }
        ErreurDesactivation::DernierResponsable => (StatusCode::CONFLICT, "dernier_responsable"),
        ErreurDesactivation::Interne(etape) => {
            return reponse_erreur_interne("désactivation d'une affectation", etape).into_response();
        }
    };
    reponse_erreur(statut, code, erreur.to_string()).into_response()
}

#[derive(Serialize)]
struct Membres {
    membres: Vec<membres::Membre>,
}

/// `GET /api/professionnels/membres` : 200 `{membres: [...]}` (DS7) ; 403
/// `acces_refuse` pour un rôle autre que `responsable`.
pub async fn lister(professionnel: ProfessionnelEnExercice, State(state): State<AppState>) -> Response {
    let responsable = match responsable(&professionnel) {
        Ok(r) => r,
        Err(reponse) => return reponse,
    };
    match membres::lister(&state.db, responsable.structure_id).await {
        Ok(membres) => (StatusCode::OK, Json(Membres { membres })).into_response(),
        Err(_) => reponse_erreur_interne("liste des membres", "erreur de la base").into_response(),
    }
}

/// `POST /api/professionnels/membres/{id}/desactiver` (`id` = l'affectation),
/// sans corps : 204 ; 404 `affectation_inconnue` ; 409 `transition_invalide`
/// ou `dernier_responsable` ; 403 `acces_refuse` pour un non-responsable.
pub async fn desactiver(
    professionnel: ProfessionnelEnExercice,
    State(state): State<AppState>,
    CheminApi(affectation_id): CheminApi<Uuid>,
) -> Response {
    let responsable = match responsable(&professionnel) {
        Ok(r) => r,
        Err(reponse) => return reponse,
    };
    match membres::desactiver(
        &state.db,
        state.email.as_ref(),
        &state.contact_securite,
        responsable.compte_id,
        responsable.structure_id,
        affectation_id,
    )
    .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(erreur) => reponse_erreur_desactivation(erreur),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_des_erreurs_de_desactivation() {
        for (erreur, statut) in [
            (ErreurDesactivation::AffectationInconnue, StatusCode::NOT_FOUND),
            (ErreurDesactivation::TransitionInvalide { statut_actuel: "desactivee".into() }, StatusCode::CONFLICT),
            (ErreurDesactivation::DernierResponsable, StatusCode::CONFLICT),
            (ErreurDesactivation::Interne("test"), StatusCode::INTERNAL_SERVER_ERROR),
        ] {
            assert_eq!(reponse_erreur_desactivation(erreur).status(), statut);
        }
    }
}
