//! Routes HTTP de la pharmacie (étapes 3 à 5 ; CLAUDE.md §15 ; contrat dans
//! docs/api.md, section 2.15) : catalogue, stock par lots, ventes, pertes,
//! inventaire express, journal, prix, seuils, réglages, alertes et
//! équivalents par DCI.
//!
//! Droits (D5 et décision du porteur du 2026-11-16) :
//! - `PersonnelPharmacie` (pharmacien ou responsable d'une pharmacie validée
//!   et sous licence) : lectures, entrées, ventes, pertes, inventaires,
//!   annulations, seuils ;
//! - `ResponsablePharmacie` : prix de vente et réglages (un pharmacien reçoit
//!   403 `acces_refuse` avant la lecture du corps).
//!
//! La pharmacie est toujours celle de l'affectation du jeton, jamais un
//! identifiant de la requête : un objet d'une autre pharmacie répond 404,
//! comme un objet inexistant (OWASP API1:2023). Aucune logique métier ici :
//! elle est dans `stock_pharmacie` et `catalogue`.
//!
//! Clé d'idempotence (draft IETF *The Idempotency-Key HTTP Header Field*) :
//! chaque création accepte l'en-tête `Idempotency-Key`. La même requête
//! renvoyée avec la même clé renvoie le même objet, sans rien créer ; la même
//! clé avec une autre requête répond 422 `cle_idempotence_reutilisee` (le
//! draft propose 422) ; une clé mal formée répond 400
//! `cle_idempotence_invalide` (le draft propose 400 pour un en-tête absent
//! ou invalide).

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};
use sqlx::types::Uuid;

use crate::AppState;
use crate::erreur_api::{
    CheminApi, Complement, ErreurApi, JsonApi, ReponseErreur, RequeteApi, reponse_erreur, reponse_erreur_interne,
};
use crate::extracteur_jwt::{PersonnelPharmacie, ResponsablePharmacie};
use laaficare_backend::benefice_pharmacie::{
    self as benefice, AnnulationDepense, ChoixPeriode, Depense, ErreurBenefice, NouvelleDepense, Rapport,
};
use laaficare_backend::catalogue::{self, Medicament};
use laaficare_backend::idempotence::{self, Cle, ErreurIdempotence};
use laaficare_backend::stock_pharmacie::{
    self as stock, Alertes, Annulation, Equivalents, ErreurStock, EtatMedicament, Inventaire, Lot, Mouvement,
    NouvelInventaire, NouvelleEntree, NouvellePerte, NouvelleVente, PrixVente, Reglages, Seuils, Vente,
};

// Mêmes bornes que les autres listes de l'API.
const LIMITE_PAR_DEFAUT: i64 = 50;
const LIMITE_MAX: i64 = 100;
/// Recherche dans le catalogue : 2 caractères au moins (une lettre seule
/// rendrait presque tout le catalogue), 100 au plus.
const RECHERCHE_MIN: usize = 2;
const RECHERCHE_MAX: usize = 100;

pub(crate) fn mapper_erreur(erreur: ErreurStock) -> ReponseErreur {
    use ErreurStock as E;
    let (statut, code) = match &erreur {
        E::MedicamentInconnu => (StatusCode::NOT_FOUND, "medicament_inconnu"),
        E::LotInconnu => (StatusCode::NOT_FOUND, "lot_inconnu"),
        E::VenteInconnue => (StatusCode::NOT_FOUND, "vente_inconnue"),
        E::MouvementInconnu => (StatusCode::NOT_FOUND, "mouvement_inconnu"),
        E::LotPerime => (StatusCode::CONFLICT, "lot_perime"),
        E::StockInsuffisant { medicament_id, quantite_disponible } => {
            let complement = Complement {
                medicament_id: *medicament_id,
                quantite_disponible: Some(*quantite_disponible),
                ..Default::default()
            };
            return (StatusCode::CONFLICT, Json(ErreurApi::avec("stock_insuffisant", erreur.to_string(), complement)));
        }
        E::PrixNonDefini { medicament_id } => {
            let complement = Complement { medicament_id: Some(*medicament_id), ..Default::default() };
            return (StatusCode::CONFLICT, Json(ErreurApi::avec("prix_non_defini", erreur.to_string(), complement)));
        }
        E::DejaAnnule => (StatusCode::CONFLICT, "deja_annule"),
        E::AnnulationImpossible => (StatusCode::CONFLICT, "annulation_impossible"),
        E::Idempotence(ErreurIdempotence::CleInvalide) => (StatusCode::BAD_REQUEST, "cle_idempotence_invalide"),
        E::Idempotence(ErreurIdempotence::CleReutilisee) => {
            (StatusCode::UNPROCESSABLE_ENTITY, "cle_idempotence_reutilisee")
        }
        E::DonneesInvalides(_) => (StatusCode::UNPROCESSABLE_ENTITY, "donnees_invalides"),
        E::Interne(detail) => return reponse_erreur_interne("stock pharmacie", detail),
    };
    reponse_erreur(statut, code, erreur.to_string())
}

/// Lit l'en-tête `Idempotency-Key`, facultatif. Une valeur qui n'est pas du
/// texte ASCII visible, ou présente deux fois, est refusée.
pub(crate) fn cle_idempotence(entetes: &HeaderMap) -> Result<Option<Cle>, ReponseErreur> {
    let mut valeurs = entetes.get_all(idempotence::ENTETE).iter();
    let Some(valeur) = valeurs.next() else {
        return Ok(None);
    };
    let invalide = || mapper_erreur(ErreurStock::Idempotence(ErreurIdempotence::CleInvalide));
    if valeurs.next().is_some() {
        return Err(invalide());
    }
    let texte = valeur.to_str().map_err(|_| invalide())?;
    Cle::lire(texte).map(Some).map_err(|_| invalide())
}

pub(crate) fn limite_valide(limite: Option<i64>) -> Result<i64, ReponseErreur> {
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

fn interne(contexte: &'static str) -> impl FnOnce(sqlx::Error) -> ReponseErreur {
    move |e| reponse_erreur_interne(contexte, &e.to_string())
}

// ---------------------------------------------------------------------
// Catalogue
// ---------------------------------------------------------------------

#[derive(Deserialize)]
pub struct Recherche {
    pub q: String,
    pub limite: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct ListeMedicaments {
    pub medicaments: Vec<Medicament>,
}

/// `GET /api/pharmacie/medicaments?q=` : recherche dans le catalogue
/// national, par dénomination ou par DCI.
pub async fn rechercher_medicaments(
    _personnel: PersonnelPharmacie,
    State(state): State<AppState>,
    RequeteApi(r): RequeteApi<Recherche>,
) -> Result<Json<ListeMedicaments>, ReponseErreur> {
    let limite = limite_valide(r.limite)?;
    let longueur = r.q.trim().chars().count();
    if !(RECHERCHE_MIN..=RECHERCHE_MAX).contains(&longueur) {
        return Err(reponse_erreur(
            StatusCode::BAD_REQUEST,
            "parametre_invalide",
            format!("q doit faire entre {RECHERCHE_MIN} et {RECHERCHE_MAX} caractères"),
        ));
    }
    let medicaments = catalogue::rechercher(&state.db, &r.q, limite).await.map_err(interne("catalogue"))?;
    Ok(Json(ListeMedicaments { medicaments }))
}

/// `GET /api/pharmacie/medicaments/{id}/equivalents` : médicaments de même
/// DCI (exactement les mêmes molécules), avec le stock de la pharmacie.
pub async fn equivalents(
    PersonnelPharmacie(personnel): PersonnelPharmacie,
    State(state): State<AppState>,
    CheminApi(medicament_id): CheminApi<Uuid>,
) -> Result<Json<Equivalents>, ReponseErreur> {
    stock::equivalents(&state.db, personnel.structure_id, medicament_id)
        .await
        .map_err(interne("equivalents"))?
        .map(Json)
        .ok_or_else(|| mapper_erreur(ErreurStock::MedicamentInconnu))
}

// ---------------------------------------------------------------------
// Stock : lectures
// ---------------------------------------------------------------------

#[derive(Deserialize)]
pub struct PageApres {
    pub limite: Option<i64>,
    /// `suivant` de la page précédente.
    pub apres: Option<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct EtatReponse {
    pub medicaments: Vec<EtatMedicament>,
    pub suivant: Option<Uuid>,
}

/// `GET /api/pharmacie/stock` : état du stock, par médicament.
pub async fn etat(
    PersonnelPharmacie(personnel): PersonnelPharmacie,
    State(state): State<AppState>,
    RequeteApi(page): RequeteApi<PageApres>,
) -> Result<Json<EtatReponse>, ReponseErreur> {
    let limite = limite_valide(page.limite)?;
    let medicaments = stock::etat(&state.db, personnel.structure_id, page.apres, limite).await.map_err(interne("etat"))?;
    let suivant = if medicaments.len() as i64 == limite { medicaments.last().map(|m| m.medicament.id) } else { None };
    Ok(Json(EtatReponse { medicaments, suivant }))
}

#[derive(Debug, Serialize)]
pub struct LotsReponse {
    pub medicament_id: Uuid,
    /// Lots encore en stock, dans l'ordre FEFO.
    pub lots: Vec<Lot>,
}

/// `GET /api/pharmacie/stock/{medicament_id}` : lots en stock d'un
/// médicament, dans l'ordre de sortie (FEFO).
pub async fn lots(
    PersonnelPharmacie(personnel): PersonnelPharmacie,
    State(state): State<AppState>,
    CheminApi(medicament_id): CheminApi<Uuid>,
) -> Result<Json<LotsReponse>, ReponseErreur> {
    let lots = stock::lots_du_medicament(&state.db, personnel.structure_id, medicament_id).await.map_err(interne("lots"))?;
    Ok(Json(LotsReponse { medicament_id, lots }))
}

#[derive(Deserialize)]
pub struct PageAvant {
    pub limite: Option<i64>,
    /// `suivant` de la page précédente.
    pub avant: Option<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct JournalReponse {
    pub mouvements: Vec<Mouvement>,
    pub suivant: Option<Uuid>,
}

/// `GET /api/pharmacie/mouvements` : journal du stock, le plus récent
/// d'abord.
pub async fn journal(
    PersonnelPharmacie(personnel): PersonnelPharmacie,
    State(state): State<AppState>,
    RequeteApi(page): RequeteApi<PageAvant>,
) -> Result<Json<JournalReponse>, ReponseErreur> {
    let limite = limite_valide(page.limite)?;
    let mouvements = stock::journal(&state.db, personnel.structure_id, page.avant, limite).await.map_err(interne("journal"))?;
    let suivant = if mouvements.len() as i64 == limite { mouvements.last().map(|m| m.id) } else { None };
    Ok(Json(JournalReponse { mouvements, suivant }))
}

/// `GET /api/pharmacie/ventes/{id}` : une vente et ses lignes.
pub async fn vente(
    PersonnelPharmacie(personnel): PersonnelPharmacie,
    State(state): State<AppState>,
    CheminApi(vente_id): CheminApi<Uuid>,
) -> Result<Json<Vente>, ReponseErreur> {
    stock::vente(&state.db, personnel.structure_id, vente_id)
        .await
        .map_err(interne("vente"))?
        .map(Json)
        .ok_or_else(|| mapper_erreur(ErreurStock::VenteInconnue))
}

/// `GET /api/pharmacie/alertes` : seuils franchis, lots bientôt périmés et
/// lots périmés encore en stock (D11 : dans l'application seulement).
pub async fn alertes(
    PersonnelPharmacie(personnel): PersonnelPharmacie,
    State(state): State<AppState>,
) -> Result<Json<Alertes>, ReponseErreur> {
    stock::alertes(&state.db, personnel.structure_id).await.map(Json).map_err(interne("alertes"))
}

// ---------------------------------------------------------------------
// Stock : écritures
// ---------------------------------------------------------------------

/// `POST /api/pharmacie/entrees` : entrée d'un lot (201).
pub async fn entrer(
    PersonnelPharmacie(personnel): PersonnelPharmacie,
    State(state): State<AppState>,
    entetes: HeaderMap,
    JsonApi(corps): JsonApi<NouvelleEntree>,
) -> Result<(StatusCode, Json<Lot>), ReponseErreur> {
    let cle = cle_idempotence(&entetes)?;
    let lot = stock::entrer(&state.db, personnel, cle.as_ref(), &corps).await.map_err(mapper_erreur)?;
    Ok((StatusCode::CREATED, Json(lot)))
}

/// `POST /api/pharmacie/ventes` : vente au comptoir (201), sans donnée
/// patient (D10).
pub async fn vendre(
    PersonnelPharmacie(personnel): PersonnelPharmacie,
    State(state): State<AppState>,
    entetes: HeaderMap,
    JsonApi(corps): JsonApi<NouvelleVente>,
) -> Result<(StatusCode, Json<Vente>), ReponseErreur> {
    let cle = cle_idempotence(&entetes)?;
    let vente = stock::vendre(&state.db, personnel, cle.as_ref(), &corps).await.map_err(mapper_erreur)?;
    Ok((StatusCode::CREATED, Json(vente)))
}

/// `POST /api/pharmacie/ventes/{id}/annuler` : annulation d'une vente
/// entière (200, la vente annulée).
pub async fn annuler_vente(
    PersonnelPharmacie(personnel): PersonnelPharmacie,
    State(state): State<AppState>,
    CheminApi(vente_id): CheminApi<Uuid>,
    entetes: HeaderMap,
    JsonApi(corps): JsonApi<Annulation>,
) -> Result<Json<Vente>, ReponseErreur> {
    let cle = cle_idempotence(&entetes)?;
    stock::annuler_vente(&state.db, personnel, cle.as_ref(), vente_id, &corps).await.map(Json).map_err(mapper_erreur)
}

/// `POST /api/pharmacie/pertes` : perte d'un lot (201).
pub async fn declarer_perte(
    PersonnelPharmacie(personnel): PersonnelPharmacie,
    State(state): State<AppState>,
    entetes: HeaderMap,
    JsonApi(corps): JsonApi<NouvellePerte>,
) -> Result<(StatusCode, Json<Mouvement>), ReponseErreur> {
    let cle = cle_idempotence(&entetes)?;
    let mouvement = stock::declarer_perte(&state.db, personnel, cle.as_ref(), &corps).await.map_err(mapper_erreur)?;
    Ok((StatusCode::CREATED, Json(mouvement)))
}

/// `POST /api/pharmacie/inventaires` : inventaire express (201).
pub async fn inventorier(
    PersonnelPharmacie(personnel): PersonnelPharmacie,
    State(state): State<AppState>,
    entetes: HeaderMap,
    JsonApi(corps): JsonApi<NouvelInventaire>,
) -> Result<(StatusCode, Json<Inventaire>), ReponseErreur> {
    let cle = cle_idempotence(&entetes)?;
    let inventaire = stock::inventorier(&state.db, personnel, cle.as_ref(), &corps).await.map_err(mapper_erreur)?;
    Ok((StatusCode::CREATED, Json(inventaire)))
}

/// `POST /api/pharmacie/mouvements/{id}/annuler` : annulation d'une entrée,
/// d'une perte ou d'un écart d'inventaire (201, l'écriture inverse).
pub async fn annuler_mouvement(
    PersonnelPharmacie(personnel): PersonnelPharmacie,
    State(state): State<AppState>,
    CheminApi(mouvement_id): CheminApi<Uuid>,
    entetes: HeaderMap,
    JsonApi(corps): JsonApi<Annulation>,
) -> Result<(StatusCode, Json<Mouvement>), ReponseErreur> {
    let cle = cle_idempotence(&entetes)?;
    let mouvement =
        stock::annuler_mouvement(&state.db, personnel, cle.as_ref(), mouvement_id, &corps).await.map_err(mapper_erreur)?;
    Ok((StatusCode::CREATED, Json(mouvement)))
}

// ---------------------------------------------------------------------
// Prix, seuils, réglages
// ---------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct NouveauPrix {
    pub prix_unitaire_fcfa: i64,
}

/// `PUT /api/pharmacie/prix/{medicament_id}` : nouveau prix de vente, par le
/// responsable seulement (D7) ; les ventes passées gardent leur prix.
pub async fn fixer_prix(
    ResponsablePharmacie(responsable): ResponsablePharmacie,
    State(state): State<AppState>,
    CheminApi(medicament_id): CheminApi<Uuid>,
    JsonApi(corps): JsonApi<NouveauPrix>,
) -> Result<Json<PrixVente>, ReponseErreur> {
    stock::fixer_prix(&state.db, responsable, medicament_id, corps.prix_unitaire_fcfa).await.map(Json).map_err(mapper_erreur)
}

/// `PUT /api/pharmacie/seuils/{medicament_id}` : seuils d'alerte (204).
pub async fn fixer_seuils(
    PersonnelPharmacie(personnel): PersonnelPharmacie,
    State(state): State<AppState>,
    CheminApi(medicament_id): CheminApi<Uuid>,
    JsonApi(corps): JsonApi<Seuils>,
) -> Result<StatusCode, ReponseErreur> {
    stock::fixer_seuils(&state.db, personnel, medicament_id, &corps).await.map_err(mapper_erreur)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `PUT /api/pharmacie/reglages` : délai d'alerte de péremption, par le
/// responsable seulement (D9 : `null` désactive l'alerte). Renvoie les
/// réglages enregistrés.
pub async fn regler(
    ResponsablePharmacie(responsable): ResponsablePharmacie,
    State(state): State<AppState>,
    JsonApi(corps): JsonApi<Reglages>,
) -> Result<Json<Reglages>, ReponseErreur> {
    stock::regler(&state.db, responsable, &corps).await.map_err(mapper_erreur)?;
    Ok(Json(corps))
}

// ---------------------------------------------------------------------
// Dépenses et bénéfice net (responsable seulement ; ce n'est pas la
// comptabilité officielle SYSCOHADA)
// ---------------------------------------------------------------------

fn mapper_erreur_benefice(erreur: ErreurBenefice) -> ReponseErreur {
    use ErreurBenefice as E;
    let (statut, code) = match &erreur {
        E::DepenseInconnue => (StatusCode::NOT_FOUND, "depense_inconnue"),
        E::DejaAnnulee => (StatusCode::CONFLICT, "deja_annule"),
        E::AnnulationImpossible => (StatusCode::CONFLICT, "annulation_impossible"),
        // Mêmes codes que pour le stock.
        E::Idempotence(ErreurIdempotence::CleInvalide) => (StatusCode::BAD_REQUEST, "cle_idempotence_invalide"),
        E::Idempotence(ErreurIdempotence::CleReutilisee) => {
            (StatusCode::UNPROCESSABLE_ENTITY, "cle_idempotence_reutilisee")
        }
        E::DonneesInvalides(_) => (StatusCode::UNPROCESSABLE_ENTITY, "donnees_invalides"),
        E::Interne(detail) => return reponse_erreur_interne("benefice pharmacie", detail),
    };
    reponse_erreur(statut, code, erreur.to_string())
}

/// `POST /api/pharmacie/depenses` : une dépense (201).
pub async fn enregistrer_depense(
    ResponsablePharmacie(responsable): ResponsablePharmacie,
    State(state): State<AppState>,
    entetes: HeaderMap,
    JsonApi(corps): JsonApi<NouvelleDepense>,
) -> Result<(StatusCode, Json<Depense>), ReponseErreur> {
    let cle = cle_idempotence(&entetes)?;
    let depense =
        benefice::enregistrer_depense(&state.db, responsable, cle.as_ref(), &corps).await.map_err(mapper_erreur_benefice)?;
    Ok((StatusCode::CREATED, Json(depense)))
}

#[derive(Debug, Serialize)]
pub struct DepensesReponse {
    pub depenses: Vec<Depense>,
    pub suivant: Option<Uuid>,
}

/// `GET /api/pharmacie/depenses?limite=&avant=` : le journal des dépenses.
pub async fn lister_depenses(
    ResponsablePharmacie(responsable): ResponsablePharmacie,
    State(state): State<AppState>,
    RequeteApi(page): RequeteApi<PageAvant>,
) -> Result<Json<DepensesReponse>, ReponseErreur> {
    let limite = limite_valide(page.limite)?;
    let depenses = benefice::lister_depenses(&state.db, responsable.structure_id, page.avant, limite)
        .await
        .map_err(interne("depenses"))?;
    let suivant = if depenses.len() as i64 == limite { depenses.last().map(|d| d.id) } else { None };
    Ok(Json(DepensesReponse { depenses, suivant }))
}

/// `POST /api/pharmacie/depenses/{id}/annuler` : l'écriture d'annulation
/// (201).
pub async fn annuler_depense(
    ResponsablePharmacie(responsable): ResponsablePharmacie,
    State(state): State<AppState>,
    CheminApi(depense_id): CheminApi<Uuid>,
    entetes: HeaderMap,
    JsonApi(corps): JsonApi<AnnulationDepense>,
) -> Result<(StatusCode, Json<Depense>), ReponseErreur> {
    let cle = cle_idempotence(&entetes)?;
    let depense = benefice::annuler_depense(&state.db, responsable, cle.as_ref(), depense_id, &corps)
        .await
        .map_err(mapper_erreur_benefice)?;
    Ok((StatusCode::CREATED, Json(depense)))
}

/// `GET /api/pharmacie/rapport?periode=semaine|mois|annee&date=` ou
/// `?du=&au=` : bénéfice net de la pharmacie sur la période.
pub async fn rapport(
    ResponsablePharmacie(responsable): ResponsablePharmacie,
    State(state): State<AppState>,
    RequeteApi(choix): RequeteApi<ChoixPeriode>,
) -> Result<Json<Rapport>, ReponseErreur> {
    benefice::rapport(&state.db, responsable.structure_id, &choix).await.map(Json).map_err(mapper_erreur_benefice)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use axum::extract::FromRequestParts;
    use axum::http::Request;
    use axum::http::header::AUTHORIZATION;
    use axum::response::IntoResponse;
    use laaficare_backend::config::CleTotp;
    use laaficare_backend::jwt::{JwtService, SorteJeton};
    use laaficare_backend::session;
    use laaficare_backend::sms::SmsSenderConsole;
    use std::sync::Arc;

    pub(crate) fn etat_de_test(pool: &sqlx::PgPool) -> AppState {
        AppState {
            db: pool.clone(),
            jwt: JwtService::new(&"a".repeat(32)),
            sms: Arc::new(SmsSenderConsole),
            cle_totp: Arc::new(CleTotp::depuis_octets([7; 32])),
            email: Arc::new(laaficare_backend::email::EmailSenderConsole),
            url_application_web: Arc::from("http://localhost:4200"),
            contact_securite: Arc::from("securite@example.org"),
            taches_reinitialisation: Arc::new(Default::default()),
        }
    }

    /// Compte professionnel (email vérifié), structure et affectation en
    /// exercice ; renvoie l'en-tête `Authorization`.
    pub(crate) async fn en_exercice(
        state: &AppState,
        telephone: &str,
        type_structure: &str,
        statut: &str,
        role: &str,
        structure: Option<Uuid>,
    ) -> (String, Uuid) {
        let db = &state.db;
        let u: Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (nom, prenom, telephone, email) VALUES ('SAWADOGO', 'Issa', $1, $2) RETURNING id",
        )
        .bind(telephone)
        .bind(format!("{}@example.org", telephone.trim_start_matches('+')))
        .fetch_one(db)
        .await
        .unwrap();
        sqlx::query("UPDATE utilisateur SET email_verifie_le = now() WHERE id = $1").bind(u).execute(db).await.unwrap();
        let compte: Uuid = sqlx::query_scalar(
            "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'professionnel', 'x') RETURNING id",
        )
        .bind(u)
        .fetch_one(db)
        .await
        .unwrap();
        let structure = match structure {
            Some(id) => id,
            None => sqlx::query_scalar(
                "INSERT INTO structure (type_structure, nom, telephone, telephone_urgence, region, email, commune, province, \
                                        latitude, longitude, createur_compte_id, statut) \
                 VALUES ($1, 'Structure fictive', '+22670000000', '+22670000000', 'Région fictive', 's@example.org', \
                         'Commune fictive', 'Province fictive', 12.3714, -1.5197, $2, $3) RETURNING id",
            )
            .bind(type_structure)
            .bind(compte)
            .bind(statut)
            .fetch_one(db)
            .await
            .unwrap(),
        };
        let affectation: Uuid =
            sqlx::query_scalar("INSERT INTO affectation (compte_id, structure_id, role) VALUES ($1, $2, $3) RETURNING id")
                .bind(compte)
                .bind(structure)
                .bind(role)
                .fetch_one(db)
                .await
                .unwrap();
        let session = session::ouvrir(db, compte, Some(affectation)).await.unwrap();
        let jeton = state
            .jwt
            .emettre(SorteJeton::ProfessionnelEnExercice, compte, 0, Some(affectation), Some(session.session_id))
            .unwrap();
        (format!("Bearer {jeton}"), structure)
    }

    fn parts(en_tete: &str, cle: Option<&str>) -> axum::http::request::Parts {
        let mut requete = Request::builder().header(AUTHORIZATION, en_tete);
        if let Some(cle) = cle {
            requete = requete.header(idempotence::ENTETE, cle);
        }
        requete.body(()).unwrap().into_parts().0
    }

    pub(crate) async fn personnel(state: &AppState, en_tete: &str) -> Result<PersonnelPharmacie, StatusCode> {
        PersonnelPharmacie::from_request_parts(&mut parts(en_tete, None), state)
            .await
            .map_err(|r| r.into_response().status())
    }

    pub(crate) async fn responsable(state: &AppState, en_tete: &str) -> Result<ResponsablePharmacie, StatusCode> {
        ResponsablePharmacie::from_request_parts(&mut parts(en_tete, None), state)
            .await
            .map_err(|r| r.into_response().status())
    }

    pub(crate) fn entetes(cle: Option<&str>) -> HeaderMap {
        parts("x", cle).headers
    }

    /// Trois médicaments fictifs posés en SQL (les tests de la bibliothèque
    /// passent par l'import ; ses aides de test ne sont pas visibles ici) :
    /// X et Y de même molécule, Z une association. Renvoie (X, Y, Z).
    pub(crate) async fn catalogue_fictif(pool: &sqlx::PgPool) -> (Uuid, Uuid, Uuid) {
        let admin: Uuid = sqlx::query_scalar(
            "WITH u AS (INSERT INTO utilisateur (nom, prenom, telephone) VALUES ('ADMIN', 'Fictif', '+22670619999') RETURNING id) \
             INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) SELECT id, 'administrateur_laaficare', 'x' FROM u RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        let fichier = "denomination;dci;forme;dosage\n\
                       Produit X fictif;Molécule-A fictive;comprimé;500 mg\n\
                       Produit Y fictif;Molécule-A fictive;comprimé;500 mg\n\
                       Produit Z fictif;Molécule-A fictive + Molécule-B fictive;comprimé;500 mg\n";
        catalogue::importer(pool, admin, "Liste fictive", fichier.as_bytes()).await.unwrap();
        let id = |nom: &'static str| async move {
            sqlx::query_scalar::<_, Uuid>("SELECT id FROM medicament WHERE denomination = $1").bind(nom).fetch_one(pool).await.unwrap()
        };
        (id("Produit X fictif").await, id("Produit Y fictif").await, id("Produit Z fictif").await)
    }

    pub(crate) fn nouvelle_entree(medicament_id: Uuid, lot: &str, jours: i64, quantite: i64, prix_achat: i64) -> NouvelleEntree {
        NouvelleEntree {
            medicament_id,
            numero_lot: lot.into(),
            date_expiration: chrono::Utc::now().date_naive() + chrono::Duration::days(jours),
            quantite,
            prix_achat_unitaire_fcfa: prix_achat,
            fournisseur: "Grossiste fictif".into(),
        }
    }

    /// Accès : un pharmacien et un responsable passent ; un agent MUNASEB,
    /// une pharmacie non validée, un rôle d'une autre sorte : 403. Le prix et
    /// les réglages : responsable seulement. Une pharmacie ne lit ni ne
    /// touche le stock d'une autre (404). Clés d'idempotence par la route.
    #[sqlx::test]
    #[ignore]
    async fn acces_idempotence_et_isolement(pool: sqlx::PgPool) {
        let state = etat_de_test(&pool);
        let (jeton_resp, pharmacie) = en_exercice(&state, "+22670610001", "pharmacie", "validee", "responsable", None).await;
        let (jeton_pharm, _) = en_exercice(&state, "+22670610002", "pharmacie", "validee", "pharmacien", Some(pharmacie)).await;
        let (jeton_autre, _) = en_exercice(&state, "+22670610003", "pharmacie", "validee", "responsable", None).await;
        let (jeton_attente, _) = en_exercice(&state, "+22670610004", "pharmacie", "en_attente", "responsable", None).await;
        let (jeton_agent, _) = en_exercice(&state, "+22670610005", "munaseb", "validee", "agent_assurance_munaseb", None).await;
        let (x, _, _) = catalogue_fictif(&pool).await;

        assert_eq!(personnel(&state, &jeton_agent).await.err(), Some(StatusCode::FORBIDDEN));
        assert_eq!(personnel(&state, &jeton_attente).await.err(), Some(StatusCode::FORBIDDEN));
        assert_eq!(personnel(&state, "Bearer abc").await.err(), Some(StatusCode::UNAUTHORIZED));
        assert_eq!(responsable(&state, &jeton_pharm).await.err(), Some(StatusCode::FORBIDDEN));

        // Prix par le responsable ; entrée par le pharmacien, rejouée.
        let Json(prix) = fixer_prix(
            responsable(&state, &jeton_resp).await.unwrap(),
            State(state.clone()),
            CheminApi(x),
            JsonApi(NouveauPrix { prix_unitaire_fcfa: 300 }),
        )
        .await
        .unwrap();
        assert_eq!(prix.prix_unitaire_fcfa, 300);
        let corps = nouvelle_entree(x, "LOT-1", 60, 10, 100);
        let entree = |cle: Option<&'static str>, corps: NouvelleEntree| {
            let state = state.clone();
            let jeton = jeton_pharm.clone();
            async move {
                entrer(personnel(&state, &jeton).await.unwrap(), State(state.clone()), entetes(cle), JsonApi(corps)).await
            }
        };
        let (statut, Json(lot)) = entree(Some("e-1"), corps.clone()).await.unwrap();
        assert_eq!(statut, StatusCode::CREATED);
        let (_, Json(rejoue)) = entree(Some("e-1"), corps.clone()).await.unwrap();
        assert_eq!(rejoue.id, lot.id);
        let (s, Json(e)) = entree(Some("e-1"), nouvelle_entree(x, "LOT-2", 60, 10, 100)).await.unwrap_err();
        assert_eq!((s, e.code), (StatusCode::UNPROCESSABLE_ENTITY, "cle_idempotence_reutilisee"));
        let (s, Json(e)) = entree(Some("é"), corps.clone()).await.unwrap_err();
        assert_eq!((s, e.code), (StatusCode::BAD_REQUEST, "cle_idempotence_invalide"));
        let mut deux = entetes(Some("a"));
        deux.append(idempotence::ENTETE, "b".parse().unwrap());
        assert!(cle_idempotence(&deux).is_err(), "deux en-têtes");

        // Vente, puis stock insuffisant avec ses champs.
        let vente_de = |quantite| NouvelleVente {
            sur_ordonnance_papier: false,
            lignes: vec![stock::LigneDemandee { medicament_id: x, quantite, lot_id: None }],
        };
        let (_, Json(v)) =
            vendre(personnel(&state, &jeton_pharm).await.unwrap(), State(state.clone()), entetes(None), JsonApi(vente_de(2)))
                .await
                .unwrap();
        assert_eq!(v.total_fcfa, 600);
        let (s, Json(e)) =
            vendre(personnel(&state, &jeton_pharm).await.unwrap(), State(state.clone()), entetes(None), JsonApi(vente_de(50)))
                .await
                .unwrap_err();
        assert_eq!((s, e.code), (StatusCode::CONFLICT, "stock_insuffisant"));
        // Les champs complémentaires sont au premier niveau du JSON.
        let json = serde_json::to_value(&e).unwrap();
        assert_eq!((json["medicament_id"].as_str(), json["quantite_disponible"].as_i64()), (Some(x.to_string().as_str()), Some(8)));
        assert!(json.get("complement").is_none() && json.get("pieces_manquantes").is_none(), "{json}");

        // L'autre pharmacie : rien à voir, rien à toucher.
        let (s, Json(e)) = vente(personnel(&state, &jeton_autre).await.unwrap(), State(state.clone()), CheminApi(v.id))
            .await
            .unwrap_err();
        assert_eq!((s, e.code), (StatusCode::NOT_FOUND, "vente_inconnue"));
        let (s, _) = annuler_vente(
            personnel(&state, &jeton_autre).await.unwrap(),
            State(state.clone()),
            CheminApi(v.id),
            entetes(None),
            JsonApi(Annulation { motif: "Intrusion".into() }),
        )
        .await
        .unwrap_err();
        assert_eq!(s, StatusCode::NOT_FOUND);
        let Json(lots_autre) =
            lots(personnel(&state, &jeton_autre).await.unwrap(), State(state.clone()), CheminApi(x)).await.unwrap();
        assert!(lots_autre.lots.is_empty());
        let Json(journal_autre) = journal(
            personnel(&state, &jeton_autre).await.unwrap(),
            State(state.clone()),
            RequeteApi(PageAvant { limite: None, avant: None }),
        )
        .await
        .unwrap();
        assert!(journal_autre.mouvements.is_empty());
        let Json(etat_autre) = etat(
            personnel(&state, &jeton_autre).await.unwrap(),
            State(state.clone()),
            RequeteApi(PageApres { limite: None, apres: None }),
        )
        .await
        .unwrap();
        assert!(etat_autre.medicaments.is_empty());
        let (s, _) = declarer_perte(
            personnel(&state, &jeton_autre).await.unwrap(),
            State(state.clone()),
            entetes(None),
            JsonApi(NouvellePerte { lot_id: lot.id, quantite: 1, motif: stock::MotifPerte::Casse, commentaire: None }),
        )
        .await
        .unwrap_err();
        assert_eq!(s, StatusCode::NOT_FOUND);

        // Lectures de la pharmacie elle-même ; paramètres hors bornes.
        let Json(e) = etat(
            personnel(&state, &jeton_pharm).await.unwrap(),
            State(state.clone()),
            RequeteApi(PageApres { limite: Some(1), apres: None }),
        )
        .await
        .unwrap();
        assert_eq!((e.medicaments.len(), e.suivant), (1, Some(x)));
        let (s, _) = journal(
            personnel(&state, &jeton_pharm).await.unwrap(),
            State(state.clone()),
            RequeteApi(PageAvant { limite: Some(101), avant: None }),
        )
        .await
        .unwrap_err();
        assert_eq!(s, StatusCode::BAD_REQUEST);
        let Json(trouves) = rechercher_medicaments(
            personnel(&state, &jeton_pharm).await.unwrap(),
            State(state.clone()),
            RequeteApi(Recherche { q: "molécule-a".into(), limite: None }),
        )
        .await
        .unwrap();
        assert_eq!(trouves.medicaments.len(), 3);
        let (s, _) = rechercher_medicaments(
            personnel(&state, &jeton_pharm).await.unwrap(),
            State(state.clone()),
            RequeteApi(Recherche { q: "m".into(), limite: None }),
        )
        .await
        .unwrap_err();
        assert_eq!(s, StatusCode::BAD_REQUEST);
        let (s, Json(e)) =
            equivalents(personnel(&state, &jeton_pharm).await.unwrap(), State(state.clone()), CheminApi(Uuid::max()))
                .await
                .unwrap_err();
        assert_eq!((s, e.code), (StatusCode::NOT_FOUND, "medicament_inconnu"));

        // Réglages par le responsable ; alertes.
        let Json(r) = regler(
            responsable(&state, &jeton_resp).await.unwrap(),
            State(state.clone()),
            JsonApi(Reglages { delai_alerte_peremption_jours: Some(90) }),
        )
        .await
        .unwrap();
        assert_eq!(r.delai_alerte_peremption_jours, Some(90));
        let Json(a) = alertes(personnel(&state, &jeton_pharm).await.unwrap(), State(state.clone())).await.unwrap();
        assert!(a.alerte_peremption_active);
        assert_eq!(a.bientot_perimes.len(), 1);
        let Json(a) = alertes(personnel(&state, &jeton_autre).await.unwrap(), State(state.clone())).await.unwrap();
        assert!(!a.alerte_peremption_active && a.bientot_perimes.is_empty());
    }

    /// Dépenses et rapport : responsable seulement (403 pour un
    /// pharmacien), une pharmacie ne voit ni n'annule les dépenses d'une
    /// autre, période mal formée refusée.
    #[sqlx::test]
    #[ignore]
    async fn depenses_et_rapport_par_la_route(pool: sqlx::PgPool) {
        let state = etat_de_test(&pool);
        let (jeton_resp, pharmacie) = en_exercice(&state, "+22670620001", "pharmacie", "validee", "responsable", None).await;
        let (jeton_pharm, _) = en_exercice(&state, "+22670620002", "pharmacie", "validee", "pharmacien", Some(pharmacie)).await;
        let (jeton_autre, _) = en_exercice(&state, "+22670620003", "pharmacie", "validee", "responsable", None).await;

        // Le pharmacien n'atteint aucune de ces routes.
        assert_eq!(responsable(&state, &jeton_pharm).await.err(), Some(StatusCode::FORBIDDEN));

        let aujourdhui = chrono::Utc::now().date_naive();
        let corps = NouvelleDepense {
            date_depense: aujourdhui,
            categorie: benefice::Categorie::Loyer,
            montant_fcfa: 50_000,
            libelle: "Loyer fictif".into(),
        };
        let (statut, Json(d)) = enregistrer_depense(
            responsable(&state, &jeton_resp).await.unwrap(),
            State(state.clone()),
            entetes(Some("dep-1")),
            JsonApi(corps.clone()),
        )
        .await
        .unwrap();
        assert_eq!(statut, StatusCode::CREATED);
        let (s, Json(e)) = enregistrer_depense(
            responsable(&state, &jeton_resp).await.unwrap(),
            State(state.clone()),
            entetes(Some("dep-1")),
            JsonApi(NouvelleDepense { montant_fcfa: 1, ..corps.clone() }),
        )
        .await
        .unwrap_err();
        assert_eq!((s, e.code), (StatusCode::UNPROCESSABLE_ENTITY, "cle_idempotence_reutilisee"));

        // L'autre pharmacie.
        let (s, Json(e)) = annuler_depense(
            responsable(&state, &jeton_autre).await.unwrap(),
            State(state.clone()),
            CheminApi(d.id),
            entetes(None),
            JsonApi(AnnulationDepense { motif: "Intrusion".into() }),
        )
        .await
        .unwrap_err();
        assert_eq!((s, e.code), (StatusCode::NOT_FOUND, "depense_inconnue"));
        let Json(liste) = lister_depenses(
            responsable(&state, &jeton_autre).await.unwrap(),
            State(state.clone()),
            RequeteApi(PageAvant { limite: None, avant: None }),
        )
        .await
        .unwrap();
        assert!(liste.depenses.is_empty());

        // Rapports.
        let semaine = ChoixPeriode { periode: Some(benefice::Periode::Semaine), ..Default::default() };
        let Json(r) = rapport(responsable(&state, &jeton_resp).await.unwrap(), State(state.clone()), RequeteApi(semaine.clone()))
            .await
            .unwrap();
        assert_eq!((r.depenses.total_fcfa, r.benefice_net_fcfa), (50_000, -50_000));
        let Json(r) = rapport(responsable(&state, &jeton_autre).await.unwrap(), State(state.clone()), RequeteApi(semaine))
            .await
            .unwrap();
        assert_eq!(r.depenses.total_fcfa, 0);
        let (s, Json(e)) =
            rapport(responsable(&state, &jeton_resp).await.unwrap(), State(state.clone()), RequeteApi(ChoixPeriode::default()))
                .await
                .unwrap_err();
        assert_eq!((s, e.code), (StatusCode::UNPROCESSABLE_ENTITY, "donnees_invalides"));

        // Annulation par le responsable.
        let (s, Json(a)) = annuler_depense(
            responsable(&state, &jeton_resp).await.unwrap(),
            State(state.clone()),
            CheminApi(d.id),
            entetes(None),
            JsonApi(AnnulationDepense { motif: "Erreur".into() }),
        )
        .await
        .unwrap();
        assert_eq!((s, a.montant_fcfa), (StatusCode::CREATED, -50_000));
    }
}
