//! Routes HTTP du contrat MUNASEB côté agent (étape 5 ; contrat dans
//! docs/api.md, section 2.14) : paramètres de la mutuelle, adhésion,
//! renouvellement, détail d'un contrat et recherche par numéro de carte.
//! Contrats : agents MUNASEB en exercice (extracteur `AgentMunaseb`, étape
//! 3.9). Paramètres : lecture par un agent ou le responsable
//! (`MembreMunaseb`), modification par le **responsable** seul
//! (`ResponsableMunaseb`, demande du porteur du 2026-11-16). Chaque écriture
//! est enregistrée avec l'affectation de son auteur (comme J8). Aucune
//! logique métier ici : elle est dans `contrat_assurance_munaseb`.
//!
//! La suspension d'un contrat n'a pas de route : ses règles sont à demander
//! à la MUNASEB (docs/points-ouverts.md).

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header::RETRY_AFTER};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use sqlx::types::Uuid;

use crate::AppState;
use crate::erreur_api::{
    CheminApi, JsonApi, ReponseErreur, RequeteApi, reponse_erreur, reponse_erreur_interne,
};
use crate::extracteur_jwt::{AgentMunaseb, MembreMunaseb, ResponsableMunaseb};
use laaficare_backend::contrat_assurance_munaseb::{
    self as contrat, Adhesion, DetailContrat, ErreurContratAssuranceMunaseb, Parametres,
    Renouvellement, VersionParametres,
};

// Mêmes bornes que les autres listes de l'API.
const LIMITE_PAR_DEFAUT: i64 = 50;
const LIMITE_MAX: i64 = 100;

fn mapper_erreur(erreur: ErreurContratAssuranceMunaseb) -> ReponseErreur {
    use ErreurContratAssuranceMunaseb as E;
    let (statut, code) = match &erreur {
        E::NumeroCarteDejaUtilise => (StatusCode::CONFLICT, "numero_carte_deja_utilise"),
        E::ContratDejaExistant => (StatusCode::CONFLICT, "contrat_deja_existant"),
        E::AucunContrat => (StatusCode::NOT_FOUND, "contrat_inconnu"),
        E::ContratSuspendu => (StatusCode::CONFLICT, "contrat_suspendu"),
        E::PeriodeFutureDejaPayee => (StatusCode::CONFLICT, "periode_future_deja_payee"),
        E::NipInvalide => (StatusCode::UNPROCESSABLE_ENTITY, "nip_invalide"),
        E::PatientInconnu => (StatusCode::NOT_FOUND, "patient_inconnu"),
        E::ReferenceTransactionDejaUtilisee => {
            (StatusCode::CONFLICT, "reference_transaction_deja_utilisee")
        }
        E::DonneesInvalides(_) => (StatusCode::UNPROCESSABLE_ENTITY, "donnees_invalides"),
        // Même code que les autres limites de l'API (O5, R5).
        E::TropDEchecs { .. } => (StatusCode::TOO_MANY_REQUESTS, "trop_de_demandes"),
        // `EnCarence` et `ContratExpire` ne viennent que de
        // `verifier_droits`, qu'aucune de ces routes n'appelle.
        E::EnCarence { .. } | E::ContratExpire => {
            return reponse_erreur_interne("contrat MUNASEB", "état de droits inattendu");
        }
        E::Interne(detail) => return reponse_erreur_interne("contrat MUNASEB", detail),
    };
    reponse_erreur(statut, code, erreur.to_string())
}

/// Réponse complète : celle de `mapper_erreur`, plus l'en-tête
/// `Retry-After` (en secondes) pour la limite d'échecs d'adhésion. RFC 6585
/// §4 : une réponse 429 « MAY include a Retry-After header indicating how
/// long to wait before making a new request ». Même construction que
/// `routes_patient::reponse_erreur_patient`.
fn reponse_erreur_contrat(erreur: ErreurContratAssuranceMunaseb) -> Response {
    let reessayer_dans_secs = match &erreur {
        ErreurContratAssuranceMunaseb::TropDEchecs {
            reessayer_dans_secs,
        } => Some(*reessayer_dans_secs),
        _ => None,
    };
    let mut reponse = mapper_erreur(erreur).into_response();
    if let Some(secs) = reessayer_dans_secs {
        reponse
            .headers_mut()
            .insert(RETRY_AFTER, HeaderValue::from(secs));
    }
    reponse
}

fn limite_valide(limite: Option<i64>) -> Result<i64, ReponseErreur> {
    let limite = limite.unwrap_or(LIMITE_PAR_DEFAUT);
    if (1..=LIMITE_MAX).contains(&limite) {
        Ok(limite)
    } else {
        Err(reponse_erreur(
            StatusCode::BAD_REQUEST,
            "parametre_invalide",
            format!("limite doit être comprise entre 1 et {LIMITE_MAX}"),
        ))
    }
}

fn contrat_inconnu() -> ReponseErreur {
    reponse_erreur(StatusCode::NOT_FOUND, "contrat_inconnu", "Contrat inconnu.")
}

/// Détail relu après une écriture : le contrat vient d'être écrit, son
/// absence serait une panne.
async fn relire(state: &AppState, contrat_id: Uuid) -> Result<DetailContrat, ReponseErreur> {
    contrat::detail_contrat(&state.db, contrat_id)
        .await
        .map_err(|e| reponse_erreur_interne("detail_contrat", &e.to_string()))?
        .ok_or_else(|| {
            reponse_erreur_interne("detail_contrat", "contrat introuvable après écriture")
        })
}

// ---------------------------------------------------------------------
// Paramètres
// ---------------------------------------------------------------------

/// `GET /api/assurance-munaseb/parametres` : valeurs en vigueur (agent ou
/// responsable).
pub async fn parametres(
    _membre: MembreMunaseb,
    State(state): State<AppState>,
) -> Result<Json<VersionParametres>, ReponseErreur> {
    contrat::parametres_en_vigueur(&state.db)
        .await
        .map(Json)
        .map_err(|e| reponse_erreur_interne("parametres_en_vigueur", &e.to_string()))
}

/// `PUT /api/assurance-munaseb/parametres` : les cinq valeurs, toutes
/// obligatoires, par le **responsable** de la MUNASEB seulement (un agent
/// reçoit 403 `acces_refuse` avant la lecture du corps). Renvoie la version
/// en vigueur après l'écriture (la même qu'avant si rien ne change).
pub async fn modifier_parametres(
    ResponsableMunaseb(responsable): ResponsableMunaseb,
    State(state): State<AppState>,
    JsonApi(corps): JsonApi<Parametres>,
) -> Result<Json<VersionParametres>, ReponseErreur> {
    contrat::modifier_parametres(&state.db, responsable, corps)
        .await
        .map(Json)
        .map_err(mapper_erreur)
}

#[derive(Deserialize)]
pub struct FiltreHistorique {
    pub limite: Option<i64>,
    /// `suivant` de la page précédente.
    pub avant: Option<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct HistoriqueReponse {
    pub versions: Vec<VersionParametres>,
    pub suivant: Option<Uuid>,
}

/// `GET /api/assurance-munaseb/parametres/historique?limite=&avant=` : la
/// version la plus récente d'abord (agent ou responsable).
pub async fn historique_parametres(
    _membre: MembreMunaseb,
    State(state): State<AppState>,
    RequeteApi(filtre): RequeteApi<FiltreHistorique>,
) -> Result<Json<HistoriqueReponse>, ReponseErreur> {
    let limite = limite_valide(filtre.limite)?;
    let versions = contrat::historique_parametres(&state.db, filtre.avant, limite)
        .await
        .map_err(|e| reponse_erreur_interne("historique_parametres", &e.to_string()))?;
    let suivant = if versions.len() as i64 == limite {
        versions.last().map(|v| v.id)
    } else {
        None
    };
    Ok(Json(HistoriqueReponse { versions, suivant }))
}

// ---------------------------------------------------------------------
// Contrats
// ---------------------------------------------------------------------

/// `POST /api/assurance-munaseb/contrats` : adhésion (201 et le détail du
/// contrat) ; 429 `trop_de_demandes` avec `Retry-After` après trop
/// d'échecs de cet agent.
pub async fn adherer(
    AgentMunaseb(agent): AgentMunaseb,
    State(state): State<AppState>,
    JsonApi(corps): JsonApi<Adhesion>,
) -> Result<(StatusCode, Json<DetailContrat>), Response> {
    let contrat_id = contrat::adherer(&state.db, agent, &corps)
        .await
        .map_err(reponse_erreur_contrat)?;
    let detail = relire(&state, contrat_id)
        .await
        .map_err(IntoResponse::into_response)?;
    Ok((StatusCode::CREATED, Json(detail)))
}

#[derive(Deserialize)]
pub struct RechercheCarte {
    pub numero_carte: String,
}

/// `GET /api/assurance-munaseb/contrats?numero_carte=` : le contrat qui
/// porte cette carte (d'une année quelconque), sinon 404.
pub async fn rechercher(
    _agent: AgentMunaseb,
    State(state): State<AppState>,
    RequeteApi(recherche): RequeteApi<RechercheCarte>,
) -> Result<Json<DetailContrat>, ReponseErreur> {
    let contrat_id = contrat::contrat_par_carte(&state.db, &recherche.numero_carte)
        .await
        .map_err(|e| reponse_erreur_interne("contrat_par_carte", &e.to_string()))?
        .ok_or_else(contrat_inconnu)?;
    Ok(Json(relire(&state, contrat_id).await?))
}

/// `GET /api/assurance-munaseb/contrats/{id}`.
pub async fn detail(
    _agent: AgentMunaseb,
    State(state): State<AppState>,
    CheminApi(contrat_id): CheminApi<Uuid>,
) -> Result<Json<DetailContrat>, ReponseErreur> {
    contrat::detail_contrat(&state.db, contrat_id)
        .await
        .map_err(|e| reponse_erreur_interne("detail_contrat", &e.to_string()))?
        .map(Json)
        .ok_or_else(contrat_inconnu)
}

/// `POST /api/assurance-munaseb/contrats/{id}/renouvellements` : nouvelle
/// période (201 et le détail du contrat).
pub async fn renouveler(
    AgentMunaseb(agent): AgentMunaseb,
    State(state): State<AppState>,
    CheminApi(contrat_id): CheminApi<Uuid>,
    JsonApi(corps): JsonApi<Renouvellement>,
) -> Result<(StatusCode, Json<DetailContrat>), ReponseErreur> {
    contrat::renouveler(&state.db, agent, contrat_id, &corps)
        .await
        .map_err(mapper_erreur)?;
    Ok((StatusCode::CREATED, Json(relire(&state, contrat_id).await?)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erreurs_du_contrat() {
        use ErreurContratAssuranceMunaseb as E;
        for (erreur, statut, code) in [
            (
                E::NumeroCarteDejaUtilise,
                StatusCode::CONFLICT,
                "numero_carte_deja_utilise",
            ),
            (
                E::ContratDejaExistant,
                StatusCode::CONFLICT,
                "contrat_deja_existant",
            ),
            (E::AucunContrat, StatusCode::NOT_FOUND, "contrat_inconnu"),
            (E::ContratSuspendu, StatusCode::CONFLICT, "contrat_suspendu"),
            (
                E::PeriodeFutureDejaPayee,
                StatusCode::CONFLICT,
                "periode_future_deja_payee",
            ),
            (
                E::NipInvalide,
                StatusCode::UNPROCESSABLE_ENTITY,
                "nip_invalide",
            ),
            (E::PatientInconnu, StatusCode::NOT_FOUND, "patient_inconnu"),
            (
                E::ReferenceTransactionDejaUtilisee,
                StatusCode::CONFLICT,
                "reference_transaction_deja_utilisee",
            ),
            (
                E::DonneesInvalides("ufr : obligatoire.".into()),
                StatusCode::UNPROCESSABLE_ENTITY,
                "donnees_invalides",
            ),
            (
                E::TropDEchecs {
                    reessayer_dans_secs: 42,
                },
                StatusCode::TOO_MANY_REQUESTS,
                "trop_de_demandes",
            ),
            (
                E::ContratExpire,
                StatusCode::INTERNAL_SERVER_ERROR,
                "erreur_interne",
            ),
            (
                E::Interne("détail secret".into()),
                StatusCode::INTERNAL_SERVER_ERROR,
                "erreur_interne",
            ),
        ] {
            let (s, Json(corps)) = mapper_erreur(erreur);
            assert_eq!((s, corps.code), (statut, code));
            assert!(!corps.erreur.contains("secret"));
        }
        // Retry-After seulement pour la limite d'échecs.
        let reponse = reponse_erreur_contrat(E::TropDEchecs {
            reessayer_dans_secs: 42,
        });
        assert_eq!(reponse.headers().get(RETRY_AFTER).unwrap(), "42");
        assert!(
            reponse_erreur_contrat(E::PatientInconnu)
                .headers()
                .get(RETRY_AFTER)
                .is_none()
        );
    }

    /// Les routes par le vrai extracteur `AgentMunaseb` (données fictives) :
    /// - un jeton patient est refusé en 401, l'agent d'une MUNASEB non
    ///   validée en 403, avant tout appel de route ;
    /// - l'agent adhère (201), retrouve le contrat par sa carte et par son
    ///   identifiant, renouvelle (409 pendant la carence), lit et modifie les
    ///   paramètres ; aucune réponse ne contient le NIP ni le téléphone.
    #[sqlx::test]
    #[ignore]
    async fn routes_du_contrat_par_l_extracteur(pool: sqlx::PgPool) {
        use axum::extract::FromRequestParts;
        use axum::http::{Request, header::AUTHORIZATION};
        use axum::response::IntoResponse;
        use laaficare_backend::config::CleTotp;
        use laaficare_backend::contrat_assurance_munaseb::{ModePaiement, NouveauPaiement};
        use laaficare_backend::jwt::{JwtService, SorteJeton};
        use laaficare_backend::{nip, session};
        use std::sync::Arc;

        let state = AppState {
            db: pool,
            jwt: JwtService::new(&"a".repeat(32)),
            sms: Arc::new(laaficare_backend::sms::SmsSenderConsole),
            cle_totp: Arc::new(CleTotp::depuis_octets([7; 32])),
            email: Arc::new(laaficare_backend::email::EmailSenderConsole),
            url_application_web: Arc::from("http://localhost:4200"),
            contact_securite: Arc::from("securite@example.org"),
            taches_reinitialisation: Arc::new(Default::default()),
        };
        let db = state.db.clone();

        // Patient fictif.
        let nip = (0..=9)
            .map(|c| format!("900000000045001{c}"))
            .find(|n| nip::valider(n))
            .unwrap();
        let identite: Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (nom, prenom, telephone) VALUES ('OUEDRAOGO', 'Awa', '+22670450001') RETURNING id",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO patient (utilisateur_id, nip, date_naissance, lieu_naissance) VALUES ($1, $2, '2000-01-01', 'Ouagadougou')",
        )
        .bind(identite)
        .bind(&nip)
        .execute(&db)
        .await
        .unwrap();

        // Compte professionnel fictif, email vérifié.
        let compte_pro = |telephone: &'static str, nom: &'static str| {
            let db = db.clone();
            async move {
                let u: Uuid = sqlx::query_scalar(
                    "INSERT INTO utilisateur (nom, prenom, telephone, email) VALUES ($2, 'Prénom', $1, $1 || '@example.org') RETURNING id",
                )
                .bind(telephone)
                .bind(nom)
                .fetch_one(&db)
                .await
                .unwrap();
                sqlx::query("UPDATE utilisateur SET email_verifie_le = now() WHERE id = $1")
                    .bind(u)
                    .execute(&db)
                    .await
                    .unwrap();
                sqlx::query_scalar::<_, Uuid>(
                    "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'professionnel', 'x') RETURNING id",
                )
                .bind(u)
                .fetch_one(&db)
                .await
                .unwrap()
            }
        };
        let structure = |createur: Uuid, statut: &'static str| {
            let db = db.clone();
            async move {
                sqlx::query_scalar::<_, Uuid>(
                    "INSERT INTO structure (type_structure, nom, telephone, telephone_urgence, region, email, commune, province, \
                                            latitude, longitude, createur_compte_id, statut) \
                     VALUES ('munaseb', 'MUNASEB', '+22670000000', '+22670000000', 'Région fictive', 'structure@example.org', \
                             'Commune fictive', 'Province fictive', 12.371400, -1.519700, $1, $2) RETURNING id",
                )
                .bind(createur)
                .bind(statut)
                .fetch_one(&db)
                .await
                .unwrap()
            }
        };
        // Affectation en exercice et son en-tête `Authorization`.
        let en_exercice = |compte: Uuid, structure: Uuid, role: &'static str| {
            let state = state.clone();
            async move {
                let affectation: Uuid =
                    sqlx::query_scalar("INSERT INTO affectation (compte_id, structure_id, role) VALUES ($1, $2, $3) RETURNING id")
                        .bind(compte)
                        .bind(structure)
                        .bind(role)
                        .fetch_one(&state.db)
                        .await
                        .unwrap();
                let session = session::ouvrir(&state.db, compte, Some(affectation))
                    .await
                    .unwrap();
                let jeton = state
                    .jwt
                    .emettre(
                        SorteJeton::ProfessionnelEnExercice,
                        compte,
                        0,
                        Some(affectation),
                        Some(session.session_id),
                    )
                    .unwrap();
                format!("Bearer {jeton}")
            }
        };
        let parts = |en_tete: &str| {
            Request::builder()
                .header(AUTHORIZATION, en_tete)
                .body(())
                .unwrap()
                .into_parts()
                .0
        };
        let extraire = |en_tete: String| {
            let state = state.clone();
            async move { AgentMunaseb::from_request_parts(&mut parts(&en_tete), &state).await }
        };

        // Refus avant la route.
        let jeton_patient = state
            .jwt
            .emettre(SorteJeton::Patient, Uuid::max(), 0, None, Some(Uuid::max()))
            .unwrap();
        let refus = extraire(format!("Bearer {jeton_patient}"))
            .await
            .map_err(IntoResponse::into_response)
            .unwrap_err();
        assert_eq!(refus.status(), StatusCode::UNAUTHORIZED);
        let createur_attente = compte_pro("+22670450002", "KABRE").await;
        let attente = structure(createur_attente, "en_attente").await;
        let en_attente = en_exercice(createur_attente, attente, "agent_assurance_munaseb").await;
        let refus = extraire(en_attente)
            .await
            .map_err(IntoResponse::into_response)
            .unwrap_err();
        assert_eq!(refus.status(), StatusCode::FORBIDDEN);

        // La MUNASEB validée : un responsable et un agent.
        let compte_responsable = compte_pro("+22670450004", "ZONGO").await;
        let munaseb = structure(compte_responsable, "validee").await;
        let jeton_responsable = en_exercice(compte_responsable, munaseb, "responsable").await;
        let jeton = en_exercice(
            compte_pro("+22670450003", "KABRE").await,
            munaseb,
            "agent_assurance_munaseb",
        )
        .await;
        let agent = || async { extraire(jeton.clone()).await.unwrap() };
        let membre = |en_tete: String| {
            let state = state.clone();
            async move {
                MembreMunaseb::from_request_parts(&mut parts(&en_tete), &state)
                    .await
                    .unwrap()
            }
        };
        let responsable = |en_tete: String| {
            let state = state.clone();
            async move { ResponsableMunaseb::from_request_parts(&mut parts(&en_tete), &state).await }
        };

        // Rôles : l'agent ne modifie pas les paramètres (403 acces_refuse),
        // le responsable n'enregistre pas de contrat (403).
        let refus = responsable(jeton.clone())
            .await
            .map_err(IntoResponse::into_response)
            .unwrap_err();
        assert_eq!(refus.status(), StatusCode::FORBIDDEN);
        let corps_refus = axum::body::to_bytes(refus.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&corps_refus).unwrap()["code"],
            "acces_refuse"
        );
        let refus = extraire(jeton_responsable.clone())
            .await
            .map_err(IntoResponse::into_response)
            .unwrap_err();
        assert_eq!(refus.status(), StatusCode::FORBIDDEN);

        // Adhésion, puis relecture par la carte et par l'identifiant.
        let corps = Adhesion {
            nip: nip.clone(),
            date_naissance: chrono::NaiveDate::from_ymd_opt(2000, 1, 1).unwrap(),
            numero_carte: "0001/2026".into(),
            ufr: "UFR fictive".into(),
            universite: "Université fictive".into(),
            num_matricule: "MAT-FICTIF".into(),
            personne_a_prevenir: None,
            niveau_etudes: None,
            antenne: None,
            paiement: NouveauPaiement {
                mode: ModePaiement::Especes,
                reference_transaction: None,
                date_declaree: None,
            },
        };
        let (statut, Json(cree)) =
            adherer(agent().await, State(state.clone()), JsonApi(corps.clone()))
                .await
                .unwrap();
        assert_eq!(statut, StatusCode::CREATED);
        let json = serde_json::to_string(&cree).unwrap();
        assert!(
            !json.contains(&nip) && !json.contains("+22670450001"),
            "{json}"
        );
        let refus = adherer(agent().await, State(state.clone()), JsonApi(corps.clone()))
            .await
            .unwrap_err();
        assert_eq!(refus.status(), StatusCode::CONFLICT);

        let Json(trouve) = rechercher(
            agent().await,
            State(state.clone()),
            RequeteApi(RechercheCarte {
                numero_carte: "0001/2026".into(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(trouve.id, cree.id);
        let (s, Json(e)) = rechercher(
            agent().await,
            State(state.clone()),
            RequeteApi(RechercheCarte {
                numero_carte: "9999/2026".into(),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!((s, e.code), (StatusCode::NOT_FOUND, "contrat_inconnu"));
        let Json(lu) = detail(agent().await, State(state.clone()), CheminApi(cree.id))
            .await
            .unwrap();
        assert_eq!(lu.periodes.len(), 1);
        let (s, _) = detail(agent().await, State(state.clone()), CheminApi(Uuid::max()))
            .await
            .unwrap_err();
        assert_eq!(s, StatusCode::NOT_FOUND);

        // Renouvellement pendant la carence : 409.
        let renouvellement = Renouvellement {
            numero_carte: "0001/2027".into(),
            niveau_etudes: None,
            antenne: None,
            paiement: NouveauPaiement {
                mode: ModePaiement::Especes,
                reference_transaction: None,
                date_declaree: None,
            },
        };
        let (s, Json(e)) = renouveler(
            agent().await,
            State(state.clone()),
            CheminApi(cree.id),
            JsonApi(renouvellement),
        )
        .await
        .unwrap_err();
        assert_eq!(
            (s, e.code),
            (StatusCode::CONFLICT, "periode_future_deja_payee")
        );

        // Paramètres : lecture par l'agent et par le responsable,
        // modification par le responsable, historique, limite hors bornes.
        let Json(actuels) = parametres(membre(jeton.clone()).await, State(state.clone()))
            .await
            .unwrap();
        let Json(memes) = parametres(
            membre(jeton_responsable.clone()).await,
            State(state.clone()),
        )
        .await
        .unwrap();
        assert_eq!(actuels.id, memes.id);
        let nouveaux = Parametres {
            fiche_fcfa: 200,
            ..actuels.valeurs
        };
        let Json(version) = modifier_parametres(
            responsable(jeton_responsable.clone()).await.unwrap(),
            State(state.clone()),
            JsonApi(nouveaux),
        )
        .await
        .unwrap();
        assert_eq!(
            (version.valeurs.fiche_fcfa, version.auteur_nom.as_deref()),
            (200, Some("ZONGO"))
        );
        let Json(h) = historique_parametres(
            membre(jeton.clone()).await,
            State(state.clone()),
            RequeteApi(FiltreHistorique {
                limite: Some(1),
                avant: None,
            }),
        )
        .await
        .unwrap();
        assert_eq!((h.versions.len(), h.suivant), (1, Some(version.id)));
        let (s, Json(e)) = historique_parametres(
            membre(jeton.clone()).await,
            State(state.clone()),
            RequeteApi(FiltreHistorique {
                limite: Some(101),
                avant: None,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!((s, e.code), (StatusCode::BAD_REQUEST, "parametre_invalide"));
        let (s, Json(e)) = modifier_parametres(
            responsable(jeton_responsable.clone()).await.unwrap(),
            State(state.clone()),
            JsonApi(Parametres {
                plafond_periode_fcfa: 0,
                ..nouveaux
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(
            (s, e.code),
            (StatusCode::UNPROCESSABLE_ENTITY, "donnees_invalides")
        );

        // Limite d'échecs : 5 couples faux, puis 429 avec Retry-After.
        let mut faux = corps;
        faux.date_naissance = chrono::NaiveDate::from_ymd_opt(1999, 12, 31).unwrap();
        faux.numero_carte = "0002/2026".into();
        for _ in 0..contrat::ECHECS_ADHESION_MAX {
            let refus = adherer(agent().await, State(state.clone()), JsonApi(faux.clone()))
                .await
                .unwrap_err();
            assert_eq!(refus.status(), StatusCode::NOT_FOUND);
        }
        let refus = adherer(agent().await, State(state.clone()), JsonApi(faux))
            .await
            .unwrap_err();
        assert_eq!(refus.status(), StatusCode::TOO_MANY_REQUESTS);
        let secondes: u64 = refus
            .headers()
            .get(RETRY_AFTER)
            .unwrap()
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        assert!((1..=15 * 60).contains(&secondes));
        let corps_refus = axum::body::to_bytes(refus.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&corps_refus).unwrap()["code"],
            "trop_de_demandes"
        );
    }
}
