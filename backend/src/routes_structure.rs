//! Routes de la demande de structure (étape 3.7 réduite à la MUNASEB,
//! décisions E1 à E13 du 2026-10-20 ; contrat dans docs/api.md). Jeton
//! patient du futur responsable (E4). Même principe que les autres fichiers
//! de routes : désérialiser, appeler `structure.rs`, traduire le résultat.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::extract::rejection::BytesRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use sqlx::types::Uuid;

use crate::AppState;
use crate::erreur_api::{
    CheminApi, Complement, ErreurApi, JsonApi, RequeteApi, reponse_erreur, reponse_erreur_interne,
};
use crate::extracteur_jwt::PatientAuthentifie;
use laaficare_backend::piece_justificative::{self, ErreurPiece, TypePiece};
use laaficare_backend::structure::{
    self, ErreurAbandon, ErreurSoumission, ErreurStructure, NouvelleStructure, TypeStructure,
};

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
        ErreurStructure::TypeNonDisponible => {
            (StatusCode::UNPROCESSABLE_ENTITY, "type_non_disponible")
        }
        ErreurStructure::NomTropLong => (StatusCode::UNPROCESSABLE_ENTITY, "nom_trop_long"),
        ErreurStructure::NomInvalide => (StatusCode::UNPROCESSABLE_ENTITY, "nom_invalide"),
        // Un code par numéro (décision du porteur, 2026-10-21).
        ErreurStructure::TelephoneServiceInvalide => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "telephone_service_invalide",
        ),
        ErreurStructure::TelephoneUrgenceInvalide => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "telephone_urgence_invalide",
        ),
        ErreurStructure::LocalisationHorsBurkina => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "localisation_hors_burkina",
        ),
        ErreurStructure::MotDePasseNonConforme(regles) => {
            let mut corps = ErreurApi::simple("mot_de_passe_non_conforme", erreur.to_string());
            corps.regles_non_respectees = Some(regles.regles_non_respectees.clone());
            return (StatusCode::UNPROCESSABLE_ENTITY, Json(corps)).into_response();
        }
        // Même code que la réauthentification du réenrôlement (A7) : le
        // jeton patient reste valable, seul le mot de passe saisi est faux.
        ErreurStructure::MotDePasseIncorrect => {
            (StatusCode::UNAUTHORIZED, "identifiants_invalides")
        }
        ErreurStructure::SecondFacteurRequis => (StatusCode::CONFLICT, "second_facteur_requis"),
        ErreurStructure::EmailDifferent => (StatusCode::CONFLICT, "email_different"),
        ErreurStructure::EmailDejaUtilise => (StatusCode::CONFLICT, "email_deja_utilise"),
        ErreurStructure::DemandeDejaEnCours => (StatusCode::CONFLICT, "demande_deja_en_cours"),
        ErreurStructure::Interne(etape) => {
            return reponse_erreur_interne("création de structure", etape).into_response();
        }
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
    match structure::creer(
        &state.db,
        state.email.as_ref(),
        &state.url_application_web,
        patient.compte_id,
        &saisie,
    )
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
        ErreurSoumission::PiecesManquantes { pieces } => {
            let complement = Complement {
                pieces_manquantes: Some(pieces.clone()),
                ..Default::default()
            };
            let corps = ErreurApi::avec("pieces_manquantes", erreur.to_string(), complement);
            return (StatusCode::CONFLICT, Json(corps)).into_response();
        }
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

fn reponse_erreur_abandon(erreur: ErreurAbandon) -> Response {
    match &erreur {
        ErreurAbandon::StructureInconnue => reponse_erreur(
            StatusCode::NOT_FOUND,
            "structure_inconnue",
            erreur.to_string(),
        )
        .into_response(),
        ErreurAbandon::TransitionInvalide { statut_actuel } => {
            let mut corps = ErreurApi::simple("transition_invalide", erreur.to_string());
            corps.statut_actuel = Some(statut_actuel.clone());
            (StatusCode::CONFLICT, Json(corps)).into_response()
        }
        ErreurAbandon::Interne(etape) => {
            reponse_erreur_interne("abandon de structure", etape).into_response()
        }
    }
}

/// `POST /api/structures/{id}/abandonner` -- jeton patient du créateur,
/// sans corps (AB1). 204 ; 404 `structure_inconnue` ; 409
/// `transition_invalide` avec `statut_actuel`. L'application demande une
/// confirmation avant l'appel (AB4) : l'abandon est définitif.
pub async fn abandonner(
    patient: PatientAuthentifie,
    State(state): State<AppState>,
    CheminApi(structure_id): CheminApi<Uuid>,
) -> Response {
    match structure::abandonner(&state.db, patient.compte_id, structure_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(erreur) => reponse_erreur_abandon(erreur),
    }
}

/// `GET /api/structures/mes-demandes` -- jeton patient. 200 avec
/// `email_a_verifier` et les structures créées par la personne.
pub async fn mes_demandes(patient: PatientAuthentifie, State(state): State<AppState>) -> Response {
    match structure::mes_demandes(&state.db, patient.compte_id).await {
        Ok(demandes) => (StatusCode::OK, Json(demandes)).into_response(),
        Err(_) => reponse_erreur_interne("suivi des demandes de structure", "erreur de la base")
            .into_response(),
    }
}

// ---------------------------------------------------------------------
// Pièces justificatives (reste de la 3.7, 2026-11-16)
// ---------------------------------------------------------------------

/// Paramètres de requête du dépôt : le fichier lui-même est le corps.
#[derive(Deserialize)]
pub struct DepotPiece {
    pub numero: String,
    pub date_piece: chrono::NaiveDate,
}

fn reponse_erreur_piece(erreur: ErreurPiece) -> Response {
    let (statut, code) = match &erreur {
        ErreurPiece::StructureInconnue => (StatusCode::NOT_FOUND, "structure_inconnue"),
        ErreurPiece::TransitionInvalide { statut_actuel } => {
            let mut corps = ErreurApi::simple("transition_invalide", erreur.to_string());
            corps.statut_actuel = Some(statut_actuel.clone());
            return (StatusCode::CONFLICT, Json(corps)).into_response();
        }
        ErreurPiece::PieceNonPrevue => (StatusCode::UNPROCESSABLE_ENTITY, "piece_non_prevue"),
        // RFC 9110 §15.5.16 : 415 pour un format de contenu refusé.
        ErreurPiece::FormatRefuse => (StatusCode::UNSUPPORTED_MEDIA_TYPE, "format_refuse"),
        ErreurPiece::FichierVide => (StatusCode::UNPROCESSABLE_ENTITY, "fichier_vide"),
        // RFC 9110 §15.5.14 : 413 pour un contenu trop grand.
        ErreurPiece::FichierTropGrand => (StatusCode::PAYLOAD_TOO_LARGE, "fichier_trop_grand"),
        ErreurPiece::DonneesInvalides(_) => (StatusCode::UNPROCESSABLE_ENTITY, "donnees_invalides"),
        ErreurPiece::Interne(detail) => {
            return reponse_erreur_interne("pièce justificative", detail).into_response();
        }
    };
    reponse_erreur(statut, code, erreur.to_string()).into_response()
}

/// `POST /api/structures/{id}/pieces/{type_piece}?numero=&date_piece=` --
/// jeton patient du créateur ; le corps est le fichier, tel quel (une
/// requête par fichier, réseau faible, §14). Le type vient des premiers
/// octets, jamais de l'en-tête `Content-Type` ni d'un nom de fichier. La
/// limite de taille est posée sur la route (`DefaultBodyLimit`, doc axum
/// 0.8.9) : au-delà, axum refuse avant de tout lire.
pub async fn deposer_piece(
    patient: PatientAuthentifie,
    State(state): State<AppState>,
    CheminApi((structure_id, type_piece)): CheminApi<(Uuid, TypePiece)>,
    RequeteApi(depot): RequeteApi<DepotPiece>,
    corps: Result<Bytes, BytesRejection>,
) -> Response {
    let octets = match corps {
        Ok(octets) => octets,
        // Le seul rejet attendu ici est le dépassement de la limite de la
        // route ; tout autre rejet (corps illisible) garde son code d'axum.
        Err(rejet) if rejet.status() == StatusCode::PAYLOAD_TOO_LARGE => {
            return reponse_erreur_piece(ErreurPiece::FichierTropGrand);
        }
        Err(rejet) => {
            return reponse_erreur(rejet.status(), "corps_illisible", rejet.body_text())
                .into_response();
        }
    };
    match piece_justificative::deposer(
        &state.db,
        patient.compte_id,
        structure_id,
        type_piece,
        &depot.numero,
        depot.date_piece,
        &octets,
    )
    .await
    {
        Ok(piece) => (StatusCode::CREATED, Json(piece)).into_response(),
        Err(erreur) => reponse_erreur_piece(erreur),
    }
}

#[derive(Serialize)]
struct ListePieces {
    pieces: Vec<piece_justificative::Piece>,
}

/// `GET /api/structures/{id}/pieces` -- jeton patient du créateur : les
/// pièces actives, sans leur contenu.
pub async fn lister_pieces(
    patient: PatientAuthentifie,
    State(state): State<AppState>,
    CheminApi(structure_id): CheminApi<Uuid>,
) -> Response {
    match piece_justificative::lister_pour_createur(&state.db, patient.compte_id, structure_id)
        .await
    {
        Ok(Some(pieces)) => (StatusCode::OK, Json(ListePieces { pieces })).into_response(),
        Ok(None) => reponse_erreur_piece(ErreurPiece::StructureInconnue),
        Err(_) => reponse_erreur_interne("liste des pièces", "erreur de la base").into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// La vraie route, servie sur un port local (aucune dépendance ajoutée :
    /// requête HTTP/1.1 écrite à la main, RFC 9112) :
    /// - un dépôt de 5 Mio + 1 octet est refusé en 413 `fichier_trop_grand`
    ///   par la limite posée sur la route (`DefaultBodyLimit`) ;
    /// - un PDF fictif est accepté (201), puis listé ;
    /// - le jeton d'une autre personne reçoit 404 `structure_inconnue`.
    #[sqlx::test]
    #[ignore]
    async fn depot_par_la_route_et_limite_de_taille(pool: sqlx::PgPool) {
        use laaficare_backend::config::CleTotp;
        use laaficare_backend::jwt::{JwtService, SorteJeton};
        use laaficare_backend::session;
        use laaficare_backend::sms::SmsSenderConsole;
        use std::sync::Arc;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let state = AppState {
            db: pool.clone(),
            jwt: JwtService::new(&"a".repeat(32)),
            sms: Arc::new(SmsSenderConsole),
            cle_totp: Arc::new(CleTotp::depuis_octets([7; 32])),
            email: Arc::new(laaficare_backend::email::EmailSenderConsole),
            url_application_web: Arc::from("http://localhost:4200"),
            contact_securite: Arc::from("securite@example.org"),
            taches_reinitialisation: Arc::new(Default::default()),
        };
        // Créateur (patient et professionnel de la même identité) et sa
        // pharmacie en brouillon ; une autre personne.
        let jeton_patient = |telephone: &'static str| {
            let state = state.clone();
            async move {
                let u: Uuid = sqlx::query_scalar("INSERT INTO utilisateur (nom, prenom, telephone) VALUES ('OUEDRAOGO', 'Awa', $1) RETURNING id")
                    .bind(telephone)
                    .fetch_one(&state.db)
                    .await
                    .unwrap();
                let patient: Uuid = sqlx::query_scalar(
                    "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'patient', 'x') RETURNING id",
                )
                .bind(u)
                .fetch_one(&state.db)
                .await
                .unwrap();
                let session = session::ouvrir(&state.db, patient, None).await.unwrap();
                let jeton = state
                    .jwt
                    .emettre(
                        SorteJeton::Patient,
                        patient,
                        0,
                        None,
                        Some(session.session_id),
                    )
                    .unwrap();
                (u, format!("Bearer {jeton}"))
            }
        };
        let (createur, jeton) = jeton_patient("+22670520001").await;
        let (_, jeton_intrus) = jeton_patient("+22670520002").await;
        let structure_id: Uuid = sqlx::query_scalar(
            "WITH c AS (INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'professionnel', 'x') RETURNING id) \
             INSERT INTO structure (type_structure, nom, telephone, telephone_urgence, region, email, commune, province, \
                                    latitude, longitude, createur_compte_id) \
             SELECT 'pharmacie', 'Pharmacie fictive', '+22670000000', '+22670000000', 'Région fictive', 'p@example.org', \
                    'Commune fictive', 'Province fictive', 12.3714, -1.5197, id FROM c RETURNING id",
        )
        .bind(createur)
        .fetch_one(&pool)
        .await
        .unwrap();

        // Le routeur du serveur, avec toutes ses couches (limites de taille,
        // identifiant de requête), pas une copie.
        let app = crate::routeur(state);
        let ecoute = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let adresse = ecoute.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(ecoute, app).await.unwrap() });

        // Statut et texte d'une réponse, connexion fermée ensuite.
        let envoyer_avec =
            |methode: &'static str, chemin: String, entetes: String, corps: Vec<u8>| async move {
                let mut flux = tokio::net::TcpStream::connect(adresse).await.unwrap();
                let entete = format!(
                    "{methode} {chemin} HTTP/1.1\r\nHost: localhost\r\n{entetes}\
                 Content-Length: {}\r\nConnection: close\r\n\r\n",
                    corps.len()
                );
                flux.write_all(entete.as_bytes()).await.unwrap();
                // Le serveur peut répondre 413 et fermer avant la fin du corps.
                let _ = flux.write_all(&corps).await;
                let mut reponse = Vec::new();
                let _ = flux.read_to_end(&mut reponse).await;
                let texte = String::from_utf8_lossy(&reponse).to_string();
                let statut: u16 = texte.split(' ').nth(1).unwrap().parse().unwrap();
                (statut, texte)
            };
        let envoyer =
            |methode: &'static str, chemin: String, autorisation: String, corps: Vec<u8>| {
                envoyer_avec(
                    methode,
                    chemin,
                    format!("Authorization: {autorisation}\r\n"),
                    corps,
                )
            };
        let chemin = format!(
            "/api/structures/{structure_id}/pieces/autorisation_ouverture_exploitation?numero=2025-001&date_piece=2025-01-10"
        );

        let mut trop_grand = b"%PDF-".to_vec();
        trop_grand.resize(piece_justificative::TAILLE_MAX_OCTETS + 1, 0);
        let (statut, texte) = envoyer("POST", chemin.clone(), jeton.clone(), trop_grand).await;
        assert_eq!(statut, 413, "{texte}");
        assert!(texte.contains("fichier_trop_grand"), "{texte}");

        // Une pièce d'exactement 5 Mio passe la limite du routeur (256 Kio) :
        // la limite propre à la route l'emporte.
        let mut juste = b"%PDF-".to_vec();
        juste.resize(piece_justificative::TAILLE_MAX_OCTETS, 0);
        let (statut, texte) = envoyer("POST", chemin.clone(), jeton.clone(), juste).await;
        assert_eq!(statut, 201, "{}", &texte[..texte.len().min(300)]);

        // Toute autre route : 256 Kio au plus, réponse au format de l'API.
        let mut gros_json = br#"{"telephone":"70000000","mot_de_passe":""#.to_vec();
        gros_json.resize(300 * 1024, b'a');
        gros_json.extend_from_slice(br#""}"#);
        let (statut, texte) = envoyer_avec(
            "POST",
            "/api/patients/connexion".into(),
            "Content-Type: application/json\r\n".into(),
            gros_json,
        )
        .await;
        assert_eq!(statut, 413, "{texte}");
        assert!(texte.contains("\"code\""), "{texte}");

        // Identifiant de requête : tiré par le serveur (UUID), jamais repris
        // de la requête (un client pourrait y mettre une donnée personnelle).
        let (statut, texte) = envoyer_avec(
            "GET",
            "/health".into(),
            "X-Request-ID: OUEDRAOGO-Awa-70000000\r\n".into(),
            Vec::new(),
        )
        .await;
        assert_eq!(statut, 200, "{texte}");
        let id = texte
            .lines()
            .find_map(|l| {
                l.to_ascii_lowercase()
                    .strip_prefix("x-request-id: ")
                    .map(str::to_string)
            })
            .expect("en-tête x-request-id dans la réponse");
        assert!(Uuid::parse_str(id.trim()).is_ok(), "{id}");
        assert!(!texte.contains("OUEDRAOGO"), "{texte}");

        let (statut, texte) = envoyer(
            "POST",
            chemin.clone(),
            jeton_intrus.clone(),
            b"%PDF-1.7".to_vec(),
        )
        .await;
        assert_eq!(statut, 404, "{texte}");

        let (statut, texte) =
            envoyer("POST", chemin, jeton.clone(), b"%PDF-1.7 fictif".to_vec()).await;
        assert_eq!(statut, 201, "{texte}");
        assert!(
            texte.contains("\"type_mime\":\"application/pdf\""),
            "{texte}"
        );

        let (statut, texte) = envoyer(
            "GET",
            format!("/api/structures/{structure_id}/pieces"),
            jeton,
            Vec::new(),
        )
        .await;
        assert_eq!(statut, 200, "{texte}");
        assert!(
            texte.contains("2025-001") && !texte.contains("fictif"),
            "{texte}"
        );
    }

    #[test]
    fn codes_des_erreurs_de_creation() {
        for (erreur, statut, code) in [
            (
                ErreurStructure::ChampObligatoire("nom"),
                StatusCode::UNPROCESSABLE_ENTITY,
                "donnees_invalides",
            ),
            (
                ErreurStructure::TypeNonDisponible,
                StatusCode::UNPROCESSABLE_ENTITY,
                "type_non_disponible",
            ),
            (
                ErreurStructure::TelephoneServiceInvalide,
                StatusCode::UNPROCESSABLE_ENTITY,
                "telephone_service_invalide",
            ),
            (
                ErreurStructure::TelephoneUrgenceInvalide,
                StatusCode::UNPROCESSABLE_ENTITY,
                "telephone_urgence_invalide",
            ),
            (
                ErreurStructure::EmailInvalide("email"),
                StatusCode::UNPROCESSABLE_ENTITY,
                "donnees_invalides",
            ),
            (
                ErreurStructure::LocalisationHorsBurkina,
                StatusCode::UNPROCESSABLE_ENTITY,
                "localisation_hors_burkina",
            ),
            (
                ErreurStructure::MotDePasseIncorrect,
                StatusCode::UNAUTHORIZED,
                "identifiants_invalides",
            ),
            (
                ErreurStructure::DemandeDejaEnCours,
                StatusCode::CONFLICT,
                "demande_deja_en_cours",
            ),
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
            taches_reinitialisation: Arc::new(Default::default()),
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
        let paire = session::ouvrir(&state.db, compte_id, None)
            .await
            .unwrap()
            .en_paire(&state.jwt)
            .unwrap();
        let en_tete = format!("Bearer {}", paire.jeton_acces);
        let patient = || async {
            let mut parts = Request::builder()
                .header(AUTHORIZATION, en_tete.as_str())
                .body(())
                .unwrap()
                .into_parts()
                .0;
            PatientAuthentifie::from_request_parts(&mut parts, &state)
                .await
                .unwrap()
        };
        let lire = |reponse: Response| async move {
            serde_json::from_slice::<serde_json::Value>(
                &to_bytes(reponse.into_body(), usize::MAX).await.unwrap(),
            )
            .unwrap()
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
        assert_eq!(
            (json["statut"].as_str(), json["email_a_verifier"].as_bool()),
            (Some("brouillon"), Some(true))
        );
        let structure_id: Uuid = serde_json::from_value(json["structure_id"].clone()).unwrap();

        let reponse = soumettre(
            patient().await,
            State(state.clone()),
            CheminApi(structure_id),
        )
        .await;
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
        assert!(
            serde_json::from_value::<CreationRequete>(serde_json::json!({"type": "ecole"}))
                .is_err()
        );
    }
}
