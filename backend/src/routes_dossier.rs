//! Routes du dossier médical (module médecin, étape 4 ; docs/api.md §2.20).
//! Toutes passent par une prise en charge de l'établissement de
//! l'affectation (`{id}` dans l'adresse), jamais par le NIP. La logique, les
//! vues par rôle et le journal sont dans `dossier_medical`.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sqlx::types::Uuid;

use crate::AppState;
use crate::erreur_api::{CheminApi, JsonApi, reponse_erreur};
use crate::extracteur_jwt::ProfessionnelEnExercice;
use crate::routes_acces_patient::reponse_erreur_acces;
use laaficare_backend::dossier_medical::{
    self as dossier, Allergie, Antecedent, ErreurDossier, NouvelAntecedent, NouvelleAllergie,
    NouvelleConsultation, NouvelleOrdonnance, OrdonnanceCreee,
};

fn reponse_erreur_dossier(erreur: ErreurDossier) -> Response {
    let message = erreur.to_string();
    let (statut, code) = match erreur {
        ErreurDossier::Acces(acces) => return reponse_erreur_acces(acces),
        ErreurDossier::DonneesInvalides(_) => {
            (StatusCode::UNPROCESSABLE_ENTITY, "donnees_invalides")
        }
        ErreurDossier::AllergieInconnue => (StatusCode::NOT_FOUND, "allergie_inconnue"),
        ErreurDossier::AllergieDejaRetiree => (StatusCode::CONFLICT, "deja_retiree"),
        ErreurDossier::ConsultationInconnue => (StatusCode::NOT_FOUND, "consultation_inconnue"),
        ErreurDossier::MedicamentInconnu => (StatusCode::NOT_FOUND, "medicament_inconnu"),
        ErreurDossier::AlerteAllergie(conflits) => {
            // CDC, BF-03 : alerte bloquante, avec la liste des conflits pour
            // que l'application les affiche (en rouge, règle du modèle). Même
            // forme que les autres erreurs, plus `conflits` (§12).
            let corps =
                json!({ "erreur": message, "code": "alerte_allergie", "conflits": conflits });
            return (StatusCode::CONFLICT, Json(corps)).into_response();
        }
    };
    reponse_erreur(statut, code, message).into_response()
}

/// `GET /api/prises-en-charge/{id}/dossier` : le dossier, dans la vue du
/// rôle (complet pour le médecin et l'infirmier, vue pharmacie pour le
/// pharmacien) ; 403 pour tout autre rôle.
pub async fn lire(
    ProfessionnelEnExercice(exercice): ProfessionnelEnExercice,
    State(state): State<AppState>,
    CheminApi(prise_en_charge_id): CheminApi<Uuid>,
) -> Result<Json<Map<String, Value>>, Response> {
    dossier::lire(&state.db, &exercice, prise_en_charge_id)
        .await
        .map(Json)
        .map_err(reponse_erreur_dossier)
}

/// `POST /api/prises-en-charge/{id}/allergies` (médecin, infirmier) : 201.
pub async fn ajouter_allergie(
    ProfessionnelEnExercice(exercice): ProfessionnelEnExercice,
    State(state): State<AppState>,
    CheminApi(prise_en_charge_id): CheminApi<Uuid>,
    JsonApi(corps): JsonApi<NouvelleAllergie>,
) -> Result<(StatusCode, Json<Allergie>), Response> {
    dossier::ajouter_allergie(&state.db, &exercice, prise_en_charge_id, &corps)
        .await
        .map(|a| (StatusCode::CREATED, Json(a)))
        .map_err(reponse_erreur_dossier)
}

#[derive(Deserialize)]
pub struct RetraitRequete {
    pub motif: String,
}

/// `POST /api/prises-en-charge/{id}/allergies/{allergie_id}/retirer`
/// (médecin) : 204.
pub async fn retirer_allergie(
    ProfessionnelEnExercice(exercice): ProfessionnelEnExercice,
    State(state): State<AppState>,
    CheminApi((prise_en_charge_id, allergie_id)): CheminApi<(Uuid, Uuid)>,
    JsonApi(corps): JsonApi<RetraitRequete>,
) -> Result<StatusCode, Response> {
    dossier::retirer_allergie(
        &state.db,
        &exercice,
        prise_en_charge_id,
        allergie_id,
        &corps.motif,
    )
    .await
    .map(|()| StatusCode::NO_CONTENT)
    .map_err(reponse_erreur_dossier)
}

/// `POST /api/prises-en-charge/{id}/antecedents` (médecin, infirmier) : 201.
pub async fn ajouter_antecedent(
    ProfessionnelEnExercice(exercice): ProfessionnelEnExercice,
    State(state): State<AppState>,
    CheminApi(prise_en_charge_id): CheminApi<Uuid>,
    JsonApi(corps): JsonApi<NouvelAntecedent>,
) -> Result<(StatusCode, Json<Antecedent>), Response> {
    dossier::ajouter_antecedent(&state.db, &exercice, prise_en_charge_id, &corps)
        .await
        .map(|a| (StatusCode::CREATED, Json(a)))
        .map_err(reponse_erreur_dossier)
}

#[derive(Serialize)]
pub struct ConsultationCreee {
    pub id: Uuid,
    pub date_consultation: DateTime<Utc>,
}

/// `POST /api/prises-en-charge/{id}/consultations` (médecin) : 201.
pub async fn creer_consultation(
    ProfessionnelEnExercice(exercice): ProfessionnelEnExercice,
    State(state): State<AppState>,
    CheminApi(prise_en_charge_id): CheminApi<Uuid>,
    JsonApi(corps): JsonApi<NouvelleConsultation>,
) -> Result<(StatusCode, Json<ConsultationCreee>), Response> {
    dossier::creer_consultation(&state.db, &exercice, prise_en_charge_id, &corps)
        .await
        .map(|(id, date_consultation)| {
            (
                StatusCode::CREATED,
                Json(ConsultationCreee {
                    id,
                    date_consultation,
                }),
            )
        })
        .map_err(reponse_erreur_dossier)
}

#[derive(Deserialize)]
pub struct AjoutRequete {
    pub texte: String,
}

/// `POST /api/prises-en-charge/{id}/consultations/{consultation_id}/ajouts`
/// (médecin) : 204.
pub async fn ajouter_a_consultation(
    ProfessionnelEnExercice(exercice): ProfessionnelEnExercice,
    State(state): State<AppState>,
    CheminApi((prise_en_charge_id, consultation_id)): CheminApi<(Uuid, Uuid)>,
    JsonApi(corps): JsonApi<AjoutRequete>,
) -> Result<StatusCode, Response> {
    dossier::ajouter_a_consultation(
        &state.db,
        &exercice,
        prise_en_charge_id,
        consultation_id,
        &corps.texte,
    )
    .await
    .map(|()| StatusCode::NO_CONTENT)
    .map_err(reponse_erreur_dossier)
}

/// `POST /api/prises-en-charge/{id}/ordonnances` (médecin) : 201 ; 409
/// `alerte_allergie` avec `conflits` sans motif de dépassement.
pub async fn prescrire(
    ProfessionnelEnExercice(exercice): ProfessionnelEnExercice,
    State(state): State<AppState>,
    CheminApi(prise_en_charge_id): CheminApi<Uuid>,
    JsonApi(corps): JsonApi<NouvelleOrdonnance>,
) -> Result<(StatusCode, Json<OrdonnanceCreee>), Response> {
    dossier::prescrire(&state.db, &exercice, prise_en_charge_id, &corps)
        .await
        .map(|o| (StatusCode::CREATED, Json(o)))
        .map_err(reponse_erreur_dossier)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use sqlx::types::Uuid;

    use crate::routes_acces_patient::tests_http::{patient, requete, serveur};
    use crate::routes_pharmacie::tests::{catalogue_fictif, en_exercice, etat_de_test};

    /// Parcours complet par le routeur du serveur : le patient crée son
    /// code de soin ; le scan du médecin ne rend aucune donnée du dossier ;
    /// le NIP seul n'ouvre rien ; le médecin ouvre une prise en charge avec
    /// le code, déclare une allergie, reçoit l'alerte bloquante (409 avec
    /// les conflits) ; l'accueil est refusé sur le dossier ; un médecin
    /// d'une autre clinique ne voit rien ; le patient voit tout dans son
    /// journal.
    #[sqlx::test]
    #[ignore]
    async fn parcours_du_dossier_par_http(pool: sqlx::PgPool) {
        let state = etat_de_test(&pool);
        let adresse = serveur(state.clone()).await;
        let (x, _, _) = catalogue_fictif(&pool).await;
        let dci_a: Uuid = sqlx::query_scalar("SELECT id FROM dci WHERE nom = 'Molécule-A fictive'")
            .fetch_one(&pool)
            .await
            .unwrap();
        let (_, nip, jeton_patient) = patient(&state, "+22670410001").await;
        let hachage = laaficare_backend::mot_de_passe::hacher_async("Soin-2026!")
            .await
            .unwrap();
        sqlx::query(
            "UPDATE compte SET mot_de_passe_hash = $1 WHERE type_compte = 'patient' \
             AND utilisateur_id = (SELECT utilisateur_id FROM patient WHERE nip = $2)",
        )
        .bind(hachage.as_str())
        .bind(&nip)
        .execute(&pool)
        .await
        .unwrap();
        let (jeton_medecin, clinique) = en_exercice(
            &state,
            "+22670410002",
            "clinique",
            "validee",
            "medecin",
            None,
        )
        .await;
        let (jeton_accueil, _) = en_exercice(
            &state,
            "+22670410003",
            "clinique",
            "validee",
            "agent_accueil",
            Some(clinique),
        )
        .await;
        let (jeton_ailleurs, _) = en_exercice(
            &state,
            "+22670410004",
            "clinique",
            "validee",
            "medecin",
            None,
        )
        .await;

        // Code de soin : mot de passe exigé, réponse jamais mise en cache.
        let (statut, _) = requete(
            adresse,
            "POST",
            "/api/patients/code-soin",
            &jeton_patient,
            Some(json!({ "mot_de_passe": "faux" })),
        )
        .await;
        assert_eq!(statut, 401);
        let (statut, code_soin) = requete(
            adresse,
            "POST",
            "/api/patients/code-soin",
            &jeton_patient,
            Some(json!({ "mot_de_passe": "Soin-2026!" })),
        )
        .await;
        assert_eq!(statut, 201, "{code_soin}");
        let secret =
            totp_rs::Secret::try_from_base32(code_soin["cle_base32"].as_str().unwrap()).unwrap();
        let code = totp_rs::Builder::new()
            .with_secret(secret)
            .build()
            .unwrap()
            .generate_current()
            .to_string();

        // Le scan ne rend rien du dossier.
        let (statut, fiche) = requete(
            adresse,
            "POST",
            "/api/professionnels/patients/scan",
            &jeton_medecin,
            Some(json!({ "nip": nip })),
        )
        .await;
        assert_eq!(statut, 200);
        for interdit in ["allergies", "consultations", "ordonnances", "telephone"] {
            assert!(fiche.get(interdit).is_none(), "{interdit}");
        }
        // Aucune route du dossier ne prend un NIP.
        let (statut, _) = requete(
            adresse,
            "GET",
            &format!("/api/prises-en-charge/{nip}/dossier"),
            &jeton_medecin,
            None,
        )
        .await;
        assert_eq!(
            statut, 400,
            "un NIP n'est pas un identifiant de prise en charge"
        );
        let (statut, erreur) = requete(
            adresse,
            "GET",
            &format!("/api/prises-en-charge/{}/dossier", Uuid::max()),
            &jeton_medecin,
            None,
        )
        .await;
        assert_eq!(
            (statut, erreur["code"].as_str()),
            (404, Some("prise_en_charge_inconnue"))
        );

        // Prise en charge avec le code de soin.
        let (statut, prise) = requete(
            adresse,
            "POST",
            "/api/prises-en-charge",
            &jeton_medecin,
            Some(json!({ "nip": nip, "code_soin": code })),
        )
        .await;
        assert_eq!(statut, 201, "{prise}");
        let prise_id = prise["id"].as_str().unwrap().to_string();
        let chemin = |fin: &str| format!("/api/prises-en-charge/{prise_id}/{fin}");

        let (statut, _) = requete(
            adresse,
            "POST",
            &chemin("allergies"),
            &jeton_medecin,
            Some(json!({ "substance": "Molécule-A", "dci_id": dci_a })),
        )
        .await;
        assert_eq!(statut, 201);
        let (statut, alerte) = requete(
            adresse,
            "POST",
            &chemin("ordonnances"),
            &jeton_medecin,
            Some(json!({ "lignes": [{ "medicament_id": x, "posologie": "1 par jour", "quantite": 5 }] })),
        )
        .await;
        assert_eq!(
            (statut, alerte["code"].as_str()),
            (409, Some("alerte_allergie"))
        );
        assert_eq!(alerte["conflits"][0]["substance"], "Molécule-A");

        let (statut, dossier) =
            requete(adresse, "GET", &chemin("dossier"), &jeton_medecin, None).await;
        assert_eq!(statut, 200);
        assert_eq!(dossier["allergies"][0]["substance"], "Molécule-A");
        let (statut, erreur) =
            requete(adresse, "GET", &chemin("dossier"), &jeton_accueil, None).await;
        assert_eq!(
            (statut, erreur["code"].as_str()),
            (403, Some("acces_refuse"))
        );
        let (statut, erreur) =
            requete(adresse, "GET", &chemin("dossier"), &jeton_ailleurs, None).await;
        assert_eq!(
            (statut, erreur["code"].as_str()),
            (404, Some("prise_en_charge_inconnue"))
        );

        let (statut, journal) = requete(
            adresse,
            "GET",
            "/api/patients/journal-acces",
            &jeton_patient,
            None,
        )
        .await;
        assert_eq!(statut, 200);
        let natures: Vec<&str> = journal["acces"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["nature"].as_str().unwrap())
            .collect();
        assert_eq!(
            natures,
            [
                "lecture_dossier",
                "lecture_dossier",
                "ecriture_dossier",
                "ouverture_prise_en_charge",
                "scan"
            ]
        );
        assert_eq!(journal["acces"][3]["moyen"], "code_soin");
    }
}
