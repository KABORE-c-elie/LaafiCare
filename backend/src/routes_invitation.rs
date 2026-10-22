//! Routes des invitations (étape 3.10 réduite, décisions I1 à I17 ; contrat
//! dans docs/api.md). Côté responsable : jeton professionnel en exercice
//! sur une affectation `responsable`, la structure venant de l'affectation,
//! jamais de l'adresse (I1). Côté invité : jeton patient (I2).

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use sqlx::types::Uuid;

use crate::AppState;
use crate::erreur_api::{CheminApi, ErreurApi, JsonApi, RequeteApi, reponse_erreur, reponse_erreur_interne};
use crate::extracteur_jwt::{PatientAuthentifie, ProfessionnelEnExercice, RefusAuthentification};
use laaficare_backend::compte_professionnel::{ErreurComptePro, Saisie};
use laaficare_backend::invitation::{self, ErreurInvitation, InvitationDeLaStructure, Responsable, RoleInvitation};

const LIMITE_PAR_DEFAUT: i64 = 50;
const LIMITE_MAX: i64 = 100;

fn reponse_erreur_invitation(erreur: ErreurInvitation) -> Response {
    let (statut, code) = match &erreur {
        ErreurInvitation::TelephoneInvalide => (StatusCode::UNPROCESSABLE_ENTITY, "telephone_invalide"),
        ErreurInvitation::RoleNonPermis => (StatusCode::UNPROCESSABLE_ENTITY, "role_non_permis"),
        ErreurInvitation::AutoInvitation => (StatusCode::UNPROCESSABLE_ENTITY, "auto_invitation"),
        ErreurInvitation::TropDeDemandes => (StatusCode::TOO_MANY_REQUESTS, "trop_de_demandes"),
        ErreurInvitation::TropDInvitationsEnAttente => (StatusCode::CONFLICT, "trop_d_invitations_en_attente"),
        ErreurInvitation::InvitationDejaEnAttente => (StatusCode::CONFLICT, "invitation_deja_en_attente"),
        ErreurInvitation::InvitationInconnue => (StatusCode::NOT_FOUND, "invitation_inconnue"),
        ErreurInvitation::InvitationExpiree => (StatusCode::CONFLICT, "invitation_expiree"),
        ErreurInvitation::TransitionInvalide { statut_actuel } => {
            let mut corps = ErreurApi::simple("transition_invalide", erreur.to_string());
            corps.statut_actuel = Some(statut_actuel.clone());
            return (StatusCode::CONFLICT, Json(corps)).into_response();
        }
        ErreurInvitation::StructureNonValidee => (StatusCode::CONFLICT, "structure_non_validee"),
        ErreurInvitation::AffectationDejaActive => (StatusCode::CONFLICT, "affectation_deja_active"),
        // Mêmes codes qu'à la création d'une structure (I13).
        ErreurInvitation::ComptePro(compte) => match compte {
            ErreurComptePro::EmailObligatoire | ErreurComptePro::EmailInvalide => {
                (StatusCode::UNPROCESSABLE_ENTITY, "donnees_invalides")
            }
            ErreurComptePro::MotDePasseNonConforme(regles) => {
                let mut corps = ErreurApi::simple("mot_de_passe_non_conforme", erreur.to_string());
                corps.regles_non_respectees = Some(regles.regles_non_respectees.clone());
                return (StatusCode::UNPROCESSABLE_ENTITY, Json(corps)).into_response();
            }
            ErreurComptePro::MotDePasseIncorrect => (StatusCode::UNAUTHORIZED, "identifiants_invalides"),
            ErreurComptePro::SecondFacteurRequis => (StatusCode::CONFLICT, "second_facteur_requis"),
            ErreurComptePro::EmailDifferent => (StatusCode::CONFLICT, "email_different"),
            ErreurComptePro::EmailDejaUtilise => (StatusCode::CONFLICT, "email_deja_utilise"),
            ErreurComptePro::CompteDejaCree => (StatusCode::CONFLICT, "requete_simultanee"),
            ErreurComptePro::Interne(etape) => return reponse_erreur_interne("invitation", etape).into_response(),
        },
        ErreurInvitation::Interne(etape) => return reponse_erreur_interne("invitation", etape).into_response(),
    };
    reponse_erreur(statut, code, erreur.to_string()).into_response()
}

/// Seul un responsable invite et gère les invitations (I3).
pub(crate) fn responsable(professionnel: &ProfessionnelEnExercice) -> Result<Responsable<'_>, Response> {
    let exercice = &professionnel.0;
    if exercice.role != "responsable" {
        return Err(RefusAuthentification::Interdit.into_response());
    }
    Ok(Responsable {
        compte_id: exercice.compte_id,
        structure_id: exercice.structure_id,
        type_structure: &exercice.type_structure,
    })
}

#[derive(Deserialize)]
pub struct CreationRequete {
    pub telephone: String,
    pub role: RoleInvitation,
}

/// `POST /api/professionnels/invitations` : 201 `{invitation_id,
/// expire_le}`, la même réponse que le numéro ait un compte ou non.
pub async fn creer(
    professionnel: ProfessionnelEnExercice,
    State(state): State<AppState>,
    JsonApi(corps): JsonApi<CreationRequete>,
) -> Response {
    let responsable = match responsable(&professionnel) {
        Ok(r) => r,
        Err(reponse) => return reponse,
    };
    match invitation::creer(&state.db, state.sms.as_ref(), responsable, &corps.telephone, corps.role).await {
        Ok(creee) => (StatusCode::CREATED, Json(creee)).into_response(),
        Err(erreur) => reponse_erreur_invitation(erreur),
    }
}

#[derive(Deserialize)]
pub struct FiltreInvitations {
    pub limite: Option<i64>,
    /// `suivant` de la page précédente.
    pub avant: Option<Uuid>,
}

#[derive(Serialize)]
struct ListeInvitations {
    invitations: Vec<InvitationDeLaStructure>,
    suivant: Option<Uuid>,
}

/// `GET /api/professionnels/invitations?limite=&avant=` : les invitations
/// de la structure, les plus récentes d'abord.
pub async fn lister(
    professionnel: ProfessionnelEnExercice,
    State(state): State<AppState>,
    RequeteApi(filtre): RequeteApi<FiltreInvitations>,
) -> Response {
    let responsable = match responsable(&professionnel) {
        Ok(r) => r,
        Err(reponse) => return reponse,
    };
    let limite = filtre.limite.unwrap_or(LIMITE_PAR_DEFAUT);
    if !(1..=LIMITE_MAX).contains(&limite) {
        return reponse_erreur(
            StatusCode::BAD_REQUEST,
            "parametre_invalide",
            format!("limite doit être comprise entre 1 et {LIMITE_MAX}"),
        )
        .into_response();
    }
    match invitation::lister_pour_structure(&state.db, responsable.structure_id, filtre.avant, limite).await {
        Ok(invitations) => {
            let suivant = if invitations.len() as i64 == limite { invitations.last().map(|i| i.id) } else { None };
            (StatusCode::OK, Json(ListeInvitations { invitations, suivant })).into_response()
        }
        Err(_) => reponse_erreur_interne("liste des invitations", "erreur de la base").into_response(),
    }
}

/// `POST /api/professionnels/invitations/{id}/annuler` : 204.
pub async fn annuler(
    professionnel: ProfessionnelEnExercice,
    State(state): State<AppState>,
    CheminApi(invitation_id): CheminApi<Uuid>,
) -> Response {
    let responsable = match responsable(&professionnel) {
        Ok(r) => r,
        Err(reponse) => return reponse,
    };
    match invitation::annuler(&state.db, responsable.structure_id, invitation_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(erreur) => reponse_erreur_invitation(erreur),
    }
}

#[derive(Serialize)]
struct InvitationsRecues {
    invitations: Vec<invitation::InvitationRecue>,
}

/// `GET /api/patients/invitations`.
pub async fn lister_recues(patient: PatientAuthentifie, State(state): State<AppState>) -> Response {
    match invitation::lister_pour_patient(&state.db, patient.compte_id).await {
        Ok(invitations) => (StatusCode::OK, Json(InvitationsRecues { invitations })).into_response(),
        Err(_) => reponse_erreur_interne("invitations reçues", "erreur de la base").into_response(),
    }
}

/// Pas de `Debug` : le mot de passe est un secret.
#[derive(Deserialize)]
pub struct AcceptationRequete {
    /// Seulement sans compte professionnel.
    pub email_professionnel: Option<String>,
    pub mot_de_passe_professionnel: String,
}

#[derive(Serialize)]
struct AcceptationReponse {
    affectation_id: Uuid,
    email_a_verifier: bool,
}

/// `POST /api/patients/invitations/{id}/accepter` : 200 `{affectation_id,
/// email_a_verifier}`.
pub async fn accepter(
    patient: PatientAuthentifie,
    State(state): State<AppState>,
    CheminApi(invitation_id): CheminApi<Uuid>,
    JsonApi(corps): JsonApi<AcceptationRequete>,
) -> Response {
    let saisie = Saisie {
        email_professionnel: corps.email_professionnel.as_deref(),
        mot_de_passe: &corps.mot_de_passe_professionnel,
    };
    match invitation::accepter(
        &state.db,
        state.email.as_ref(),
        &state.url_application_web,
        patient.compte_id,
        invitation_id,
        &saisie,
    )
    .await
    {
        Ok(acceptation) => (
            StatusCode::OK,
            Json(AcceptationReponse {
                affectation_id: acceptation.affectation_id,
                email_a_verifier: acceptation.email_a_verifier,
            }),
        )
            .into_response(),
        Err(erreur) => reponse_erreur_invitation(erreur),
    }
}

/// `POST /api/patients/invitations/{id}/refuser` : 204.
pub async fn refuser(
    patient: PatientAuthentifie,
    State(state): State<AppState>,
    CheminApi(invitation_id): CheminApi<Uuid>,
) -> Response {
    match invitation::refuser(&state.db, patient.compte_id, invitation_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(erreur) => reponse_erreur_invitation(erreur),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_des_invitations() {
        for (erreur, statut) in [
            (ErreurInvitation::TelephoneInvalide, StatusCode::UNPROCESSABLE_ENTITY),
            (ErreurInvitation::RoleNonPermis, StatusCode::UNPROCESSABLE_ENTITY),
            (ErreurInvitation::AutoInvitation, StatusCode::UNPROCESSABLE_ENTITY),
            (ErreurInvitation::TropDeDemandes, StatusCode::TOO_MANY_REQUESTS),
            (ErreurInvitation::TropDInvitationsEnAttente, StatusCode::CONFLICT),
            (ErreurInvitation::InvitationDejaEnAttente, StatusCode::CONFLICT),
            (ErreurInvitation::InvitationInconnue, StatusCode::NOT_FOUND),
            (ErreurInvitation::InvitationExpiree, StatusCode::CONFLICT),
            (ErreurInvitation::StructureNonValidee, StatusCode::CONFLICT),
            (ErreurInvitation::AffectationDejaActive, StatusCode::CONFLICT),
            (ErreurInvitation::ComptePro(ErreurComptePro::MotDePasseIncorrect), StatusCode::UNAUTHORIZED),
        ] {
            assert_eq!(reponse_erreur_invitation(erreur).status(), statut);
        }
    }
}
