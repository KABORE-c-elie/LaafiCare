//! Stock d'une pharmacie (étapes 4 et 5 ; CLAUDE.md §15 ; CDC BF-04 et
//! BF-05 sans ordonnance ; migration 0031).
//!
//! **Le stock est un journal.** Chaque entrée crée un lot ; chaque entrée,
//! vente, perte, écart d'inventaire ou annulation est un mouvement, jamais
//! modifié ni effacé. La quantité d'un lot est la somme de ses mouvements ;
//! la base refuse qu'elle devienne négative (trigger de la 0031).
//!
//! Règles :
//! - **FEFO** (D8) : une vente sort d'abord le lot qui périme le plus tôt,
//!   sauf si le pharmacien choisit un lot ; source OMS à vérifier
//!   (docs/points-ouverts.md) ;
//! - **lot périmé** : un lot est utilisable jusqu'à sa date d'expiration
//!   **comprise** (date du Burkina, D9) ; après, il ne se vend plus et ne
//!   compte plus dans le stock disponible ; il reste en stock jusqu'à sa
//!   sortie en perte (motif `peremption`) ;
//! - **prix figés** : chaque mouvement garde le prix d'achat de son lot, et
//!   une vente son prix de vente (besoin du porteur du 2026-11-16) ;
//! - **prix de vente** : fixé par le **responsable** de la pharmacie (une
//!   version par changement) ; sans prix fixé, un médicament ne se vend pas
//!   (valeur bloquante par défaut, règle « rien ne s'invente ») ;
//! - **une pharmacie ne voit et ne touche que son stock** : la structure
//!   vient toujours de l'affectation de l'appelant, jamais de la requête ; un
//!   lot, une vente ou un mouvement d'une autre pharmacie répond comme un
//!   objet inexistant (OWASP API1:2023) ; la base le garantit aussi par des
//!   clés étrangères composites (lot, vente et auteur de la même pharmacie).
//!
//! **Ordre des verrous** : les lots, toujours **par identifiant croissant**,
//! quelle que soit l'opération (l'ordre FEFO de sortie est appliqué ensuite,
//! en mémoire) : deux opérations qui touchent les mêmes lots ne peuvent pas
//! se bloquer mutuellement. Aucune opération du stock ne verrouille autre
//! chose.

use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use sqlx::types::Uuid;

use crate::catalogue::{Medicament, select_medicament};
use crate::idempotence::{self, Cle, ErreurIdempotence, Reservation};

// Garde-fous techniques contre une faute de frappe (pas des règles de
// métier) : un million d'unités par mouvement, cent millions de FCFA par
// prix unitaire, cent lignes par vente ou par inventaire.
const QUANTITE_MAX: i64 = 1_000_000;
const PRIX_MAX: i64 = 100_000_000;
const LIGNES_MAX: usize = 100;
const TEXTE_COURT_MAX: usize = 100;
const FOURNISSEUR_MAX: usize = 200;
const COMMENTAIRE_MAX: usize = 1000;

/// Rôle d'une affectation qui agit sur le stock (D5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RolePharmacie {
    Pharmacien,
    Responsable,
    /// Auxiliaire de pharmacie (décision du porteur du 2026-11-17, qui
    /// remplace D5) : entrées, ventes, pertes et inventaires seulement ;
    /// jamais les prix d'achat, les dépenses ni le bénéfice net (les routes
    /// retirent les prix d'achat de ses réponses, `routes_pharmacie`).
    Auxiliaire,
}

impl RolePharmacie {
    pub fn as_str(self) -> &'static str {
        match self {
            RolePharmacie::Pharmacien => "pharmacien",
            RolePharmacie::Responsable => "responsable",
            RolePharmacie::Auxiliaire => "auxiliaire_pharmacie",
        }
    }

    /// Le pharmacien et le responsable voient les prix d'achat ;
    /// l'auxiliaire jamais.
    pub fn voit_prix_achat(self) -> bool {
        !matches!(self, RolePharmacie::Auxiliaire)
    }
}

/// L'affectation qui agit : elle donne la pharmacie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AffectationPharmacie {
    pub affectation_id: Uuid,
    pub structure_id: Uuid,
    pub role: RolePharmacie,
}

#[derive(Debug, thiserror::Error)]
pub enum ErreurStock {
    #[error("Médicament inconnu.")]
    MedicamentInconnu,
    #[error("Lot inconnu.")]
    LotInconnu,
    #[error("Vente inconnue.")]
    VenteInconnue,
    #[error("Mouvement inconnu.")]
    MouvementInconnu,
    #[error("Ce lot est périmé : il ne se vend plus.")]
    LotPerime,
    #[error("Stock insuffisant.")]
    StockInsuffisant {
        medicament_id: Option<Uuid>,
        quantite_disponible: i64,
    },
    #[error("Aucun prix de vente n'est fixé pour ce médicament : le responsable doit le fixer.")]
    PrixNonDefini { medicament_id: Uuid },
    #[error("Déjà annulé.")]
    DejaAnnule,
    #[error(
        "Ce mouvement ne s'annule pas ainsi (une vente s'annule en entier, une annulation ne s'annule pas)."
    )]
    AnnulationImpossible,
    #[error("{0}")]
    Idempotence(#[from] ErreurIdempotence),
    /// Le message ne cite que le nom du champ, jamais la valeur saisie.
    #[error("{0}")]
    DonneesInvalides(String),
    #[error("erreur interne : {0}")]
    Interne(String),
}

impl From<sqlx::Error> for ErreurStock {
    fn from(erreur: sqlx::Error) -> Self {
        // Garanties de la base traduites en erreurs nommées (migration
        // 0031) : stock négatif, annulation en double.
        match erreur.as_database_error().and_then(|e| e.constraint()) {
            Some("mouvement_stock_jamais_negatif") => ErreurStock::StockInsuffisant {
                medicament_id: None,
                quantite_disponible: 0,
            },
            Some("mouvement_stock_annule_mouvement_id_key") => ErreurStock::DejaAnnule,
            _ => ErreurStock::Interne(erreur.to_string()),
        }
    }
}

type Erreur = ErreurStock;

fn invalide(message: impl Into<String>) -> Erreur {
    Erreur::DonneesInvalides(message.into())
}

fn texte(valeur: &str, champ: &str, max: usize) -> Result<String, Erreur> {
    let valeur = valeur.trim();
    if valeur.is_empty() || valeur.chars().count() > max || valeur.chars().any(char::is_control) {
        return Err(invalide(format!(
            "{champ} : obligatoire, {max} caractères au plus, sans caractère de contrôle."
        )));
    }
    Ok(valeur.to_string())
}

fn texte_facultatif(
    valeur: Option<&str>,
    champ: &str,
    max: usize,
) -> Result<Option<String>, Erreur> {
    match valeur.map(str::trim) {
        None | Some("") => Ok(None),
        Some(v) => texte(v, champ, max).map(Some),
    }
}

fn quantite_positive(quantite: i64, champ: &str) -> Result<i32, Erreur> {
    if (1..=QUANTITE_MAX).contains(&quantite) {
        Ok(quantite as i32)
    } else {
        Err(invalide(format!("{champ} : entre 1 et {QUANTITE_MAX}.")))
    }
}

fn prix(valeur: i64, champ: &str) -> Result<i32, Erreur> {
    if (0..=PRIX_MAX).contains(&valeur) {
        Ok(valeur as i32)
    } else {
        Err(invalide(format!("{champ} : entre 0 et {PRIX_MAX} FCFA.")))
    }
}

async fn medicament_existe<'e, E: sqlx::PgExecutor<'e>>(
    executeur: E,
    medicament_id: Uuid,
) -> Result<(), Erreur> {
    let existe: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM medicament WHERE id = $1)")
        .bind(medicament_id)
        .fetch_one(executeur)
        .await?;
    if existe {
        Ok(())
    } else {
        Err(Erreur::MedicamentInconnu)
    }
}

/// Date du jour au Burkina, dans la transaction (D9).
async fn aujourdhui(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<NaiveDate, sqlx::Error> {
    sqlx::query_scalar("SELECT (now() AT TIME ZONE 'Africa/Ouagadougou')::date")
        .fetch_one(&mut **tx)
        .await
}

// ---------------------------------------------------------------------
// Lectures
// ---------------------------------------------------------------------

/// Un lot, avec sa quantité en stock.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Lot {
    pub id: Uuid,
    pub medicament_id: Uuid,
    pub numero_lot: String,
    /// Dernier jour utilisable, compris.
    pub date_expiration: NaiveDate,
    /// Mois imprimé sur la boîte (`AAAA-MM`) quand elle n'indiquait que le
    /// mois ; `date_expiration` est alors la veille de ce mois (règle
    /// prudente, voir `expiration`).
    pub mois_expiration_imprime: Option<String>,
    pub prix_achat_unitaire_fcfa: i32,
    pub fournisseur: String,
    pub quantite: i64,
    pub perime: bool,
    pub date_creation: DateTime<Utc>,
}

macro_rules! select_lot {
    () => {
        "SELECT l.id, l.medicament_id, l.numero_lot, l.date_expiration, \
                CASE WHEN l.expiration_mois_seul THEN to_char(l.date_expiration + 1, 'YYYY-MM') END AS mois_expiration_imprime, \
                l.prix_achat_unitaire_fcfa, l.fournisseur, \
                COALESCE((SELECT sum(m.quantite) FROM mouvement_stock m WHERE m.lot_id = l.id), 0)::BIGINT AS quantite, \
                l.date_expiration < (now() AT TIME ZONE 'Africa/Ouagadougou')::date AS perime, l.date_creation \
         FROM lot_stock l"
    };
}

/// Lots d'un médicament dans la pharmacie, encore en stock, dans l'ordre
/// FEFO (le plus proche de sa péremption d'abord).
pub async fn lots_du_medicament(
    pool: &PgPool,
    structure_id: Uuid,
    medicament_id: Uuid,
) -> Result<Vec<Lot>, sqlx::Error> {
    sqlx::query_as(concat!(
        "SELECT * FROM (",
        select_lot!(),
        " WHERE l.structure_id = $1 AND l.medicament_id = $2) x WHERE quantite > 0 ORDER BY date_expiration, id"
    ))
    .bind(structure_id)
    .bind(medicament_id)
    .fetch_all(pool)
    .await
}

/// Une ligne de l'état du stock : un médicament présent dans la pharmacie.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct EtatMedicament {
    #[serde(flatten)]
    #[sqlx(flatten)]
    pub medicament: Medicament,
    /// Lots non périmés.
    pub quantite_disponible: i64,
    pub quantite_perimee: i64,
    /// Valeur de tout le stock de ce médicament au prix d'achat.
    pub valeur_achat_fcfa: i64,
    pub seuil_min: Option<i32>,
    pub seuil_max: Option<i32>,
    pub prix_vente_unitaire_fcfa: Option<i32>,
}

/// État du stock de la pharmacie, par médicament (ceux qui ont eu au moins
/// un lot), par pages sur l'identifiant du médicament.
pub async fn etat(
    pool: &PgPool,
    structure_id: Uuid,
    apres: Option<Uuid>,
    limite: i64,
) -> Result<Vec<EtatMedicament>, sqlx::Error> {
    sqlx::query_as(concat!(
        "WITH lots AS (",
        select_lot!(),
        " WHERE l.structure_id = $1) \
         SELECT med.*, \
                COALESCE(sum(lots.quantite) FILTER (WHERE NOT lots.perime), 0)::BIGINT AS quantite_disponible, \
                COALESCE(sum(lots.quantite) FILTER (WHERE lots.perime), 0)::BIGINT AS quantite_perimee, \
                COALESCE(sum(lots.quantite * lots.prix_achat_unitaire_fcfa), 0)::BIGINT AS valeur_achat_fcfa, \
                s.seuil_min, s.seuil_max, \
                (SELECT p.prix_unitaire_fcfa FROM prix_vente_pharmacie p WHERE p.structure_id = $1 AND p.medicament_id = med.id \
                 ORDER BY p.date_effet DESC, p.id DESC LIMIT 1) AS prix_vente_unitaire_fcfa \
         FROM (",
        select_medicament!(),
        ") med \
         JOIN lots ON lots.medicament_id = med.id \
         LEFT JOIN seuil_stock s ON s.structure_id = $1 AND s.medicament_id = med.id \
         WHERE ($2::uuid IS NULL OR med.id > $2) \
         GROUP BY med.id, med.denomination, med.forme, med.dosage, med.dci, s.seuil_min, s.seuil_max \
         ORDER BY med.id LIMIT $3"
    ))
    .bind(structure_id)
    .bind(apres)
    .bind(limite)
    .fetch_all(pool)
    .await
}

/// Une ligne du journal.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Mouvement {
    pub id: Uuid,
    pub nature: String,
    pub lot_id: Uuid,
    pub medicament_id: Uuid,
    pub denomination: String,
    pub numero_lot: String,
    pub quantite: i32,
    pub prix_achat_unitaire_fcfa: i32,
    pub prix_vente_unitaire_fcfa: Option<i32>,
    pub vente_id: Option<Uuid>,
    pub inventaire_id: Option<Uuid>,
    pub motif_perte: Option<String>,
    pub commentaire: Option<String>,
    pub annule_mouvement_id: Option<Uuid>,
    /// Annulé par ce mouvement, s'il l'a été.
    pub annule_par: Option<Uuid>,
    pub date_mouvement: DateTime<Utc>,
    pub auteur_nom: Option<String>,
    pub auteur_prenom: Option<String>,
}

macro_rules! select_mouvement {
    () => {
        "SELECT mv.id, mv.nature, mv.lot_id, l.medicament_id, med.denomination, l.numero_lot, mv.quantite, \
                mv.prix_achat_unitaire_fcfa, mv.prix_vente_unitaire_fcfa, mv.vente_id, mv.inventaire_id, mv.motif_perte, \
                mv.commentaire, mv.annule_mouvement_id, \
                (SELECT a.id FROM mouvement_stock a WHERE a.annule_mouvement_id = mv.id) AS annule_par, \
                mv.date_mouvement, u.nom AS auteur_nom, u.prenom AS auteur_prenom \
         FROM mouvement_stock mv \
         JOIN lot_stock l ON l.id = mv.lot_id \
         JOIN medicament med ON med.id = l.medicament_id \
         JOIN affectation af ON af.id = mv.auteur_affectation_id \
         JOIN compte c ON c.id = af.compte_id \
         JOIN utilisateur u ON u.id = c.utilisateur_id"
    };
}

/// Journal de la pharmacie, le plus récent d'abord (`avant` : identifiant
/// du dernier mouvement de la page précédente).
pub async fn journal(
    pool: &PgPool,
    structure_id: Uuid,
    avant: Option<Uuid>,
    limite: i64,
) -> Result<Vec<Mouvement>, sqlx::Error> {
    sqlx::query_as(concat!(
        select_mouvement!(),
        " WHERE mv.structure_id = $1 AND ($2::uuid IS NULL OR mv.id < $2) ORDER BY mv.id DESC LIMIT $3"
    ))
    .bind(structure_id)
    .bind(avant)
    .bind(limite)
    .fetch_all(pool)
    .await
}

async fn mouvement(pool: &PgPool, structure_id: Uuid, id: Uuid) -> Result<Mouvement, sqlx::Error> {
    sqlx::query_as(concat!(
        select_mouvement!(),
        " WHERE mv.structure_id = $1 AND mv.id = $2"
    ))
    .bind(structure_id)
    .bind(id)
    .fetch_one(pool)
    .await
}

async fn lot(pool: &PgPool, structure_id: Uuid, id: Uuid) -> Result<Lot, sqlx::Error> {
    sqlx::query_as(concat!(
        select_lot!(),
        " WHERE l.structure_id = $1 AND l.id = $2"
    ))
    .bind(structure_id)
    .bind(id)
    .fetch_one(pool)
    .await
}

/// Une vente et ses lignes.
#[derive(Debug, Serialize)]
pub struct Vente {
    pub id: Uuid,
    pub sur_ordonnance_papier: bool,
    pub date_vente: DateTime<Utc>,
    pub auteur_nom: Option<String>,
    pub auteur_prenom: Option<String>,
    pub lignes: Vec<Mouvement>,
    /// Total des lignes non annulées.
    pub total_fcfa: i64,
    pub annulee: bool,
}

/// Une vente de la pharmacie, `None` si elle n'existe pas ou est d'une autre
/// pharmacie.
pub async fn vente(
    pool: &PgPool,
    structure_id: Uuid,
    vente_id: Uuid,
) -> Result<Option<Vente>, sqlx::Error> {
    // Ordonnance papier, date, nom et prénom de l'auteur.
    type EnteteVente = (bool, DateTime<Utc>, Option<String>, Option<String>);
    let entete: Option<EnteteVente> = sqlx::query_as(
        "SELECT v.sur_ordonnance_papier, v.date_vente, u.nom, u.prenom FROM vente v \
         JOIN affectation af ON af.id = v.auteur_affectation_id JOIN compte c ON c.id = af.compte_id \
         JOIN utilisateur u ON u.id = c.utilisateur_id \
         WHERE v.structure_id = $1 AND v.id = $2",
    )
    .bind(structure_id)
    .bind(vente_id)
    .fetch_optional(pool)
    .await?;
    let Some((sur_ordonnance_papier, date_vente, auteur_nom, auteur_prenom)) = entete else {
        return Ok(None);
    };
    let lignes: Vec<Mouvement> = sqlx::query_as(concat!(
        select_mouvement!(),
        " WHERE mv.structure_id = $1 AND mv.vente_id = $2 AND mv.nature = 'vente' ORDER BY mv.id"
    ))
    .bind(structure_id)
    .bind(vente_id)
    .fetch_all(pool)
    .await?;
    let total_fcfa = lignes
        .iter()
        .filter(|l| l.annule_par.is_none())
        .map(|l| i64::from(-l.quantite) * i64::from(l.prix_vente_unitaire_fcfa.unwrap_or(0)))
        .sum();
    let annulee = !lignes.is_empty() && lignes.iter().all(|l| l.annule_par.is_some());
    Ok(Some(Vente {
        id: vente_id,
        sur_ordonnance_papier,
        date_vente,
        auteur_nom,
        auteur_prenom,
        lignes,
        total_fcfa,
        annulee,
    }))
}

// ---------------------------------------------------------------------
// Écritures communes
// ---------------------------------------------------------------------

/// Lot verrouillé : (médicament, prix d'achat, date d'expiration, quantité).
struct LotVerrouille {
    id: Uuid,
    medicament_id: Uuid,
    prix_achat: i32,
    date_expiration: NaiveDate,
    quantite: i64,
}

/// Verrouille des lots de la pharmacie, **par identifiant croissant** (ordre
/// unique des verrous, voir l'en-tête du module), et lit leur quantité.
async fn verrouiller_lots(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    structure_id: Uuid,
    lots: &[Uuid],
) -> Result<BTreeMap<Uuid, LotVerrouille>, sqlx::Error> {
    let lignes: Vec<(Uuid, Uuid, i32, NaiveDate)> = sqlx::query_as(
        "SELECT id, medicament_id, prix_achat_unitaire_fcfa, date_expiration FROM lot_stock \
         WHERE structure_id = $1 AND id = ANY($2) ORDER BY id FOR UPDATE",
    )
    .bind(structure_id)
    .bind(lots)
    .fetch_all(&mut **tx)
    .await?;
    let mut resultat = BTreeMap::new();
    for (id, medicament_id, prix_achat, date_expiration) in lignes {
        let quantite: i64 = sqlx::query_scalar(
            "SELECT COALESCE(sum(quantite), 0)::BIGINT FROM mouvement_stock WHERE lot_id = $1",
        )
        .bind(id)
        .fetch_one(&mut **tx)
        .await?;
        resultat.insert(
            id,
            LotVerrouille {
                id,
                medicament_id,
                prix_achat,
                date_expiration,
                quantite,
            },
        );
    }
    Ok(resultat)
}

/// Un mouvement à écrire.
struct NouveauMouvement<'a> {
    nature: &'static str,
    lot_id: Uuid,
    quantite: i32,
    prix_achat: i32,
    prix_vente: Option<i32>,
    vente_id: Option<Uuid>,
    inventaire_id: Option<Uuid>,
    motif_perte: Option<&'a str>,
    commentaire: Option<&'a str>,
    annule_mouvement_id: Option<Uuid>,
}

async fn inserer_mouvement(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    auteur: AffectationPharmacie,
    id: Option<Uuid>,
    m: NouveauMouvement<'_>,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO mouvement_stock (id, structure_id, lot_id, nature, quantite, prix_achat_unitaire_fcfa, \
                                      prix_vente_unitaire_fcfa, vente_id, inventaire_id, motif_perte, commentaire, \
                                      annule_mouvement_id, auteur_affectation_id, auteur_structure_id, auteur_role) \
         VALUES (COALESCE($1, uuidv7()), $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $2, $14) RETURNING id",
    )
    .bind(id)
    .bind(auteur.structure_id)
    .bind(m.lot_id)
    .bind(m.nature)
    .bind(m.quantite)
    .bind(m.prix_achat)
    .bind(m.prix_vente)
    .bind(m.vente_id)
    .bind(m.inventaire_id)
    .bind(m.motif_perte)
    .bind(m.commentaire)
    .bind(m.annule_mouvement_id)
    .bind(auteur.affectation_id)
    .bind(auteur.role.as_str())
    .fetch_one(&mut **tx)
    .await
}

// ---------------------------------------------------------------------
// Entrée en stock (BF-04 : fournisseur, lot, expiration, quantité, prix)
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NouvelleEntree {
    pub medicament_id: Uuid,
    pub numero_lot: String,
    /// Date complète imprimée sur la boîte (`AAAA-MM-JJ`) : dernier jour
    /// utilisable, compris. Exactement un de `date_expiration` et
    /// `mois_expiration`.
    #[serde(default)]
    pub date_expiration: Option<NaiveDate>,
    /// Boîte qui n'indique que le mois et l'année (`AAAA-MM`, ex.
    /// `2027-06` pour « EXP 06/2027 ») : voir `expiration`.
    #[serde(default)]
    pub mois_expiration: Option<String>,
    pub quantite: i64,
    pub prix_achat_unitaire_fcfa: i64,
    pub fournisseur: String,
}

/// Dernier jour utilisable du lot, et si la boîte n'indiquait que le mois.
///
/// Boîte au mois et à l'année : aucune source officielle burkinabè ni de
/// l'UEMOA n'a été trouvée sur la convention (docs/points-ouverts.md ; la
/// pharmacopée des États-Unis, USP, *General Notices*, lit « le dernier jour
/// du mois imprimé », mais elle ne s'applique pas au Burkina). Règle la plus
/// prudente pour le patient (demande du porteur du 2026-11-16) : le lot est
/// périmé **dès le premier jour du mois imprimé** ; on enregistre donc le
/// dernier jour du mois précédent (`2027-06` donne le 31/05/2027). Le prix
/// de cette prudence : au plus un mois de stock sorti plus tôt.
pub fn expiration(e: &NouvelleEntree) -> Result<(NaiveDate, bool), Erreur> {
    match (e.date_expiration, e.mois_expiration.as_deref()) {
        (Some(date), None) => Ok((date, false)),
        (None, Some(mois)) => {
            let mois = mois.trim();
            // Exactement `AAAA-MM` : 7 caractères, mois de 01 à 12.
            let premier = (mois.len() == 7 && mois.as_bytes()[4] == b'-')
                .then(|| NaiveDate::parse_from_str(&format!("{mois}-01"), "%Y-%m-%d").ok())
                .flatten()
                .ok_or_else(|| invalide("mois_expiration : au format AAAA-MM (ex. 2027-06)."))?;
            let veille = premier
                .pred_opt()
                .ok_or_else(|| invalide("mois_expiration : hors limites."))?;
            Ok((veille, true))
        }
        _ => Err(invalide(
            "date_expiration ou mois_expiration : donner exactement l'un des deux.",
        )),
    }
}

/// Entrée en stock : un lot et son mouvement d'entrée. Un lot déjà périmé
/// n'entre pas (il serait aussitôt une perte).
pub async fn entrer(
    pool: &PgPool,
    auteur: AffectationPharmacie,
    cle: Option<&Cle>,
    e: &NouvelleEntree,
) -> Result<Lot, Erreur> {
    let quantite = quantite_positive(e.quantite, "quantite")?;
    let prix_achat = prix(e.prix_achat_unitaire_fcfa, "prix_achat_unitaire_fcfa")?;
    let numero_lot = texte(&e.numero_lot, "numero_lot", TEXTE_COURT_MAX)?;
    let fournisseur = texte(&e.fournisseur, "fournisseur", FOURNISSEUR_MAX)?;
    let (date_expiration, mois_seul) = expiration(e)?;

    let mut tx = pool.begin().await?;
    let lot_id = match idempotence::reserver(
        &mut tx,
        auteur.structure_id,
        cle,
        "entree",
        &idempotence::empreinte("entree", e),
        None,
    )
    .await??
    {
        Reservation::DejaFaite(id) => {
            drop(tx);
            return Ok(lot(pool, auteur.structure_id, id).await?);
        }
        Reservation::Nouvelle(id) => id,
    };
    medicament_existe(&mut *tx, e.medicament_id).await?;
    if date_expiration < aujourdhui(&mut tx).await? {
        return Err(invalide("date_expiration : ce lot est déjà périmé."));
    }
    sqlx::query(
        "INSERT INTO lot_stock (id, structure_id, medicament_id, numero_lot, date_expiration, prix_achat_unitaire_fcfa, \
                                fournisseur, expiration_mois_seul) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(lot_id)
    .bind(auteur.structure_id)
    .bind(e.medicament_id)
    .bind(&numero_lot)
    .bind(date_expiration)
    .bind(prix_achat)
    .bind(&fournisseur)
    .bind(mois_seul)
    .execute(&mut *tx)
    .await?;
    inserer_mouvement(
        &mut tx,
        auteur,
        None,
        NouveauMouvement {
            nature: "entree",
            lot_id,
            quantite,
            prix_achat,
            prix_vente: None,
            vente_id: None,
            inventaire_id: None,
            motif_perte: None,
            commentaire: None,
            annule_mouvement_id: None,
        },
    )
    .await?;
    tx.commit().await?;
    Ok(lot(pool, auteur.structure_id, lot_id).await?)
}

// ---------------------------------------------------------------------
// Vente au comptoir (D10)
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LigneDemandee {
    pub medicament_id: Uuid,
    pub quantite: i64,
    /// Lot choisi par le pharmacien ; sinon FEFO.
    pub lot_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NouvelleVente {
    pub sur_ordonnance_papier: bool,
    pub lignes: Vec<LigneDemandee>,
}

/// Vente : un en-tête et une ligne par lot sorti, au prix de vente en
/// vigueur et au prix d'achat du lot, figés. Tout ou rien.
pub async fn vendre(
    pool: &PgPool,
    auteur: AffectationPharmacie,
    cle: Option<&Cle>,
    v: &NouvelleVente,
) -> Result<Vente, Erreur> {
    if v.lignes.is_empty() || v.lignes.len() > LIGNES_MAX {
        return Err(invalide(format!("lignes : entre 1 et {LIGNES_MAX}.")));
    }
    for ligne in &v.lignes {
        quantite_positive(ligne.quantite, "lignes.quantite")?;
    }

    let mut tx = pool.begin().await?;
    let vente_id = match idempotence::reserver(
        &mut tx,
        auteur.structure_id,
        cle,
        "vente",
        &idempotence::empreinte("vente", v),
        None,
    )
    .await??
    {
        Reservation::DejaFaite(id) => {
            drop(tx);
            return vente(pool, auteur.structure_id, id)
                .await?
                .ok_or(Erreur::VenteInconnue);
        }
        Reservation::Nouvelle(id) => id,
    };
    let jour = aujourdhui(&mut tx).await?;

    // Prix de vente en vigueur de chaque médicament (D7), et lots candidats.
    let mut medicaments: Vec<Uuid> = v.lignes.iter().map(|l| l.medicament_id).collect();
    medicaments.sort();
    medicaments.dedup();
    let mut prix_vente = BTreeMap::new();
    for medicament_id in &medicaments {
        medicament_existe(&mut *tx, *medicament_id).await?;
        let prix: Option<i32> = sqlx::query_scalar(
            "SELECT prix_unitaire_fcfa FROM prix_vente_pharmacie WHERE structure_id = $1 AND medicament_id = $2 \
             ORDER BY date_effet DESC, id DESC LIMIT 1",
        )
        .bind(auteur.structure_id)
        .bind(medicament_id)
        .fetch_optional(&mut *tx)
        .await?;
        prix_vente.insert(
            *medicament_id,
            prix.ok_or(Erreur::PrixNonDefini {
                medicament_id: *medicament_id,
            })?,
        );
    }
    let candidats: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM lot_stock WHERE structure_id = $1 AND medicament_id = ANY($2) ORDER BY id",
    )
    .bind(auteur.structure_id)
    .bind(&medicaments)
    .fetch_all(&mut *tx)
    .await?;
    let mut lots = verrouiller_lots(&mut tx, auteur.structure_id, &candidats).await?;

    sqlx::query(
        "INSERT INTO vente (id, structure_id, sur_ordonnance_papier, auteur_affectation_id, auteur_structure_id, auteur_role) \
         VALUES ($1, $2, $3, $4, $2, $5)",
    )
    .bind(vente_id)
    .bind(auteur.structure_id)
    .bind(v.sur_ordonnance_papier)
    .bind(auteur.affectation_id)
    .bind(auteur.role.as_str())
    .execute(&mut *tx)
    .await?;

    for ligne in &v.lignes {
        let mut reste = ligne.quantite;
        // Lots à sortir, dans l'ordre : le lot choisi, ou FEFO (expiration
        // la plus proche, puis le plus ancien lot) parmi les non périmés.
        let ordre: Vec<Uuid> = match ligne.lot_id {
            Some(lot_id) => {
                let choisi = lots
                    .get(&lot_id)
                    .filter(|l| l.medicament_id == ligne.medicament_id)
                    .ok_or(Erreur::LotInconnu)?;
                if choisi.date_expiration < jour {
                    return Err(Erreur::LotPerime);
                }
                vec![lot_id]
            }
            None => {
                let mut fefo: Vec<&LotVerrouille> = lots
                    .values()
                    .filter(|l| {
                        l.medicament_id == ligne.medicament_id
                            && l.date_expiration >= jour
                            && l.quantite > 0
                    })
                    .collect();
                fefo.sort_by_key(|l| (l.date_expiration, l.id));
                fefo.into_iter().map(|l| l.id).collect()
            }
        };
        let disponible: i64 = ordre
            .iter()
            .filter_map(|id| lots.get(id))
            .map(|l| l.quantite.max(0))
            .sum();
        if disponible < reste {
            return Err(Erreur::StockInsuffisant {
                medicament_id: Some(ligne.medicament_id),
                quantite_disponible: disponible,
            });
        }
        for lot_id in ordre {
            if reste == 0 {
                break;
            }
            let lot = lots.get_mut(&lot_id).ok_or(Erreur::LotInconnu)?;
            let pris = reste.min(lot.quantite);
            if pris <= 0 {
                continue;
            }
            inserer_mouvement(
                &mut tx,
                auteur,
                None,
                NouveauMouvement {
                    nature: "vente",
                    lot_id,
                    quantite: -(pris as i32),
                    prix_achat: lot.prix_achat,
                    prix_vente: Some(prix_vente[&ligne.medicament_id]),
                    vente_id: Some(vente_id),
                    inventaire_id: None,
                    motif_perte: None,
                    commentaire: None,
                    annule_mouvement_id: None,
                },
            )
            .await?;
            lot.quantite -= pris;
            reste -= pris;
        }
    }
    tx.commit().await?;
    vente(pool, auteur.structure_id, vente_id)
        .await?
        .ok_or(Erreur::VenteInconnue)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Annulation {
    pub motif: String,
}

/// Annulation d'une vente entière : une écriture inverse par ligne, aux
/// mêmes prix ; le stock revient dans les lots d'origine.
pub async fn annuler_vente(
    pool: &PgPool,
    auteur: AffectationPharmacie,
    cle: Option<&Cle>,
    vente_id: Uuid,
    a: &Annulation,
) -> Result<Vente, Erreur> {
    let motif = texte(&a.motif, "motif", COMMENTAIRE_MAX)?;
    let mut tx = pool.begin().await?;
    let empreinte = idempotence::empreinte("annulation_vente", &(vente_id, a));
    if let Reservation::DejaFaite(id) = idempotence::reserver(
        &mut tx,
        auteur.structure_id,
        cle,
        "annulation_vente",
        &empreinte,
        Some(vente_id),
    )
    .await??
    {
        drop(tx);
        return vente(pool, auteur.structure_id, id)
            .await?
            .ok_or(Erreur::VenteInconnue);
    }
    let lignes: Vec<(Uuid, Uuid, i32, i32, Option<i32>, bool)> = sqlx::query_as(
        "SELECT mv.id, mv.lot_id, mv.quantite, mv.prix_achat_unitaire_fcfa, mv.prix_vente_unitaire_fcfa, \
                EXISTS (SELECT 1 FROM mouvement_stock a WHERE a.annule_mouvement_id = mv.id) \
         FROM mouvement_stock mv WHERE mv.structure_id = $1 AND mv.vente_id = $2 AND mv.nature = 'vente' ORDER BY mv.id",
    )
    .bind(auteur.structure_id)
    .bind(vente_id)
    .fetch_all(&mut *tx)
    .await?;
    if lignes.is_empty() {
        return Err(Erreur::VenteInconnue);
    }
    let a_annuler: Vec<_> = lignes.into_iter().filter(|l| !l.5).collect();
    if a_annuler.is_empty() {
        return Err(Erreur::DejaAnnule);
    }
    let mut ids: Vec<Uuid> = a_annuler.iter().map(|l| l.1).collect();
    ids.sort();
    ids.dedup();
    verrouiller_lots(&mut tx, auteur.structure_id, &ids).await?;
    for (mouvement_id, lot_id, quantite, prix_achat, prix_vente, _) in a_annuler {
        inserer_mouvement(
            &mut tx,
            auteur,
            None,
            NouveauMouvement {
                nature: "annulation",
                lot_id,
                quantite: -quantite,
                prix_achat,
                prix_vente,
                vente_id: Some(vente_id),
                inventaire_id: None,
                motif_perte: None,
                commentaire: Some(&motif),
                annule_mouvement_id: Some(mouvement_id),
            },
        )
        .await?;
    }
    tx.commit().await?;
    vente(pool, auteur.structure_id, vente_id)
        .await?
        .ok_or(Erreur::VenteInconnue)
}

// ---------------------------------------------------------------------
// Pertes, inventaire, annulation d'un mouvement
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MotifPerte {
    Peremption,
    Casse,
    Autre,
}

impl MotifPerte {
    fn as_str(self) -> &'static str {
        match self {
            MotifPerte::Peremption => "peremption",
            MotifPerte::Casse => "casse",
            MotifPerte::Autre => "autre",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NouvellePerte {
    pub lot_id: Uuid,
    pub quantite: i64,
    pub motif: MotifPerte,
    pub commentaire: Option<String>,
}

/// Perte (péremption, casse, autre) : valorisée au prix d'achat du lot.
pub async fn declarer_perte(
    pool: &PgPool,
    auteur: AffectationPharmacie,
    cle: Option<&Cle>,
    p: &NouvellePerte,
) -> Result<Mouvement, Erreur> {
    let quantite = quantite_positive(p.quantite, "quantite")?;
    let commentaire = texte_facultatif(p.commentaire.as_deref(), "commentaire", COMMENTAIRE_MAX)?;
    if p.motif == MotifPerte::Autre && commentaire.is_none() {
        return Err(invalide(
            "commentaire : obligatoire pour le motif « autre ».",
        ));
    }
    let mut tx = pool.begin().await?;
    let id = match idempotence::reserver(
        &mut tx,
        auteur.structure_id,
        cle,
        "perte",
        &idempotence::empreinte("perte", p),
        None,
    )
    .await??
    {
        Reservation::DejaFaite(id) => {
            drop(tx);
            return Ok(mouvement(pool, auteur.structure_id, id).await?);
        }
        Reservation::Nouvelle(id) => id,
    };
    let lots = verrouiller_lots(&mut tx, auteur.structure_id, &[p.lot_id]).await?;
    let lot = lots.get(&p.lot_id).ok_or(Erreur::LotInconnu)?;
    if lot.quantite < i64::from(quantite) {
        return Err(Erreur::StockInsuffisant {
            medicament_id: Some(lot.medicament_id),
            quantite_disponible: lot.quantite,
        });
    }
    inserer_mouvement(
        &mut tx,
        auteur,
        Some(id),
        NouveauMouvement {
            nature: "perte",
            lot_id: p.lot_id,
            quantite: -quantite,
            prix_achat: lot.prix_achat,
            prix_vente: None,
            vente_id: None,
            inventaire_id: None,
            motif_perte: Some(p.motif.as_str()),
            commentaire: commentaire.as_deref(),
            annule_mouvement_id: None,
        },
    )
    .await?;
    tx.commit().await?;
    Ok(mouvement(pool, auteur.structure_id, id).await?)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LigneInventaire {
    pub lot_id: Uuid,
    pub quantite_comptee: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NouvelInventaire {
    pub lignes: Vec<LigneInventaire>,
}

#[derive(Debug, Serialize)]
pub struct Inventaire {
    pub inventaire_id: Uuid,
    /// Les écarts enregistrés (lots dont le compte différait du stock).
    pub ecarts: Vec<Mouvement>,
}

/// Inventaire express (BF-04) : pour chaque lot compté, l'écart entre le
/// compte et le stock devient un mouvement `inventaire`, valorisé au prix
/// d'achat du lot. Un lot sans écart n'écrit rien.
pub async fn inventorier(
    pool: &PgPool,
    auteur: AffectationPharmacie,
    cle: Option<&Cle>,
    i: &NouvelInventaire,
) -> Result<Inventaire, Erreur> {
    if i.lignes.is_empty() || i.lignes.len() > LIGNES_MAX {
        return Err(invalide(format!("lignes : entre 1 et {LIGNES_MAX}.")));
    }
    let mut ids: Vec<Uuid> = i.lignes.iter().map(|l| l.lot_id).collect();
    ids.sort();
    if ids.windows(2).any(|w| w[0] == w[1]) {
        return Err(invalide("lignes : un même lot compté deux fois."));
    }
    for ligne in &i.lignes {
        if !(0..=QUANTITE_MAX).contains(&ligne.quantite_comptee) {
            return Err(invalide(format!(
                "lignes.quantite_comptee : entre 0 et {QUANTITE_MAX}."
            )));
        }
    }
    let mut tx = pool.begin().await?;
    let inventaire_id = match idempotence::reserver(
        &mut tx,
        auteur.structure_id,
        cle,
        "inventaire",
        &idempotence::empreinte("inventaire", i),
        None,
    )
    .await??
    {
        Reservation::DejaFaite(id) => {
            drop(tx);
            return inventaire(pool, auteur.structure_id, id).await;
        }
        Reservation::Nouvelle(id) => id,
    };
    let lots = verrouiller_lots(&mut tx, auteur.structure_id, &ids).await?;
    if lots.len() != ids.len() {
        return Err(Erreur::LotInconnu);
    }
    for ligne in &i.lignes {
        let lot = &lots[&ligne.lot_id];
        let ecart = ligne.quantite_comptee - lot.quantite;
        if ecart == 0 {
            continue;
        }
        inserer_mouvement(
            &mut tx,
            auteur,
            None,
            NouveauMouvement {
                nature: "inventaire",
                lot_id: lot.id,
                quantite: ecart as i32,
                prix_achat: lot.prix_achat,
                prix_vente: None,
                vente_id: None,
                inventaire_id: Some(inventaire_id),
                motif_perte: None,
                commentaire: None,
                annule_mouvement_id: None,
            },
        )
        .await?;
    }
    tx.commit().await?;
    inventaire(pool, auteur.structure_id, inventaire_id).await
}

async fn inventaire(
    pool: &PgPool,
    structure_id: Uuid,
    inventaire_id: Uuid,
) -> Result<Inventaire, Erreur> {
    let ecarts = sqlx::query_as(concat!(
        select_mouvement!(),
        " WHERE mv.structure_id = $1 AND mv.inventaire_id = $2 ORDER BY mv.id"
    ))
    .bind(structure_id)
    .bind(inventaire_id)
    .fetch_all(pool)
    .await?;
    Ok(Inventaire {
        inventaire_id,
        ecarts,
    })
}

/// Annulation d'une entrée, d'une perte ou d'un écart d'inventaire : une
/// écriture inverse, au même prix d'achat. Refusée si le stock du lot ne le
/// permet plus (une entrée dont une partie a été vendue).
pub async fn annuler_mouvement(
    pool: &PgPool,
    auteur: AffectationPharmacie,
    cle: Option<&Cle>,
    mouvement_id: Uuid,
    a: &Annulation,
) -> Result<Mouvement, Erreur> {
    let motif = texte(&a.motif, "motif", COMMENTAIRE_MAX)?;
    let mut tx = pool.begin().await?;
    let empreinte = idempotence::empreinte("annulation_mouvement", &(mouvement_id, a));
    let id = match idempotence::reserver(
        &mut tx,
        auteur.structure_id,
        cle,
        "annulation_mouvement",
        &empreinte,
        None,
    )
    .await??
    {
        Reservation::DejaFaite(id) => {
            drop(tx);
            return Ok(mouvement(pool, auteur.structure_id, id).await?);
        }
        Reservation::Nouvelle(id) => id,
    };
    let origine: Option<(Uuid, String, i32, i32)> = sqlx::query_as(
        "SELECT lot_id, nature, quantite, prix_achat_unitaire_fcfa FROM mouvement_stock WHERE structure_id = $1 AND id = $2",
    )
    .bind(auteur.structure_id)
    .bind(mouvement_id)
    .fetch_optional(&mut *tx)
    .await?;
    let (lot_id, nature, quantite, prix_achat) = origine.ok_or(Erreur::MouvementInconnu)?;
    if !matches!(nature.as_str(), "entree" | "perte" | "inventaire") {
        return Err(Erreur::AnnulationImpossible);
    }
    let lots = verrouiller_lots(&mut tx, auteur.structure_id, &[lot_id]).await?;
    let lot = &lots[&lot_id];
    if lot.quantite - i64::from(quantite) < 0 {
        return Err(Erreur::StockInsuffisant {
            medicament_id: Some(lot.medicament_id),
            quantite_disponible: lot.quantite,
        });
    }
    inserer_mouvement(
        &mut tx,
        auteur,
        Some(id),
        NouveauMouvement {
            nature: "annulation",
            lot_id,
            quantite: -quantite,
            prix_achat,
            prix_vente: None,
            vente_id: None,
            inventaire_id: None,
            motif_perte: None,
            commentaire: Some(&motif),
            annule_mouvement_id: Some(mouvement_id),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(mouvement(pool, auteur.structure_id, id).await?)
}

// ---------------------------------------------------------------------
// Prix, seuils, réglages
// ---------------------------------------------------------------------

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct PrixVente {
    pub medicament_id: Uuid,
    pub prix_unitaire_fcfa: i32,
    pub date_effet: DateTime<Utc>,
}

/// Nouveau prix de vente (le responsable seulement, D7) ; les ventes déjà
/// faites gardent leur prix.
pub async fn fixer_prix(
    pool: &PgPool,
    responsable: AffectationPharmacie,
    medicament_id: Uuid,
    prix_fcfa: i64,
) -> Result<PrixVente, Erreur> {
    let prix_fcfa = prix(prix_fcfa, "prix_unitaire_fcfa")?;
    medicament_existe(pool, medicament_id).await?;
    Ok(sqlx::query_as(
        "INSERT INTO prix_vente_pharmacie (structure_id, medicament_id, prix_unitaire_fcfa, auteur_affectation_id, \
                                           auteur_structure_id, auteur_role) \
         VALUES ($1, $2, $3, $4, $1, $5) RETURNING medicament_id, prix_unitaire_fcfa, date_effet",
    )
    .bind(responsable.structure_id)
    .bind(medicament_id)
    .bind(prix_fcfa)
    .bind(responsable.affectation_id)
    .bind(responsable.role.as_str())
    .fetch_one(pool)
    .await?)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Seuils {
    pub seuil_min: i64,
    pub seuil_max: Option<i64>,
}

/// Seuils minimal et maximal d'un médicament (BF-04), pour la pharmacie.
pub async fn fixer_seuils(
    pool: &PgPool,
    auteur: AffectationPharmacie,
    medicament_id: Uuid,
    s: &Seuils,
) -> Result<(), Erreur> {
    if !(0..=QUANTITE_MAX).contains(&s.seuil_min)
        || s.seuil_max
            .is_some_and(|m| m < s.seuil_min || m > QUANTITE_MAX)
    {
        return Err(invalide(format!(
            "seuils : seuil_min entre 0 et {QUANTITE_MAX}, seuil_max au moins égal à seuil_min."
        )));
    }
    medicament_existe(pool, medicament_id).await?;
    sqlx::query(
        "INSERT INTO seuil_stock (structure_id, medicament_id, seuil_min, seuil_max, auteur_affectation_id, auteur_structure_id, auteur_role) \
         VALUES ($1, $2, $3, $4, $5, $1, $6) \
         ON CONFLICT (structure_id, medicament_id) DO UPDATE SET seuil_min = EXCLUDED.seuil_min, seuil_max = EXCLUDED.seuil_max, \
             auteur_affectation_id = EXCLUDED.auteur_affectation_id, auteur_structure_id = EXCLUDED.auteur_structure_id, \
             auteur_role = EXCLUDED.auteur_role, modifie_le = now()",
    )
    .bind(auteur.structure_id)
    .bind(medicament_id)
    .bind(s.seuil_min as i32)
    .bind(s.seuil_max.map(|m| m as i32))
    .bind(auteur.affectation_id)
    .bind(auteur.role.as_str())
    .execute(pool)
    .await?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reglages {
    /// `null` : alerte de péremption désactivée (D9).
    pub delai_alerte_peremption_jours: Option<i64>,
}

/// Réglages de la pharmacie (le responsable seulement).
pub async fn regler(
    pool: &PgPool,
    responsable: AffectationPharmacie,
    r: &Reglages,
) -> Result<(), Erreur> {
    if r.delai_alerte_peremption_jours
        .is_some_and(|d| !(1..=365).contains(&d))
    {
        return Err(invalide(
            "delai_alerte_peremption_jours : entre 1 et 365, ou null pour désactiver.",
        ));
    }
    sqlx::query(
        "INSERT INTO reglage_pharmacie (structure_id, delai_alerte_peremption_jours, auteur_affectation_id, auteur_structure_id, auteur_role) \
         VALUES ($1, $2, $3, $1, $4) \
         ON CONFLICT (structure_id) DO UPDATE SET delai_alerte_peremption_jours = EXCLUDED.delai_alerte_peremption_jours, \
             auteur_affectation_id = EXCLUDED.auteur_affectation_id, auteur_structure_id = EXCLUDED.auteur_structure_id, \
             auteur_role = EXCLUDED.auteur_role, modifie_le = now()",
    )
    .bind(responsable.structure_id)
    .bind(r.delai_alerte_peremption_jours.map(|d| d as i16))
    .bind(responsable.affectation_id)
    .bind(responsable.role.as_str())
    .execute(pool)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------
// Alertes (D11 : dans l'application, calculées à chaque lecture)
// ---------------------------------------------------------------------

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct AlerteSeuil {
    pub medicament_id: Uuid,
    pub denomination: String,
    pub quantite_disponible: i64,
    pub seuil_min: i32,
    pub seuil_max: Option<i32>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct AlerteLot {
    pub lot_id: Uuid,
    pub medicament_id: Uuid,
    pub denomination: String,
    pub numero_lot: String,
    pub date_expiration: NaiveDate,
    pub quantite: i64,
    /// Négatif pour un lot déjà périmé.
    pub jours_restants: i32,
}

#[derive(Debug, Serialize)]
pub struct Alertes {
    pub sous_seuil_min: Vec<AlerteSeuil>,
    pub au_dessus_seuil_max: Vec<AlerteSeuil>,
    /// Faux tant que le responsable n'a pas réglé le délai (D9).
    pub alerte_peremption_active: bool,
    pub delai_alerte_peremption_jours: Option<i16>,
    pub bientot_perimes: Vec<AlerteLot>,
    /// Lots périmés encore en stock : à sortir en perte.
    pub perimes_en_stock: Vec<AlerteLot>,
}

/// Alertes du moment. Calculées à chaque lecture, sur le stock réel : elles
/// apparaissent dès le mouvement qui franchit un seuil (CDC BF-04 : « en
/// moins d'une minute »).
pub async fn alertes(pool: &PgPool, structure_id: Uuid) -> Result<Alertes, sqlx::Error> {
    let seuils: Vec<AlerteSeuil> = sqlx::query_as(
        "SELECT s.medicament_id, m.denomination, \
                COALESCE((SELECT sum(mv.quantite) FROM mouvement_stock mv JOIN lot_stock l ON l.id = mv.lot_id \
                          WHERE l.structure_id = s.structure_id AND l.medicament_id = s.medicament_id \
                            AND l.date_expiration >= (now() AT TIME ZONE 'Africa/Ouagadougou')::date), 0)::BIGINT AS quantite_disponible, \
                s.seuil_min, s.seuil_max \
         FROM seuil_stock s JOIN medicament m ON m.id = s.medicament_id \
         WHERE s.structure_id = $1 ORDER BY m.cle, m.id",
    )
    .bind(structure_id)
    .fetch_all(pool)
    .await?;
    let (sous, autres): (Vec<_>, Vec<_>) = seuils
        .into_iter()
        .partition(|s| s.quantite_disponible < i64::from(s.seuil_min));
    let au_dessus = autres
        .into_iter()
        .filter(|s| {
            s.seuil_max
                .is_some_and(|m| s.quantite_disponible > i64::from(m))
        })
        .collect();

    let delai: Option<i16> = sqlx::query_scalar(
        "SELECT delai_alerte_peremption_jours FROM reglage_pharmacie WHERE structure_id = $1",
    )
    .bind(structure_id)
    .fetch_optional(pool)
    .await?
    .flatten();
    let lots: Vec<AlerteLot> = sqlx::query_as(
        "SELECT * FROM (SELECT l.id AS lot_id, l.medicament_id, m.denomination, l.numero_lot, l.date_expiration, \
                COALESCE((SELECT sum(mv.quantite) FROM mouvement_stock mv WHERE mv.lot_id = l.id), 0)::BIGINT AS quantite, \
                (l.date_expiration - (now() AT TIME ZONE 'Africa/Ouagadougou')::date) AS jours_restants \
         FROM lot_stock l JOIN medicament m ON m.id = l.medicament_id WHERE l.structure_id = $1) x \
         WHERE quantite > 0 AND (jours_restants < 0 OR jours_restants <= COALESCE($2, -1)) \
         ORDER BY date_expiration, lot_id",
    )
    .bind(structure_id)
    .bind(delai.map(i32::from))
    .fetch_all(pool)
    .await?;
    let (perimes, bientot): (Vec<_>, Vec<_>) = lots.into_iter().partition(|l| l.jours_restants < 0);
    Ok(Alertes {
        sous_seuil_min: sous,
        au_dessus_seuil_max: au_dessus,
        alerte_peremption_active: delai.is_some(),
        delai_alerte_peremption_jours: delai,
        bientot_perimes: bientot,
        perimes_en_stock: perimes,
    })
}

// ---------------------------------------------------------------------
// Équivalents par DCI (BF-05, partie sans ordonnance)
// ---------------------------------------------------------------------

/// Ressemblance d'un équivalent avec le médicament demandé (demande du
/// porteur du 2026-11-16 : la même DCI ne suffit pas pour la sécurité du
/// patient). Toujours la même DCI ; le dosage et la forme sont comparés.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Correspondance {
    /// Même DCI, même dosage et même forme.
    MemeDciMemeDosageMemeForme,
    /// Même DCI, mais le dosage ou la forme diffère : **substitution à
    /// juger par le pharmacien** (posologie, voie d'administration).
    MemeDciDosageOuFormeDifferent,
}

#[derive(Debug, sqlx::FromRow)]
struct LigneEquivalent {
    #[sqlx(flatten)]
    medicament: Medicament,
    quantite_disponible: i64,
    prix_vente_unitaire_fcfa: Option<i32>,
}

#[derive(Debug, Serialize)]
pub struct Equivalent {
    #[serde(flatten)]
    pub medicament: Medicament,
    pub correspondance: Correspondance,
    /// Faux : le dosage diffère de celui du médicament demandé.
    pub meme_dosage: bool,
    /// Faux : la forme diffère de celle du médicament demandé.
    pub meme_forme: bool,
    /// Stock non périmé dans **cette** pharmacie.
    pub quantite_disponible: i64,
    pub prix_vente_unitaire_fcfa: Option<i32>,
}

#[derive(Debug, Serialize)]
pub struct Equivalents {
    pub medicament: Medicament,
    pub equivalents: Vec<Equivalent>,
}

/// Nombre maximal d'équivalents renvoyés (garde-fou technique).
const EQUIVALENTS_MAX: usize = 100;

/// Médicaments qui ont **exactement les mêmes DCI** (CDC, BF-05 et cadre
/// réglementaire : l'équivalence « repose exclusivement sur l'équivalence de
/// molécule active »), avec leur stock dans la pharmacie. **Classés** :
/// d'abord même dosage et même forme, ensuite dosage ou forme différents,
/// marqués comme tels (`correspondance`, `meme_dosage`, `meme_forme`) ; dans
/// chaque groupe, ceux en stock d'abord. Rien n'est caché : le pharmacien
/// décide. Le dosage et la forme sont comparés sur leur clé du catalogue
/// (espaces réduites, sans majuscules, `catalogue::cle`) : « 500 mg » et
/// « 500mg » restent différents, ce qui est le sens prudent (signalé, jamais
/// caché). `None` si le médicament n'existe pas.
pub async fn equivalents(
    pool: &PgPool,
    structure_id: Uuid,
    medicament_id: Uuid,
) -> Result<Option<Equivalents>, sqlx::Error> {
    let Some(medicament) =
        sqlx::query_as::<_, Medicament>(concat!(select_medicament!(), " WHERE m.id = $1"))
            .bind(medicament_id)
            .fetch_optional(pool)
            .await?
    else {
        return Ok(None);
    };
    let lignes: Vec<LigneEquivalent> = sqlx::query_as(concat!(
        "SELECT med.*, \
                COALESCE((SELECT sum(mv.quantite) FROM mouvement_stock mv JOIN lot_stock l ON l.id = mv.lot_id \
                          WHERE l.structure_id = $2 AND l.medicament_id = med.id \
                            AND l.date_expiration >= (now() AT TIME ZONE 'Africa/Ouagadougou')::date), 0)::BIGINT AS quantite_disponible, \
                (SELECT p.prix_unitaire_fcfa FROM prix_vente_pharmacie p WHERE p.structure_id = $2 AND p.medicament_id = med.id \
                 ORDER BY p.date_effet DESC, p.id DESC LIMIT 1) AS prix_vente_unitaire_fcfa \
         FROM (",
        select_medicament!(),
        " WHERE m.id <> $1 \
            AND (SELECT array_agg(dci_id ORDER BY dci_id) FROM medicament_dci WHERE medicament_id = m.id) \
              = (SELECT array_agg(dci_id ORDER BY dci_id) FROM medicament_dci WHERE medicament_id = $1)) med \
         LIMIT 1000"
    ))
    .bind(medicament_id)
    .bind(structure_id)
    .fetch_all(pool)
    .await?;
    let (dosage, forme) = (
        crate::catalogue::cle(&medicament.dosage),
        crate::catalogue::cle(&medicament.forme),
    );
    let mut equivalents: Vec<Equivalent> = lignes
        .into_iter()
        .map(|l| {
            let meme_dosage = crate::catalogue::cle(&l.medicament.dosage) == dosage;
            let meme_forme = crate::catalogue::cle(&l.medicament.forme) == forme;
            Equivalent {
                correspondance: if meme_dosage && meme_forme {
                    Correspondance::MemeDciMemeDosageMemeForme
                } else {
                    Correspondance::MemeDciDosageOuFormeDifferent
                },
                meme_dosage,
                meme_forme,
                quantite_disponible: l.quantite_disponible,
                prix_vente_unitaire_fcfa: l.prix_vente_unitaire_fcfa,
                medicament: l.medicament,
            }
        })
        .collect();
    // Le classement se fait ici, avant la limite, pour qu'un équivalent
    // identique ne soit jamais coupé au profit d'un autre.
    equivalents.sort_by(|a, b| {
        (
            a.correspondance,
            std::cmp::Reverse(a.quantite_disponible > 0),
            &a.medicament.denomination,
            a.medicament.id,
        )
            .cmp(&(
                b.correspondance,
                std::cmp::Reverse(b.quantite_disponible > 0),
                &b.medicament.denomination,
                b.medicament.id,
            ))
    });
    equivalents.truncate(EQUIVALENTS_MAX);
    Ok(Some(Equivalents {
        medicament,
        equivalents,
    }))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    // Base temporaire par test (`#[sqlx::test]`), données fictives.

    /// Pharmacie validée avec un responsable et un pharmacien. Renvoie
    /// (responsable, pharmacien).
    pub(crate) async fn pharmacie(
        pool: &PgPool,
        prefixe: &str,
    ) -> (AffectationPharmacie, AffectationPharmacie) {
        let compte = |suffixe: &'static str| {
            let pool = pool.clone();
            let telephone = format!("+226{prefixe}{suffixe}");
            async move {
                sqlx::query_scalar::<_, Uuid>(
                    "WITH u AS (INSERT INTO utilisateur (nom, prenom, telephone) VALUES ('SANOU', 'Ali', $1) RETURNING id) \
                     INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) SELECT id, 'professionnel', 'x' FROM u RETURNING id",
                )
                .bind(telephone)
                .fetch_one(&pool)
                .await
                .unwrap()
            }
        };
        let (titulaire, employe) = (compte("01").await, compte("02").await);
        let structure_id: Uuid = sqlx::query_scalar(
            "INSERT INTO structure (type_structure, nom, telephone, telephone_urgence, region, email, commune, province, \
                                    latitude, longitude, createur_compte_id, statut) \
             VALUES ('pharmacie', 'Pharmacie fictive', '+22670000000', '+22670000000', 'Région fictive', 'p@example.org', \
                     'Commune fictive', 'Province fictive', 12.3714, -1.5197, $1, 'validee') RETURNING id",
        )
        .bind(titulaire)
        .fetch_one(pool)
        .await
        .unwrap();
        let affecter = |compte: Uuid, role: &'static str| {
            let pool = pool.clone();
            async move {
                sqlx::query_scalar::<_, Uuid>(
                    "INSERT INTO affectation (compte_id, structure_id, role) VALUES ($1, $2, $3) RETURNING id",
                )
                .bind(compte)
                .bind(structure_id)
                .bind(role)
                .fetch_one(&pool)
                .await
                .unwrap()
            }
        };
        (
            AffectationPharmacie {
                affectation_id: affecter(titulaire, "responsable").await,
                structure_id,
                role: RolePharmacie::Responsable,
            },
            AffectationPharmacie {
                affectation_id: affecter(employe, "pharmacien").await,
                structure_id,
                role: RolePharmacie::Pharmacien,
            },
        )
    }

    /// Trois médicaments fictifs : X et Y partagent la même molécule, Z est
    /// une association. Renvoie (X, Y, Z).
    pub(crate) async fn catalogue_fictif(pool: &PgPool) -> (Uuid, Uuid, Uuid) {
        let admin = crate::catalogue::tests::administrateur(pool).await;
        let fichier = "denomination;dci;forme;dosage\n\
                       Produit X fictif;Molécule-A fictive;comprimé;500 mg\n\
                       Produit Y fictif;Molécule-A fictive;comprimé;500 mg\n\
                       Produit Z fictif;Molécule-A fictive + Molécule-B fictive;comprimé;500 mg\n";
        crate::catalogue::importer(pool, admin, "Liste fictive", fichier.as_bytes())
            .await
            .unwrap();
        let id = |nom: &'static str| {
            let pool = pool.clone();
            async move {
                sqlx::query_scalar::<_, Uuid>("SELECT id FROM medicament WHERE denomination = $1")
                    .bind(nom)
                    .fetch_one(&pool)
                    .await
                    .unwrap()
            }
        };
        (
            id("Produit X fictif").await,
            id("Produit Y fictif").await,
            id("Produit Z fictif").await,
        )
    }

    pub(crate) fn dans(jours: i64) -> NaiveDate {
        // Date du Burkina = date UTC (UTC+0 toute l'année).
        Utc::now().date_naive() + chrono::Duration::days(jours)
    }

    pub(crate) fn entree(
        medicament_id: Uuid,
        lot: &str,
        jours: i64,
        quantite: i64,
        prix_achat: i64,
    ) -> NouvelleEntree {
        NouvelleEntree {
            medicament_id,
            numero_lot: lot.into(),
            date_expiration: Some(dans(jours)),
            mois_expiration: None,
            quantite,
            prix_achat_unitaire_fcfa: prix_achat,
            fournisseur: "Grossiste fictif".into(),
        }
    }

    fn ligne(medicament_id: Uuid, quantite: i64, lot_id: Option<Uuid>) -> LigneDemandee {
        LigneDemandee {
            medicament_id,
            quantite,
            lot_id,
        }
    }

    pub(crate) fn vente_de(lignes: Vec<LigneDemandee>) -> NouvelleVente {
        NouvelleVente {
            sur_ordonnance_papier: false,
            lignes,
        }
    }

    async fn quantite(pool: &PgPool, lot_id: Uuid) -> i64 {
        sqlx::query_scalar(
            "SELECT COALESCE(sum(quantite), 0)::BIGINT FROM mouvement_stock WHERE lot_id = $1",
        )
        .bind(lot_id)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// Lot périmé posé en SQL (l'entrée refuse un lot déjà périmé).
    async fn lot_perime(
        pool: &PgPool,
        auteur: AffectationPharmacie,
        medicament_id: Uuid,
        quantite: i32,
    ) -> Uuid {
        let lot: Uuid = sqlx::query_scalar(
            "INSERT INTO lot_stock (structure_id, medicament_id, numero_lot, date_expiration, prix_achat_unitaire_fcfa, fournisseur) \
             VALUES ($1, $2, 'PERIME-1', CURRENT_DATE - 2, 50, 'Grossiste fictif') RETURNING id",
        )
        .bind(auteur.structure_id)
        .bind(medicament_id)
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO mouvement_stock (structure_id, lot_id, nature, quantite, prix_achat_unitaire_fcfa, \
                                          auteur_affectation_id, auteur_structure_id, auteur_role) \
             VALUES ($1, $2, 'entree', $3, 50, $4, $1, $5)",
        )
        .bind(auteur.structure_id)
        .bind(lot)
        .bind(quantite)
        .bind(auteur.affectation_id)
        .bind(auteur.role.as_str())
        .execute(pool)
        .await
        .unwrap();
        lot
    }

    /// Entrées ; vente refusée sans prix ; FEFO sur deux lots, prix figés ;
    /// lot choisi ; lot périmé jamais vendu ; stock insuffisant ; annulation
    /// d'une vente ; état du stock ; prix réservé au responsable par la base.
    #[sqlx::test]
    #[ignore]
    async fn entrees_ventes_fefo_et_prix_figes(pool: PgPool) {
        let (responsable, pharmacien) = pharmacie(&pool, "706001").await;
        let (x, _, _) = catalogue_fictif(&pool).await;

        assert!(matches!(
            entrer(&pool, pharmacien, None, &entree(x, "L-PASSE", -1, 5, 100)).await,
            Err(Erreur::DonneesInvalides(_))
        ));
        assert!(matches!(
            entrer(&pool, pharmacien, None, &entree(x, "L", 30, 0, 100)).await,
            Err(Erreur::DonneesInvalides(_))
        ));
        assert!(matches!(
            entrer(
                &pool,
                pharmacien,
                None,
                &entree(Uuid::max(), "L", 30, 5, 100)
            )
            .await,
            Err(Erreur::MedicamentInconnu)
        ));
        let lot_a = entrer(&pool, pharmacien, None, &entree(x, "LOT-A", 30, 10, 100))
            .await
            .unwrap();
        let lot_b = entrer(&pool, pharmacien, None, &entree(x, "LOT-B", 10, 5, 120))
            .await
            .unwrap();
        assert_eq!((lot_a.quantite, lot_a.perime), (10, false));
        let perime = lot_perime(&pool, pharmacien, x, 4).await;

        // Sans prix : bloqué. Le prix ne se fixe que par le responsable.
        assert!(matches!(
            vendre(&pool, pharmacien, None, &vente_de(vec![ligne(x, 1, None)])).await,
            Err(Erreur::PrixNonDefini { .. })
        ));
        assert!(
            fixer_prix(&pool, pharmacien, x, 200).await.is_err(),
            "la base refuse un pharmacien comme auteur d'un prix"
        );
        fixer_prix(&pool, responsable, x, 200).await.unwrap();

        // FEFO : le lot B (10 jours) part avant le lot A (30 jours) ; le lot
        // périmé jamais.
        let v = vendre(&pool, pharmacien, None, &vente_de(vec![ligne(x, 7, None)]))
            .await
            .unwrap();
        let sorties: Vec<(Uuid, i32, Option<i32>, i32)> = v
            .lignes
            .iter()
            .map(|l| {
                (
                    l.lot_id,
                    l.quantite,
                    l.prix_vente_unitaire_fcfa,
                    l.prix_achat_unitaire_fcfa,
                )
            })
            .collect();
        assert_eq!(
            sorties,
            [
                (lot_b.id, -5, Some(200), 120),
                (lot_a.id, -2, Some(200), 100)
            ]
        );
        assert_eq!(v.total_fcfa, 1400);
        assert_eq!(quantite(&pool, perime).await, 4);

        // Un nouveau prix ne change pas la vente passée.
        fixer_prix(&pool, responsable, x, 250).await.unwrap();
        assert_eq!(
            vente(&pool, responsable.structure_id, v.id)
                .await
                .unwrap()
                .unwrap()
                .total_fcfa,
            1400
        );

        // Disponible : 8 (le lot périmé ne compte pas).
        match vendre(&pool, pharmacien, None, &vente_de(vec![ligne(x, 9, None)])).await {
            Err(Erreur::StockInsuffisant {
                quantite_disponible,
                ..
            }) => assert_eq!(quantite_disponible, 8),
            autre => panic!("{autre:?}"),
        }
        assert!(matches!(
            vendre(
                &pool,
                pharmacien,
                None,
                &vente_de(vec![ligne(x, 1, Some(perime))])
            )
            .await,
            Err(Erreur::LotPerime)
        ));
        let choisi = vendre(
            &pool,
            pharmacien,
            None,
            &vente_de(vec![ligne(x, 1, Some(lot_a.id))]),
        )
        .await
        .unwrap();
        assert_eq!(
            (choisi.lignes[0].lot_id, choisi.total_fcfa),
            (lot_a.id, 250)
        );

        // Annulation d'une vente entière : le stock revient dans ses lots.
        let annulee = annuler_vente(
            &pool,
            pharmacien,
            None,
            v.id,
            &Annulation {
                motif: "Erreur de saisie".into(),
            },
        )
        .await
        .unwrap();
        assert!(annulee.annulee && annulee.total_fcfa == 0);
        assert_eq!(
            (
                quantite(&pool, lot_a.id).await,
                quantite(&pool, lot_b.id).await
            ),
            (9, 5)
        );
        assert!(matches!(
            annuler_vente(
                &pool,
                pharmacien,
                None,
                v.id,
                &Annulation {
                    motif: "Encore".into()
                }
            )
            .await,
            Err(Erreur::DejaAnnule)
        ));

        // État du stock.
        let etat = etat(&pool, responsable.structure_id, None, 50)
            .await
            .unwrap();
        assert_eq!(etat.len(), 1);
        assert_eq!(
            (
                etat[0].quantite_disponible,
                etat[0].quantite_perimee,
                etat[0].prix_vente_unitaire_fcfa
            ),
            (14, 4, Some(250))
        );
        assert_eq!(etat[0].valeur_achat_fcfa, 9 * 100 + 5 * 120 + 4 * 50);
        let lots = lots_du_medicament(&pool, responsable.structure_id, x)
            .await
            .unwrap();
        assert_eq!(
            lots.iter().map(|l| l.id).collect::<Vec<_>>(),
            [perime, lot_b.id, lot_a.id],
            "ordre FEFO"
        );

        // Le journal ne se modifie ni ne s'efface.
        for requete in [
            "UPDATE mouvement_stock SET quantite = 1",
            "DELETE FROM lot_stock",
            "TRUNCATE vente CASCADE",
        ] {
            let erreur = sqlx::query(requete).execute(&pool).await.unwrap_err();
            assert!(
                erreur.to_string().contains("jamais modifiees"),
                "{requete} : {erreur}"
            );
        }
    }

    /// Une pharmacie ne voit et ne touche jamais le stock d'une autre :
    /// lots, ventes, mouvements, état, journal, alertes.
    #[sqlx::test]
    #[ignore]
    async fn une_pharmacie_ne_touche_pas_le_stock_d_une_autre(pool: PgPool) {
        let (resp_1, pharm_1) = pharmacie(&pool, "706101").await;
        let (resp_2, pharm_2) = pharmacie(&pool, "706102").await;
        let (x, _, _) = catalogue_fictif(&pool).await;
        let lot = entrer(&pool, pharm_1, None, &entree(x, "LOT-1", 30, 10, 100))
            .await
            .unwrap();
        fixer_prix(&pool, resp_1, x, 200).await.unwrap();
        fixer_prix(&pool, resp_2, x, 200).await.unwrap();
        let v = vendre(&pool, pharm_1, None, &vente_de(vec![ligne(x, 1, None)]))
            .await
            .unwrap();

        assert!(matches!(
            vendre(
                &pool,
                pharm_2,
                None,
                &vente_de(vec![ligne(x, 1, Some(lot.id))])
            )
            .await,
            Err(Erreur::LotInconnu)
        ));
        assert!(matches!(
            vendre(&pool, pharm_2, None, &vente_de(vec![ligne(x, 1, None)])).await,
            Err(Erreur::StockInsuffisant {
                quantite_disponible: 0,
                ..
            })
        ));
        let perte = NouvellePerte {
            lot_id: lot.id,
            quantite: 1,
            motif: MotifPerte::Casse,
            commentaire: None,
        };
        assert!(matches!(
            declarer_perte(&pool, pharm_2, None, &perte).await,
            Err(Erreur::LotInconnu)
        ));
        let compte = NouvelInventaire {
            lignes: vec![LigneInventaire {
                lot_id: lot.id,
                quantite_comptee: 0,
            }],
        };
        assert!(matches!(
            inventorier(&pool, pharm_2, None, &compte).await,
            Err(Erreur::LotInconnu)
        ));
        let motif = Annulation {
            motif: "Intrusion".into(),
        };
        assert!(matches!(
            annuler_vente(&pool, pharm_2, None, v.id, &motif).await,
            Err(Erreur::VenteInconnue)
        ));
        assert!(matches!(
            annuler_mouvement(&pool, pharm_2, None, v.lignes[0].id, &motif).await,
            Err(Erreur::MouvementInconnu)
        ));
        assert!(
            vente(&pool, resp_2.structure_id, v.id)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            etat(&pool, resp_2.structure_id, None, 50)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            journal(&pool, resp_2.structure_id, None, 50)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            lots_du_medicament(&pool, resp_2.structure_id, x)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            equivalents(&pool, resp_2.structure_id, x)
                .await
                .unwrap()
                .unwrap()
                .equivalents
                .iter()
                .map(|e| e.quantite_disponible)
                .sum::<i64>(),
            0
        );
        assert_eq!(
            quantite(&pool, lot.id).await,
            9,
            "rien n'a bougé chez la pharmacie 1"
        );

        // La base refuse un mouvement qui viserait le lot d'une autre
        // pharmacie (clé étrangère composite).
        let erreur = sqlx::query(
            "INSERT INTO mouvement_stock (structure_id, lot_id, nature, quantite, prix_achat_unitaire_fcfa, \
                                          auteur_affectation_id, auteur_structure_id, auteur_role, motif_perte) \
             VALUES ($1, $2, 'perte', -1, 100, $3, $1, 'pharmacien', 'casse')",
        )
        .bind(pharm_2.structure_id)
        .bind(lot.id)
        .bind(pharm_2.affectation_id)
        .execute(&pool)
        .await
        .unwrap_err();
        assert_eq!(
            erreur.as_database_error().and_then(|e| e.constraint()),
            Some("mouvement_lot_de_sa_pharmacie")
        );
    }

    /// Clé d'idempotence : la même requête rejouée renvoie le même objet,
    /// sans rien créer ; la même clé avec une autre requête est refusée ;
    /// deux ventes simultanées du dernier stock : une seule passe.
    #[sqlx::test]
    #[ignore]
    async fn idempotence_et_ventes_simultanees(pool: PgPool) {
        let (responsable, pharmacien) = pharmacie(&pool, "706201").await;
        let (x, _, _) = catalogue_fictif(&pool).await;
        fixer_prix(&pool, responsable, x, 200).await.unwrap();
        let cle = |texte: &str| Cle::lire(texte).unwrap();

        let e = entree(x, "LOT-1", 30, 3, 100);
        let premier = entrer(&pool, pharmacien, Some(&cle("cle-entree-1")), &e)
            .await
            .unwrap();
        let rejoue = entrer(&pool, pharmacien, Some(&cle("cle-entree-1")), &e)
            .await
            .unwrap();
        assert_eq!(premier.id, rejoue.id);
        let lots: i64 = sqlx::query_scalar("SELECT count(*) FROM lot_stock")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(lots, 1);
        assert!(matches!(
            entrer(
                &pool,
                pharmacien,
                Some(&cle("cle-entree-1")),
                &entree(x, "LOT-2", 30, 3, 100)
            )
            .await,
            Err(Erreur::Idempotence(ErreurIdempotence::CleReutilisee))
        ));

        let v = vente_de(vec![ligne(x, 1, None)]);
        let vente_1 = vendre(&pool, pharmacien, Some(&cle("cle-vente-1")), &v)
            .await
            .unwrap();
        let vente_1_bis = vendre(&pool, pharmacien, Some(&cle("cle-vente-1")), &v)
            .await
            .unwrap();
        assert_eq!(vente_1.id, vente_1_bis.id);
        assert_eq!(quantite(&pool, premier.id).await, 2, "une seule sortie");
        let a = Annulation {
            motif: "Erreur".into(),
        };
        let annulee = annuler_vente(&pool, pharmacien, Some(&cle("cle-annul-1")), vente_1.id, &a)
            .await
            .unwrap();
        let annulee_bis =
            annuler_vente(&pool, pharmacien, Some(&cle("cle-annul-1")), vente_1.id, &a)
                .await
                .unwrap();
        assert!(annulee.annulee && annulee_bis.annulee && annulee_bis.id == vente_1.id);
        assert_eq!(quantite(&pool, premier.id).await, 3);

        // Dernières unités : deux ventes simultanées de 2 sur 3.
        let (p1, p2) = (pool.clone(), pool.clone());
        let (v1, v2) = (
            vente_de(vec![ligne(x, 2, None)]),
            vente_de(vec![ligne(x, 2, None)]),
        );
        let t1 = tokio::spawn(async move { vendre(&p1, pharmacien, None, &v1).await });
        let t2 = tokio::spawn(async move { vendre(&p2, pharmacien, None, &v2).await });
        let resultats = [t1.await.unwrap(), t2.await.unwrap()];
        assert_eq!(resultats.iter().filter(|r| r.is_ok()).count(), 1);
        assert!(
            resultats
                .iter()
                .any(|r| matches!(r, Err(Erreur::StockInsuffisant { .. })))
        );
        assert_eq!(quantite(&pool, premier.id).await, 1);
    }

    #[test]
    fn expiration_au_mois_regle_prudente() {
        let avec = |date: Option<NaiveDate>, mois: Option<&str>| NouvelleEntree {
            mois_expiration: mois.map(String::from),
            date_expiration: date,
            ..entree(Uuid::nil(), "L", 0, 1, 1)
        };
        let jour = |a, m, j| NaiveDate::from_ymd_opt(a, m, j).unwrap();
        // « EXP 06/2027 » : périmé dès le 1er juin, utilisable jusqu'au 31 mai.
        assert_eq!(
            expiration(&avec(None, Some("2027-06"))).unwrap(),
            (jour(2027, 5, 31), true)
        );
        assert_eq!(
            expiration(&avec(None, Some(" 2028-03 "))).unwrap(),
            (jour(2028, 2, 29), true),
            "année bissextile"
        );
        assert_eq!(
            expiration(&avec(None, Some("2027-01"))).unwrap(),
            (jour(2026, 12, 31), true)
        );
        assert_eq!(
            expiration(&avec(Some(jour(2027, 6, 15)), None)).unwrap(),
            (jour(2027, 6, 15), false)
        );
        for mauvais in ["2027-6", "06/2027", "2027-13", "2027-00", "2027-06-01", ""] {
            assert!(
                matches!(
                    expiration(&avec(None, Some(mauvais))),
                    Err(Erreur::DonneesInvalides(_))
                ),
                "{mauvais:?}"
            );
        }
        assert!(matches!(
            expiration(&avec(Some(jour(2027, 6, 1)), Some("2027-06"))),
            Err(Erreur::DonneesInvalides(_))
        ));
        assert!(matches!(
            expiration(&avec(None, None)),
            Err(Erreur::DonneesInvalides(_))
        ));
    }

    /// Entrée d'une boîte au mois seul : date enregistrée, mois imprimé
    /// rendu, boîte du mois en cours refusée (déjà périmée selon la règle
    /// prudente), garantie de la base sur la fin de mois.
    #[sqlx::test]
    #[ignore]
    async fn entree_au_mois_seul(pool: PgPool) {
        let (_, pharmacien) = pharmacie(&pool, "706801").await;
        let (x, _, _) = catalogue_fictif(&pool).await;
        let aujourdhui = dans(0);
        let premier_du_mois = |date: NaiveDate| chrono::Datelike::with_day(&date, 1).unwrap();
        // Le mois qui commence dans trois mois environ.
        let futur = premier_du_mois(premier_du_mois(aujourdhui) + chrono::Duration::days(95));
        let e = NouvelleEntree {
            date_expiration: None,
            mois_expiration: Some(futur.format("%Y-%m").to_string()),
            ..entree(x, "LOT-M", 0, 5, 100)
        };
        let lot = entrer(&pool, pharmacien, None, &e).await.unwrap();
        assert_eq!(lot.date_expiration, futur.pred_opt().unwrap());
        assert_eq!(
            lot.mois_expiration_imprime.as_deref(),
            Some(futur.format("%Y-%m").to_string().as_str())
        );
        let date_complete = entrer(&pool, pharmacien, None, &entree(x, "LOT-J", 30, 5, 100))
            .await
            .unwrap();
        assert_eq!(date_complete.mois_expiration_imprime, None);

        // Une boîte « mois en cours » est déjà périmée : refusée à l'entrée.
        let en_cours = NouvelleEntree {
            date_expiration: None,
            mois_expiration: Some(aujourdhui.format("%Y-%m").to_string()),
            ..entree(x, "LOT-C", 0, 5, 100)
        };
        assert!(matches!(
            entrer(&pool, pharmacien, None, &en_cours).await,
            Err(Erreur::DonneesInvalides(_))
        ));

        let erreur = sqlx::query(
            "INSERT INTO lot_stock (structure_id, medicament_id, numero_lot, date_expiration, prix_achat_unitaire_fcfa, \
                                    fournisseur, expiration_mois_seul) \
             VALUES ($1, $2, 'LOT-X', '2027-06-15', 100, 'Grossiste fictif', true)",
        )
        .bind(pharmacien.structure_id)
        .bind(x)
        .execute(&pool)
        .await
        .unwrap_err();
        assert_eq!(
            erreur.as_database_error().and_then(|e| e.constraint()),
            Some("lot_expiration_mois_seul_fin_de_mois")
        );
    }

    /// Équivalents classés (demande du porteur du 2026-11-16) : d'abord
    /// même DCI, même dosage et même forme ; ensuite dosage ou forme
    /// différents, marqués ; jamais une association ; rien n'est caché, même
    /// sans stock.
    #[sqlx::test]
    #[ignore]
    async fn equivalents_classes_par_dosage_et_forme(pool: PgPool) {
        let (_, pharmacien) = pharmacie(&pool, "706601").await;
        let (x, y, z) = catalogue_fictif(&pool).await;
        let admin: Uuid =
            sqlx::query_scalar("SELECT administrateur_compte_id FROM import_catalogue")
                .fetch_one(&pool)
                .await
                .unwrap();
        let complement = "denomination;dci;forme;dosage\n\
                          Produit W fictif;Molécule-A fictive;comprimé;1 g\n\
                          Produit V fictif;MOLÉCULE-A FICTIVE;sirop;500 mg\n\
                          Produit U fictif;Molécule-A fictive;Comprimé;500  MG\n";
        crate::catalogue::importer(&pool, admin, "Liste fictive 2", complement.as_bytes())
            .await
            .unwrap();
        let id = |nom: &'static str| {
            let pool = pool.clone();
            async move {
                sqlx::query_scalar::<_, Uuid>("SELECT id FROM medicament WHERE denomination = $1")
                    .bind(nom)
                    .fetch_one(&pool)
                    .await
                    .unwrap()
            }
        };
        let (w, v, u) = (
            id("Produit W fictif").await,
            id("Produit V fictif").await,
            id("Produit U fictif").await,
        );
        // Seul W est en stock : il reste quand même après les identiques.
        entrer(&pool, pharmacien, None, &entree(w, "LOT-W", 60, 3, 100))
            .await
            .unwrap();

        let eq = equivalents(&pool, pharmacien.structure_id, x)
            .await
            .unwrap()
            .unwrap();
        let vus: Vec<(Uuid, Correspondance, bool, bool, i64)> = eq
            .equivalents
            .iter()
            .map(|e| {
                (
                    e.medicament.id,
                    e.correspondance,
                    e.meme_dosage,
                    e.meme_forme,
                    e.quantite_disponible,
                )
            })
            .collect();
        use Correspondance::*;
        assert_eq!(
            vus,
            [
                // Identiques (majuscules et espaces en trop ignorées), par nom.
                (u, MemeDciMemeDosageMemeForme, true, true, 0),
                (y, MemeDciMemeDosageMemeForme, true, true, 0),
                // Différents : en stock d'abord.
                (w, MemeDciDosageOuFormeDifferent, false, true, 3),
                (v, MemeDciDosageOuFormeDifferent, true, false, 0),
            ]
        );
        assert!(
            vus.iter().all(|e| e.0 != z && e.0 != x),
            "ni l'association ni le médicament lui-même"
        );
        let json = serde_json::to_value(&eq.equivalents[2]).unwrap();
        assert_eq!(json["correspondance"], "meme_dci_dosage_ou_forme_different");
        assert_eq!(
            (json["meme_dosage"].as_bool(), json["denomination"].as_str()),
            (Some(false), Some("Produit W fictif"))
        );
    }

    /// Pertes, inventaire express, annulations, seuils et alertes,
    /// réglages, équivalents par DCI.
    #[sqlx::test]
    #[ignore]
    async fn pertes_inventaire_alertes_et_equivalents(pool: PgPool) {
        let (responsable, pharmacien) = pharmacie(&pool, "706301").await;
        let (x, y, z) = catalogue_fictif(&pool).await;
        let lot_a = entrer(&pool, pharmacien, None, &entree(x, "LOT-A", 30, 10, 100))
            .await
            .unwrap();
        let lot_b = entrer(&pool, pharmacien, None, &entree(x, "LOT-B", 200, 5, 100))
            .await
            .unwrap();
        let lot_y = entrer(&pool, pharmacien, None, &entree(y, "LOT-Y", 200, 4, 90))
            .await
            .unwrap();
        let perime = lot_perime(&pool, pharmacien, x, 3).await;

        // Pertes.
        let perte = |lot_id, quantite, motif, commentaire: Option<&str>| NouvellePerte {
            lot_id,
            quantite,
            motif,
            commentaire: commentaire.map(String::from),
        };
        let casse = declarer_perte(
            &pool,
            pharmacien,
            None,
            &perte(lot_a.id, 2, MotifPerte::Casse, None),
        )
        .await
        .unwrap();
        assert_eq!(
            (
                casse.quantite,
                casse.motif_perte.as_deref(),
                casse.prix_achat_unitaire_fcfa
            ),
            (-2, Some("casse"), 100)
        );
        assert!(matches!(
            declarer_perte(
                &pool,
                pharmacien,
                None,
                &perte(lot_a.id, 1, MotifPerte::Autre, None)
            )
            .await,
            Err(Erreur::DonneesInvalides(_))
        ));
        assert!(matches!(
            declarer_perte(
                &pool,
                pharmacien,
                None,
                &perte(lot_a.id, 99, MotifPerte::Casse, None)
            )
            .await,
            Err(Erreur::StockInsuffisant {
                quantite_disponible: 8,
                ..
            })
        ));
        declarer_perte(
            &pool,
            pharmacien,
            None,
            &perte(perime, 3, MotifPerte::Peremption, None),
        )
        .await
        .unwrap();

        // Inventaire : A compté 6 (stock 8, écart -2), B juste.
        let compte = NouvelInventaire {
            lignes: vec![
                LigneInventaire {
                    lot_id: lot_a.id,
                    quantite_comptee: 6,
                },
                LigneInventaire {
                    lot_id: lot_b.id,
                    quantite_comptee: 5,
                },
            ],
        };
        let inventaire = inventorier(&pool, pharmacien, None, &compte).await.unwrap();
        assert_eq!(
            inventaire
                .ecarts
                .iter()
                .map(|e| (e.lot_id, e.quantite))
                .collect::<Vec<_>>(),
            [(lot_a.id, -2)]
        );
        let double = NouvelInventaire {
            lignes: vec![compte.lignes[0].clone(), compte.lignes[0].clone()],
        };
        assert!(matches!(
            inventorier(&pool, pharmacien, None, &double).await,
            Err(Erreur::DonneesInvalides(_))
        ));

        // Annulations : une perte revient ; une annulation ne s'annule pas ;
        // une entrée en partie sortie ne s'annule pas.
        let motif = Annulation {
            motif: "Erreur de saisie".into(),
        };
        let retour = annuler_mouvement(&pool, pharmacien, None, casse.id, &motif)
            .await
            .unwrap();
        assert_eq!(
            (
                retour.nature.as_str(),
                retour.quantite,
                retour.annule_mouvement_id
            ),
            ("annulation", 2, Some(casse.id))
        );
        assert_eq!(quantite(&pool, lot_a.id).await, 8);
        assert!(matches!(
            annuler_mouvement(&pool, pharmacien, None, retour.id, &motif).await,
            Err(Erreur::AnnulationImpossible)
        ));
        assert!(matches!(
            annuler_mouvement(&pool, pharmacien, None, casse.id, &motif).await,
            Err(Erreur::DejaAnnule)
        ));
        let entree_a = journal(&pool, pharmacien.structure_id, None, 100)
            .await
            .unwrap()
            .into_iter()
            .find(|m| m.lot_id == lot_a.id && m.nature == "entree")
            .unwrap();
        assert!(matches!(
            annuler_mouvement(&pool, pharmacien, None, entree_a.id, &motif).await,
            Err(Erreur::StockInsuffisant { .. })
        ));

        // Seuils et alertes ; alerte de péremption désactivée par défaut.
        fixer_seuils(
            &pool,
            pharmacien,
            x,
            &Seuils {
                seuil_min: 20,
                seuil_max: None,
            },
        )
        .await
        .unwrap();
        fixer_seuils(
            &pool,
            pharmacien,
            y,
            &Seuils {
                seuil_min: 0,
                seuil_max: Some(1),
            },
        )
        .await
        .unwrap();
        assert!(matches!(
            fixer_seuils(
                &pool,
                pharmacien,
                y,
                &Seuils {
                    seuil_min: 5,
                    seuil_max: Some(1)
                }
            )
            .await,
            Err(Erreur::DonneesInvalides(_))
        ));
        let a = alertes(&pool, pharmacien.structure_id).await.unwrap();
        assert_eq!(
            a.sous_seuil_min
                .iter()
                .map(|s| (s.medicament_id, s.quantite_disponible))
                .collect::<Vec<_>>(),
            [(x, 13)]
        );
        assert_eq!(
            a.au_dessus_seuil_max
                .iter()
                .map(|s| s.medicament_id)
                .collect::<Vec<_>>(),
            [y]
        );
        assert!(!a.alerte_peremption_active && a.bientot_perimes.is_empty());
        assert!(
            a.perimes_en_stock.is_empty(),
            "le lot périmé est sorti en perte"
        );

        assert!(
            regler(
                &pool,
                pharmacien,
                &Reglages {
                    delai_alerte_peremption_jours: Some(40)
                }
            )
            .await
            .is_err(),
            "réservé au responsable par la base"
        );
        assert!(matches!(
            regler(
                &pool,
                responsable,
                &Reglages {
                    delai_alerte_peremption_jours: Some(0)
                }
            )
            .await,
            Err(Erreur::DonneesInvalides(_))
        ));
        regler(
            &pool,
            responsable,
            &Reglages {
                delai_alerte_peremption_jours: Some(40),
            },
        )
        .await
        .unwrap();
        let a = alertes(&pool, pharmacien.structure_id).await.unwrap();
        assert!(a.alerte_peremption_active);
        assert_eq!(
            a.bientot_perimes
                .iter()
                .map(|l| (l.lot_id, l.jours_restants))
                .collect::<Vec<_>>(),
            [(lot_a.id, 30)]
        );

        // Équivalents de X : Y (même molécule, en stock), jamais Z
        // (association).
        let eq = equivalents(&pool, pharmacien.structure_id, x)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            eq.equivalents
                .iter()
                .map(|e| (e.medicament.id, e.quantite_disponible))
                .collect::<Vec<_>>(),
            [(y, 4)]
        );
        assert!(eq.equivalents.iter().all(|e| e.medicament.id != z));
        assert!(
            equivalents(&pool, pharmacien.structure_id, Uuid::max())
                .await
                .unwrap()
                .is_none()
        );
        let _ = lot_y;
    }
}
