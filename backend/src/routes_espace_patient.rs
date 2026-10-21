//! Routes de l'espace patient (étape 6, décisions E1 à E9 du 2026-10-21 ;
//! contrat dans docs/api.md, section 2.13) : droits MUNASEB, demandes de
//! remboursement, notifications. Jeton `laaficare:patient` seulement ; le
//! patient vient du jeton, jamais de l'adresse (E2).

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use sqlx::types::Uuid;

use crate::AppState;
use crate::erreur_api::{CheminApi, RequeteApi, reponse_erreur, reponse_erreur_interne};
use crate::extracteur_jwt::PatientAuthentifie;
use laaficare_backend::demande_remboursement_munaseb::StatutDemande;
use laaficare_backend::espace_patient_munaseb::{self as espace, DemandePatient};
use laaficare_backend::notification::{self, NotificationPatient};

// Mêmes bornes que les autres listes de l'API.
const LIMITE_PAR_DEFAUT: i64 = 50;
const LIMITE_MAX: i64 = 100;

fn limite_valide(limite: Option<i64>) -> Result<i64, Response> {
    let limite = limite.unwrap_or(LIMITE_PAR_DEFAUT);
    if (1..=LIMITE_MAX).contains(&limite) {
        Ok(limite)
    } else {
        Err(reponse_erreur(
            StatusCode::BAD_REQUEST,
            "parametre_invalide",
            format!("limite doit être comprise entre 1 et {LIMITE_MAX}"),
        )
        .into_response())
    }
}

/// `GET /api/patients/munaseb/droits` : toujours 200, avec `etat` (E3).
pub async fn droits(patient: PatientAuthentifie, State(state): State<AppState>) -> Response {
    match espace::droits(&state.db, patient.compte_id).await {
        Ok(droits) => (StatusCode::OK, Json(droits)).into_response(),
        Err(erreur) => reponse_erreur_interne("droits MUNASEB du patient", &erreur.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
pub struct FiltreDemandes {
    pub statut: Option<StatutDemande>,
    pub limite: Option<i64>,
    /// `suivant` de la page précédente.
    pub avant: Option<Uuid>,
}

#[derive(Serialize)]
struct ListeDemandes {
    demandes: Vec<DemandePatient>,
    suivant: Option<Uuid>,
}

/// `GET /api/patients/munaseb/demandes?statut=&limite=&avant=`.
pub async fn lister_demandes(
    patient: PatientAuthentifie,
    State(state): State<AppState>,
    RequeteApi(filtre): RequeteApi<FiltreDemandes>,
) -> Response {
    let limite = match limite_valide(filtre.limite) {
        Ok(l) => l,
        Err(reponse) => return reponse,
    };
    match espace::lister_demandes(&state.db, patient.compte_id, filtre.statut, filtre.avant, limite).await {
        Ok(demandes) => {
            let suivant = if demandes.len() as i64 == limite { demandes.last().map(|d| d.id) } else { None };
            (StatusCode::OK, Json(ListeDemandes { demandes, suivant })).into_response()
        }
        Err(erreur) => reponse_erreur_interne("demandes MUNASEB du patient", &erreur.to_string()).into_response(),
    }
}

/// `GET /api/patients/munaseb/demandes/{id}` : 404 `demande_inconnue` pour
/// une demande inexistante comme pour celle d'un autre patient (E2).
pub async fn detail_demande(
    patient: PatientAuthentifie,
    State(state): State<AppState>,
    CheminApi(demande_id): CheminApi<Uuid>,
) -> Response {
    match espace::detail_demande(&state.db, patient.compte_id, demande_id).await {
        Ok(Some(detail)) => (StatusCode::OK, Json(detail)).into_response(),
        Ok(None) => reponse_erreur(StatusCode::NOT_FOUND, "demande_inconnue", "Demande inconnue.").into_response(),
        Err(erreur) => reponse_erreur_interne("détail d'une demande du patient", &erreur.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
pub struct FiltreNotifications {
    pub limite: Option<i64>,
    pub avant: Option<Uuid>,
}

#[derive(Serialize)]
struct ListeNotifications {
    notifications: Vec<NotificationPatient>,
    non_lues: i64,
    suivant: Option<Uuid>,
}

/// `GET /api/patients/notifications?limite=&avant=` (E6).
pub async fn lister_notifications(
    patient: PatientAuthentifie,
    State(state): State<AppState>,
    RequeteApi(filtre): RequeteApi<FiltreNotifications>,
) -> Response {
    let limite = match limite_valide(filtre.limite) {
        Ok(l) => l,
        Err(reponse) => return reponse,
    };
    match notification::lister_pour_patient(&state.db, patient.compte_id, filtre.avant, limite).await {
        Ok((notifications, non_lues)) => {
            let suivant = if notifications.len() as i64 == limite { notifications.last().map(|n| n.id) } else { None };
            (StatusCode::OK, Json(ListeNotifications { notifications, non_lues, suivant })).into_response()
        }
        Err(_) => reponse_erreur_interne("notifications du patient", "erreur de la base").into_response(),
    }
}

/// `POST /api/patients/notifications/{id}/lue` : 204, même si elle était
/// déjà lue ; 404 `notification_inconnue`.
pub async fn marquer_lue(
    patient: PatientAuthentifie,
    State(state): State<AppState>,
    CheminApi(notification_id): CheminApi<Uuid>,
) -> Response {
    match notification::marquer_lue(&state.db, patient.compte_id, notification_id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => {
            reponse_erreur(StatusCode::NOT_FOUND, "notification_inconnue", "Notification inconnue.").into_response()
        }
        Err(_) => reponse_erreur_interne("notification lue", "erreur de la base").into_response(),
    }
}

/// `POST /api/patients/notifications/lues` : 204.
pub async fn marquer_toutes_lues(patient: PatientAuthentifie, State(state): State<AppState>) -> Response {
    match notification::marquer_toutes_lues(&state.db, patient.compte_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => reponse_erreur_interne("notifications lues", "erreur de la base").into_response(),
    }
}
