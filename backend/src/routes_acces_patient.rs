//! Routes de l'accès aux données d'un patient (module médecin ; décisions
//! du porteur du 2026-11-17 ; docs/api.md §2.18) :
//! - `POST /api/professionnels/patients/scan` : le scan du NIP, qui renvoie
//!   une fiche d'identité selon le rôle, jamais le dossier ;
//! - `GET /api/patients/journal-acces` : le patient lit qui a accédé à ses
//!   données.
//!
//! La logique et le journal sont dans `acces_patient` ; ici, seulement la
//! traduction HTTP.

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header::RETRY_AFTER};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sqlx::types::Uuid;

use crate::AppState;
use crate::erreur_api::{
    JsonApi, ReponseErreur, RequeteApi, reponse_erreur, reponse_erreur_interne,
};
use crate::extracteur_jwt::{PatientAuthentifie, ProfessionnelEnExercice};
use crate::routes_pharmacie::limite_valide;
use laaficare_backend::acces_patient::{self, AccesVu, ErreurAcces};

/// Traduction des erreurs d'accès, partagée avec la prise en charge et le
/// dossier. 429 avec `Retry-After` (RFC 6585 §4), comme les autres limites.
pub(crate) fn reponse_erreur_acces(erreur: ErreurAcces) -> Response {
    match erreur {
        ErreurAcces::RoleNonAutorise => {
            reponse_erreur(StatusCode::FORBIDDEN, "acces_refuse", "accès refusé").into_response()
        }
        ErreurAcces::PatientInconnu => {
            reponse_erreur(StatusCode::NOT_FOUND, "patient_inconnu", erreur.to_string())
                .into_response()
        }
        ErreurAcces::TropDEssais {
            reessayer_dans_secs,
        } => {
            let mut reponse = reponse_erreur(
                StatusCode::TOO_MANY_REQUESTS,
                "trop_de_demandes",
                erreur.to_string(),
            )
            .into_response();
            reponse
                .headers_mut()
                .insert(RETRY_AFTER, HeaderValue::from(reessayer_dans_secs));
            reponse
        }
        ErreurAcces::Interne(detail) => {
            reponse_erreur_interne("accès patient", &detail).into_response()
        }
    }
}

#[derive(Deserialize)]
pub struct ScanRequete {
    /// Le NIP lu sur le QR code ou saisi. Dans le corps, jamais dans
    /// l'adresse : une adresse finit dans des journaux.
    pub nip: String,
}

/// `POST /api/professionnels/patients/scan` : 200 et la fiche du rôle ;
/// 403 `acces_refuse` (rôle sans droit) ; 404 `patient_inconnu` (NIP
/// inconnu ou mal formé, jamais distingués) ; 429 `trop_de_demandes`.
pub async fn scanner(
    ProfessionnelEnExercice(exercice): ProfessionnelEnExercice,
    State(state): State<AppState>,
    JsonApi(corps): JsonApi<ScanRequete>,
) -> Result<Json<Map<String, Value>>, Response> {
    acces_patient::scanner(&state.db, &exercice, &corps.nip)
        .await
        .map(Json)
        .map_err(reponse_erreur_acces)
}

#[derive(Deserialize)]
pub struct PageJournal {
    pub limite: Option<i64>,
    pub avant: Option<Uuid>,
}

#[derive(Serialize)]
pub struct JournalReponse {
    pub acces: Vec<AccesVu>,
    pub suivant: Option<Uuid>,
}

/// `GET /api/patients/journal-acces?limite=&avant=` : le journal du patient
/// du jeton, le plus récent d'abord (décisions MD4 et MD5).
pub async fn journal(
    patient: PatientAuthentifie,
    State(state): State<AppState>,
    RequeteApi(page): RequeteApi<PageJournal>,
) -> Result<Json<JournalReponse>, ReponseErreur> {
    let limite = limite_valide(page.limite)?;
    let acces = acces_patient::journal_du_patient(&state.db, patient.compte_id, page.avant, limite)
        .await
        .map_err(|e| reponse_erreur_interne("journal des accès", &e.to_string()))?;
    let suivant = if acces.len() as i64 == limite {
        acces.last().map(|a| a.id)
    } else {
        None
    };
    Ok(Json(JournalReponse { acces, suivant }))
}

/// Outils des tests par HTTP du module médecin : le routeur du serveur,
/// avec toutes ses couches, sur un port local, et des requêtes brutes.
#[cfg(test)]
pub(crate) mod tests_http {
    use std::net::SocketAddr;

    use laaficare_backend::jwt::SorteJeton;
    use laaficare_backend::{nip, session};
    use sqlx::types::Uuid;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use crate::AppState;

    pub(crate) async fn serveur(state: AppState) -> SocketAddr {
        let app = crate::routeur(state);
        let ecoute = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let adresse = ecoute.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(ecoute, app).await.unwrap() });
        adresse
    }

    /// Statut et corps JSON (`Null` si le corps est vide).
    pub(crate) async fn requete(
        adresse: SocketAddr,
        methode: &str,
        chemin: &str,
        autorisation: &str,
        corps: Option<serde_json::Value>,
    ) -> (u16, serde_json::Value) {
        let corps = corps.map(|c| c.to_string()).unwrap_or_default();
        let mut flux = tokio::net::TcpStream::connect(adresse).await.unwrap();
        let entete = format!(
            "{methode} {chemin} HTTP/1.1\r\nHost: localhost\r\nAuthorization: {autorisation}\r\n\
             Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            corps.len()
        );
        flux.write_all(entete.as_bytes()).await.unwrap();
        flux.write_all(corps.as_bytes()).await.unwrap();
        let mut reponse = Vec::new();
        flux.read_to_end(&mut reponse).await.unwrap();
        let texte = String::from_utf8_lossy(&reponse).to_string();
        let statut: u16 = texte.split(' ').nth(1).unwrap().parse().unwrap();
        let corps = texte.split("\r\n\r\n").nth(1).unwrap_or("");
        // Réponse découpée (« chunked ») : la ligne de taille précède le JSON.
        let json = corps
            .find(['{', '['])
            .and_then(|debut| {
                let fin = corps.rfind(['}', ']'])?;
                serde_json::from_str(&corps[debut..=fin]).ok()
            })
            .unwrap_or(serde_json::Value::Null);
        (statut, json)
    }

    /// Un patient fictif avec un jeton : (patient_id, NIP, en-tête).
    pub(crate) async fn patient(state: &AppState, telephone: &str) -> (Uuid, String, String) {
        let u: Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (nom, prenom, telephone) VALUES ('OUEDRAOGO', 'Awa', $1) RETURNING id",
        )
        .bind(telephone)
        .fetch_one(&state.db)
        .await
        .unwrap();
        let compte: Uuid = sqlx::query_scalar(
            "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'patient', 'x') RETURNING id",
        )
        .bind(u)
        .fetch_one(&state.db)
        .await
        .unwrap();
        let nip = nip::generer(&state.db).await.unwrap();
        let patient_id: Uuid = sqlx::query_scalar(
            "INSERT INTO patient (utilisateur_id, nip, date_naissance, lieu_naissance) \
             VALUES ($1, $2, '1995-04-12', 'Koudougou') RETURNING id",
        )
        .bind(u)
        .bind(&nip)
        .fetch_one(&state.db)
        .await
        .unwrap();
        let session = session::ouvrir(&state.db, compte, None).await.unwrap();
        let jeton = state
            .jwt
            .emettre(
                SorteJeton::Patient,
                compte,
                0,
                None,
                Some(session.session_id),
            )
            .unwrap();
        (patient_id, nip, format!("Bearer {jeton}"))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use sqlx::types::Uuid;

    use super::tests_http::{patient, requete, serveur};
    use crate::routes_pharmacie::tests::{en_exercice, etat_de_test};

    /// Par le routeur du serveur : le scan d'un agent MUNASEB rend le
    /// téléphone, celui d'un responsable est refusé ; le détail d'un
    /// contrat rend le téléphone à l'agent ; le patient voit les deux
    /// accès dans son journal. Le NIP n'est dans aucune réponse.
    #[sqlx::test]
    #[ignore]
    async fn scan_detail_et_journal_par_http(pool: sqlx::PgPool) {
        let state = etat_de_test(&pool);
        let adresse = serveur(state.clone()).await;
        let (patient_id, nip, jeton_patient) = patient(&state, "+22670370001").await;
        let (jeton_agent, _) = en_exercice(
            &state,
            "+22670370002",
            "munaseb",
            "validee",
            "agent_assurance_munaseb",
            None,
        )
        .await;
        let (jeton_resp, _) = en_exercice(
            &state,
            "+22670370003",
            "clinique",
            "validee",
            "responsable",
            None,
        )
        .await;

        let (statut, fiche) = requete(
            adresse,
            "POST",
            "/api/professionnels/patients/scan",
            &jeton_agent,
            Some(json!({ "nip": nip })),
        )
        .await;
        assert_eq!(statut, 200, "{fiche}");
        assert_eq!(fiche["telephone"], "+22670370001");
        assert!(!fiche.to_string().contains(&nip));
        let (statut, erreur) = requete(
            adresse,
            "POST",
            "/api/professionnels/patients/scan",
            &jeton_resp,
            Some(json!({ "nip": nip })),
        )
        .await;
        assert_eq!(
            (statut, erreur["code"].as_str()),
            (403, Some("acces_refuse"))
        );
        let (statut, erreur) = requete(
            adresse,
            "POST",
            "/api/professionnels/patients/scan",
            &jeton_agent,
            Some(json!({ "nip": "0000000000000000" })),
        )
        .await;
        assert_eq!(
            (statut, erreur["code"].as_str()),
            (404, Some("patient_inconnu"))
        );

        let contrat_id: Uuid = sqlx::query_scalar(
            "INSERT INTO contrat_assurance_munaseb (patient_id, numero_carte, ufr, universite, num_matricule) \
             VALUES ($1, '0001/2026', 'UFR fictive', 'Université fictive', 'MAT-1') RETURNING id",
        )
        .bind(patient_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let (statut, detail) = requete(
            adresse,
            "GET",
            &format!("/api/assurance-munaseb/contrats/{contrat_id}"),
            &jeton_agent,
            None,
        )
        .await;
        assert_eq!(statut, 200, "{detail}");
        assert_eq!(detail["telephone"], "+22670370001");
        assert!(!detail.to_string().contains(&nip));
        // La recherche par carte ne rend pas le téléphone.
        let (statut, par_carte) = requete(
            adresse,
            "GET",
            "/api/assurance-munaseb/contrats?numero_carte=0001%2F2026",
            &jeton_agent,
            None,
        )
        .await;
        assert_eq!(statut, 200, "{par_carte}");
        assert!(par_carte.get("telephone").is_none());

        let (statut, journal) = requete(
            adresse,
            "GET",
            "/api/patients/journal-acces",
            &jeton_patient,
            None,
        )
        .await;
        assert_eq!(statut, 200, "{journal}");
        let natures: Vec<(&str, bool)> = journal["acces"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| {
                (
                    a["nature"].as_str().unwrap(),
                    a["telephone_affiche"].as_bool().unwrap(),
                )
            })
            .collect();
        assert_eq!(natures, [("detail_contrat", true), ("scan", true)]);
        assert_eq!(journal["acces"][0]["role"], "agent_assurance_munaseb");
        assert!(journal["acces"][0]["structure_nom"].is_string());
    }
}
