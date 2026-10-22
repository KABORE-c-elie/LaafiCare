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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use laaficare_backend::config::CleTotp;
    use laaficare_backend::demande_remboursement_munaseb::{self as remboursement, Acte, AffectationAgent, TypeActeMunaseb};
    use laaficare_backend::jwt::JwtService;
    use laaficare_backend::nip;
    use laaficare_backend::sms::SmsSenderConsole;
    use serde_json::Value;
    use sqlx::PgPool;
    use std::sync::Arc;

    // Base temporaire par test ; données fictives.

    fn etat(pool: PgPool) -> AppState {
        AppState {
            db: pool,
            jwt: JwtService::new(&"a".repeat(32)),
            sms: Arc::new(SmsSenderConsole),
            cle_totp: Arc::new(CleTotp::depuis_octets([7; 32])),
            email: Arc::new(laaficare_backend::email::EmailSenderConsole),
            url_application_web: Arc::from("http://localhost:4200"),
            contact_securite: Arc::from("securite@example.org"),
            taches_reinitialisation: Arc::new(Default::default()),
        }
    }

    fn nip_valide(payload: &str) -> String {
        (0..=9).map(|d| format!("{payload}{d}")).find(|n| nip::valider(n)).unwrap()
    }

    /// Compte patient, dossier, contrat et période en cours. Renvoie
    /// (compte, NIP).
    async fn adherent(pool: &PgPool, telephone: &str, payload_nip: &str, carte: &str) -> (Uuid, String) {
        let nip = nip_valide(payload_nip);
        let u: Uuid =
            sqlx::query_scalar("INSERT INTO utilisateur (nom, prenom, telephone) VALUES ('OUEDRAOGO', 'Awa', $1) RETURNING id")
                .bind(telephone)
                .fetch_one(pool)
                .await
                .unwrap();
        let compte: Uuid = sqlx::query_scalar(
            "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'patient', 'x') RETURNING id",
        )
        .bind(u)
        .fetch_one(pool)
        .await
        .unwrap();
        let patient_id: Uuid = sqlx::query_scalar(
            "INSERT INTO patient (utilisateur_id, nip, date_naissance, lieu_naissance) \
             VALUES ($1, $2, '2000-01-01', 'Ouagadougou') RETURNING id",
        )
        .bind(u)
        .bind(&nip)
        .fetch_one(pool)
        .await
        .unwrap();
        let contrat: Uuid = sqlx::query_scalar(
            "INSERT INTO contrat_assurance_munaseb (patient_id, numero_carte, ufr, universite, num_matricule) \
             VALUES ($1, $2, 'UFR fictive', 'Université fictive', 'MAT-FICTIF') RETURNING id",
        )
        .bind(patient_id)
        .bind(carte)
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO periode_adhesion_munaseb (contrat_id, date_paiement, date_debut, date_fin) \
             VALUES ($1, CURRENT_DATE - 30, CURRENT_DATE - 30, CURRENT_DATE + 300)",
        )
        .bind(contrat)
        .execute(pool)
        .await
        .unwrap();
        (compte, nip)
    }

    async fn agent(pool: &PgPool) -> AffectationAgent {
        let u: Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (nom, prenom, telephone) VALUES ('KABRE', 'Inoussa', '+22670030098') RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        let compte: Uuid = sqlx::query_scalar(
            "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'professionnel', 'x') RETURNING id",
        )
        .bind(u)
        .fetch_one(pool)
        .await
        .unwrap();
        let structure_id: Uuid = sqlx::query_scalar(
            "INSERT INTO structure (type_structure, nom, telephone, telephone_urgence, region, email, commune, province, \
                                    latitude, longitude, createur_compte_id, statut) \
             VALUES ('munaseb', 'MUNASEB', '+22670000000', '+22670000000', 'Région fictive', 'structure@example.org', \
                     'Commune fictive', 'Province fictive', 12.371400, -1.519700, $1, 'validee') RETURNING id",
        )
        .bind(compte)
        .fetch_one(pool)
        .await
        .unwrap();
        let affectation_id: Uuid = sqlx::query_scalar(
            "INSERT INTO affectation (compte_id, structure_id, role) VALUES ($1, $2, 'agent_assurance_munaseb') RETURNING id",
        )
        .bind(compte)
        .bind(structure_id)
        .fetch_one(pool)
        .await
        .unwrap();
        AffectationAgent { affectation_id, structure_id }
    }

    async fn demande(pool: &PgPool, nip: &str, partenaire_id: Uuid, reference: &str) -> Uuid {
        remboursement::creer_depuis_acte(
            pool,
            &Acte {
                nip: nip.to_string(),
                partenaire_id,
                type_acte: TypeActeMunaseb::Pharmacie,
                reference_acte: reference.to_string(),
                date_acte: chrono::Utc::now().date_naive(),
                montant_acte_fcfa: 10_000,
            },
        )
        .await
        .unwrap()
    }

    async fn notifications_de(pool: &PgPool, nip: &str) -> Vec<Uuid> {
        sqlx::query_scalar(
            "SELECT n.id FROM notification n JOIN patient p ON p.utilisateur_id = n.utilisateur_id \
             WHERE p.nip = $1 ORDER BY n.id",
        )
        .bind(nip)
        .fetch_all(pool)
        .await
        .unwrap()
    }

    /// Statut et corps JSON (`Null` si vide).
    async fn lire(reponse: Response) -> (StatusCode, Value) {
        let statut = reponse.status();
        let octets = to_bytes(reponse.into_body(), usize::MAX).await.unwrap();
        (statut, if octets.is_empty() { Value::Null } else { serde_json::from_slice(&octets).unwrap() })
    }

    fn ids(valeur: &Value, cle: &str) -> Vec<String> {
        valeur[cle].as_array().unwrap().iter().map(|e| e["id"].as_str().unwrap().to_string()).collect()
    }

    /// E2 et E9, par les six routes, avec deux adhérents A et B qui ont
    /// chacun un contrat, des demandes et des notifications :
    /// - B ne voit jamais rien de A, sur chaque route (droits, liste,
    ///   détail, notifications, marquer une, marquer toutes) ;
    /// - aucune réponse ne contient de NIP, de `reference_acte`, de nom
    ///   d'agent ni d'identifiant d'affectation ou de structure d'agent ;
    /// - motif de rejet dans le détail, jamais dans la liste ;
    /// - pagination par `suivant` (demandes et notifications), limite hors
    ///   bornes en 400 ;
    /// - marquer deux fois la même notification : 204 puis 204.
    #[sqlx::test]
    #[ignore]
    async fn six_routes_et_donnees_d_un_autre_patient(pool: PgPool) {
        let state = etat(pool.clone());
        let ag = agent(&pool).await;
        let partenaire: Uuid = sqlx::query_scalar(
            "INSERT INTO partenaire_sante_munaseb (nom, type, ville, telephone) \
             VALUES ('Pharmacie fictive', 'pharmacie', 'Ouagadougou', '+22670030099') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let (a, nip_a) = adherent(&pool, "+22670030001", "900000000030001", "CARTE-A").await;
        let (b, nip_b) = adherent(&pool, "+22670030002", "900000000030002", "CARTE-B").await;
        let a1 = demande(&pool, &nip_a, partenaire, "REF-SECRETE-A1").await;
        let a2 = demande(&pool, &nip_a, partenaire, "REF-SECRETE-A2").await;
        remboursement::rejeter(&pool, a2, ag, "Motif fictif A").await.unwrap();
        let b1 = demande(&pool, &nip_b, partenaire, "REF-SECRETE-B1").await;
        let notif_a = notifications_de(&pool, &nip_a).await;
        let notif_b = notifications_de(&pool, &nip_b).await;
        assert_eq!((notif_a.len(), notif_b.len()), (3, 1), "création ×2 et rejet pour A, création pour B");

        let pa = || PatientAuthentifie { compte_id: a };
        let pb = || PatientAuthentifie { compte_id: b };
        let filtre = |limite, avant| RequeteApi(FiltreDemandes { statut: None, limite, avant });
        let filtre_n = |limite, avant| RequeteApi(FiltreNotifications { limite, avant });
        let mut corps = Vec::new();

        // 1. Droits : chacun sa carte.
        let (s, v) = lire(droits(pb(), State(state.clone())).await).await;
        assert_eq!((s, v["etat"].as_str(), v["numero_carte"].as_str()), (StatusCode::OK, Some("couvert"), Some("CARTE-B")));
        corps.push(v);
        let (_, v) = lire(droits(pa(), State(state.clone())).await).await;
        assert_eq!(v["numero_carte"].as_str(), Some("CARTE-A"));
        corps.push(v);

        // 2. Liste : B ne voit que la sienne ; A, les siennes, sans motif ;
        // pagination.
        let (_, v) = lire(lister_demandes(pb(), State(state.clone()), filtre(None, None)).await).await;
        assert_eq!(ids(&v, "demandes"), [b1.to_string()]);
        corps.push(v);
        let (_, p1) = lire(lister_demandes(pa(), State(state.clone()), filtre(Some(1), None)).await).await;
        assert_eq!(ids(&p1, "demandes"), [a2.to_string()]);
        let suivant: Uuid = p1["suivant"].as_str().unwrap().parse().unwrap();
        let (_, p2) = lire(lister_demandes(pa(), State(state.clone()), filtre(Some(1), Some(suivant))).await).await;
        assert_eq!(ids(&p2, "demandes"), [a1.to_string()]);
        let suivant: Uuid = p2["suivant"].as_str().unwrap().parse().unwrap();
        let (_, p3) = lire(lister_demandes(pa(), State(state.clone()), filtre(Some(1), Some(suivant))).await).await;
        assert!(ids(&p3, "demandes").is_empty() && p3["suivant"].is_null());
        for page in [&p1, &p2] {
            assert!(!page.to_string().contains("Motif fictif") && !page.to_string().contains("motif_rejet"));
        }
        corps.extend([p1, p2]);
        let (s, _) = lire(lister_demandes(pa(), State(state.clone()), filtre(Some(101), None)).await).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);

        // 3. Détail : celui de A introuvable pour B ; A voit son motif.
        let (s, v) = lire(detail_demande(pb(), State(state.clone()), CheminApi(a2)).await).await;
        assert_eq!((s, v["code"].as_str()), (StatusCode::NOT_FOUND, Some("demande_inconnue")));
        let (s, v) = lire(detail_demande(pa(), State(state.clone()), CheminApi(a2)).await).await;
        assert_eq!((s, v["motif_rejet"].as_str()), (StatusCode::OK, Some("Motif fictif A")));
        corps.push(v);

        // 4. Notifications : B ne voit que la sienne ; pagination de A.
        let (_, v) = lire(lister_notifications(pb(), State(state.clone()), filtre_n(None, None)).await).await;
        assert_eq!(ids(&v, "notifications"), [notif_b[0].to_string()]);
        assert_eq!(v["non_lues"], 1);
        corps.push(v);
        let (_, n1) = lire(lister_notifications(pa(), State(state.clone()), filtre_n(Some(2), None)).await).await;
        assert_eq!(n1["non_lues"], 3);
        let suivant: Uuid = n1["suivant"].as_str().unwrap().parse().unwrap();
        let (_, n2) = lire(lister_notifications(pa(), State(state.clone()), filtre_n(Some(2), Some(suivant))).await).await;
        let mut vus = ids(&n1, "notifications");
        vus.extend(ids(&n2, "notifications"));
        assert_eq!(vus, notif_a.iter().rev().map(Uuid::to_string).collect::<Vec<_>>());
        assert!(n2["suivant"].is_null());
        corps.extend([n1, n2]);

        // 5. Marquer une notification de A en étant B : 404, rien ne change ;
        // A marque la sienne deux fois : 204, 204.
        let (s, v) = lire(marquer_lue(pb(), State(state.clone()), CheminApi(notif_a[0])).await).await;
        assert_eq!((s, v["code"].as_str()), (StatusCode::NOT_FOUND, Some("notification_inconnue")));
        for _ in 0..2 {
            let (s, _) = lire(marquer_lue(pa(), State(state.clone()), CheminApi(notif_a[0])).await).await;
            assert_eq!(s, StatusCode::NO_CONTENT);
        }

        // 6. B marque toutes les siennes : celles de A ne changent pas.
        let (s, _) = lire(marquer_toutes_lues(pb(), State(state.clone())).await).await;
        assert_eq!(s, StatusCode::NO_CONTENT);
        let (_, v) = lire(lister_notifications(pa(), State(state.clone()), filtre_n(None, None)).await).await;
        assert_eq!(v["non_lues"], 2);
        let (_, v) = lire(lister_notifications(pb(), State(state.clone()), filtre_n(None, None)).await).await;
        assert_eq!(v["non_lues"], 0);

        // Aucune réponse ne contient de NIP, de référence d'acte, ni rien de
        // l'agent.
        let interdits = [
            nip_a.clone(),
            nip_b.clone(),
            "REF-SECRETE".to_string(),
            "KABRE".to_string(),
            "Inoussa".to_string(),
            ag.affectation_id.to_string(),
            ag.structure_id.to_string(),
            "MAT-FICTIF".to_string(),
        ];
        for v in &corps {
            let texte = v.to_string();
            for interdit in &interdits {
                assert!(!texte.contains(interdit.as_str()), "{interdit} dans {texte}");
            }
        }
    }
}
