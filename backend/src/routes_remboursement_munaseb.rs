//! Routes HTTP des demandes de remboursement MUNASEB (section 12), toutes
//! réservées aux agents MUNASEB en exercice (extracteur `AgentMunaseb`,
//! étape 3.9). Même principe que les autres fichiers de routes :
//! désérialiser, appeler le service de `demande_remboursement_munaseb`,
//! traduire le résultat -- aucune logique métier ici.
//!
//! Regroupées dans ce fichier (décision L6). L'agent se connecte par la
//! connexion professionnelle, puis active son affectation
//! `agent_assurance_munaseb` (étape 3.9) ; chaque transition est
//! enregistrée avec cette affectation (J8).

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use sqlx::types::Uuid;

use crate::AppState;
use crate::erreur_api::{
    CheminApi, ErreurApi, JsonApi, ReponseErreur, RequeteApi, reponse_erreur, reponse_erreur_interne,
};
use crate::extracteur_jwt::AgentMunaseb;
use laaficare_backend::demande_remboursement_munaseb::{
    self as remboursement, Acte, DetailDemande, ErreurDemandeRemboursement, ErreurTransition, ResumeDemande,
    StatutDemande, TypeActeMunaseb,
};

// ---------------------------------------------------------------------
// simuler-acte : test manuel de `creer_depuis_acte`, en attendant les
// modules Pharmacie/Laboratoire/Hôpital qui appelleront le même service.
// ---------------------------------------------------------------------

fn mapper_erreur_remboursement(erreur: ErreurDemandeRemboursement) -> ReponseErreur {
    use ErreurDemandeRemboursement as E;
    let (statut, code, debut_couverture) = match &erreur {
        E::MontantInvalide => (StatusCode::UNPROCESSABLE_ENTITY, "montant_invalide", None),
        E::NipInvalide => (StatusCode::UNPROCESSABLE_ENTITY, "nip_invalide", None),
        E::PatientInconnu => (StatusCode::NOT_FOUND, "patient_inconnu", None),
        E::PartenaireInconnu => (StatusCode::NOT_FOUND, "partenaire_inconnu", None),
        E::ReferenceActeDejaUtilisee => (StatusCode::CONFLICT, "reference_acte_deja_utilisee", None),
        E::PartenaireSuspendu => (StatusCode::UNPROCESSABLE_ENTITY, "partenaire_suspendu", None),
        E::NonCouvert => (StatusCode::UNPROCESSABLE_ENTITY, "non_couvert", None),
        E::EnCarence { debut_couverture } => {
            (StatusCode::UNPROCESSABLE_ENTITY, "en_carence", Some(*debut_couverture))
        }
        E::Interne(detail) => return reponse_erreur_interne("DemandeRemboursement", detail),
    };
    (
        statut,
        Json(ErreurApi { debut_couverture, ..ErreurApi::simple(code, erreur.to_string()) }),
    )
}

#[derive(Deserialize)]
pub struct SimulerActeRequete {
    pub nip: String,
    pub partenaire_id: Uuid,
    pub type_acte: TypeActeMunaseb,
    pub reference_acte: String,
    pub date_acte: NaiveDate,
    pub montant_acte_fcfa: i32,
}

#[derive(Serialize)]
pub struct SimulerActeReponse {
    pub demande_id: Uuid,
}

/// `POST /api/assurance-munaseb/remboursements/simuler-acte` -- réservée
/// aux agents MUNASEB : sans jeton agent valide, l'extracteur refuse la
/// requête avant l'appel du handler (condition posée pour
/// `PatientInconnu` : jamais exposé sans authentification).
pub async fn simuler_acte(
    _agent: AgentMunaseb,
    State(state): State<AppState>,
    JsonApi(corps): JsonApi<SimulerActeRequete>,
) -> Result<Json<SimulerActeReponse>, ReponseErreur> {
    let acte = Acte {
        nip: corps.nip,
        partenaire_id: corps.partenaire_id,
        type_acte: corps.type_acte,
        reference_acte: corps.reference_acte,
        date_acte: corps.date_acte,
        montant_acte_fcfa: corps.montant_acte_fcfa,
    };
    let demande_id = remboursement::creer_depuis_acte(&state.db, &acte)
        .await
        .map_err(mapper_erreur_remboursement)?;
    Ok(Json(SimulerActeReponse { demande_id }))
}

// ---------------------------------------------------------------------
// Lectures
// ---------------------------------------------------------------------

// Pagination par curseur (décision L1 b) : 50 par défaut, 100 au maximum.
const LIMITE_PAR_DEFAUT: i64 = 50;
const LIMITE_MAX: i64 = 100;

#[derive(Deserialize)]
pub struct FiltreListe {
    pub statut: Option<StatutDemande>,
    pub limite: Option<i64>,
    /// Identifiant de la dernière demande de la page précédente (`suivant`
    /// de la réponse précédente).
    pub apres: Option<Uuid>,
}

#[derive(Serialize)]
pub struct ListeReponse {
    pub demandes: Vec<ResumeDemande>,
    /// Curseur de la page suivante ; `null` quand la page n'est pas pleine
    /// (plus rien à lire pour l'instant).
    pub suivant: Option<Uuid>,
}

/// `GET /api/assurance-munaseb/remboursements?statut=&limite=&apres=`
pub async fn lister(
    _agent: AgentMunaseb,
    State(state): State<AppState>,
    RequeteApi(filtre): RequeteApi<FiltreListe>,
) -> Result<Json<ListeReponse>, ReponseErreur> {
    let limite = filtre.limite.unwrap_or(LIMITE_PAR_DEFAUT);
    if !(1..=LIMITE_MAX).contains(&limite) {
        return Err(reponse_erreur(
            StatusCode::BAD_REQUEST,
            "parametre_invalide",
            format!("limite doit être comprise entre 1 et {LIMITE_MAX}"),
        ));
    }

    let demandes = remboursement::lister_demandes(&state.db, filtre.statut, filtre.apres, limite)
        .await
        .map_err(|e| reponse_erreur_interne("lister_demandes", &e.to_string()))?;
    let suivant = if demandes.len() as i64 == limite { demandes.last().map(|d| d.id) } else { None };
    Ok(Json(ListeReponse { demandes, suivant }))
}

/// `GET /api/assurance-munaseb/remboursements/{id}` -- détail et historique.
pub async fn detail(
    _agent: AgentMunaseb,
    State(state): State<AppState>,
    CheminApi(demande_id): CheminApi<Uuid>,
) -> Result<Json<DetailDemande>, ReponseErreur> {
    remboursement::detail_demande(&state.db, demande_id)
        .await
        .map_err(|e| reponse_erreur_interne("detail_demande", &e.to_string()))?
        .map(Json)
        .ok_or_else(|| reponse_erreur(StatusCode::NOT_FOUND, "demande_inconnue", "demande inconnue"))
}

// ---------------------------------------------------------------------
// Transitions (204 sans corps, sauf la validation -- décision L5)
// ---------------------------------------------------------------------

fn mapper_erreur_transition(erreur: ErreurTransition) -> ReponseErreur {
    use ErreurTransition as E;
    let (statut, code, statut_actuel) = match &erreur {
        E::DemandeInconnue => (StatusCode::NOT_FOUND, "demande_inconnue", None),
        E::TransitionInvalide { statut_actuel } => {
            (StatusCode::CONFLICT, "transition_invalide", Some(statut_actuel.clone()))
        }
        E::MotifVide => (StatusCode::UNPROCESSABLE_ENTITY, "motif_vide", None),
        E::MotifTropLong => (StatusCode::UNPROCESSABLE_ENTITY, "motif_trop_long", None),
        E::Interne(detail) => return reponse_erreur_interne("transition de demande", detail),
    };
    (
        statut,
        Json(ErreurApi { statut_actuel, ..ErreurApi::simple(code, erreur.to_string()) }),
    )
}

/// `POST …/remboursements/{id}/prendre-en-charge`
pub async fn prendre_en_charge(
    AgentMunaseb(agent): AgentMunaseb,
    State(state): State<AppState>,
    CheminApi(demande_id): CheminApi<Uuid>,
) -> Result<StatusCode, ReponseErreur> {
    remboursement::prendre_en_charge(&state.db, demande_id, agent)
        .await
        .map_err(mapper_erreur_transition)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
pub struct ValidationReponse {
    pub montant_rembourse_fcfa: i32,
}

/// `POST …/remboursements/{id}/valider` -- renvoie le montant remboursé
/// (décision V3) : l'agent voit tout de suite si le plafond l'a limité.
pub async fn valider(
    AgentMunaseb(agent): AgentMunaseb,
    State(state): State<AppState>,
    CheminApi(demande_id): CheminApi<Uuid>,
) -> Result<Json<ValidationReponse>, ReponseErreur> {
    let montant_rembourse_fcfa = remboursement::valider(&state.db, demande_id, agent)
        .await
        .map_err(mapper_erreur_transition)?;
    Ok(Json(ValidationReponse { montant_rembourse_fcfa }))
}

#[derive(Deserialize)]
pub struct RejetRequete {
    pub motif: String,
}

/// `POST …/remboursements/{id}/rejeter`, corps `{"motif": "…"}`
pub async fn rejeter(
    AgentMunaseb(agent): AgentMunaseb,
    State(state): State<AppState>,
    CheminApi(demande_id): CheminApi<Uuid>,
    JsonApi(corps): JsonApi<RejetRequete>,
) -> Result<StatusCode, ReponseErreur> {
    remboursement::rejeter(&state.db, demande_id, agent, &corps.motif)
        .await
        .map_err(mapper_erreur_transition)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST …/remboursements/{id}/payer`
pub async fn payer(
    AgentMunaseb(agent): AgentMunaseb,
    State(state): State<AppState>,
    CheminApi(demande_id): CheminApi<Uuid>,
) -> Result<StatusCode, ReponseErreur> {
    remboursement::marquer_payee(&state.db, demande_id, agent)
        .await
        .map_err(mapper_erreur_transition)?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erreurs_de_simuler_acte() {
        use ErreurDemandeRemboursement as E;
        let debut = NaiveDate::from_ymd_opt(2026, 10, 25).unwrap();
        for (erreur, statut, code) in [
            (E::MontantInvalide, StatusCode::UNPROCESSABLE_ENTITY, "montant_invalide"),
            (E::NipInvalide, StatusCode::UNPROCESSABLE_ENTITY, "nip_invalide"),
            (E::PatientInconnu, StatusCode::NOT_FOUND, "patient_inconnu"),
            (E::PartenaireInconnu, StatusCode::NOT_FOUND, "partenaire_inconnu"),
            (E::ReferenceActeDejaUtilisee, StatusCode::CONFLICT, "reference_acte_deja_utilisee"),
            (E::PartenaireSuspendu, StatusCode::UNPROCESSABLE_ENTITY, "partenaire_suspendu"),
            (E::NonCouvert, StatusCode::UNPROCESSABLE_ENTITY, "non_couvert"),
            (E::EnCarence { debut_couverture: debut }, StatusCode::UNPROCESSABLE_ENTITY, "en_carence"),
            (E::Interne("détail secret".into()), StatusCode::INTERNAL_SERVER_ERROR, "erreur_interne"),
        ] {
            let est_carence = matches!(erreur, E::EnCarence { .. });
            let (s, Json(corps)) = mapper_erreur_remboursement(erreur);
            assert_eq!((s, corps.code), (statut, code));
            assert_eq!(corps.debut_couverture, est_carence.then_some(debut));
            assert!(!corps.erreur.contains("secret"));
        }
    }

    /// D5 avec deux structures MUNASEB (décision du porteur, 2026-10-18) :
    /// sur la même demande, l'agent de la MUNASEB en attente de validation
    /// est refusé (403) avant l'appel de la route, la demande ne change pas ;
    /// l'agent de la MUNASEB validée la prend en charge, et l'historique
    /// garde son affectation (J8).
    #[sqlx::test]
    #[ignore]
    async fn transition_par_l_agent_de_la_seule_munaseb_validee(pool: sqlx::PgPool) {
        use axum::extract::FromRequestParts;
        use axum::http::{Request, header::AUTHORIZATION};
        use axum::response::IntoResponse;
        use laaficare_backend::config::CleTotp;
        use laaficare_backend::jwt::{JwtService, SorteJeton};
        use laaficare_backend::session;
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
        };
        let db = &state.db;
        let scalaire = |requete: &'static str, telephone: &'static str| async move {
            sqlx::query_scalar::<_, Uuid>(requete).bind(telephone).fetch_one(db).await.unwrap()
        };

        // Demande en attente (données fictives).
        let personne = scalaire("INSERT INTO utilisateur (nom, prenom, telephone) VALUES ('OUEDRAOGO', 'Awa', $1) RETURNING id", "+22670190001").await;
        let patient: Uuid = sqlx::query_scalar(
            "INSERT INTO patient (utilisateur_id, nip, date_naissance, lieu_naissance) \
             VALUES ($1, '0000000000000000', '2000-01-01', 'Ouagadougou') RETURNING id",
        )
        .bind(personne)
        .fetch_one(db)
        .await
        .unwrap();
        let contrat: Uuid = sqlx::query_scalar(
            "INSERT INTO contrat_assurance_munaseb (patient_id, numero_carte, ufr, universite, num_matricule) \
             VALUES ($1, 'TEST-D5', 'UFR', 'Université', 'M-D5') RETURNING id",
        )
        .bind(patient)
        .fetch_one(db)
        .await
        .unwrap();
        let periode: Uuid = sqlx::query_scalar(
            "INSERT INTO periode_adhesion_munaseb (contrat_id, date_paiement, date_debut, date_fin) \
             VALUES ($1, '2026-01-01', '2026-01-01', '2026-12-31') RETURNING id",
        )
        .bind(contrat)
        .fetch_one(db)
        .await
        .unwrap();
        let partenaire = scalaire(
            "INSERT INTO partenaire_sante_munaseb (nom, type, ville, telephone) \
             VALUES ('Pharmacie de test', 'pharmacie', 'Ouagadougou', $1) RETURNING id",
            "+22670190002",
        )
        .await;
        let demande: Uuid = sqlx::query_scalar(
            "INSERT INTO demande_remboursement_munaseb \
             (partenaire_id, type_acte, reference_acte, date_acte, montant_acte_fcfa, montant_demande_fcfa, periode_id) \
             VALUES ($1, 'pharmacie', 'REF-D5', '2026-06-01', 1000, 800, $2) RETURNING id",
        )
        .bind(partenaire)
        .bind(periode)
        .fetch_one(db)
        .await
        .unwrap();

        // Deux structures MUNASEB : la base n'en admet qu'une validée.
        let agent_en_exercice = |telephone: &'static str, statut: &'static str| {
            let state = state.clone();
            async move {
                let identite: Uuid = sqlx::query_scalar(
                    "INSERT INTO utilisateur (telephone, email) VALUES ($1, $1 || '@example.org') RETURNING id",
                )
                .bind(telephone)
                .fetch_one(&state.db)
                .await
                .unwrap();
                sqlx::query("UPDATE utilisateur SET email_verifie_le = now() WHERE id = $1")
                    .bind(identite)
                    .execute(&state.db)
                    .await
                    .unwrap();
                let compte: Uuid = sqlx::query_scalar(
                    "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'professionnel', 'x') RETURNING id",
                )
                .bind(identite)
                .fetch_one(&state.db)
                .await
                .unwrap();
                let structure: Uuid = sqlx::query_scalar(
                    "INSERT INTO structure (type_structure, nom, telephone, telephone_urgence, region, email, commune, province, latitude, \
                                            longitude, createur_compte_id, statut) \
                     VALUES ('munaseb', 'MUNASEB', '+22670000000', '+22670000000', 'Région fictive', 'structure@example.org', 'Ouagadougou', 'Kadiogo', \
                             12.371400, -1.519700, $1, $2) RETURNING id",
                )
                .bind(compte)
                .bind(statut)
                .fetch_one(&state.db)
                .await
                .unwrap();
                let affectation: Uuid = sqlx::query_scalar(
                    "INSERT INTO affectation (compte_id, structure_id, role) VALUES ($1, $2, 'agent_assurance_munaseb') RETURNING id",
                )
                .bind(compte)
                .bind(structure)
                .fetch_one(&state.db)
                .await
                .unwrap();
                let session = session::ouvrir(&state.db, compte, Some(affectation)).await.unwrap();
                let jeton = state
                    .jwt
                    .emettre(SorteJeton::ProfessionnelEnExercice, compte, 0, Some(affectation), Some(session.session_id))
                    .unwrap();
                (affectation, format!("Bearer {jeton}"))
            }
        };
        let (affectation_a, jeton_a) = agent_en_exercice("+22670190003", "validee").await;
        let (_, jeton_b) = agent_en_exercice("+22670190004", "en_attente").await;
        let extraire = |en_tete: String| {
            let state = state.clone();
            async move {
                let mut parts = Request::builder().header(AUTHORIZATION, en_tete).body(()).unwrap().into_parts().0;
                AgentMunaseb::from_request_parts(&mut parts, &state).await
            }
        };
        let statut_demande = || async {
            sqlx::query_scalar::<_, String>("SELECT statut FROM demande_remboursement_munaseb WHERE id = $1")
                .bind(demande)
                .fetch_one(db)
                .await
                .unwrap()
        };

        let refus = extraire(jeton_b).await.map_err(IntoResponse::into_response).unwrap_err();
        assert_eq!(refus.status(), StatusCode::FORBIDDEN);
        assert_eq!(statut_demande().await, "en_attente", "la demande n'a pas changé");

        let agent_a = extraire(jeton_a).await.unwrap();
        let statut = prendre_en_charge(agent_a, State(state.clone()), CheminApi(demande)).await.unwrap();
        assert_eq!(statut, StatusCode::NO_CONTENT);
        assert_eq!(statut_demande().await, "en_cours");
        let par: Option<Uuid> = sqlx::query_scalar(
            "SELECT agent_affectation_id FROM historique_statut_demande WHERE demande_id = $1 AND statut_precedent IS NOT NULL",
        )
        .bind(demande)
        .fetch_one(db)
        .await
        .unwrap();
        assert_eq!(par, Some(affectation_a));
    }

    #[test]
    fn erreurs_de_transition() {
        use ErreurTransition as E;
        for (erreur, statut, code) in [
            (E::DemandeInconnue, StatusCode::NOT_FOUND, "demande_inconnue"),
            (E::TransitionInvalide { statut_actuel: "valide".into() }, StatusCode::CONFLICT, "transition_invalide"),
            (E::MotifVide, StatusCode::UNPROCESSABLE_ENTITY, "motif_vide"),
            (E::MotifTropLong, StatusCode::UNPROCESSABLE_ENTITY, "motif_trop_long"),
            (E::Interne("détail secret".into()), StatusCode::INTERNAL_SERVER_ERROR, "erreur_interne"),
        ] {
            let est_transition = matches!(erreur, E::TransitionInvalide { .. });
            let (s, Json(corps)) = mapper_erreur_transition(erreur);
            assert_eq!((s, corps.code), (statut, code));
            assert_eq!(corps.statut_actuel.as_deref(), est_transition.then_some("valide"));
            assert!(!corps.erreur.contains("secret"));
        }
    }
}
