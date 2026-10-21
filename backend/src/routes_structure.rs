//! Routes de la demande de structure (étape 3.7 réduite à la MUNASEB,
//! décisions E1 à E13 du 2026-10-20 ; contrat dans docs/api.md). Jeton
//! patient du futur responsable (E4). Même principe que les autres fichiers
//! de routes : désérialiser, appeler `structure.rs`, traduire le résultat.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use sqlx::types::Uuid;

use crate::AppState;
use crate::erreur_api::{CheminApi, ErreurApi, JsonApi, reponse_erreur, reponse_erreur_interne};
use crate::extracteur_jwt::PatientAuthentifie;
use laaficare_backend::structure::{self, ErreurSoumission, ErreurStructure, NouvelleStructure, TypeStructure};

/// Corps de `POST /api/structures` (E1). Pas de `Debug` : le mot de passe
/// est un secret.
#[derive(Deserialize)]
pub struct CreationRequete {
    #[serde(rename = "type")]
    pub type_structure: TypeStructure,
    pub nom: String,
    pub telephone: String,
    pub telephone_urgence: String,
    pub region: String,
    pub province: String,
    pub commune: String,
    pub parcelle: Option<String>,
    pub lot: Option<String>,
    pub section: Option<String>,
    pub latitude: f64,
    pub longitude: f64,
    /// Email de la structure, obligatoire.
    pub email: String,
    /// Email de connexion du responsable, seulement sans compte
    /// professionnel (E1).
    pub email_professionnel: Option<String>,
    pub mot_de_passe_professionnel: String,
}

#[derive(Serialize)]
struct CreationReponse {
    structure_id: Uuid,
    statut: &'static str,
    email_a_verifier: bool,
}

fn reponse_erreur_structure(erreur: ErreurStructure) -> Response {
    let (statut, code) = match &erreur {
        ErreurStructure::ChampObligatoire(_) | ErreurStructure::EmailInvalide(_) => {
            (StatusCode::UNPROCESSABLE_ENTITY, "donnees_invalides")
        }
        ErreurStructure::TypeNonDisponible => (StatusCode::UNPROCESSABLE_ENTITY, "type_non_disponible"),
        ErreurStructure::NomTropLong => (StatusCode::UNPROCESSABLE_ENTITY, "nom_trop_long"),
        ErreurStructure::NomInvalide => (StatusCode::UNPROCESSABLE_ENTITY, "nom_invalide"),
        // Un code par numéro (décision du porteur, 2026-10-21).
        ErreurStructure::TelephoneServiceInvalide => (StatusCode::UNPROCESSABLE_ENTITY, "telephone_service_invalide"),
        ErreurStructure::TelephoneUrgenceInvalide => (StatusCode::UNPROCESSABLE_ENTITY, "telephone_urgence_invalide"),
        ErreurStructure::LocalisationHorsBurkina => (StatusCode::UNPROCESSABLE_ENTITY, "localisation_hors_burkina"),
        ErreurStructure::MotDePasseNonConforme(regles) => {
            let mut corps = ErreurApi::simple("mot_de_passe_non_conforme", erreur.to_string());
            corps.regles_non_respectees = Some(regles.regles_non_respectees.clone());
            return (StatusCode::UNPROCESSABLE_ENTITY, Json(corps)).into_response();
        }
        // Même code que la réauthentification du réenrôlement (A7) : le
        // jeton patient reste valable, seul le mot de passe saisi est faux.
        ErreurStructure::MotDePasseIncorrect => (StatusCode::UNAUTHORIZED, "identifiants_invalides"),
        ErreurStructure::SecondFacteurRequis => (StatusCode::CONFLICT, "second_facteur_requis"),
        ErreurStructure::EmailDifferent => (StatusCode::CONFLICT, "email_different"),
        ErreurStructure::EmailDejaUtilise => (StatusCode::CONFLICT, "email_deja_utilise"),
        ErreurStructure::DemandeDejaEnCours => (StatusCode::CONFLICT, "demande_deja_en_cours"),
        ErreurStructure::Interne(etape) => return reponse_erreur_interne("création de structure", etape).into_response(),
    };
    reponse_erreur(statut, code, erreur.to_string()).into_response()
}

/// `POST /api/structures` -- jeton patient. 201 `{structure_id, statut,
/// email_a_verifier}` ; erreurs dans docs/api.md.
pub async fn creer(
    patient: PatientAuthentifie,
    State(state): State<AppState>,
    JsonApi(corps): JsonApi<CreationRequete>,
) -> Response {
    let saisie = NouvelleStructure {
        type_structure: corps.type_structure,
        nom: &corps.nom,
        telephone: &corps.telephone,
        telephone_urgence: &corps.telephone_urgence,
        region: &corps.region,
        province: &corps.province,
        commune: &corps.commune,
        parcelle: corps.parcelle.as_deref(),
        lot: corps.lot.as_deref(),
        section: corps.section.as_deref(),
        latitude: corps.latitude,
        longitude: corps.longitude,
        email: &corps.email,
        email_professionnel: corps.email_professionnel.as_deref(),
        mot_de_passe_professionnel: &corps.mot_de_passe_professionnel,
    };
    match structure::creer(&state.db, state.email.as_ref(), &state.url_application_web, patient.compte_id, &saisie)
        .await
    {
        Ok(creee) => (
            StatusCode::CREATED,
            Json(CreationReponse {
                structure_id: creee.structure_id,
                statut: "brouillon",
                email_a_verifier: creee.email_a_verifier,
            }),
        )
            .into_response(),
        Err(erreur) => reponse_erreur_structure(erreur),
    }
}

fn reponse_erreur_soumission(erreur: ErreurSoumission) -> Response {
    let (statut, code) = match &erreur {
        ErreurSoumission::StructureInconnue => (StatusCode::NOT_FOUND, "structure_inconnue"),
        // Même forme que les transitions des demandes de remboursement (V2).
        ErreurSoumission::TransitionInvalide { statut_actuel } => {
            let mut corps = ErreurApi::simple("transition_invalide", erreur.to_string());
            corps.statut_actuel = Some(statut_actuel.clone());
            return (StatusCode::CONFLICT, Json(corps)).into_response();
        }
        // Même code et même message que la connexion professionnelle (P3).
        ErreurSoumission::EmailNonVerifie => (StatusCode::FORBIDDEN, "email_non_verifie"),
        ErreurSoumission::CompteProfessionnelDesactive => (StatusCode::FORBIDDEN, "acces_refuse"),
        ErreurSoumission::TypeNonDisponible => (StatusCode::CONFLICT, "type_non_disponible"),
        ErreurSoumission::Interne(etape) => {
            return reponse_erreur_interne("soumission de structure", etape).into_response();
        }
    };
    reponse_erreur(statut, code, erreur.to_string()).into_response()
}

/// `POST /api/structures/{id}/soumettre` -- jeton patient du créateur, sans
/// corps. 204 ; 404 `structure_inconnue` ; 409 `transition_invalide` ; 403
/// `email_non_verifie`.
pub async fn soumettre(
    patient: PatientAuthentifie,
    State(state): State<AppState>,
    CheminApi(structure_id): CheminApi<Uuid>,
) -> Response {
    match structure::soumettre(&state.db, patient.compte_id, structure_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(erreur) => reponse_erreur_soumission(erreur),
    }
}

/// `GET /api/structures/mes-demandes` -- jeton patient. 200 avec
/// `email_a_verifier` et les structures créées par la personne.
pub async fn mes_demandes(patient: PatientAuthentifie, State(state): State<AppState>) -> Response {
    match structure::mes_demandes(&state.db, patient.compte_id).await {
        Ok(demandes) => (StatusCode::OK, Json(demandes)).into_response(),
        Err(_) => reponse_erreur_interne("suivi des demandes de structure", "erreur de la base").into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_des_erreurs_de_creation() {
        for (erreur, statut, code) in [
            (ErreurStructure::ChampObligatoire("nom"), StatusCode::UNPROCESSABLE_ENTITY, "donnees_invalides"),
            (ErreurStructure::TypeNonDisponible, StatusCode::UNPROCESSABLE_ENTITY, "type_non_disponible"),
            (ErreurStructure::TelephoneServiceInvalide, StatusCode::UNPROCESSABLE_ENTITY, "telephone_service_invalide"),
            (ErreurStructure::TelephoneUrgenceInvalide, StatusCode::UNPROCESSABLE_ENTITY, "telephone_urgence_invalide"),
            (ErreurStructure::EmailInvalide("email"), StatusCode::UNPROCESSABLE_ENTITY, "donnees_invalides"),
            (ErreurStructure::LocalisationHorsBurkina, StatusCode::UNPROCESSABLE_ENTITY, "localisation_hors_burkina"),
            (ErreurStructure::MotDePasseIncorrect, StatusCode::UNAUTHORIZED, "identifiants_invalides"),
            (ErreurStructure::DemandeDejaEnCours, StatusCode::CONFLICT, "demande_deja_en_cours"),
        ] {
            assert_eq!(reponse_erreur_structure(erreur).status(), statut, "{code}");
        }
        assert_eq!(
            ErreurStructure::ChampObligatoire("region").to_string(),
            "Le champ « region » est obligatoire."
        );
    }

    /// De bout en bout, avec un vrai jeton patient : création (201),
    /// soumission refusée (403 `email_non_verifie`), suivi (200), type
    /// inconnu (422 par `JsonApi`).
    #[sqlx::test]
    #[ignore]
    async fn creation_et_suivi_de_bout_en_bout(pool: sqlx::PgPool) {
        use axum::body::to_bytes;
        use axum::extract::FromRequestParts;
        use axum::http::{Request, header::AUTHORIZATION};
        use laaficare_backend::config::CleTotp;
        use laaficare_backend::jwt::JwtService;
        use laaficare_backend::sms::SmsSenderConsole;
        use laaficare_backend::{mot_de_passe, session};
        use std::sync::Arc;

        let state = AppState {
            db: pool,
            jwt: JwtService::new(&"a".repeat(32)),
            sms: Arc::new(SmsSenderConsole),
            cle_totp: Arc::new(CleTotp::depuis_octets([7; 32])),
            email: Arc::new(laaficare_backend::email::EmailSenderConsole),
            url_application_web: Arc::from("http://localhost:4200"),
            contact_securite: Arc::from("securite@example.org"),
        };
        let utilisateur_id: Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (nom, prenom, telephone) VALUES ('OUEDRAOGO', 'Awa', '+22670160001') RETURNING id",
        )
        .fetch_one(&state.db)
        .await
        .unwrap();
        let compte_id: Uuid = sqlx::query_scalar(
            "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'patient', $2) RETURNING id",
        )
        .bind(utilisateur_id)
        .bind(mot_de_passe::hacher("Patient#2026").unwrap())
        .fetch_one(&state.db)
        .await
        .unwrap();
        let paire = session::ouvrir(&state.db, compte_id, None).await.unwrap().en_paire(&state.jwt).unwrap();
        let en_tete = format!("Bearer {}", paire.jeton_acces);
        let patient = || async {
            let mut parts = Request::builder().header(AUTHORIZATION, en_tete.as_str()).body(()).unwrap().into_parts().0;
            PatientAuthentifie::from_request_parts(&mut parts, &state).await.unwrap()
        };
        let lire = |reponse: Response| async move {
            serde_json::from_slice::<serde_json::Value>(&to_bytes(reponse.into_body(), usize::MAX).await.unwrap()).unwrap()
        };

        let corps: CreationRequete = serde_json::from_value(serde_json::json!({
            "type": "munaseb",
            "nom": "MUNASEB (test)",
            "telephone": "70000002",
            "telephone_urgence": "70000003",
            "region": "Région fictive",
            "province": "Province fictive",
            "commune": "Commune fictive",
            "latitude": 12.3714,
            "longitude": -1.5197,
            "email": "contact.munaseb@example.org",
            "email_professionnel": "responsable.route@example.org",
            "mot_de_passe_professionnel": "Professionnel#2026"
        }))
        .unwrap();
        let reponse = creer(patient().await, State(state.clone()), JsonApi(corps)).await;
        assert_eq!(reponse.status(), StatusCode::CREATED);
        let json = lire(reponse).await;
        assert_eq!((json["statut"].as_str(), json["email_a_verifier"].as_bool()), (Some("brouillon"), Some(true)));
        let structure_id: Uuid = serde_json::from_value(json["structure_id"].clone()).unwrap();

        let reponse = soumettre(patient().await, State(state.clone()), CheminApi(structure_id)).await;
        assert_eq!(reponse.status(), StatusCode::FORBIDDEN);
        assert_eq!(lire(reponse).await["code"], "email_non_verifie");

        let reponse = mes_demandes(patient().await, State(state.clone())).await;
        assert_eq!(reponse.status(), StatusCode::OK);
        let json = lire(reponse).await;
        assert_eq!(json["email_a_verifier"], true);
        assert_eq!(json["demandes"][0]["statut"], "brouillon");
        assert_eq!(json["demandes"][0]["type"], "munaseb");

        // Type inconnu : refusé à la lecture du corps, comme les autres
        // routes (`donnees_invalides`).
        assert!(serde_json::from_value::<CreationRequete>(serde_json::json!({"type": "ecole"})).is_err());
    }
}
