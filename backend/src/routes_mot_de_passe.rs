//! Routes du mot de passe oublié, professionnels et administrateurs (étape
//! 3.12, décisions R10 à R21 ; contrat dans docs/api.md). Les patients
//! gardent la réinitialisation par SMS (`routes_patient.rs`).

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::AppState;
use crate::erreur_api::{ErreurApi, JsonApi, reponse_erreur, reponse_erreur_interne};
use laaficare_backend::reinitialisation::{self, ErreurReinitialisation, SorteCompte};

#[derive(Deserialize)]
pub struct OubliRequete {
    pub email: String,
}

/// Même travail pour les deux sortes de comptes : la tâche de fond est
/// lancée (ou ignorée si la limite est atteinte), et la réponse est toujours
/// 204 tout de suite (R4, R11).
fn lancer(state: &AppState, sorte: SorteCompte, email: String) -> Response {
    reinitialisation::demander(
        state.db.clone(),
        state.email.clone(),
        state.url_application_web.clone(),
        &state.taches_reinitialisation,
        sorte,
        email,
    );
    StatusCode::NO_CONTENT.into_response()
}

/// `POST /api/professionnels/mot-de-passe/oubli` -- `{"email"}`, toujours
/// 204 (422 seulement pour un champ manquant).
pub async fn oubli_professionnel(State(state): State<AppState>, JsonApi(corps): JsonApi<OubliRequete>) -> Response {
    lancer(&state, SorteCompte::Professionnel, corps.email)
}

/// `POST /api/administrateurs/mot-de-passe/oubli` -- même contrat.
pub async fn oubli_administrateur(State(state): State<AppState>, JsonApi(corps): JsonApi<OubliRequete>) -> Response {
    lancer(&state, SorteCompte::Administrateur, corps.email)
}

/// Pas de `Debug` : le jeton et le mot de passe sont des secrets.
#[derive(Deserialize)]
pub struct ReinitialiserRequete {
    pub jeton: String,
    pub nouveau_mot_de_passe: String,
}

fn reponse_erreur_reinitialisation(erreur: ErreurReinitialisation) -> Response {
    match &erreur {
        ErreurReinitialisation::LienInvalide => {
            reponse_erreur(StatusCode::BAD_REQUEST, "lien_invalide", erreur.to_string()).into_response()
        }
        ErreurReinitialisation::MotDePasseNonConforme(regles) => {
            let mut corps = ErreurApi::simple("mot_de_passe_non_conforme", erreur.to_string());
            corps.regles_non_respectees = Some(regles.regles_non_respectees.clone());
            (StatusCode::UNPROCESSABLE_ENTITY, Json(corps)).into_response()
        }
        ErreurReinitialisation::MotDePasseIdentiquePatient => {
            reponse_erreur(StatusCode::UNPROCESSABLE_ENTITY, "mot_de_passe_identique_patient", erreur.to_string())
                .into_response()
        }
        ErreurReinitialisation::Interne(etape) => {
            reponse_erreur_interne("réinitialisation du mot de passe", etape).into_response()
        }
    }
}

/// `POST /api/mot-de-passe/reinitialiser` -- `{"jeton", "nouveau_mot_de_passe"}`,
/// envoyé par la page `/reinitialiser-mot-de-passe` (R21). 204 ; 400
/// `lien_invalide` ; 422 `mot_de_passe_non_conforme` ou
/// `mot_de_passe_identique_patient`.
pub async fn reinitialiser(State(state): State<AppState>, JsonApi(corps): JsonApi<ReinitialiserRequete>) -> Response {
    match reinitialisation::reinitialiser(
        &state.db,
        state.email.as_ref(),
        &state.contact_securite,
        &corps.jeton,
        &corps.nouveau_mot_de_passe,
    )
    .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(erreur) => reponse_erreur_reinitialisation(erreur),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_de_la_reinitialisation() {
        assert_eq!(reponse_erreur_reinitialisation(ErreurReinitialisation::LienInvalide).status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            reponse_erreur_reinitialisation(ErreurReinitialisation::MotDePasseIdentiquePatient).status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }

    /// Réponse identique (204) pour un email connu ou inconnu.
    #[sqlx::test]
    #[ignore]
    async fn oubli_toujours_204(pool: sqlx::PgPool) {
        use laaficare_backend::config::CleTotp;
        use laaficare_backend::jwt::JwtService;
        use laaficare_backend::reinitialisation::LimiteTaches;
        use laaficare_backend::sms::SmsSenderConsole;
        use std::sync::Arc;

        let state = AppState {
            db: pool,
            jwt: JwtService::new(&"a".repeat(32)),
            sms: Arc::new(SmsSenderConsole),
            cle_totp: Arc::new(CleTotp::depuis_octets([7; 32])),
            email: Arc::new(laaficare_backend::email::EmailSenderConsole),
            url_application_web: Arc::from("http://localhost:4200"),
            contact_securite: Arc::from("securite@example.org"),
            taches_reinitialisation: Arc::new(LimiteTaches::default()),
        };
        let u: sqlx::types::Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (telephone, email) VALUES ('+22670310001', 'connu@example.org') RETURNING id",
        )
        .fetch_one(&state.db)
        .await
        .unwrap();
        sqlx::query("UPDATE utilisateur SET email_verifie_le = now() WHERE id = $1").bind(u).execute(&state.db).await.unwrap();
        sqlx::query("INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'professionnel', 'x')")
            .bind(u)
            .execute(&state.db)
            .await
            .unwrap();

        for email in ["connu@example.org", "inconnu@example.org", "pas une adresse"] {
            let reponse = oubli_professionnel(State(state.clone()), JsonApi(OubliRequete { email: email.into() })).await;
            assert_eq!(reponse.status(), StatusCode::NO_CONTENT, "{email}");
        }
    }
}
