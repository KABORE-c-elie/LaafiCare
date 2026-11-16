//! Dépenses et bénéfice net d'une pharmacie (besoin du porteur du
//! 2026-11-16, absent du CDC ; CLAUDE.md §15 ; migration 0032).
//!
//! **Ce n'est pas la comptabilité officielle (SYSCOHADA)** : un tableau de
//! gestion pour le responsable, en FCFA entiers, sans aucun calcul d'impôt
//! (les impôts et taxes sont une dépense saisie).
//!
//! Calcul du rapport, sur une période de dates du Burkina (du … au,
//! comprises) :
//! - **ventes** : somme des lignes de vente au prix de vente figé, moins les
//!   ventes annulées pendant la période ;
//! - **coût d'achat des produits vendus** : les mêmes lignes au prix d'achat
//!   figé de leur lot ;
//! - **marge brute** = ventes − coût d'achat des produits vendus ;
//! - **pertes** au prix d'achat du lot : péremption, casse, autre, et écarts
//!   d'inventaire (un manque compte comme une perte, un excédent la
//!   diminue) ;
//! - **dépenses** : somme du journal, par catégorie ;
//! - **bénéfice net** = marge brute − pertes − dépenses ;
//! - **valeur du stock restant** à la fin de la période, au prix d'achat,
//!   dont la part déjà périmée à cette date.
//!
//! **Annulations, une seule règle** (décision du porteur du 2026-11-16) :
//! toute annulation (vente, perte, écart d'inventaire, entrée, dépense)
//! corrige la **période du mouvement d'origine**, dans la catégorie de
//! celui-ci. Le journal n'est jamais modifié : l'annulation est une ligne
//! tracée avec sa propre date, mais le rapport la rattache à la date du
//! mouvement annulé.
//!
//! Choix fait seul, noté dans docs/points-ouverts.md : les **achats**
//! (entrées en stock) ne sont pas des dépenses : leur coût compte quand le
//! produit est vendu ou perdu, le reste est dans la valeur du stock.

use chrono::{Datelike, Duration, NaiveDate};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use sqlx::types::Uuid;

use crate::idempotence::{self, Cle, ErreurIdempotence, Reservation};
use crate::stock_pharmacie::AffectationPharmacie;

/// Garde-fou technique contre une faute de frappe : mille milliards de FCFA.
const MONTANT_MAX: i64 = 1_000_000_000_000;
const LIBELLE_MAX: usize = 200;
const MOTIF_MAX: usize = 1000;
/// Rapport sur dix ans au plus (garde-fou contre une requête trop longue).
const DUREE_RAPPORT_MAX_JOURS: i64 = 3660;

/// Catégories de dépenses (liste fermée du porteur, migration 0032).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Categorie {
    Loyer,
    Salaires,
    Electricite,
    Eau,
    Transport,
    ImpotsTaxes,
    Entretien,
    Autres,
}

impl Categorie {
    pub const TOUTES: [Categorie; 8] = [
        Categorie::Loyer,
        Categorie::Salaires,
        Categorie::Electricite,
        Categorie::Eau,
        Categorie::Transport,
        Categorie::ImpotsTaxes,
        Categorie::Entretien,
        Categorie::Autres,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Categorie::Loyer => "loyer",
            Categorie::Salaires => "salaires",
            Categorie::Electricite => "electricite",
            Categorie::Eau => "eau",
            Categorie::Transport => "transport",
            Categorie::ImpotsTaxes => "impots_taxes",
            Categorie::Entretien => "entretien",
            Categorie::Autres => "autres",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ErreurBenefice {
    #[error("Dépense inconnue.")]
    DepenseInconnue,
    #[error("Cette dépense est déjà annulée.")]
    DejaAnnulee,
    #[error("Une annulation ne s'annule pas.")]
    AnnulationImpossible,
    #[error("{0}")]
    Idempotence(#[from] ErreurIdempotence),
    /// Le message ne cite que le nom du champ, jamais la valeur saisie.
    #[error("{0}")]
    DonneesInvalides(String),
    #[error("erreur interne : {0}")]
    Interne(String),
}

impl From<sqlx::Error> for ErreurBenefice {
    fn from(erreur: sqlx::Error) -> Self {
        // Deux annulations simultanées de la même dépense : l'index unique
        // de la migration 0032 refuse la seconde.
        match erreur.as_database_error().and_then(|e| e.constraint()) {
            Some("depense_pharmacie_annule_depense_id_key") => ErreurBenefice::DejaAnnulee,
            _ => ErreurBenefice::Interne(erreur.to_string()),
        }
    }
}

type Erreur = ErreurBenefice;

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

async fn aujourdhui<'e, E: sqlx::PgExecutor<'e>>(executeur: E) -> Result<NaiveDate, sqlx::Error> {
    sqlx::query_scalar("SELECT (now() AT TIME ZONE 'Africa/Ouagadougou')::date")
        .fetch_one(executeur)
        .await
}

// ---------------------------------------------------------------------
// Dépenses
// ---------------------------------------------------------------------

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Depense {
    pub id: Uuid,
    pub date_depense: NaiveDate,
    pub categorie: String,
    /// Négatif pour une annulation.
    pub montant_fcfa: i64,
    pub libelle: String,
    pub annule_depense_id: Option<Uuid>,
    pub motif_annulation: Option<String>,
    /// L'annulation de cette dépense, s'il y en a une.
    pub annulee_par: Option<Uuid>,
    pub date_creation: chrono::DateTime<chrono::Utc>,
    pub auteur_nom: Option<String>,
    pub auteur_prenom: Option<String>,
}

macro_rules! select_depense {
    () => {
        "SELECT d.id, d.date_depense, d.categorie, d.montant_fcfa, d.libelle, d.annule_depense_id, d.motif_annulation, \
                (SELECT a.id FROM depense_pharmacie a WHERE a.annule_depense_id = d.id) AS annulee_par, \
                d.date_creation, u.nom AS auteur_nom, u.prenom AS auteur_prenom \
         FROM depense_pharmacie d \
         JOIN affectation af ON af.id = d.auteur_affectation_id \
         JOIN compte c ON c.id = af.compte_id \
         JOIN utilisateur u ON u.id = c.utilisateur_id"
    };
}

async fn depense<'e, E: sqlx::PgExecutor<'e>>(
    executeur: E,
    structure_id: Uuid,
    id: Uuid,
) -> Result<Option<Depense>, sqlx::Error> {
    sqlx::query_as(concat!(
        select_depense!(),
        " WHERE d.structure_id = $1 AND d.id = $2"
    ))
    .bind(structure_id)
    .bind(id)
    .fetch_optional(executeur)
    .await
}

/// Journal des dépenses de la pharmacie, la plus récemment saisie d'abord
/// (`avant` : identifiant de la dernière dépense de la page précédente).
pub async fn lister_depenses(
    pool: &PgPool,
    structure_id: Uuid,
    avant: Option<Uuid>,
    limite: i64,
) -> Result<Vec<Depense>, sqlx::Error> {
    sqlx::query_as(concat!(
        select_depense!(),
        " WHERE d.structure_id = $1 AND ($2::uuid IS NULL OR d.id < $2) ORDER BY d.id DESC LIMIT $3"
    ))
    .bind(structure_id)
    .bind(avant)
    .bind(limite)
    .fetch_all(pool)
    .await
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NouvelleDepense {
    pub date_depense: NaiveDate,
    pub categorie: Categorie,
    pub montant_fcfa: i64,
    pub libelle: String,
}

/// Enregistre une dépense (le responsable seulement, garanti aussi par la
/// base). Date jamais dans le futur (date du Burkina).
pub async fn enregistrer_depense(
    pool: &PgPool,
    responsable: AffectationPharmacie,
    cle: Option<&Cle>,
    d: &NouvelleDepense,
) -> Result<Depense, Erreur> {
    let libelle = texte(&d.libelle, "libelle", LIBELLE_MAX)?;
    if !(1..=MONTANT_MAX).contains(&d.montant_fcfa) {
        return Err(invalide(format!(
            "montant_fcfa : entre 1 et {MONTANT_MAX} FCFA."
        )));
    }
    let mut tx = pool.begin().await?;
    if d.date_depense > aujourdhui(&mut *tx).await? {
        return Err(invalide("date_depense : jamais dans le futur."));
    }
    let empreinte = idempotence::empreinte("depense", d);
    let id = match idempotence::reserver(
        &mut tx,
        responsable.structure_id,
        cle,
        "depense",
        &empreinte,
        None,
    )
    .await??
    {
        Reservation::DejaFaite(id) => {
            tx.rollback().await?;
            return depense(pool, responsable.structure_id, id)
                .await?
                .ok_or_else(|| Erreur::Interne("dépense absente".into()));
        }
        Reservation::Nouvelle(id) => id,
    };
    sqlx::query(
        "INSERT INTO depense_pharmacie (id, structure_id, date_depense, categorie, montant_fcfa, libelle, \
                                        auteur_affectation_id, auteur_structure_id, auteur_role) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $2, $8)",
    )
    .bind(id)
    .bind(responsable.structure_id)
    .bind(d.date_depense)
    .bind(d.categorie.as_str())
    .bind(d.montant_fcfa)
    .bind(&libelle)
    .bind(responsable.affectation_id)
    .bind(responsable.role.as_str())
    .execute(&mut *tx)
    .await?;
    let cree = depense(&mut *tx, responsable.structure_id, id)
        .await?
        .ok_or_else(|| Erreur::Interne("dépense absente".into()))?;
    tx.commit().await?;
    Ok(cree)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnulationDepense {
    pub motif: String,
}

/// Annule une dépense : une écriture au montant opposé, même date, même
/// catégorie et même libellé, avec un motif. Une seule annulation par
/// dépense (index unique) ; une annulation ne s'annule pas.
pub async fn annuler_depense(
    pool: &PgPool,
    responsable: AffectationPharmacie,
    cle: Option<&Cle>,
    depense_id: Uuid,
    a: &AnnulationDepense,
) -> Result<Depense, Erreur> {
    let motif = texte(&a.motif, "motif", MOTIF_MAX)?;
    let mut tx = pool.begin().await?;
    let empreinte = idempotence::empreinte("annulation_depense", &(depense_id, a));
    let id = match idempotence::reserver(
        &mut tx,
        responsable.structure_id,
        cle,
        "annulation_depense",
        &empreinte,
        None,
    )
    .await??
    {
        Reservation::DejaFaite(id) => {
            tx.rollback().await?;
            return depense(pool, responsable.structure_id, id)
                .await?
                .ok_or_else(|| Erreur::Interne("dépense absente".into()));
        }
        Reservation::Nouvelle(id) => id,
    };
    // Lue dans la pharmacie de l'appelant seulement : une dépense d'une
    // autre pharmacie répond comme une dépense inexistante.
    let Some(originale) = depense(&mut *tx, responsable.structure_id, depense_id).await? else {
        return Err(Erreur::DepenseInconnue);
    };
    if originale.annule_depense_id.is_some() {
        return Err(Erreur::AnnulationImpossible);
    }
    if originale.annulee_par.is_some() {
        return Err(Erreur::DejaAnnulee);
    }
    sqlx::query(
        "INSERT INTO depense_pharmacie (id, structure_id, date_depense, categorie, montant_fcfa, libelle, annule_depense_id, \
                                        motif_annulation, auteur_affectation_id, auteur_structure_id, auteur_role) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $2, $10)",
    )
    .bind(id)
    .bind(responsable.structure_id)
    .bind(originale.date_depense)
    .bind(&originale.categorie)
    .bind(-originale.montant_fcfa)
    .bind(&originale.libelle)
    .bind(depense_id)
    .bind(&motif)
    .bind(responsable.affectation_id)
    .bind(responsable.role.as_str())
    .execute(&mut *tx)
    .await?;
    let cree = depense(&mut *tx, responsable.structure_id, id)
        .await?
        .ok_or_else(|| Erreur::Interne("dépense absente".into()))?;
    tx.commit().await?;
    Ok(cree)
}

// ---------------------------------------------------------------------
// Période et rapport
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Periode {
    /// Du lundi au dimanche.
    Semaine,
    Mois,
    Annee,
}

/// Bornes d'une période qui contient `date` (comprises).
pub fn bornes(periode: Periode, date: NaiveDate) -> (NaiveDate, NaiveDate) {
    match periode {
        Periode::Semaine => {
            let lundi = date - Duration::days(i64::from(date.weekday().num_days_from_monday()));
            (lundi, lundi + Duration::days(6))
        }
        Periode::Mois => {
            let premier = date.with_day(1).unwrap_or(date);
            let suivant = if premier.month() == 12 {
                NaiveDate::from_ymd_opt(premier.year() + 1, 1, 1)
            } else {
                NaiveDate::from_ymd_opt(premier.year(), premier.month() + 1, 1)
            };
            (premier, suivant.map_or(premier, |s| s - Duration::days(1)))
        }
        Periode::Annee => (
            NaiveDate::from_ymd_opt(date.year(), 1, 1).unwrap_or(date),
            NaiveDate::from_ymd_opt(date.year(), 12, 31).unwrap_or(date),
        ),
    }
}

/// Ce que demande l'appelant : une période (semaine, mois, année) autour
/// d'une date (aujourd'hui par défaut), **ou** deux dates.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ChoixPeriode {
    pub periode: Option<Periode>,
    pub date: Option<NaiveDate>,
    pub du: Option<NaiveDate>,
    pub au: Option<NaiveDate>,
}

#[derive(Debug, Serialize)]
pub struct Pertes {
    pub peremption_fcfa: i64,
    pub casse_fcfa: i64,
    pub autre_fcfa: i64,
    /// Manques moins excédents d'inventaire (négatif si les excédents
    /// l'emportent).
    pub ecarts_inventaire_fcfa: i64,
    pub total_fcfa: i64,
}

#[derive(Debug, Serialize)]
pub struct DepenseParCategorie {
    pub categorie: &'static str,
    pub montant_fcfa: i64,
}

#[derive(Debug, Serialize)]
pub struct Depenses {
    /// Les huit catégories, dans l'ordre de la liste, même à 0.
    pub par_categorie: Vec<DepenseParCategorie>,
    pub total_fcfa: i64,
}

#[derive(Debug, Serialize)]
pub struct Rapport {
    pub du: NaiveDate,
    pub au: NaiveDate,
    pub ventes_fcfa: i64,
    pub cout_achat_vendu_fcfa: i64,
    pub marge_brute_fcfa: i64,
    pub pertes: Pertes,
    pub depenses: Depenses,
    pub benefice_net_fcfa: i64,
    /// Stock restant à la fin du dernier jour, au prix d'achat.
    pub valeur_stock_fin_fcfa: i64,
    /// Dont les lots déjà périmés à cette date.
    pub dont_perime_fcfa: i64,
}

/// Rapport de la pharmacie sur la période choisie. Les trois lectures se
/// font dans une même transaction en lecture seule, `REPEATABLE READ` (doc
/// PostgreSQL 18, §13.2.2 : toutes les requêtes voient le même instantané) :
/// une vente enregistrée pendant le calcul ne peut pas compter dans les
/// ventes sans compter dans le stock.
pub async fn rapport(
    pool: &PgPool,
    structure_id: Uuid,
    choix: &ChoixPeriode,
) -> Result<Rapport, Erreur> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await?;
    let (du, au) = match (choix.periode, choix.du, choix.au) {
        (Some(periode), None, None) => {
            let date = match choix.date {
                Some(date) => date,
                None => aujourdhui(&mut *tx).await?,
            };
            bornes(periode, date)
        }
        (None, Some(du), Some(au)) if choix.date.is_none() => (du, au),
        _ => {
            return Err(invalide(
                "periode : donner soit periode (semaine, mois, annee) et éventuellement date, soit du et au.",
            ));
        }
    };
    if au < du || (au - du).num_days() > DUREE_RAPPORT_MAX_JOURS {
        return Err(invalide(format!(
            "periode : au après du, {DUREE_RAPPORT_MAX_JOURS} jours au plus."
        )));
    }

    // Règle unique des annulations (demande du porteur du 2026-11-16) :
    // toute annulation corrige la période du mouvement d'origine. Le journal
    // n'est jamais modifié (l'annulation garde sa propre date) ; le rapport
    // compte chaque mouvement à sa **date effective** : celle du mouvement
    // annulé pour une annulation, la sienne sinon. Conséquence voulue : le
    // rapport d'une période passée change quand une de ses opérations est
    // annulée plus tard.
    //
    // Bornes en instants : minuit au Burkina le premier jour, minuit le
    // lendemain du dernier.
    let (ventes, cout, peremption, casse, autre, ecarts): (i64, i64, i64, i64, i64, i64) = sqlx::query_as(
        "WITH mv AS ( \
             SELECT m.quantite::BIGINT AS q, m.prix_achat_unitaire_fcfa::BIGINT AS pa, \
                    m.prix_vente_unitaire_fcfa::BIGINT AS pv, \
                    COALESCE(o.nature, m.nature) AS nature, COALESCE(o.motif_perte, m.motif_perte) AS motif \
             FROM mouvement_stock m LEFT JOIN mouvement_stock o ON o.id = m.annule_mouvement_id \
             WHERE m.structure_id = $1 \
               AND COALESCE(o.date_mouvement, m.date_mouvement) >= ($2::date::timestamp AT TIME ZONE 'Africa/Ouagadougou') \
               AND COALESCE(o.date_mouvement, m.date_mouvement) < (($3::date + 1)::timestamp AT TIME ZONE 'Africa/Ouagadougou')) \
         SELECT COALESCE(sum(-q * pv) FILTER (WHERE nature = 'vente'), 0)::BIGINT, \
                COALESCE(sum(-q * pa) FILTER (WHERE nature = 'vente'), 0)::BIGINT, \
                COALESCE(sum(-q * pa) FILTER (WHERE nature = 'perte' AND motif = 'peremption'), 0)::BIGINT, \
                COALESCE(sum(-q * pa) FILTER (WHERE nature = 'perte' AND motif = 'casse'), 0)::BIGINT, \
                COALESCE(sum(-q * pa) FILTER (WHERE nature = 'perte' AND motif = 'autre'), 0)::BIGINT, \
                COALESCE(sum(-q * pa) FILTER (WHERE nature = 'inventaire'), 0)::BIGINT \
         FROM mv",
    )
    .bind(structure_id)
    .bind(du)
    .bind(au)
    .fetch_one(&mut *tx)
    .await?;

    // Même règle pour les dépenses : une annulation est comptée à la date de
    // la dépense annulée (lue sur elle, même si l'annulation la recopie).
    let lignes: Vec<(String, i64)> = sqlx::query_as(
        "SELECT d.categorie, sum(d.montant_fcfa)::BIGINT \
         FROM depense_pharmacie d LEFT JOIN depense_pharmacie o ON o.id = d.annule_depense_id \
         WHERE d.structure_id = $1 AND COALESCE(o.date_depense, d.date_depense) BETWEEN $2 AND $3 \
         GROUP BY d.categorie",
    )
    .bind(structure_id)
    .bind(du)
    .bind(au)
    .fetch_all(&mut *tx)
    .await?;
    let par_categorie: Vec<DepenseParCategorie> = Categorie::TOUTES
        .iter()
        .map(|c| DepenseParCategorie {
            categorie: c.as_str(),
            montant_fcfa: lignes
                .iter()
                .find(|(nom, _)| nom == c.as_str())
                .map_or(0, |(_, m)| *m),
        })
        .collect();
    let total_depenses = par_categorie.iter().map(|d| d.montant_fcfa).sum();

    // Un lot est utilisable jusqu'à sa date d'expiration comprise
    // (stock_pharmacie) : il est périmé à la fin du jour `au` si sa date est
    // antérieure à `au`. Même date effective que ci-dessus : une vente
    // annulée plus tard n'a jamais fait sortir le produit de la période, il
    // compte dans le stock restant.
    let (valeur_stock, perime): (i64, i64) = sqlx::query_as(
        "SELECT COALESCE(sum(m.quantite::BIGINT * m.prix_achat_unitaire_fcfa), 0)::BIGINT, \
                COALESCE(sum(m.quantite::BIGINT * m.prix_achat_unitaire_fcfa) FILTER (WHERE l.date_expiration < $2), 0)::BIGINT \
         FROM mouvement_stock m JOIN lot_stock l ON l.id = m.lot_id \
         LEFT JOIN mouvement_stock o ON o.id = m.annule_mouvement_id \
         WHERE m.structure_id = $1 \
           AND COALESCE(o.date_mouvement, m.date_mouvement) < (($2::date + 1)::timestamp AT TIME ZONE 'Africa/Ouagadougou')",
    )
    .bind(structure_id)
    .bind(au)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;

    let total_pertes = peremption + casse + autre + ecarts;
    let marge_brute = ventes - cout;
    Ok(Rapport {
        du,
        au,
        ventes_fcfa: ventes,
        cout_achat_vendu_fcfa: cout,
        marge_brute_fcfa: marge_brute,
        pertes: Pertes {
            peremption_fcfa: peremption,
            casse_fcfa: casse,
            autre_fcfa: autre,
            ecarts_inventaire_fcfa: ecarts,
            total_fcfa: total_pertes,
        },
        depenses: Depenses {
            par_categorie,
            total_fcfa: total_depenses,
        },
        benefice_net_fcfa: marge_brute - total_pertes - total_depenses,
        valeur_stock_fin_fcfa: valeur_stock,
        dont_perime_fcfa: perime,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stock_pharmacie::tests::{catalogue_fictif, dans, entree, pharmacie, vente_de};
    use crate::stock_pharmacie::{
        self as stock, Annulation, LigneDemandee, LigneInventaire, MotifPerte, NouvelInventaire,
        NouvellePerte,
    };

    fn jour(a: i32, m: u32, j: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(a, m, j).unwrap()
    }

    #[test]
    fn bornes_des_periodes() {
        // Le 2026-11-16 est un lundi.
        assert_eq!(
            bornes(Periode::Semaine, jour(2026, 11, 22)),
            (jour(2026, 11, 16), jour(2026, 11, 22))
        );
        assert_eq!(
            bornes(Periode::Semaine, jour(2026, 11, 16)),
            (jour(2026, 11, 16), jour(2026, 11, 22))
        );
        assert_eq!(
            bornes(Periode::Semaine, jour(2027, 1, 1)),
            (jour(2026, 12, 28), jour(2027, 1, 3))
        );
        assert_eq!(
            bornes(Periode::Mois, jour(2026, 12, 9)),
            (jour(2026, 12, 1), jour(2026, 12, 31))
        );
        assert_eq!(
            bornes(Periode::Mois, jour(2028, 2, 10)),
            (jour(2028, 2, 1), jour(2028, 2, 29))
        );
        assert_eq!(
            bornes(Periode::Annee, jour(2026, 6, 30)),
            (jour(2026, 1, 1), jour(2026, 12, 31))
        );
    }

    fn depense_de(
        categorie: Categorie,
        montant_fcfa: i64,
        date_depense: NaiveDate,
    ) -> NouvelleDepense {
        NouvelleDepense {
            date_depense,
            categorie,
            montant_fcfa,
            libelle: "Libellé fictif".into(),
        }
    }

    /// Un rapport chiffré à la main : ventes, coût, pertes de chaque sorte,
    /// écarts d'inventaire dans les deux sens, dépenses et leur annulation,
    /// valeur du stock et part périmée. Une autre pharmacie ne voit rien.
    #[sqlx::test]
    #[ignore]
    async fn rapport_chiffre(pool: PgPool) {
        let (responsable, pharmacien) = pharmacie(&pool, "706401").await;
        let (autre_responsable, _) = pharmacie(&pool, "706402").await;
        let (x, _, _) = catalogue_fictif(&pool).await;
        stock::fixer_prix(&pool, responsable, x, 200).await.unwrap();
        let lot_a = stock::entrer(&pool, pharmacien, None, &entree(x, "LOT-A", 30, 10, 100))
            .await
            .unwrap();
        let lot_b = stock::entrer(&pool, pharmacien, None, &entree(x, "LOT-B", 200, 5, 120))
            .await
            .unwrap();
        // Deux lots périmés posés en SQL : l'un sort en perte, l'autre reste.
        let mut perimes = Vec::new();
        for (numero, quantite) in [("PERIME-1", 4), ("PERIME-2", 2)] {
            let lot: Uuid = sqlx::query_scalar(
                "INSERT INTO lot_stock (structure_id, medicament_id, numero_lot, date_expiration, prix_achat_unitaire_fcfa, fournisseur) \
                 VALUES ($1, $2, $3, CURRENT_DATE - 2, 50, 'Grossiste fictif') RETURNING id",
            )
            .bind(pharmacien.structure_id)
            .bind(x)
            .bind(numero)
            .fetch_one(&pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO mouvement_stock (structure_id, lot_id, nature, quantite, prix_achat_unitaire_fcfa, \
                                              auteur_affectation_id, auteur_structure_id, auteur_role) \
                 VALUES ($1, $2, 'entree', $3, 50, $4, $1, 'pharmacien')",
            )
            .bind(pharmacien.structure_id)
            .bind(lot)
            .bind(quantite)
            .bind(pharmacien.affectation_id)
            .execute(&pool)
            .await
            .unwrap();
            perimes.push(lot);
        }

        // Ventes : 3 (gardée), 2 (annulée).
        stock::vendre(
            &pool,
            pharmacien,
            None,
            &vente_de(vec![LigneDemandee {
                medicament_id: x,
                quantite: 3,
                lot_id: None,
            }]),
        )
        .await
        .unwrap();
        let annulee = stock::vendre(
            &pool,
            pharmacien,
            None,
            &vente_de(vec![LigneDemandee {
                medicament_id: x,
                quantite: 2,
                lot_id: None,
            }]),
        )
        .await
        .unwrap();
        stock::annuler_vente(
            &pool,
            pharmacien,
            None,
            annulee.id,
            &Annulation {
                motif: "Erreur".into(),
            },
        )
        .await
        .unwrap();
        // Pertes : casse 1 sur A (100), péremption 4 (200), une perte
        // « autre » annulée (0).
        let perte = |lot_id, quantite, motif, commentaire: Option<&str>| NouvellePerte {
            lot_id,
            quantite,
            motif,
            commentaire: commentaire.map(String::from),
        };
        stock::declarer_perte(
            &pool,
            pharmacien,
            None,
            &perte(lot_a.id, 1, MotifPerte::Casse, None),
        )
        .await
        .unwrap();
        stock::declarer_perte(
            &pool,
            pharmacien,
            None,
            &perte(perimes[0], 4, MotifPerte::Peremption, None),
        )
        .await
        .unwrap();
        let autre = stock::declarer_perte(
            &pool,
            pharmacien,
            None,
            &perte(lot_b.id, 1, MotifPerte::Autre, Some("Vol")),
        )
        .await
        .unwrap();
        stock::annuler_mouvement(
            &pool,
            pharmacien,
            None,
            autre.id,
            &Annulation {
                motif: "Retrouvé".into(),
            },
        )
        .await
        .unwrap();
        // Inventaire : A compté 5 (stock 6, manque 100), B compté 6 (stock 5,
        // excédent 120).
        let compte = NouvelInventaire {
            lignes: vec![
                LigneInventaire {
                    lot_id: lot_a.id,
                    quantite_comptee: 5,
                },
                LigneInventaire {
                    lot_id: lot_b.id,
                    quantite_comptee: 6,
                },
            ],
        };
        stock::inventorier(&pool, pharmacien, None, &compte)
            .await
            .unwrap();

        // Dépenses : loyer gardé, électricité annulée, transport hors de la
        // période demandée.
        let aujourdhui = dans(0);
        enregistrer_depense(
            &pool,
            responsable,
            None,
            &depense_de(Categorie::Loyer, 50_000, aujourdhui),
        )
        .await
        .unwrap();
        let electricite = enregistrer_depense(
            &pool,
            responsable,
            None,
            &depense_de(Categorie::Electricite, 10_000, aujourdhui),
        )
        .await
        .unwrap();
        let annulation = annuler_depense(
            &pool,
            responsable,
            None,
            electricite.id,
            &AnnulationDepense {
                motif: "Doublon".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            (
                annulation.montant_fcfa,
                annulation.date_depense,
                annulation.categorie.as_str()
            ),
            (-10_000, aujourdhui, "electricite")
        );
        enregistrer_depense(
            &pool,
            responsable,
            None,
            &depense_de(Categorie::Transport, 3_000, dans(-400)),
        )
        .await
        .unwrap();

        let r = rapport(
            &pool,
            responsable.structure_id,
            &ChoixPeriode {
                du: Some(aujourdhui),
                au: Some(aujourdhui),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            (r.ventes_fcfa, r.cout_achat_vendu_fcfa, r.marge_brute_fcfa),
            (600, 300, 300)
        );
        assert_eq!(
            (
                r.pertes.peremption_fcfa,
                r.pertes.casse_fcfa,
                r.pertes.autre_fcfa,
                r.pertes.ecarts_inventaire_fcfa,
                r.pertes.total_fcfa
            ),
            (200, 100, 0, -20, 280)
        );
        assert_eq!(r.depenses.total_fcfa, 50_000);
        assert_eq!(r.depenses.par_categorie.len(), 8);
        assert_eq!(r.depenses.par_categorie[0].categorie, "loyer");
        assert_eq!(
            r.depenses
                .par_categorie
                .iter()
                .find(|d| d.categorie == "electricite")
                .unwrap()
                .montant_fcfa,
            0
        );
        assert_eq!(r.benefice_net_fcfa, 300 - 280 - 50_000);
        assert_eq!(
            (r.valeur_stock_fin_fcfa, r.dont_perime_fcfa),
            (5 * 100 + 6 * 120 + 2 * 50, 2 * 50)
        );

        // L'année en cours ne compte pas la dépense de transport (400 jours).
        let annee = rapport(
            &pool,
            responsable.structure_id,
            &ChoixPeriode {
                periode: Some(Periode::Annee),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            (annee.du, annee.depenses.total_fcfa),
            (bornes(Periode::Annee, aujourdhui).0, 50_000)
        );
        // Avant toute entrée : rien ; la veille, le stock est vide.
        let veille = rapport(
            &pool,
            responsable.structure_id,
            &ChoixPeriode {
                du: Some(dans(-1)),
                au: Some(dans(-1)),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!((veille.ventes_fcfa, veille.valeur_stock_fin_fcfa), (0, 0));

        // L'autre pharmacie : un rapport vide, aucune dépense, aucune
        // annulation possible.
        let vide = rapport(
            &pool,
            autre_responsable.structure_id,
            &ChoixPeriode {
                du: Some(aujourdhui),
                au: Some(aujourdhui),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            (
                vide.ventes_fcfa,
                vide.pertes.total_fcfa,
                vide.depenses.total_fcfa,
                vide.valeur_stock_fin_fcfa
            ),
            (0, 0, 0, 0)
        );
        assert!(
            lister_depenses(&pool, autre_responsable.structure_id, None, 50)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(matches!(
            annuler_depense(
                &pool,
                autre_responsable,
                None,
                electricite.id,
                &AnnulationDepense {
                    motif: "Intrusion".into()
                }
            )
            .await,
            Err(Erreur::DepenseInconnue)
        ));
    }

    /// Mouvement daté d'un jour passé, posé en SQL (le code date toujours
    /// ses écritures du jour) : midi à l'heure du Burkina.
    #[allow(clippy::too_many_arguments)]
    async fn mouvement_du(
        pool: &PgPool,
        a: AffectationPharmacie,
        lot: Uuid,
        nature: &str,
        quantite: i32,
        prix_vente: Option<i32>,
        vente_id: Option<Uuid>,
        motif: Option<&str>,
        jour: NaiveDate,
    ) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO mouvement_stock (structure_id, lot_id, nature, quantite, prix_achat_unitaire_fcfa, prix_vente_unitaire_fcfa, \
                                          vente_id, motif_perte, auteur_affectation_id, auteur_structure_id, auteur_role, date_mouvement) \
             VALUES ($1, $2, $3, $4, 100, $5, $6, $7, $8, $1, $9, ($10::date::timestamp + interval '12 hours') AT TIME ZONE 'Africa/Ouagadougou') \
             RETURNING id",
        )
        .bind(a.structure_id)
        .bind(lot)
        .bind(nature)
        .bind(quantite)
        .bind(prix_vente)
        .bind(vente_id)
        .bind(motif)
        .bind(a.affectation_id)
        .bind(a.role.as_str())
        .bind(jour)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn vente_du(
        pool: &PgPool,
        a: AffectationPharmacie,
        lot: Uuid,
        quantite: i32,
        jour: NaiveDate,
    ) -> Uuid {
        let vente: Uuid = sqlx::query_scalar(
            "INSERT INTO vente (structure_id, sur_ordonnance_papier, auteur_affectation_id, auteur_structure_id, auteur_role, date_vente) \
             VALUES ($1, false, $2, $1, $3, ($4::date::timestamp + interval '12 hours') AT TIME ZONE 'Africa/Ouagadougou') RETURNING id",
        )
        .bind(a.structure_id)
        .bind(a.affectation_id)
        .bind(a.role.as_str())
        .bind(jour)
        .fetch_one(pool)
        .await
        .unwrap();
        mouvement_du(
            pool,
            a,
            lot,
            "vente",
            -quantite,
            Some(250),
            Some(vente),
            None,
            jour,
        )
        .await;
        vente
    }

    /// Règle unique des annulations (demande du porteur du 2026-11-16) : une
    /// vente, une perte et une dépense du mois précédent, annulées ce mois-ci,
    /// corrigent le mois précédent ; ce mois-ci n'en garde aucune trace dans
    /// le rapport. Chiffré à la main sur deux mois.
    #[sqlx::test]
    #[ignore]
    async fn annulations_rattachees_au_mois_d_origine(pool: PgPool) {
        let (responsable, pharmacien) = pharmacie(&pool, "706701").await;
        let (x, _, _) = catalogue_fictif(&pool).await;
        let aujourdhui = dans(0);
        // Le mois A est le mois précédent ; le mois B, le mois en cours.
        let debut_b = bornes(Periode::Mois, aujourdhui).0;
        let jour_a = |j: u32| (debut_b - Duration::days(1)).with_day(j).unwrap();

        // Mois A : entrée de 20 à 100 FCFA, deux ventes (3 et 2 à 250), une
        // casse de 1, deux dépenses.
        let lot: Uuid = sqlx::query_scalar(
            "INSERT INTO lot_stock (structure_id, medicament_id, numero_lot, date_expiration, prix_achat_unitaire_fcfa, fournisseur) \
             VALUES ($1, $2, 'LOT-1', $3, 100, 'Grossiste fictif') RETURNING id",
        )
        .bind(pharmacien.structure_id)
        .bind(x)
        .bind(dans(365))
        .fetch_one(&pool)
        .await
        .unwrap();
        mouvement_du(
            &pool,
            pharmacien,
            lot,
            "entree",
            20,
            None,
            None,
            None,
            jour_a(10),
        )
        .await;
        vente_du(&pool, pharmacien, lot, 3, jour_a(12)).await;
        let v2 = vente_du(&pool, pharmacien, lot, 2, jour_a(12)).await;
        let casse = mouvement_du(
            &pool,
            pharmacien,
            lot,
            "perte",
            -1,
            None,
            None,
            Some("casse"),
            jour_a(14),
        )
        .await;
        enregistrer_depense(
            &pool,
            responsable,
            None,
            &depense_de(Categorie::Loyer, 40_000, jour_a(15)),
        )
        .await
        .unwrap();
        let eau = enregistrer_depense(
            &pool,
            responsable,
            None,
            &depense_de(Categorie::Eau, 5_000, jour_a(15)),
        )
        .await
        .unwrap();

        // Mois B (aujourd'hui) : les trois annulations, par le vrai code ;
        // une vente de 1 à 300 et une dépense de transport.
        let motif = Annulation {
            motif: "Erreur de saisie".into(),
        };
        stock::annuler_vente(&pool, pharmacien, None, v2, &motif)
            .await
            .unwrap();
        stock::annuler_mouvement(&pool, pharmacien, None, casse, &motif)
            .await
            .unwrap();
        annuler_depense(
            &pool,
            responsable,
            None,
            eau.id,
            &AnnulationDepense {
                motif: "Doublon".into(),
            },
        )
        .await
        .unwrap();
        stock::fixer_prix(&pool, responsable, x, 300).await.unwrap();
        stock::vendre(
            &pool,
            pharmacien,
            None,
            &vente_de(vec![LigneDemandee {
                medicament_id: x,
                quantite: 1,
                lot_id: None,
            }]),
        )
        .await
        .unwrap();
        enregistrer_depense(
            &pool,
            responsable,
            None,
            &depense_de(Categorie::Transport, 2_000, aujourdhui),
        )
        .await
        .unwrap();

        // Le journal n'a pas bougé : les annulations y sont datées d'aujourd'hui.
        let annulations: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM mouvement_stock WHERE nature = 'annulation' \
             AND (date_mouvement AT TIME ZONE 'Africa/Ouagadougou')::date = $1",
        )
        .bind(aujourdhui)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(annulations, 2);

        let mois = |date| ChoixPeriode {
            periode: Some(Periode::Mois),
            date: Some(date),
            ..Default::default()
        };
        // Mois A : ventes 3 × 250 = 750, coût 300, marge 450 ; casse annulée :
        // 0 ; dépenses 40 000 (eau annulée) ; stock 20 − 3 = 17 × 100.
        let a = rapport(&pool, responsable.structure_id, &mois(jour_a(1)))
            .await
            .unwrap();
        assert_eq!(
            (a.ventes_fcfa, a.cout_achat_vendu_fcfa, a.marge_brute_fcfa),
            (750, 300, 450)
        );
        assert_eq!((a.pertes.casse_fcfa, a.pertes.total_fcfa), (0, 0));
        let montant = |r: &Rapport, c: &str| {
            r.depenses
                .par_categorie
                .iter()
                .find(|d| d.categorie == c)
                .unwrap()
                .montant_fcfa
        };
        assert_eq!(
            (
                a.depenses.total_fcfa,
                montant(&a, "eau"),
                montant(&a, "loyer")
            ),
            (40_000, 0, 40_000)
        );
        assert_eq!(a.benefice_net_fcfa, 450 - 40_000);
        assert_eq!(a.valeur_stock_fin_fcfa, 1_700);

        // Mois B : seulement la vente du jour (300, coût 100) et le
        // transport ; aucune trace des annulations ; stock 17 − 1 = 16.
        let b = rapport(&pool, responsable.structure_id, &mois(aujourdhui))
            .await
            .unwrap();
        assert_eq!(
            (b.ventes_fcfa, b.cout_achat_vendu_fcfa, b.marge_brute_fcfa),
            (300, 100, 200)
        );
        assert_eq!(
            (
                b.pertes.total_fcfa,
                b.depenses.total_fcfa,
                montant(&b, "eau")
            ),
            (0, 2_000, 0)
        );
        assert_eq!(b.benefice_net_fcfa, 200 - 2_000);
        assert_eq!(b.valeur_stock_fin_fcfa, 1_600);

        // Les deux mois ensemble : la somme des deux.
        let tout = rapport(
            &pool,
            responsable.structure_id,
            &ChoixPeriode {
                du: Some(jour_a(1)),
                au: Some(aujourdhui),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            (
                tout.ventes_fcfa,
                tout.depenses.total_fcfa,
                tout.benefice_net_fcfa
            ),
            (1_050, 42_000, 650 - 42_000)
        );
    }

    /// Dépenses : contrôles, idempotence, annulations refusées, auteur
    /// responsable garanti par la base, journal jamais modifié.
    #[sqlx::test]
    #[ignore]
    async fn depenses_controles_et_idempotence(pool: PgPool) {
        let (responsable, pharmacien) = pharmacie(&pool, "706501").await;
        let aujourdhui = dans(0);
        assert!(matches!(
            enregistrer_depense(
                &pool,
                responsable,
                None,
                &depense_de(Categorie::Eau, 1_000, dans(1))
            )
            .await,
            Err(Erreur::DonneesInvalides(_))
        ));
        for montant in [0, -5, MONTANT_MAX + 1] {
            assert!(matches!(
                enregistrer_depense(
                    &pool,
                    responsable,
                    None,
                    &depense_de(Categorie::Eau, montant, aujourdhui)
                )
                .await,
                Err(Erreur::DonneesInvalides(_))
            ));
        }
        let sans_libelle = NouvelleDepense {
            libelle: "  ".into(),
            ..depense_de(Categorie::Eau, 1_000, aujourdhui)
        };
        assert!(matches!(
            enregistrer_depense(&pool, responsable, None, &sans_libelle).await,
            Err(Erreur::DonneesInvalides(_))
        ));
        assert!(
            enregistrer_depense(
                &pool,
                pharmacien,
                None,
                &depense_de(Categorie::Eau, 1_000, aujourdhui)
            )
            .await
            .is_err(),
            "la base refuse un pharmacien comme auteur"
        );

        let cle = Cle::lire("depense-1").unwrap();
        let d = depense_de(Categorie::ImpotsTaxes, 25_000, aujourdhui);
        let premiere = enregistrer_depense(&pool, responsable, Some(&cle), &d)
            .await
            .unwrap();
        let rejouee = enregistrer_depense(&pool, responsable, Some(&cle), &d)
            .await
            .unwrap();
        assert_eq!(premiere.id, rejouee.id);
        assert!(matches!(
            enregistrer_depense(
                &pool,
                responsable,
                Some(&cle),
                &depense_de(Categorie::ImpotsTaxes, 26_000, aujourdhui)
            )
            .await,
            Err(Erreur::Idempotence(ErreurIdempotence::CleReutilisee))
        ));
        assert_eq!(
            lister_depenses(&pool, responsable.structure_id, None, 50)
                .await
                .unwrap()
                .len(),
            1
        );

        let motif = AnnulationDepense {
            motif: "Erreur".into(),
        };
        let cle_annulation = Cle::lire("annulation-1").unwrap();
        let annulation = annuler_depense(
            &pool,
            responsable,
            Some(&cle_annulation),
            premiere.id,
            &motif,
        )
        .await
        .unwrap();
        assert_eq!(
            annuler_depense(
                &pool,
                responsable,
                Some(&cle_annulation),
                premiere.id,
                &motif
            )
            .await
            .unwrap()
            .id,
            annulation.id
        );
        assert!(matches!(
            annuler_depense(&pool, responsable, None, premiere.id, &motif).await,
            Err(Erreur::DejaAnnulee)
        ));
        assert!(matches!(
            annuler_depense(&pool, responsable, None, annulation.id, &motif).await,
            Err(Erreur::AnnulationImpossible)
        ));
        assert!(matches!(
            annuler_depense(&pool, responsable, None, Uuid::max(), &motif).await,
            Err(Erreur::DepenseInconnue)
        ));
        let journal = lister_depenses(&pool, responsable.structure_id, None, 50)
            .await
            .unwrap();
        assert_eq!(
            journal.iter().map(|d| d.id).collect::<Vec<_>>(),
            [annulation.id, premiere.id]
        );
        assert_eq!(journal[1].annulee_par, Some(annulation.id));
        assert_eq!(journal[0].auteur_nom.as_deref(), Some("SANOU"));

        // Période mal choisie.
        for choix in [
            ChoixPeriode::default(),
            ChoixPeriode {
                periode: Some(Periode::Mois),
                du: Some(aujourdhui),
                au: Some(aujourdhui),
                ..Default::default()
            },
            ChoixPeriode {
                du: Some(aujourdhui),
                au: Some(dans(-1)),
                ..Default::default()
            },
            ChoixPeriode {
                du: Some(aujourdhui),
                ..Default::default()
            },
            ChoixPeriode {
                du: Some(dans(-4000)),
                au: Some(aujourdhui),
                ..Default::default()
            },
        ] {
            assert!(
                matches!(
                    rapport(&pool, responsable.structure_id, &choix).await,
                    Err(Erreur::DonneesInvalides(_))
                ),
                "{choix:?}"
            );
        }

        for requete in [
            "UPDATE depense_pharmacie SET montant_fcfa = 1",
            "DELETE FROM depense_pharmacie",
            "TRUNCATE depense_pharmacie",
        ] {
            let erreur = sqlx::query(requete).execute(&pool).await.unwrap_err();
            assert!(
                erreur.to_string().contains("jamais modifiees"),
                "{requete} : {erreur}"
            );
        }
    }
}
