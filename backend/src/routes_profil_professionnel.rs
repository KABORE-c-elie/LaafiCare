//! Routes du profil professionnel (module médecin, étape 1 ; décisions MD9
//! et MD10 du porteur, 2026-11-17 ; docs/api.md §2.17) :
//! - le professionnel lit et remplit son numéro d'ordre et sa spécialité ;
//! - l'équipe LaafiCare liste les profils à contrôler et les marque
//!   vérifiés.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::types::Uuid;

use crate::AppState;
use crate::erreur_api::{
    CheminApi, JsonApi, ReponseErreur, RequeteApi, reponse_erreur, reponse_erreur_interne,
};
use crate::extracteur_jwt::{AdministrateurAuthentifie, ProfessionnelConnecte};
use crate::routes_pharmacie::limite_valide;
use laaficare_backend::profil_professionnel::{
    self as profil, ErreurProfil, ProfilAVerifier, ProfilProfessionnel, Specialite,
};

fn mapper_erreur(erreur: ErreurProfil) -> ReponseErreur {
    match erreur {
        ErreurProfil::NumeroOrdreInvalide => reponse_erreur(
            StatusCode::UNPROCESSABLE_ENTITY,
            "numero_ordre_invalide",
            erreur.to_string(),
        ),
        ErreurProfil::VerificationImpossible => reponse_erreur(
            StatusCode::CONFLICT,
            "verification_impossible",
            erreur.to_string(),
        ),
        ErreurProfil::Interne(detail) => reponse_erreur_interne("profil professionnel", &detail),
    }
}

/// `GET /api/professionnels/profil` : avec ou sans affectation active.
pub async fn lire(
    professionnel: ProfessionnelConnecte,
    State(state): State<AppState>,
) -> Result<Json<ProfilProfessionnel>, ReponseErreur> {
    profil::lire(&state.db, professionnel.compte_id)
        .await
        .map(Json)
        .map_err(mapper_erreur)
}

#[derive(Deserialize)]
pub struct ProfilRequete {
    pub numero_ordre: Option<String>,
    pub specialite: Option<Specialite>,
}

/// `PUT /api/professionnels/profil` : remplace le numéro et la spécialité
/// (un champ absent ou `null` est vidé) ; 200 et le profil. Un changement
/// annule la vérification de l'équipe.
pub async fn modifier(
    professionnel: ProfessionnelConnecte,
    State(state): State<AppState>,
    JsonApi(corps): JsonApi<ProfilRequete>,
) -> Result<Json<ProfilProfessionnel>, ReponseErreur> {
    profil::modifier(
        &state.db,
        professionnel.compte_id,
        corps.numero_ordre.as_deref(),
        corps.specialite,
    )
    .await
    .map(Json)
    .map_err(mapper_erreur)
}

#[derive(Deserialize)]
pub struct PageProfils {
    pub limite: Option<i64>,
    pub apres: Option<Uuid>,
}

#[derive(Serialize)]
pub struct ListeProfils {
    pub profils: Vec<ProfilAVerifier>,
    pub suivant: Option<Uuid>,
}

/// `GET /api/administrateurs/profils-professionnels` : profils avec un
/// numéro, pas encore vérifiés.
pub async fn lister_a_verifier(
    _administrateur: AdministrateurAuthentifie,
    State(state): State<AppState>,
    RequeteApi(page): RequeteApi<PageProfils>,
) -> Result<Json<ListeProfils>, ReponseErreur> {
    let limite = limite_valide(page.limite)?;
    let profils = profil::lister_a_verifier(&state.db, page.apres, limite)
        .await
        .map_err(|e| reponse_erreur_interne("profils à vérifier", &e.to_string()))?;
    let suivant = if profils.len() as i64 == limite {
        profils.last().map(|p| p.compte_id)
    } else {
        None
    };
    Ok(Json(ListeProfils { profils, suivant }))
}

#[derive(Deserialize)]
pub struct VerificationRequete {
    /// Le numéro et la spécialité que l'équipe a lus et contrôlés.
    pub numero_ordre: String,
    pub specialite: Option<Specialite>,
}

/// `POST /api/administrateurs/profils-professionnels/{compte_id}/verifier` :
/// 204 ; 409 `verification_impossible` si le profil a changé depuis la
/// lecture, n'a pas de numéro, est déjà vérifié ou n'existe pas.
pub async fn verifier(
    administrateur: AdministrateurAuthentifie,
    State(state): State<AppState>,
    CheminApi(compte_id): CheminApi<Uuid>,
    JsonApi(corps): JsonApi<VerificationRequete>,
) -> Result<StatusCode, ReponseErreur> {
    profil::verifier(
        &state.db,
        administrateur.compte_id,
        compte_id,
        &corps.numero_ordre,
        corps.specialite,
    )
    .await
    .map_err(mapper_erreur)?;
    Ok(StatusCode::NO_CONTENT)
}
