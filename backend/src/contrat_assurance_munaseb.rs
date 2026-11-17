//! ContratAssurance MUNASEB (section 12 du CLAUDE.md) : adhésion et
//! renouvellement enregistrés par un agent, paramètres de la mutuelle,
//! détail d'un contrat, solde d'une période (étape 5 ; « Modèle retenu pour
//! l'étape 5 », docs/points-ouverts.md, accord du porteur du 2026-10-22 ;
//! migration 0028). Champs basés sur les sources MUNASEB (migration 0006) :
//! aucune classe `Contrat`/`ContratAssurance` n'existe dans
//! `docs/diagrammes/diagramme_classes_munaseb oki11.drawio`, son équivalent
//! réel est `carteAdhesion`.
//!
//! Les dates vivent dans `periode_adhesion_munaseb` (migration 0009) : une
//! période par adhésion ou renouvellement, jamais écrasée. Depuis la 0028,
//! chaque période porte aussi son numéro de carte (il change chaque année),
//! son plafond (figé à sa création) et son paiement
//! (`paiement_cotisation_munaseb`).
//!
//! Seule exception aux sources : la colonne `statut` (actif/suspendu,
//! migration 0008), décision propre à LaafiCare. **La suspension n'est pas
//! codée** : ses règles sont à demander à la MUNASEB (docs/points-ouverts.md).
//! Un contrat suspendu (posé en SQL) n'est ni renouvelable ni couvert.
//!
//! Le taux de prise en charge n'est pas ici : il dépend du type d'acte et
//! vit en base (`tarif_acte_munaseb`, migration 0007).
//!
//! Ordre des verrous : le renouvellement verrouille le **contrat**, puis
//! insère la période et son paiement. Aucune autre opération ne verrouille
//! un contrat (la validation d'une demande verrouille la demande, puis la
//! période, §12) : aucun cycle n'est possible.

use chrono::{DateTime, Days, Months, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use sqlx::types::Uuid;

use crate::demande_remboursement_munaseb::AffectationAgent;
use crate::espace_patient_munaseb::EtatDroits;
use crate::nip;

// Durée d'une période : la fin est le jour anniversaire de l'adhésion,
// compris (réponse 1 du porteur, 2026-10-22, d'après la carte MUNASEB 2024 :
// validité = date d'adhésion + 1 an ; adhésion le 09/04/2024, fin le
// 09/04/2025). Avec la carence, la couverture réelle d'une première
// adhésion est de 11 mois. Une constante et non un paramètre : la carte
// dit « 1 an », aucune source ne la fait varier.
const DUREE: Months = Months::new(12);

// Garde-fous techniques des paramètres, identiques aux CHECK de la
// migration 0028 (contrôlés ici d'abord pour renvoyer un message clair).
pub const DELAI_GRACE_MAX_JOURS: i16 = 60;
pub const CARENCE_MAX_MOIS: i16 = 12;

// Longueurs maximales des textes saisis par l'agent : garde-fous techniques
// (même principe que le motif de 1 000 caractères, §12), pas des règles de
// la MUNASEB. Numéro de carte et antenne, niveau d'études et référence :
// mêmes bornes que les CHECK de la migration 0028.
const MAX_NUMERO_CARTE: usize = 50;
const MAX_TEXTE_COURT: usize = 100;
const MAX_MATRICULE: usize = 50;
const MAX_PERSONNE_A_PREVENIR: usize = 200;

// Limite d'échecs d'adhésion par agent (demande du porteur du 2026-11-16 ;
// migration 0029). Une adhésion désigne le patient par son NIP et sa date
// de naissance : sans limite, un agent (ou un jeton d'agent volé) pourrait
// essayer des couples en masse pour savoir quels NIP existent, et chez
// qui. Sources : OWASP API Security Top 10 2023, API4:2023 (Unrestricted
// Resource Consumption : limiter le nombre d'appels par client sur une
// période) ; OWASP Authentication Cheat Sheet, « Protect Against Automated
// Attacks » (limiter les essais, compter les échecs). Aucune des deux ne
// donne de valeur : on reprend celle que le projet applique déjà aux essais
// qui devinent un secret, 5 échecs en 15 minutes (verrouillage BF-01, §11),
// en fenêtre glissante comme les envois de code SMS (O2) : un succès ne
// remet pas le compte à zéro, sinon on alternerait essais et adhésions
// réelles. Un agent honnête qui fait une faute de frappe a quatre essais de
// marge, et attend au plus 15 minutes.
pub const ECHECS_ADHESION_MAX: i64 = 5;
pub const FENETRE_ECHECS_ADHESION_MINUTES: i32 = 15;

#[derive(Debug, thiserror::Error)]
pub enum ErreurContratAssuranceMunaseb {
    #[error("Ce numéro de carte est déjà utilisé.")]
    NumeroCarteDejaUtilise,

    #[error("Ce patient a déjà un contrat MUNASEB.")]
    ContratDejaExistant,

    #[error("Contrat inconnu.")]
    AucunContrat,

    /// Échoue par défaut (décision actée) : un appelant ne peut pas traiter
    /// par oubli un contrat suspendu comme actif. Pour le renouvellement :
    /// les règles de suspension sont inconnues, le contrat reste bloqué.
    #[error("Ce contrat est suspendu.")]
    ContratSuspendu,

    /// Aucune période en cours, mais une période payée commence plus tard.
    /// Distinct de `ContratExpire` : quelqu'un qui vient de payer ne doit
    /// pas lire "expiré" (section 12).
    #[error("En carence, couverture à partir du {debut_couverture}.")]
    EnCarence { debut_couverture: NaiveDate },

    #[error("Contrat expiré.")]
    ContratExpire,

    /// Une période payée et pas encore commencée existe déjà : au plus une
    /// période d'avance (décision C2). Couvre aussi un renouvellement tenté
    /// pendant une carence.
    #[error("Une période future est déjà payée.")]
    PeriodeFutureDejaPayee,

    #[error("NIP invalide.")]
    NipInvalide,

    /// NIP inconnu **ou** date de naissance différente : jamais distingués
    /// (voir `adherer`).
    #[error("Aucun patient ne correspond à ce NIP et à cette date de naissance.")]
    PatientInconnu,

    #[error("Cette référence de transaction est déjà utilisée pour une autre cotisation.")]
    ReferenceTransactionDejaUtilisee,

    /// Trop d'échecs d'adhésion récents pour cet agent
    /// (`ECHECS_ADHESION_MAX` dans la fenêtre) ; `reessayer_dans_secs` :
    /// en-tête `Retry-After` de la réponse 429 (RFC 6585 §4).
    #[error("Trop d'essais sans résultat. Réessayez plus tard.")]
    TropDEchecs { reessayer_dans_secs: u64 },

    /// Le message ne cite que le nom du champ, jamais la valeur saisie.
    #[error("{0}")]
    DonneesInvalides(String),

    #[error("erreur interne : {0}")]
    Interne(String),
}

type Erreur = ErreurContratAssuranceMunaseb;

impl From<sqlx::Error> for ErreurContratAssuranceMunaseb {
    fn from(erreur: sqlx::Error) -> Self {
        ErreurContratAssuranceMunaseb::Interne(erreur.to_string())
    }
}

/// Erreur d'écriture : une contrainte d'unicité violée par une requête
/// simultanée (les contrôles préalables ne suffisent pas seuls) devient
/// l'erreur métier qu'aurait donnée le contrôle. Noms des contraintes :
/// migrations 0006 et 0028 (doc sqlx 0.9, `DatabaseError::constraint`).
fn erreur_ecriture(erreur: sqlx::Error) -> Erreur {
    if let sqlx::Error::Database(e) = &erreur {
        match e.constraint() {
            Some("contrat_assurance_munaseb_patient_id_key") => return Erreur::ContratDejaExistant,
            Some("contrat_assurance_munaseb_numero_carte_key" | "periode_numero_carte_unique") => {
                return Erreur::NumeroCarteDejaUtilise;
            }
            Some("paiement_reference_transaction_unique") => {
                return Erreur::ReferenceTransactionDejaUtilisee;
            }
            _ => {}
        }
    }
    erreur.into()
}

// ---------------------------------------------------------------------
// Paramètres de la MUNASEB (migration 0028)
// ---------------------------------------------------------------------

/// Valeurs modifiables par les agents, en base, jamais en constante (modèle
/// retenu pour l'étape 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct Parametres {
    pub cotisation_fcfa: i32,
    pub fiche_fcfa: i32,
    pub delai_grace_jours: i16,
    pub carence_mois: i16,
    pub plafond_periode_fcfa: i32,
}

impl Parametres {
    /// Mêmes bornes que les CHECK de la migration 0028.
    fn controler(&self) -> Result<(), Erreur> {
        let refus = |message: &str| Err(Erreur::DonneesInvalides(message.to_string()));
        if self.cotisation_fcfa < 0 {
            return refus("cotisation_fcfa : doit être positive ou nulle.");
        }
        if self.fiche_fcfa < 0 {
            return refus("fiche_fcfa : doit être positive ou nulle.");
        }
        if !(0..=DELAI_GRACE_MAX_JOURS).contains(&self.delai_grace_jours) {
            return refus("delai_grace_jours : doit être compris entre 0 et 60.");
        }
        if !(0..=CARENCE_MAX_MOIS).contains(&self.carence_mois) {
            return refus("carence_mois : doit être compris entre 0 et 12.");
        }
        if self.plafond_periode_fcfa <= 0 {
            return refus("plafond_periode_fcfa : doit être strictement positif.");
        }
        Ok(())
    }

    fn regles(&self) -> ReglesPeriode {
        // Bornes garanties par la base (CHECK 0 à 12 et 0 à 60) : la
        // conversion ne perd rien ; `max(0)` la garde sûre dans tous les cas.
        ReglesPeriode {
            carence: Months::new(self.carence_mois.max(0) as u32),
            delai_grace: Days::new(self.delai_grace_jours.max(0) as u64),
        }
    }
}

/// Une version des paramètres, avec son auteur (absent pour les valeurs
/// initiales de la migration).
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct VersionParametres {
    pub id: Uuid,
    #[serde(flatten)]
    #[sqlx(flatten)]
    pub valeurs: Parametres,
    pub date_effet: DateTime<Utc>,
    /// Le responsable de la MUNASEB qui a posé cette version (migration
    /// 0029) ; absent pour les valeurs initiales.
    pub auteur_nom: Option<String>,
    pub auteur_prenom: Option<String>,
}

// Macro plutôt que `const` + `format!` : sqlx 0.9 n'accepte que des
// chaînes SQL littérales (même raison que `select_resume!`).
macro_rules! select_version {
    () => {
        "SELECT pm.id, pm.cotisation_fcfa, pm.fiche_fcfa, pm.delai_grace_jours, pm.carence_mois, \
         pm.plafond_periode_fcfa, pm.date_effet, u.nom AS auteur_nom, u.prenom AS auteur_prenom \
         FROM parametre_munaseb pm \
         LEFT JOIN affectation a ON a.id = pm.auteur_affectation_id \
         LEFT JOIN compte ca ON ca.id = a.compte_id \
         LEFT JOIN utilisateur u ON u.id = ca.utilisateur_id"
    };
}

/// Paramètres en vigueur : la dernière version (même ordre que la fonction
/// `plafond_munaseb_en_vigueur` de la migration 0028).
pub async fn parametres_en_vigueur<'e, E>(executeur: E) -> Result<VersionParametres, sqlx::Error>
where
    E: sqlx::PgExecutor<'e>,
{
    sqlx::query_as(concat!(
        select_version!(),
        " ORDER BY pm.date_effet DESC, pm.id DESC LIMIT 1"
    ))
    .fetch_one(executeur)
    .await
}

/// Nouvelle version des paramètres, enregistrée avec l'affectation de son
/// auteur, le **responsable** de la structure MUNASEB (décision du porteur
/// du 2026-11-16 : ces valeurs touchent l'argent de toutes les adhésions
/// suivantes ; les agents ne font que les lire). La base n'accepte que ce
/// rôle comme auteur (migration 0029) ; l'appelant passe l'affectation que
/// l'extracteur `ResponsableMunaseb` a contrôlée. Les versions précédentes
/// ne changent jamais (triggers de la
/// 0028) : l'historique donne l'ancienne et la nouvelle valeur de chaque
/// modification. Des valeurs identiques aux valeurs en vigueur n'ajoutent
/// aucune version (renvoyer deux fois la même requête ne crée pas de bruit
/// dans l'historique). Deux modifications simultanées donnent deux
/// versions, la dernière s'applique : aucune n'est perdue de l'historique.
///
/// Un changement ne touche que les **périodes suivantes** (plafond figé
/// sur chaque période) et les **paiements suivants** (montants figés sur
/// chaque paiement).
pub async fn modifier_parametres(
    pool: &PgPool,
    responsable: AffectationAgent,
    nouveaux: Parametres,
) -> Result<VersionParametres, Erreur> {
    nouveaux.controler()?;
    let actuels = parametres_en_vigueur(pool).await?;
    if actuels.valeurs == nouveaux {
        return Ok(actuels);
    }
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO parametre_munaseb \
         (cotisation_fcfa, fiche_fcfa, delai_grace_jours, carence_mois, plafond_periode_fcfa, \
          auteur_affectation_id, auteur_structure_id, auteur_role) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'responsable') RETURNING id",
    )
    .bind(nouveaux.cotisation_fcfa)
    .bind(nouveaux.fiche_fcfa)
    .bind(nouveaux.delai_grace_jours)
    .bind(nouveaux.carence_mois)
    .bind(nouveaux.plafond_periode_fcfa)
    .bind(responsable.affectation_id)
    .bind(responsable.structure_id)
    .fetch_one(pool)
    .await?;
    Ok(
        sqlx::query_as(concat!(select_version!(), " WHERE pm.id = $1"))
            .bind(id)
            .fetch_one(pool)
            .await?,
    )
}

/// Versions des paramètres, la plus récente d'abord, par pages (`avant` :
/// identifiant de la dernière version de la page précédente).
pub async fn historique_parametres(
    pool: &PgPool,
    avant: Option<Uuid>,
    limite: i64,
) -> Result<Vec<VersionParametres>, sqlx::Error> {
    sqlx::query_as(concat!(
        select_version!(),
        " WHERE $1::uuid IS NULL \
           OR (pm.date_effet, pm.id) < (SELECT date_effet, id FROM parametre_munaseb WHERE id = $1) \
         ORDER BY pm.date_effet DESC, pm.id DESC LIMIT $2"
    ))
    .bind(avant)
    .bind(limite)
    .fetch_all(pool)
    .await
}

// ---------------------------------------------------------------------
// Solde d'une période
// ---------------------------------------------------------------------

/// Droits d'un patient sous contrat MUNASEB, tels que visibles par le
/// patient.
#[derive(Debug, PartialEq)]
pub struct DroitsContrat {
    pub plafond_annuel_fcfa: i32,
    pub plafond_restant_fcfa: i32,
}

/// Plafond, consommation et solde d'une période (décision E4 du
/// 2026-10-21) : le seul calcul du solde, pour la validation (sous verrou),
/// le détail vu par l'agent et les droits vus par le patient.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SoldePeriode {
    pub plafond_fcfa: i32,
    pub consomme_fcfa: i32,
    pub solde_fcfa: i32,
}

/// Solde d'une période : son plafond (figé à sa création, migration 0028)
/// moins la somme des montants Validé et Payé (décisions actées, section
/// 12). Chaque demande est rattachée à sa période dès sa création : aucun
/// filtre de dates n'est nécessaire.
pub async fn solde_periode<'e, E>(
    executeur: E,
    periode_id: Uuid,
) -> Result<SoldePeriode, sqlx::Error>
where
    E: sqlx::PgExecutor<'e>,
{
    let (plafond, consomme): (i32, i64) = sqlx::query_as(
        "SELECT pe.plafond_fcfa, \
                (SELECT COALESCE(SUM(d.montant_rembourse_fcfa), 0)::BIGINT FROM demande_remboursement_munaseb d \
                 WHERE d.periode_id = pe.id AND d.statut IN ('valide', 'paye')) \
         FROM periode_adhesion_munaseb pe WHERE pe.id = $1",
    )
    .bind(periode_id)
    .fetch_one(executeur)
    .await?;
    // Le plafonnement au passage à Validé empêche tout dépassement : la
    // consommation d'une période reste <= son plafond, qui tient dans un
    // i32. `max(0)` et `min` gardent les conversions sûres dans tous les cas.
    let solde = (i64::from(plafond) - consomme).max(0) as i32;
    Ok(SoldePeriode {
        plafond_fcfa: plafond,
        consomme_fcfa: consomme.min(i64::from(i32::MAX)) as i32,
        solde_fcfa: solde,
    })
}

/// Date du jour au Burkina, lue dans PostgreSQL (décision D9 : une seule
/// horloge, `AT TIME ZONE 'Africa/Ouagadougou'`, doc PostgreSQL 18
/// §9.9.4). Sert de date de paiement (§12 : jamais saisie par l'appelant)
/// et aux lectures des droits (décision E4).
pub(crate) async fn aujourdhui_au_burkina<'e, E>(executeur: E) -> Result<NaiveDate, sqlx::Error>
where
    E: sqlx::PgExecutor<'e>,
{
    sqlx::query_scalar("SELECT (now() AT TIME ZONE 'Africa/Ouagadougou')::date")
        .fetch_one(executeur)
        .await
}

// ---------------------------------------------------------------------
// Calcul d'une période
// ---------------------------------------------------------------------

/// Paramètres dont dépend le début d'une période.
#[derive(Debug, Clone, Copy)]
pub struct ReglesPeriode {
    pub carence: Months,
    pub delai_grace: Days,
}

/// Dates (début, fin) de la prochaine période, d'après la dernière période
/// du contrat (`None` pour une première adhésion) et la date de paiement.
/// Fonction pure : toutes les règles de début et de fin de période sont
/// ici, testées sans base. Règles du modèle retenu pour l'étape 5 (réponses
/// 1 et 2 du porteur, 2026-10-22, à confirmer par la MUNASEB) :
/// - **nouvelle adhésion** (première, ou au-delà du délai de grâce) :
///   début = paiement + carence ; fin = paiement + 12 mois (jour
///   anniversaire compris) ;
/// - **renouvellement dans les temps** (payé au plus tard le dernier jour) :
///   début = lendemain de la fin, sans carence ; fin = fin + 12 mois ;
/// - **renouvellement dans le délai de grâce** (option A) : début = jour du
///   paiement, sans carence ; les jours d'interruption ne sont **pas**
///   couverts (aucun effet rétroactif, §12) ; fin = fin précédente +
///   12 mois (date anniversaire suivante) ;
/// - au plus une période d'avance (décision C2).
///
/// Ajout de mois : `NaiveDate::checked_add_months` (doc chrono 0.4) ramène
/// au dernier jour du mois quand le jour d'origine n'existe pas (le 29
/// février plus 12 mois donne le 28 février) et ne renvoie `None` qu'hors
/// de la plage de dates valides.
pub fn calculer_periode(
    derniere: Option<(NaiveDate, NaiveDate)>,
    date_paiement: NaiveDate,
    regles: ReglesPeriode,
) -> Result<(NaiveDate, NaiveDate), Erreur> {
    let hors_plage = || Erreur::Interne("date hors de la plage valide".into());
    let nouvelle_adhesion = || -> Result<(NaiveDate, NaiveDate), Erreur> {
        let debut = date_paiement
            .checked_add_months(regles.carence)
            .ok_or_else(hors_plage)?;
        let fin = date_paiement
            .checked_add_months(DUREE)
            .ok_or_else(hors_plage)?;
        Ok((debut, fin))
    };

    match derniere {
        None => nouvelle_adhesion(),
        Some((debut_derniere, _)) if debut_derniere > date_paiement => {
            Err(Erreur::PeriodeFutureDejaPayee)
        }
        Some((_, fin_derniere)) if date_paiement <= fin_derniere => Ok((
            fin_derniere
                .checked_add_days(Days::new(1))
                .ok_or_else(hors_plage)?,
            fin_derniere
                .checked_add_months(DUREE)
                .ok_or_else(hors_plage)?,
        )),
        Some((_, fin_derniere))
            if date_paiement
                <= fin_derniere
                    .checked_add_days(regles.delai_grace)
                    .ok_or_else(hors_plage)? =>
        {
            Ok((
                date_paiement,
                fin_derniere
                    .checked_add_months(DUREE)
                    .ok_or_else(hors_plage)?,
            ))
        }
        Some(_) => nouvelle_adhesion(),
    }
}

// ---------------------------------------------------------------------
// Saisie de l'agent
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModePaiement {
    Especes,
    MobileMoney,
}

impl ModePaiement {
    fn as_str(self) -> &'static str {
        match self {
            ModePaiement::Especes => "especes",
            ModePaiement::MobileMoney => "mobile_money",
        }
    }
}

/// Paiement de la cotisation, tel que l'agent le saisit. Les montants ne
/// sont pas saisis : ce sont ceux des paramètres en vigueur.
#[derive(Debug, Clone, Deserialize)]
pub struct NouveauPaiement {
    pub mode: ModePaiement,
    /// Obligatoire pour le mobile money, refusée pour les espèces.
    pub reference_transaction: Option<String>,
    /// Pour information seulement, jamais après aujourd'hui.
    pub date_declaree: Option<NaiveDate>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Adhesion {
    pub nip: String,
    pub date_naissance: NaiveDate,
    pub numero_carte: String,
    pub ufr: String,
    pub universite: String,
    pub num_matricule: String,
    pub personne_a_prevenir: Option<String>,
    pub niveau_etudes: Option<String>,
    pub antenne: Option<String>,
    pub paiement: NouveauPaiement,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Renouvellement {
    pub numero_carte: String,
    pub niveau_etudes: Option<String>,
    pub antenne: Option<String>,
    pub paiement: NouveauPaiement,
}

/// Texte obligatoire : espaces de début et de fin retirées, non vide,
/// `max` caractères au plus, sans caractère de contrôle.
fn texte(valeur: &str, champ: &str, max: usize) -> Result<String, Erreur> {
    let valeur = valeur.trim();
    if valeur.is_empty() {
        return Err(Erreur::DonneesInvalides(format!("{champ} : obligatoire.")));
    }
    if valeur.chars().count() > max {
        return Err(Erreur::DonneesInvalides(format!(
            "{champ} : {max} caractères au plus."
        )));
    }
    if valeur.chars().any(char::is_control) {
        return Err(Erreur::DonneesInvalides(format!(
            "{champ} : caractère de contrôle refusé."
        )));
    }
    Ok(valeur.to_string())
}

/// Texte facultatif : absent ou vide après retrait des espaces = absent.
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

/// Paiement contrôlé, prêt à écrire.
struct PaiementControle {
    mode: ModePaiement,
    reference_transaction: Option<String>,
    date_declaree: Option<NaiveDate>,
}

fn controler_paiement(paiement: &NouveauPaiement) -> Result<PaiementControle, Erreur> {
    let reference = texte_facultatif(
        paiement.reference_transaction.as_deref(),
        "paiement.reference_transaction",
        MAX_TEXTE_COURT,
    )?;
    match (paiement.mode, &reference) {
        (ModePaiement::MobileMoney, None) => Err(Erreur::DonneesInvalides(
            "paiement.reference_transaction : obligatoire pour le mobile money.".into(),
        )),
        (ModePaiement::Especes, Some(_)) => Err(Erreur::DonneesInvalides(
            "paiement.reference_transaction : seulement pour le mobile money.".into(),
        )),
        _ => Ok(PaiementControle {
            mode: paiement.mode,
            reference_transaction: reference,
            date_declaree: paiement.date_declaree,
        }),
    }
}

/// Champs propres à une période.
struct PeriodeControlee {
    numero_carte: String,
    niveau_etudes: Option<String>,
    antenne: Option<String>,
    paiement: PaiementControle,
}

fn controler_periode(
    numero_carte: &str,
    niveau_etudes: Option<&str>,
    antenne: Option<&str>,
    paiement: &NouveauPaiement,
) -> Result<PeriodeControlee, Erreur> {
    Ok(PeriodeControlee {
        numero_carte: texte(numero_carte, "numero_carte", MAX_NUMERO_CARTE)?,
        niveau_etudes: texte_facultatif(niveau_etudes, "niveau_etudes", MAX_TEXTE_COURT)?,
        antenne: texte_facultatif(antenne, "antenne", MAX_TEXTE_COURT)?,
        paiement: controler_paiement(paiement)?,
    })
}

// ---------------------------------------------------------------------
// Adhésion et renouvellement
// ---------------------------------------------------------------------

/// Contrôles et écritures communs à l'adhésion et au renouvellement, dans
/// la transaction de l'appelant : numéro de carte et référence libres, date
/// déclarée passée, période, paiement. Renvoie l'identifiant de la période.
async fn enregistrer_periode(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    agent: AffectationAgent,
    contrat_id: Uuid,
    derniere: Option<(NaiveDate, NaiveDate)>,
    periode: &PeriodeControlee,
) -> Result<Uuid, Erreur> {
    let parametres = parametres_en_vigueur(&mut **tx).await?.valeurs;
    // Date de paiement fixée par le système, jamais par l'appelant (§12 :
    // un paiement en retard antidaté ferait disparaître la carence) ; date
    // du Burkina lue dans PostgreSQL (D9).
    let date_paiement = aujourdhui_au_burkina(&mut **tx).await?;
    if periode
        .paiement
        .date_declaree
        .is_some_and(|d| d > date_paiement)
    {
        return Err(Erreur::DonneesInvalides(
            "paiement.date_declaree : ne peut pas être postérieure à aujourd'hui.".into(),
        ));
    }
    let (debut, fin) = calculer_periode(derniere, date_paiement, parametres.regles())?;

    // Contrôles préalables, pour une erreur nommée ; une requête simultanée
    // est rattrapée par les index uniques (`erreur_ecriture`).
    let carte_prise: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM periode_adhesion_munaseb WHERE numero_carte = $1) \
             OR EXISTS (SELECT 1 FROM contrat_assurance_munaseb WHERE numero_carte = $1 AND id <> $2)",
    )
    .bind(&periode.numero_carte)
    .bind(contrat_id)
    .fetch_one(&mut **tx)
    .await?;
    if carte_prise {
        return Err(Erreur::NumeroCarteDejaUtilise);
    }
    if let Some(reference) = &periode.paiement.reference_transaction {
        let reference_prise: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM paiement_cotisation_munaseb WHERE reference_transaction = $1)",
        )
        .bind(reference)
        .fetch_one(&mut **tx)
        .await?;
        if reference_prise {
            return Err(Erreur::ReferenceTransactionDejaUtilisee);
        }
    }

    let periode_id: Uuid = sqlx::query_scalar(
        "INSERT INTO periode_adhesion_munaseb \
         (contrat_id, date_paiement, date_debut, date_fin, plafond_fcfa, numero_carte, antenne, niveau_etudes) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING id",
    )
    .bind(contrat_id)
    .bind(date_paiement)
    .bind(debut)
    .bind(fin)
    .bind(parametres.plafond_periode_fcfa)
    .bind(&periode.numero_carte)
    .bind(&periode.antenne)
    .bind(&periode.niveau_etudes)
    .fetch_one(&mut **tx)
    .await
    .map_err(erreur_ecriture)?;

    sqlx::query(
        "INSERT INTO paiement_cotisation_munaseb \
         (periode_id, mode, montant_cotisation_fcfa, montant_fiche_fcfa, reference_transaction, date_declaree, \
          agent_affectation_id, agent_structure_id, agent_role) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'agent_assurance_munaseb')",
    )
    .bind(periode_id)
    .bind(periode.paiement.mode.as_str())
    .bind(parametres.cotisation_fcfa)
    .bind(parametres.fiche_fcfa)
    .bind(&periode.paiement.reference_transaction)
    .bind(periode.paiement.date_declaree)
    .bind(agent.affectation_id)
    .bind(agent.structure_id)
    .execute(&mut **tx)
    .await
    .map_err(erreur_ecriture)?;

    Ok(periode_id)
}

/// Adhésion d'un patient, enregistrée par un agent : contrat, première
/// période (avec carence) et paiement, dans une seule transaction. Renvoie
/// l'identifiant du contrat.
///
/// Le patient est désigné par son **NIP** (la clé d'interopérabilité de
/// LaafiCare, CLAUDE.md §5, montrée par l'application du patient) **et sa
/// date de naissance** (lue sur sa pièce d'identité). Un NIP inconnu et une
/// date différente donnent la même réponse : une faute de frappe qui
/// tomberait sur le NIP valide d'un autre patient ne lui crée pas de
/// contrat, et la route ne permet pas de vérifier si un NIP existe (OWASP
/// API3:2023, ne rien exposer de plus que nécessaire). Il n'existe aucune
/// recherche de patient par NIP pour l'agent.
///
/// **Limite d'échecs** (`ECHECS_ADHESION_MAX`, migration 0029) : chaque
/// couple NIP et date qui ne désigne aucun patient est tracé avec
/// l'affectation de l'agent ; à la limite, toute adhésion de cet agent est
/// refusée (`TropDEchecs`) **avant** la recherche du patient, même avec un
/// couple correct, jusqu'à ce que le plus ancien échec sorte de la fenêtre.
/// Un NIP mal formé (clé de Luhn) n'est pas compté : il est refusé sans
/// lire la base et n'apprend rien sur un patient. L'affectation est
/// verrouillée en tête de transaction (comme l'identité pour les codes SMS,
/// `otp::generer_et_enregistrer`) : des adhésions simultanées du même agent
/// passent l'une après l'autre et ne dépassent jamais la limite ensemble.
/// Ordre des verrous : l'affectation, puis les insertions ; aucune autre
/// opération ne verrouille une affectation puis un contrat (la
/// désactivation verrouille structure → affectation → sessions) : aucun
/// cycle n'est possible.
pub async fn adherer(
    pool: &PgPool,
    agent: AffectationAgent,
    adhesion: &Adhesion,
) -> Result<Uuid, Erreur> {
    // Contrôles sans base d'abord (même ordre que `creer_depuis_acte`).
    if !nip::valider(&adhesion.nip) {
        return Err(Erreur::NipInvalide);
    }
    let ufr = texte(&adhesion.ufr, "ufr", MAX_TEXTE_COURT)?;
    let universite = texte(&adhesion.universite, "universite", MAX_TEXTE_COURT)?;
    let num_matricule = texte(&adhesion.num_matricule, "num_matricule", MAX_MATRICULE)?;
    let personne_a_prevenir = texte_facultatif(
        adhesion.personne_a_prevenir.as_deref(),
        "personne_a_prevenir",
        MAX_PERSONNE_A_PREVENIR,
    )?;
    let periode = controler_periode(
        &adhesion.numero_carte,
        adhesion.niveau_etudes.as_deref(),
        adhesion.antenne.as_deref(),
        &adhesion.paiement,
    )?;

    let mut tx = pool.begin().await?;

    sqlx::query("SELECT id FROM affectation WHERE id = $1 FOR UPDATE")
        .bind(agent.affectation_id)
        .fetch_one(&mut *tx)
        .await?;
    // Échecs dans la fenêtre, et secondes avant que le plus ancien en sorte
    // (horloge de PostgreSQL, comme pour les codes SMS). `now()` est l'heure
    // de début de la transaction : la même pour le comptage et la trace.
    let (echecs, attente): (i64, Option<i64>) = sqlx::query_as(
        "SELECT count(*), \
                ceil(extract(epoch FROM min(date_echec) + make_interval(mins => $2) - now()))::bigint \
         FROM echec_adhesion_munaseb \
         WHERE agent_affectation_id = $1 AND date_echec > now() - make_interval(mins => $2)",
    )
    .bind(agent.affectation_id)
    .bind(FENETRE_ECHECS_ADHESION_MINUTES)
    .fetch_one(&mut *tx)
    .await?;
    if echecs >= ECHECS_ADHESION_MAX {
        // Au moins 1 s : un échec pile à la limite de la fenêtre compte encore.
        return Err(Erreur::TropDEchecs {
            reessayer_dans_secs: attente.unwrap_or(1).max(1) as u64,
        });
    }

    let patient_id: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM patient WHERE nip = $1 AND date_naissance = $2")
            .bind(&adhesion.nip)
            .bind(adhesion.date_naissance)
            .fetch_optional(&mut *tx)
            .await?;
    let Some(patient_id) = patient_id else {
        // L'échec est tracé et validé en base avant la réponse : sinon
        // l'annulation de la transaction l'effacerait et la limite ne
        // compterait rien.
        sqlx::query(
            "INSERT INTO echec_adhesion_munaseb (agent_affectation_id, agent_structure_id, agent_role) \
             VALUES ($1, $2, 'agent_assurance_munaseb')",
        )
        .bind(agent.affectation_id)
        .bind(agent.structure_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        return Err(Erreur::PatientInconnu);
    };

    let deja: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM contrat_assurance_munaseb WHERE patient_id = $1)",
    )
    .bind(patient_id)
    .fetch_one(&mut *tx)
    .await?;
    if deja {
        return Err(Erreur::ContratDejaExistant);
    }

    // Le numéro de la première carte reste aussi sur le contrat (colonne
    // NOT NULL UNIQUE de la 0006, lue pour les périodes d'avant la 0028).
    let carte_prise: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM periode_adhesion_munaseb WHERE numero_carte = $1) \
             OR EXISTS (SELECT 1 FROM contrat_assurance_munaseb WHERE numero_carte = $1)",
    )
    .bind(&periode.numero_carte)
    .fetch_one(&mut *tx)
    .await?;
    if carte_prise {
        return Err(Erreur::NumeroCarteDejaUtilise);
    }

    let contrat_id: Uuid = sqlx::query_scalar(
        "INSERT INTO contrat_assurance_munaseb \
         (patient_id, numero_carte, ufr, universite, num_matricule, personne_a_prevenir) \
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
    )
    .bind(patient_id)
    .bind(&periode.numero_carte)
    .bind(&ufr)
    .bind(&universite)
    .bind(&num_matricule)
    .bind(&personne_a_prevenir)
    .fetch_one(&mut *tx)
    .await
    .map_err(erreur_ecriture)?;

    enregistrer_periode(&mut tx, agent, contrat_id, None, &periode).await?;
    tx.commit().await?;
    Ok(contrat_id)
}

/// Renouvellement d'un contrat par un agent : nouvelle période (règles de
/// `calculer_periode`) et son paiement, sans jamais toucher aux
/// précédentes. Renvoie l'identifiant de la nouvelle période.
pub async fn renouveler(
    pool: &PgPool,
    agent: AffectationAgent,
    contrat_id: Uuid,
    renouvellement: &Renouvellement,
) -> Result<Uuid, Erreur> {
    let periode = controler_periode(
        &renouvellement.numero_carte,
        renouvellement.niveau_etudes.as_deref(),
        renouvellement.antenne.as_deref(),
        &renouvellement.paiement,
    )?;

    let mut tx = pool.begin().await?;

    // Verrou sur la ligne du contrat : deux renouvellements simultanés
    // liraient la même dernière période et calculeraient le même début.
    // La contrainte d'exclusion de la migration 0009 refuserait le second,
    // mais sous forme d'erreur SQL brute ; le verrou les fait passer l'un
    // après l'autre.
    let statut: Option<String> =
        sqlx::query_scalar("SELECT statut FROM contrat_assurance_munaseb WHERE id = $1 FOR UPDATE")
            .bind(contrat_id)
            .fetch_optional(&mut *tx)
            .await?;
    match statut.as_deref() {
        None => return Err(Erreur::AucunContrat),
        Some("actif") => {}
        // Règles de suspension inconnues (à demander à la MUNASEB) : on
        // bloque plutôt que de deviner si un renouvellement lève la
        // suspension.
        Some(_) => return Err(Erreur::ContratSuspendu),
    }

    // Les périodes d'un contrat ne se chevauchent pas : la dernière est
    // celle qui finit le plus tard.
    let derniere: Option<(NaiveDate, NaiveDate)> = sqlx::query_as(
        "SELECT date_debut, date_fin FROM periode_adhesion_munaseb \
         WHERE contrat_id = $1 ORDER BY date_fin DESC LIMIT 1",
    )
    .bind(contrat_id)
    .fetch_optional(&mut *tx)
    .await?;

    let periode_id = enregistrer_periode(&mut tx, agent, contrat_id, derniere, &periode).await?;
    tx.commit().await?;
    Ok(periode_id)
}

// ---------------------------------------------------------------------
// Lectures de l'agent
// ---------------------------------------------------------------------

/// Paiement d'une période, tel que l'agent le voit.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct PaiementCotisation {
    pub mode: String,
    pub montant_cotisation_fcfa: i32,
    pub montant_fiche_fcfa: i32,
    pub reference_transaction: Option<String>,
    pub date_declaree: Option<NaiveDate>,
    pub date_enregistrement: DateTime<Utc>,
    pub agent_nom: Option<String>,
    pub agent_prenom: Option<String>,
}

#[derive(sqlx::FromRow)]
struct LignePeriode {
    id: Uuid,
    numero_carte: Option<String>,
    date_paiement: NaiveDate,
    date_debut: NaiveDate,
    date_fin: NaiveDate,
    antenne: Option<String>,
    niveau_etudes: Option<String>,
}

/// Une période, avec son solde (indicatif, sans verrou : la validation le
/// recalcule sous verrou) et son paiement (absent pour une période d'avant
/// la migration 0028).
#[derive(Debug, Serialize)]
pub struct PeriodeContrat {
    pub id: Uuid,
    /// Absent pour une période d'avant la 0028 autre que la première.
    pub numero_carte: Option<String>,
    pub date_paiement: NaiveDate,
    pub debut: NaiveDate,
    pub fin: NaiveDate,
    pub antenne: Option<String>,
    pub niveau_etudes: Option<String>,
    pub en_cours: bool,
    #[serde(flatten)]
    pub solde: SoldePeriode,
    pub paiement: Option<PaiementCotisation>,
}

/// Contrat tel que l'agent le voit : nom et prénom du patient, **jamais son
/// NIP**. Le **téléphone** n'est rendu que par la route du détail
/// (`GET /api/assurance-munaseb/contrats/{id}`), et chaque affichage est
/// écrit dans le journal des accès (décision du porteur du 2026-11-17, qui
/// remplace « jamais son téléphone » ; `acces_patient::telephone_pour_agent`).
#[derive(Debug, Serialize)]
pub struct DetailContrat {
    pub id: Uuid,
    pub patient_nom: Option<String>,
    pub patient_prenom: Option<String>,
    /// Absent de la réponse sauf dans le détail du contrat (champ ajouté :
    /// les réponses existantes ne changent pas).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telephone: Option<String>,
    pub ufr: String,
    pub universite: String,
    pub num_matricule: String,
    pub personne_a_prevenir: Option<String>,
    /// `actif` ou `suspendu`.
    pub statut: String,
    /// Même état que celui que voit le patient (`couvert`, `en_carence`,
    /// `expire`, `suspendu`).
    pub etat: EtatDroits,
    pub date_creation: DateTime<Utc>,
    /// La plus récente d'abord.
    pub periodes: Vec<PeriodeContrat>,
}

/// Détail d'un contrat, `None` s'il n'existe pas.
pub async fn detail_contrat(
    pool: &PgPool,
    contrat_id: Uuid,
) -> Result<Option<DetailContrat>, sqlx::Error> {
    type LigneContrat = (
        Option<String>,
        Option<String>,
        String,
        String,
        String,
        Option<String>,
        String,
        DateTime<Utc>,
    );
    let contrat: Option<LigneContrat> = sqlx::query_as(
        "SELECT u.nom, u.prenom, c.ufr, c.universite, c.num_matricule, c.personne_a_prevenir, c.statut, c.date_creation \
         FROM contrat_assurance_munaseb c \
         JOIN patient p ON p.id = c.patient_id \
         JOIN utilisateur u ON u.id = p.utilisateur_id \
         WHERE c.id = $1",
    )
    .bind(contrat_id)
    .fetch_optional(pool)
    .await?;
    let Some((
        patient_nom,
        patient_prenom,
        ufr,
        universite,
        num_matricule,
        personne_a_prevenir,
        statut,
        date_creation,
    )) = contrat
    else {
        return Ok(None);
    };

    let jour = aujourdhui_au_burkina(pool).await?;
    let lignes: Vec<LignePeriode> = sqlx::query_as(
        "SELECT id, numero_carte, date_paiement, date_debut, date_fin, antenne, niveau_etudes \
         FROM periode_adhesion_munaseb WHERE contrat_id = $1 ORDER BY date_debut DESC, id DESC",
    )
    .bind(contrat_id)
    .fetch_all(pool)
    .await?;

    let mut periodes = Vec::with_capacity(lignes.len());
    for ligne in lignes {
        // L'agent d'un paiement se lit par son affectation (comme J8).
        let paiement: Option<PaiementCotisation> = sqlx::query_as(
            "SELECT pc.mode, pc.montant_cotisation_fcfa, pc.montant_fiche_fcfa, pc.reference_transaction, \
                    pc.date_declaree, pc.date_enregistrement, u.nom AS agent_nom, u.prenom AS agent_prenom \
             FROM paiement_cotisation_munaseb pc \
             JOIN affectation a ON a.id = pc.agent_affectation_id \
             JOIN compte ca ON ca.id = a.compte_id \
             JOIN utilisateur u ON u.id = ca.utilisateur_id \
             WHERE pc.periode_id = $1",
        )
        .bind(ligne.id)
        .fetch_optional(pool)
        .await?;
        periodes.push(PeriodeContrat {
            id: ligne.id,
            numero_carte: ligne.numero_carte,
            date_paiement: ligne.date_paiement,
            debut: ligne.date_debut,
            fin: ligne.date_fin,
            antenne: ligne.antenne,
            niveau_etudes: ligne.niveau_etudes,
            en_cours: (ligne.date_debut..=ligne.date_fin).contains(&jour),
            solde: solde_periode(pool, ligne.id).await?,
            paiement,
        });
    }

    let etat = if statut != "actif" {
        EtatDroits::Suspendu
    } else if periodes.iter().any(|p| p.en_cours) {
        EtatDroits::Couvert
    } else if periodes.iter().any(|p| p.debut > jour) {
        EtatDroits::EnCarence
    } else {
        EtatDroits::Expire
    };

    Ok(Some(DetailContrat {
        id: contrat_id,
        patient_nom,
        patient_prenom,
        telephone: None,
        ufr,
        universite,
        num_matricule,
        personne_a_prevenir,
        statut,
        etat,
        date_creation,
        periodes,
    }))
}

/// Contrat qui porte ce numéro de carte (celle d'une période, ou la carte
/// d'origine du contrat), `None` sinon. C'est la seule recherche de
/// l'agent : le numéro est imprimé sur la carte que l'adhérent présente.
pub async fn contrat_par_carte(
    pool: &PgPool,
    numero_carte: &str,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT id FROM contrat_assurance_munaseb WHERE numero_carte = $1 \
         UNION SELECT contrat_id FROM periode_adhesion_munaseb WHERE numero_carte = $1 \
         LIMIT 1",
    )
    .bind(numero_carte.trim())
    .fetch_optional(pool)
    .await
}

// ---------------------------------------------------------------------
// Droits (service)
// ---------------------------------------------------------------------

/// Vérifie les droits d'un patient : contrat actif et période en cours,
/// avec le plafond restant de cette période.
pub async fn verifier_droits(pool: &PgPool, patient_id: Uuid) -> Result<DroitsContrat, Erreur> {
    let contrat: Option<(Uuid, String)> =
        sqlx::query_as("SELECT id, statut FROM contrat_assurance_munaseb WHERE patient_id = $1")
            .bind(patient_id)
            .fetch_optional(pool)
            .await?;
    let (contrat_id, statut) = contrat.ok_or(Erreur::AucunContrat)?;

    if statut != "actif" {
        return Err(Erreur::ContratSuspendu);
    }

    let jour = aujourdhui_au_burkina(pool).await?;

    let periode_en_cours: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM periode_adhesion_munaseb \
         WHERE contrat_id = $1 AND $2 BETWEEN date_debut AND date_fin",
    )
    .bind(contrat_id)
    .bind(jour)
    .fetch_optional(pool)
    .await?;

    let Some(periode_id) = periode_en_cours else {
        // Pas de période en cours : soit une période payée commence plus
        // tard (carence), soit tout est expiré.
        let prochain_debut: Option<NaiveDate> = sqlx::query_scalar(
            "SELECT min(date_debut) FROM periode_adhesion_munaseb WHERE contrat_id = $1 AND date_debut > $2",
        )
        .bind(contrat_id)
        .bind(jour)
        .fetch_one(pool)
        .await?;
        return Err(match prochain_debut {
            Some(debut_couverture) => Erreur::EnCarence { debut_couverture },
            None => Erreur::ContratExpire,
        });
    };

    let solde = solde_periode(pool, periode_id).await?;
    Ok(DroitsContrat {
        plafond_annuel_fcfa: solde.plafond_fcfa,
        plafond_restant_fcfa: solde.solde_fcfa,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(annee: i32, mois: u32, jour: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(annee, mois, jour).unwrap()
    }

    /// Valeurs initiales de la migration 0028 (carence 1 mois, grâce 10 j).
    fn regles() -> ReglesPeriode {
        ReglesPeriode {
            carence: Months::new(1),
            delai_grace: Days::new(10),
        }
    }

    #[test]
    fn premiere_adhesion_carence_d_un_mois_fin_au_jour_anniversaire() {
        // Exemple du porteur (réponse 1) : adhésion le 09/04/2024, effet le
        // 09/05/2024, fin le 09/04/2025 compris.
        assert_eq!(
            calculer_periode(None, d(2024, 4, 9), regles()).unwrap(),
            (d(2024, 5, 9), d(2025, 4, 9))
        );
    }

    #[test]
    fn paiement_un_31_janvier_ramene_au_dernier_jour_de_fevrier() {
        assert_eq!(
            calculer_periode(None, d(2026, 1, 31), regles()).unwrap(),
            (d(2026, 2, 28), d(2027, 1, 31))
        );
        // Année bissextile : 29 février.
        assert_eq!(
            calculer_periode(None, d(2028, 1, 31), regles()).unwrap(),
            (d(2028, 2, 29), d(2029, 1, 31))
        );
    }

    #[test]
    fn adhesion_un_29_fevrier() {
        // 29/02/2028 + 12 mois = 28/02/2029 (ramené au dernier jour).
        let (debut, fin) = calculer_periode(None, d(2028, 2, 29), regles()).unwrap();
        assert_eq!((debut, fin), (d(2028, 3, 29), d(2029, 2, 28)));
        // Renouvellement dans les temps : le lendemain, sans jour perdu.
        assert_eq!(
            calculer_periode(Some((debut, fin)), d(2029, 2, 1), regles()).unwrap(),
            (d(2029, 3, 1), d(2030, 2, 28))
        );
    }

    #[test]
    fn renouvellement_dans_les_temps_sans_trou_ni_carence() {
        let en_cours = Some((d(2024, 5, 9), d(2025, 4, 9)));
        let suivante = (d(2025, 4, 10), d(2026, 4, 9));
        // Payé le dernier jour de la période.
        assert_eq!(
            calculer_periode(en_cours, d(2025, 4, 9), regles()).unwrap(),
            suivante
        );
        // Payé longtemps avant la fin : même période suivante.
        assert_eq!(
            calculer_periode(en_cours, d(2024, 6, 1), regles()).unwrap(),
            suivante
        );
    }

    #[test]
    fn renouvellement_dans_le_delai_de_grace_option_a() {
        let expiree = Some((d(2024, 5, 9), d(2025, 4, 9)));
        // Payé le lendemain de la fin : couvert à partir du jour du
        // paiement, fin à la date anniversaire suivante.
        assert_eq!(
            calculer_periode(expiree, d(2025, 4, 10), regles()).unwrap(),
            (d(2025, 4, 10), d(2026, 4, 9))
        );
        // Payé le 10e jour après la fin : encore dans le délai ; les jours
        // du 10/04 au 18/04 ne sont pas couverts.
        assert_eq!(
            calculer_periode(expiree, d(2025, 4, 19), regles()).unwrap(),
            (d(2025, 4, 19), d(2026, 4, 9))
        );
    }

    #[test]
    fn renouvellement_au_dela_du_delai_de_grace_nouvelle_carence() {
        let expiree = Some((d(2024, 5, 9), d(2025, 4, 9)));
        // 11e jour après la fin : nouvelle adhésion, avec carence.
        assert_eq!(
            calculer_periode(expiree, d(2025, 4, 20), regles()).unwrap(),
            (d(2025, 5, 20), d(2026, 4, 20))
        );
    }

    #[test]
    fn delai_de_grace_nul_et_carence_nulle() {
        let sans = ReglesPeriode {
            carence: Months::new(0),
            delai_grace: Days::new(0),
        };
        assert_eq!(
            calculer_periode(None, d(2026, 3, 15), sans).unwrap(),
            (d(2026, 3, 15), d(2027, 3, 15))
        );
        // Sans délai de grâce, le lendemain de la fin est déjà en retard.
        let expiree = Some((d(2026, 3, 15), d(2027, 3, 15)));
        assert_eq!(
            calculer_periode(expiree, d(2027, 3, 16), sans).unwrap(),
            (d(2027, 3, 16), d(2028, 3, 16))
        );
    }

    #[test]
    fn refus_d_une_deuxieme_periode_d_avance() {
        // La dernière période n'a pas encore commencé à la date du paiement
        // (période suivante déjà payée, ou carence en cours).
        let future = Some((d(2027, 4, 15), d(2028, 4, 14)));
        assert!(matches!(
            calculer_periode(future, d(2027, 1, 10), regles()),
            Err(ErreurContratAssuranceMunaseb::PeriodeFutureDejaPayee)
        ));
    }

    #[test]
    fn bornes_des_parametres() {
        let bons = Parametres {
            cotisation_fcfa: 5000,
            fiche_fcfa: 100,
            delai_grace_jours: 10,
            carence_mois: 1,
            plafond_periode_fcfa: 100_000,
        };
        assert!(bons.controler().is_ok());
        for mauvais in [
            Parametres {
                cotisation_fcfa: -1,
                ..bons
            },
            Parametres {
                fiche_fcfa: -1,
                ..bons
            },
            Parametres {
                delai_grace_jours: 61,
                ..bons
            },
            Parametres {
                delai_grace_jours: -1,
                ..bons
            },
            Parametres {
                carence_mois: 13,
                ..bons
            },
            Parametres {
                carence_mois: -1,
                ..bons
            },
            Parametres {
                plafond_periode_fcfa: 0,
                ..bons
            },
        ] {
            assert!(matches!(
                mauvais.controler(),
                Err(ErreurContratAssuranceMunaseb::DonneesInvalides(_))
            ));
        }
        // Valeurs limites permises.
        assert!(
            Parametres {
                cotisation_fcfa: 0,
                fiche_fcfa: 0,
                delai_grace_jours: 60,
                carence_mois: 12,
                ..bons
            }
            .controler()
            .is_ok()
        );
    }

    #[test]
    fn textes_saisis() {
        assert_eq!(
            texte("  0552/2024 ", "numero_carte", 50).unwrap(),
            "0552/2024"
        );
        assert!(
            matches!(texte("   ", "ufr", 100), Err(ErreurContratAssuranceMunaseb::DonneesInvalides(m)) if m.starts_with("ufr"))
        );
        assert!(
            texte(&"é".repeat(50), "numero_carte", 50).is_ok(),
            "longueur en caractères, pas en octets"
        );
        assert!(texte(&"x".repeat(51), "numero_carte", 50).is_err());
        assert!(texte("ligne\nautre", "ufr", 100).is_err());
        assert_eq!(texte_facultatif(Some("  "), "antenne", 100).unwrap(), None);
        assert_eq!(texte_facultatif(None, "antenne", 100).unwrap(), None);
        // Le message ne reprend jamais la valeur saisie.
        let Err(ErreurContratAssuranceMunaseb::DonneesInvalides(m)) =
            texte("secret\u{7}", "ufr", 100)
        else {
            panic!()
        };
        assert!(!m.contains("secret"));
    }

    #[test]
    fn reference_selon_le_mode() {
        let paiement = |mode, reference: Option<&str>| NouveauPaiement {
            mode,
            reference_transaction: reference.map(String::from),
            date_declaree: None,
        };
        assert!(controler_paiement(&paiement(ModePaiement::Especes, None)).is_ok());
        assert!(controler_paiement(&paiement(ModePaiement::MobileMoney, Some("OM-123"))).is_ok());
        assert!(controler_paiement(&paiement(ModePaiement::MobileMoney, None)).is_err());
        assert!(controler_paiement(&paiement(ModePaiement::MobileMoney, Some("  "))).is_err());
        assert!(controler_paiement(&paiement(ModePaiement::Especes, Some("OM-123"))).is_err());
    }

    // ------------------------------------------------------------------
    // En base (`#[sqlx::test]` : base temporaire par test, données fictives)
    // ------------------------------------------------------------------

    fn nip_valide(payload: &str) -> String {
        (0..=9)
            .map(|c| format!("{payload}{c}"))
            .find(|n| nip::valider(n))
            .unwrap()
    }

    /// Identité et dossier patient, né le 2000-01-01. Renvoie (patient, NIP).
    async fn patient(pool: &PgPool, telephone: &str, payload_nip: &str) -> (Uuid, String) {
        let nip = nip_valide(payload_nip);
        let u: Uuid =
            sqlx::query_scalar("INSERT INTO utilisateur (nom, prenom, telephone) VALUES ('OUEDRAOGO', 'Awa', $1) RETURNING id")
                .bind(telephone)
                .fetch_one(pool)
                .await
                .unwrap();
        let p: Uuid = sqlx::query_scalar(
            "INSERT INTO patient (utilisateur_id, nip, date_naissance, lieu_naissance) \
             VALUES ($1, $2, '2000-01-01', 'Ouagadougou') RETURNING id",
        )
        .bind(u)
        .bind(&nip)
        .fetch_one(pool)
        .await
        .unwrap();
        (p, nip)
    }

    /// Affectation d'agent dans une MUNASEB validée.
    async fn agent(pool: &PgPool, telephone: &str) -> AffectationAgent {
        let u: Uuid =
            sqlx::query_scalar("INSERT INTO utilisateur (nom, prenom, telephone) VALUES ('KABRE', 'Inoussa', $1) RETURNING id")
                .bind(telephone)
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
        AffectationAgent {
            affectation_id,
            structure_id,
        }
    }

    /// Affectation `responsable` dans la même structure qu'une affectation
    /// existante (la base n'admet qu'une MUNASEB validée), ou un second
    /// agent avec `role` = `agent_assurance_munaseb`.
    async fn collegue(
        pool: &PgPool,
        telephone: &str,
        de: AffectationAgent,
        role: &str,
    ) -> AffectationAgent {
        let u: Uuid =
            sqlx::query_scalar("INSERT INTO utilisateur (nom, prenom, telephone) VALUES ('ZONGO', 'Mariam', $1) RETURNING id")
                .bind(telephone)
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
        let affectation_id: Uuid =
            sqlx::query_scalar("INSERT INTO affectation (compte_id, structure_id, role) VALUES ($1, $2, $3) RETURNING id")
                .bind(compte)
                .bind(de.structure_id)
                .bind(role)
                .fetch_one(pool)
                .await
                .unwrap();
        AffectationAgent {
            affectation_id,
            structure_id: de.structure_id,
        }
    }

    fn especes() -> NouveauPaiement {
        NouveauPaiement {
            mode: ModePaiement::Especes,
            reference_transaction: None,
            date_declaree: None,
        }
    }

    fn mobile_money(reference: &str) -> NouveauPaiement {
        NouveauPaiement {
            mode: ModePaiement::MobileMoney,
            reference_transaction: Some(reference.to_string()),
            date_declaree: None,
        }
    }

    fn adhesion(nip: &str, carte: &str, paiement: NouveauPaiement) -> Adhesion {
        Adhesion {
            nip: nip.to_string(),
            date_naissance: d(2000, 1, 1),
            numero_carte: carte.to_string(),
            ufr: " UFR fictive ".into(),
            universite: "Université fictive".into(),
            num_matricule: "MAT-FICTIF".into(),
            personne_a_prevenir: Some("   ".into()),
            niveau_etudes: Some("Licence 2".into()),
            antenne: None,
            paiement,
        }
    }

    fn renouvellement(carte: &str, paiement: NouveauPaiement) -> Renouvellement {
        Renouvellement {
            numero_carte: carte.to_string(),
            niveau_etudes: None,
            antenne: Some("Bobo".into()),
            paiement,
        }
    }

    /// Contrat posé en SQL avec une période passée (dates que la route ne
    /// permet pas de créer), décalée en jours par rapport à aujourd'hui.
    async fn contrat_avec_periode(
        pool: &PgPool,
        patient_id: Uuid,
        carte: &str,
        debut: i32,
        fin: i32,
    ) -> Uuid {
        let c: Uuid = sqlx::query_scalar(
            "INSERT INTO contrat_assurance_munaseb (patient_id, numero_carte, ufr, universite, num_matricule) \
             VALUES ($1, $2, 'UFR', 'Université', 'MAT') RETURNING id",
        )
        .bind(patient_id)
        .bind(carte)
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO periode_adhesion_munaseb (contrat_id, date_paiement, date_debut, date_fin, numero_carte) \
             VALUES ($1, CURRENT_DATE + $2 - 30, CURRENT_DATE + $2, CURRENT_DATE + $3, $4)",
        )
        .bind(c)
        .bind(debut)
        .bind(fin)
        .bind(carte)
        .execute(pool)
        .await
        .unwrap();
        c
    }

    async fn compter(pool: &PgPool, table: &'static str) -> i64 {
        let requete = match table {
            "parametre_munaseb" => "SELECT count(*) FROM parametre_munaseb",
            "paiement_cotisation_munaseb" => "SELECT count(*) FROM paiement_cotisation_munaseb",
            "contrat_assurance_munaseb" => "SELECT count(*) FROM contrat_assurance_munaseb",
            _ => unreachable!(),
        };
        sqlx::query_scalar(requete).fetch_one(pool).await.unwrap()
    }

    /// Adhésion : NIP et date de naissance, refus sans écriture, première
    /// période avec carence et fin au jour anniversaire, paiement figé avec
    /// l'agent, détail sans NIP ni téléphone, recherche par carte.
    #[sqlx::test]
    #[ignore]
    async fn adhesion_et_detail(pool: PgPool) {
        let ag = agent(&pool, "+22670400001").await;
        let (_, nip) = patient(&pool, "+22670400002", "900000000040002").await;
        let jour = aujourdhui_au_burkina(&pool).await.unwrap();

        // Refus : NIP mal formé, NIP inconnu, date de naissance différente.
        let mut mauvais = adhesion("1234", "0001/2026", especes());
        assert!(matches!(
            adherer(&pool, ag, &mauvais).await,
            Err(Erreur::NipInvalide)
        ));
        mauvais.nip = nip_valide("900000000049999");
        assert!(matches!(
            adherer(&pool, ag, &mauvais).await,
            Err(Erreur::PatientInconnu)
        ));
        mauvais.nip = nip.clone();
        mauvais.date_naissance = d(2000, 1, 2);
        assert!(matches!(
            adherer(&pool, ag, &mauvais).await,
            Err(Erreur::PatientInconnu)
        ));
        // Données : champ vide, référence pour des espèces, date future.
        let mut vide = adhesion(&nip, "0001/2026", especes());
        vide.universite = " ".into();
        assert!(
            matches!(adherer(&pool, ag, &vide).await, Err(Erreur::DonneesInvalides(m)) if m.starts_with("universite"))
        );
        let mut future = adhesion(&nip, "0001/2026", mobile_money("OM-0001"));
        future.paiement.date_declaree = jour.succ_opt();
        assert!(
            matches!(adherer(&pool, ag, &future).await, Err(Erreur::DonneesInvalides(m)) if m.contains("date_declaree"))
        );
        assert_eq!(
            compter(&pool, "contrat_assurance_munaseb").await,
            0,
            "aucun refus n'écrit"
        );

        // Adhésion en espèces, avec une date déclarée d'aujourd'hui.
        let mut bonne = adhesion(&nip, " 0001/2026 ", especes());
        bonne.paiement.date_declaree = Some(jour);
        let contrat_id = adherer(&pool, ag, &bonne).await.unwrap();
        let detail = detail_contrat(&pool, contrat_id).await.unwrap().unwrap();
        assert_eq!(detail.etat, EtatDroits::EnCarence);
        assert_eq!(
            (detail.statut.as_str(), detail.ufr.as_str()),
            ("actif", "UFR fictive")
        );
        assert_eq!(detail.personne_a_prevenir, None, "vide = absent");
        assert_eq!(detail.patient_nom.as_deref(), Some("OUEDRAOGO"));
        let [p] = detail.periodes.as_slice() else {
            panic!("une période attendue")
        };
        let attendu = calculer_periode(None, jour, regles()).unwrap();
        assert_eq!(
            (p.date_paiement, p.debut, p.fin),
            (jour, attendu.0, attendu.1)
        );
        assert_eq!(p.numero_carte.as_deref(), Some("0001/2026"));
        assert_eq!(p.niveau_etudes.as_deref(), Some("Licence 2"));
        assert!(!p.en_cours);
        assert_eq!(
            p.solde,
            SoldePeriode {
                plafond_fcfa: 100_000,
                consomme_fcfa: 0,
                solde_fcfa: 100_000
            }
        );
        let paiement = p.paiement.as_ref().unwrap();
        assert_eq!(
            (
                paiement.mode.as_str(),
                paiement.montant_cotisation_fcfa,
                paiement.montant_fiche_fcfa
            ),
            ("especes", 5000, 100)
        );
        assert_eq!(
            (paiement.date_declaree, paiement.agent_prenom.as_deref()),
            (Some(jour), Some("Inoussa"))
        );
        let json = serde_json::to_string(&detail).unwrap();
        for interdit in [nip.as_str(), "+22670400002", "nip", "telephone"] {
            assert!(!json.contains(interdit), "{interdit} dans {json}");
        }

        // Recherche par carte, espaces autour ignorés ; carte inconnue.
        assert_eq!(
            contrat_par_carte(&pool, " 0001/2026").await.unwrap(),
            Some(contrat_id)
        );
        assert_eq!(contrat_par_carte(&pool, "0002/2026").await.unwrap(), None);
        assert!(detail_contrat(&pool, Uuid::max()).await.unwrap().is_none());

        // Même patient : un seul contrat. Autre patient, même carte : refus.
        assert!(matches!(
            adherer(&pool, ag, &adhesion(&nip, "0003/2026", especes())).await,
            Err(Erreur::ContratDejaExistant)
        ));
        let (_, nip2) = patient(&pool, "+22670400003", "900000000040003").await;
        assert!(matches!(
            adherer(&pool, ag, &adhesion(&nip2, "0001/2026", especes())).await,
            Err(Erreur::NumeroCarteDejaUtilise)
        ));
        // Mobile money : référence unique.
        adherer(
            &pool,
            ag,
            &adhesion(&nip2, "0004/2026", mobile_money("OM-REF-1")),
        )
        .await
        .unwrap();
        let (_, nip3) = patient(&pool, "+22670400004", "900000000040004").await;
        assert!(matches!(
            adherer(
                &pool,
                ag,
                &adhesion(&nip3, "0005/2026", mobile_money("OM-REF-1"))
            )
            .await,
            Err(Erreur::ReferenceTransactionDejaUtilisee)
        ));
        assert_eq!(compter(&pool, "contrat_assurance_munaseb").await, 2);
        assert_eq!(compter(&pool, "paiement_cotisation_munaseb").await, 2);
    }

    /// Renouvellement selon la date de paiement : dans les temps, dans le
    /// délai de grâce, au-delà ; refus pendant une carence, pour une seconde
    /// période d'avance, pour un contrat suspendu ou inconnu.
    #[sqlx::test]
    #[ignore]
    async fn renouvellements_selon_la_date_de_paiement(pool: PgPool) {
        let ag = agent(&pool, "+22670410001").await;
        let jour = aujourdhui_au_burkina(&pool).await.unwrap();
        let plus = |n: i64| jour.checked_add_signed(chrono::Duration::days(n)).unwrap();

        // Dans les temps : période en cours jusqu'à J+30.
        let (p, _) = patient(&pool, "+22670410002", "900000000041002").await;
        let c = contrat_avec_periode(&pool, p, "R-1/2026", -300, 30).await;
        let id = renouveler(&pool, ag, c, &renouvellement("R-1/2027", especes()))
            .await
            .unwrap();
        let (debut, fin): (NaiveDate, NaiveDate) = sqlx::query_as(
            "SELECT date_debut, date_fin FROM periode_adhesion_munaseb WHERE id = $1",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            (debut, fin),
            (plus(31), plus(30).checked_add_months(DUREE).unwrap())
        );
        let detail = detail_contrat(&pool, c).await.unwrap().unwrap();
        assert_eq!(detail.etat, EtatDroits::Couvert);
        assert_eq!(detail.periodes.len(), 2);
        assert_eq!(
            detail.periodes[0].numero_carte.as_deref(),
            Some("R-1/2027"),
            "la plus récente d'abord"
        );
        assert_eq!(detail.periodes[0].antenne.as_deref(), Some("Bobo"));
        assert!(
            detail.periodes[1].en_cours && detail.periodes[1].paiement.is_none(),
            "période posée en SQL"
        );
        // Seconde période d'avance : refusée.
        assert!(matches!(
            renouveler(&pool, ag, c, &renouvellement("R-1/2028", especes())).await,
            Err(Erreur::PeriodeFutureDejaPayee)
        ));
        // Le numéro de carte de l'année précédente ne se réutilise pas.
        let (p_bis, _) = patient(&pool, "+22670410009", "900000000041009").await;
        let c_bis = contrat_avec_periode(&pool, p_bis, "R-9/2026", -300, 30).await;
        assert!(matches!(
            renouveler(&pool, ag, c_bis, &renouvellement("R-1/2026", especes())).await,
            Err(Erreur::NumeroCarteDejaUtilise)
        ));

        // Délai de grâce : fin hier ; couvert dès aujourd'hui, fin à la
        // date anniversaire suivante.
        let (p, _) = patient(&pool, "+22670410003", "900000000041003").await;
        let c = contrat_avec_periode(&pool, p, "R-2/2026", -360, -1).await;
        renouveler(&pool, ag, c, &renouvellement("R-2/2027", especes()))
            .await
            .unwrap();
        let detail = detail_contrat(&pool, c).await.unwrap().unwrap();
        assert_eq!(detail.etat, EtatDroits::Couvert);
        assert_eq!(
            (detail.periodes[0].debut, detail.periodes[0].fin),
            (jour, plus(-1).checked_add_months(DUREE).unwrap())
        );

        // Au-delà du délai (fin il y a 11 jours) : nouvelle carence.
        let (p, _) = patient(&pool, "+22670410004", "900000000041004").await;
        let c = contrat_avec_periode(&pool, p, "R-3/2026", -370, -11).await;
        renouveler(&pool, ag, c, &renouvellement("R-3/2027", especes()))
            .await
            .unwrap();
        let detail = detail_contrat(&pool, c).await.unwrap().unwrap();
        assert_eq!(detail.etat, EtatDroits::EnCarence);
        assert_eq!(
            (detail.periodes[0].debut, detail.periodes[0].fin),
            calculer_periode(None, jour, regles()).unwrap()
        );

        // Pendant la carence d'une première adhésion : refusé.
        let (_, nip) = patient(&pool, "+22670410005", "900000000041005").await;
        let c = adherer(&pool, ag, &adhesion(&nip, "R-5/2026", especes()))
            .await
            .unwrap();
        assert!(matches!(
            renouveler(&pool, ag, c, &renouvellement("R-5/2027", especes())).await,
            Err(Erreur::PeriodeFutureDejaPayee)
        ));

        // Suspendu (posé en SQL, aucune route) : bloqué. Inconnu : 404.
        let (p, _) = patient(&pool, "+22670410006", "900000000041006").await;
        let c = contrat_avec_periode(&pool, p, "R-6/2026", -300, -1).await;
        sqlx::query("UPDATE contrat_assurance_munaseb SET statut = 'suspendu' WHERE id = $1")
            .bind(c)
            .execute(&pool)
            .await
            .unwrap();
        assert!(matches!(
            renouveler(&pool, ag, c, &renouvellement("R-6/2027", especes())).await,
            Err(Erreur::ContratSuspendu)
        ));
        assert_eq!(
            detail_contrat(&pool, c).await.unwrap().unwrap().etat,
            EtatDroits::Suspendu
        );
        assert!(matches!(
            renouveler(
                &pool,
                ag,
                Uuid::max(),
                &renouvellement("R-7/2027", especes())
            )
            .await,
            Err(Erreur::AucunContrat)
        ));
    }

    /// Deux renouvellements simultanés du même contrat : le verrou du
    /// contrat les fait passer l'un après l'autre ; un seul réussit, l'autre
    /// reçoit `PeriodeFutureDejaPayee` (et non une erreur SQL brute).
    #[sqlx::test]
    #[ignore]
    async fn renouvellements_simultanes(pool: PgPool) {
        let ag = agent(&pool, "+22670420001").await;
        let (p, _) = patient(&pool, "+22670420002", "900000000042002").await;
        let c = contrat_avec_periode(&pool, p, "S-1/2026", -300, 30).await;
        let (p1, p2) = (pool.clone(), pool.clone());
        let t1 = tokio::spawn(async move {
            renouveler(&p1, ag, c, &renouvellement("S-1/2027", especes())).await
        });
        let t2 = tokio::spawn(async move {
            renouveler(&p2, ag, c, &renouvellement("S-2/2027", especes())).await
        });
        let resultats = [t1.await.unwrap(), t2.await.unwrap()];
        assert_eq!(resultats.iter().filter(|r| r.is_ok()).count(), 1);
        assert!(
            resultats
                .iter()
                .any(|r| matches!(r, Err(Erreur::PeriodeFutureDejaPayee)))
        );
        assert_eq!(
            detail_contrat(&pool, c)
                .await
                .unwrap()
                .unwrap()
                .periodes
                .len(),
            2
        );
    }

    /// Paramètres : valeurs initiales, refus hors bornes, requête identique
    /// sans nouvelle version, nouvelle version avec le responsable,
    /// historique paginé, effet sur les périodes et les paiements suivants
    /// seulement, lignes jamais modifiées ; la base refuse un agent comme
    /// auteur (migration 0029).
    #[sqlx::test]
    #[ignore]
    async fn parametres_modifies_et_historises(pool: PgPool) {
        let ag = agent(&pool, "+22670430001").await;
        let resp = collegue(&pool, "+22670430009", ag, "responsable").await;
        let initiaux = parametres_en_vigueur(&pool).await.unwrap();
        let valeurs = Parametres {
            cotisation_fcfa: 5000,
            fiche_fcfa: 100,
            delai_grace_jours: 10,
            carence_mois: 1,
            plafond_periode_fcfa: 100_000,
        };
        assert_eq!(initiaux.valeurs, valeurs);
        assert_eq!(initiaux.auteur_nom, None, "posés par la migration");

        // Période créée avant la modification.
        let (_, nip_avant) = patient(&pool, "+22670430002", "900000000043002").await;
        let avant = adherer(&pool, ag, &adhesion(&nip_avant, "P-1/2026", especes()))
            .await
            .unwrap();

        assert!(matches!(
            modifier_parametres(
                &pool,
                resp,
                Parametres {
                    carence_mois: 13,
                    ..valeurs
                }
            )
            .await,
            Err(Erreur::DonneesInvalides(_))
        ));
        assert_eq!(
            modifier_parametres(&pool, resp, valeurs).await.unwrap().id,
            initiaux.id,
            "rien ne change"
        );
        assert_eq!(compter(&pool, "parametre_munaseb").await, 1);

        // Garantie de la base : une version écrite au nom d'un agent est
        // refusée (rôle de l'auteur contrôlé par la 0029, clé MATCH FULL).
        let refus = sqlx::query(
            "INSERT INTO parametre_munaseb (cotisation_fcfa, fiche_fcfa, delai_grace_jours, carence_mois, plafond_periode_fcfa, \
                                            auteur_affectation_id, auteur_structure_id, auteur_role) \
             VALUES (1, 1, 1, 1, 1, $1, $2, 'agent_assurance_munaseb')",
        )
        .bind(ag.affectation_id)
        .bind(ag.structure_id)
        .execute(&pool)
        .await
        .unwrap_err();
        assert_eq!(
            refus.as_database_error().and_then(|e| e.constraint()),
            Some("parametre_munaseb_auteur_responsable")
        );

        let nouveaux = Parametres {
            cotisation_fcfa: 6000,
            carence_mois: 0,
            plafond_periode_fcfa: 120_000,
            ..valeurs
        };
        let version = modifier_parametres(&pool, resp, nouveaux).await.unwrap();
        assert_eq!(
            (version.valeurs, version.auteur_prenom.as_deref()),
            (nouveaux, Some("Mariam"))
        );
        assert_eq!(parametres_en_vigueur(&pool).await.unwrap().id, version.id);

        let tout = historique_parametres(&pool, None, 10).await.unwrap();
        assert_eq!(
            tout.iter().map(|v| v.id).collect::<Vec<_>>(),
            [version.id, initiaux.id],
            "la plus récente d'abord"
        );
        let page = historique_parametres(&pool, Some(version.id), 10)
            .await
            .unwrap();
        assert_eq!(page.iter().map(|v| v.id).collect::<Vec<_>>(), [initiaux.id]);

        // Période suivante : nouveau plafond, sans carence, nouvelle
        // cotisation. La période d'avant garde les siens.
        let (_, nip_apres) = patient(&pool, "+22670430003", "900000000043003").await;
        let apres = adherer(&pool, ag, &adhesion(&nip_apres, "P-2/2026", especes()))
            .await
            .unwrap();
        let jour = aujourdhui_au_burkina(&pool).await.unwrap();
        let d_apres = detail_contrat(&pool, apres).await.unwrap().unwrap();
        assert_eq!(d_apres.etat, EtatDroits::Couvert, "carence nulle");
        assert_eq!(
            (
                d_apres.periodes[0].debut,
                d_apres.periodes[0].solde.plafond_fcfa
            ),
            (jour, 120_000)
        );
        assert_eq!(
            d_apres.periodes[0]
                .paiement
                .as_ref()
                .unwrap()
                .montant_cotisation_fcfa,
            6000
        );
        let d_avant = detail_contrat(&pool, avant).await.unwrap().unwrap();
        assert_eq!(d_avant.periodes[0].solde.plafond_fcfa, 100_000);
        assert_eq!(
            d_avant.periodes[0]
                .paiement
                .as_ref()
                .unwrap()
                .montant_cotisation_fcfa,
            5000
        );

        // Jamais modifiées ni supprimées, même par le propriétaire.
        for requete in [
            "UPDATE parametre_munaseb SET cotisation_fcfa = 1",
            "DELETE FROM parametre_munaseb",
            "TRUNCATE parametre_munaseb",
            "UPDATE paiement_cotisation_munaseb SET montant_cotisation_fcfa = 1",
            "DELETE FROM paiement_cotisation_munaseb",
            "TRUNCATE paiement_cotisation_munaseb",
        ] {
            let erreur = sqlx::query(requete).execute(&pool).await.unwrap_err();
            assert!(
                erreur.to_string().contains("jamais modifiees"),
                "{requete} : {erreur}"
            );
        }
    }

    /// Limite d'échecs d'adhésion (migration 0029) :
    /// - chaque couple NIP et date sans patient est tracé avec l'affectation
    ///   de l'agent, sans le NIP ni la date ; un NIP mal formé ne compte pas ;
    /// - au 5e échec dans les 15 minutes, l'adhésion suivante est refusée
    ///   avant la recherche du patient, même avec un couple correct, avec
    ///   un délai d'attente ; un autre agent n'est pas touché ;
    /// - un échec sorti de la fenêtre ne compte plus ;
    /// - les traces ne se modifient ni ne s'effacent.
    #[sqlx::test]
    #[ignore]
    async fn limite_d_echecs_d_adhesion(pool: PgPool) {
        let ag = agent(&pool, "+22670480001").await;
        let autre = collegue(&pool, "+22670480002", ag, "agent_assurance_munaseb").await;
        let (_, nip) = patient(&pool, "+22670480003", "900000000048003").await;
        let mauvaise_date = || {
            let mut a = adhesion(&nip, "L-1/2026", especes());
            a.date_naissance = d(1999, 12, 31);
            a
        };
        let echecs_de = |affectation: Uuid| {
            let pool = pool.clone();
            async move {
                sqlx::query_scalar::<_, i64>(
                    "SELECT count(*) FROM echec_adhesion_munaseb WHERE agent_affectation_id = $1",
                )
                .bind(affectation)
                .fetch_one(&pool)
                .await
                .unwrap()
            }
        };

        // Un NIP mal formé ne compte pas.
        assert!(matches!(
            adherer(&pool, ag, &adhesion("123", "L-1/2026", especes())).await,
            Err(Erreur::NipInvalide)
        ));
        assert_eq!(echecs_de(ag.affectation_id).await, 0);

        for _ in 0..ECHECS_ADHESION_MAX {
            assert!(matches!(
                adherer(&pool, ag, &mauvaise_date()).await,
                Err(Erreur::PatientInconnu)
            ));
        }
        assert_eq!(
            echecs_de(ag.affectation_id).await,
            ECHECS_ADHESION_MAX,
            "chaque échec tracé"
        );

        // Limite atteinte : refusé même avec le bon couple, rien n'est écrit.
        match adherer(&pool, ag, &adhesion(&nip, "L-1/2026", especes())).await {
            Err(Erreur::TropDEchecs {
                reessayer_dans_secs,
            }) => {
                assert!(
                    (1..=15 * 60).contains(&reessayer_dans_secs),
                    "{reessayer_dans_secs}"
                )
            }
            autre => panic!("TropDEchecs attendu, obtenu {autre:?}"),
        }
        assert_eq!(compter(&pool, "contrat_assurance_munaseb").await, 0);
        assert_eq!(
            echecs_de(ag.affectation_id).await,
            ECHECS_ADHESION_MAX,
            "le refus n'ajoute pas d'échec"
        );

        // Un autre agent de la même MUNASEB n'est pas touché.
        adherer(&pool, autre, &adhesion(&nip, "L-1/2026", especes()))
            .await
            .unwrap();
        assert_eq!(echecs_de(autre.affectation_id).await, 0);

        // Un échec de plus de 15 minutes ne compte plus : un agent dont 4
        // échecs sont récents et 1 ancien peut encore essayer.
        let ag2 = collegue(&pool, "+22670480004", ag, "agent_assurance_munaseb").await;
        sqlx::query(
            "INSERT INTO echec_adhesion_munaseb (agent_affectation_id, agent_structure_id, agent_role, date_echec) \
             SELECT $1, $2, 'agent_assurance_munaseb', now() - interval '16 minutes' \
             UNION ALL SELECT $1, $2, 'agent_assurance_munaseb', now() - interval '1 minute' FROM generate_series(1, 4)",
        )
        .bind(ag2.affectation_id)
        .bind(ag2.structure_id)
        .execute(&pool)
        .await
        .unwrap();
        assert!(matches!(
            adherer(&pool, ag2, &mauvaise_date()).await,
            Err(Erreur::PatientInconnu)
        ));
        assert!(matches!(
            adherer(&pool, ag2, &mauvaise_date()).await,
            Err(Erreur::TropDEchecs { .. })
        ));

        // La trace ne garde que qui et quand ; jamais modifiée ni effacée.
        let colonnes: Vec<String> = sqlx::query_scalar(
            "SELECT column_name::text FROM information_schema.columns WHERE table_name = 'echec_adhesion_munaseb' ORDER BY ordinal_position",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            colonnes,
            [
                "id",
                "agent_affectation_id",
                "agent_structure_id",
                "agent_role",
                "date_echec"
            ]
        );
        for requete in [
            "UPDATE echec_adhesion_munaseb SET date_echec = now()",
            "DELETE FROM echec_adhesion_munaseb",
            "TRUNCATE echec_adhesion_munaseb",
        ] {
            let erreur = sqlx::query(requete).execute(&pool).await.unwrap_err();
            assert!(
                erreur.to_string().contains("jamais modifiees"),
                "{requete} : {erreur}"
            );
        }
    }

    /// Adhésions simultanées du même agent avec des couples faux : le verrou
    /// de l'affectation les fait passer l'une après l'autre ; jamais plus de
    /// `ECHECS_ADHESION_MAX` échecs tracés, les autres reçoivent `TropDEchecs`.
    #[sqlx::test]
    #[ignore]
    async fn echecs_simultanes_sans_depasser_la_limite(pool: PgPool) {
        let ag = agent(&pool, "+22670490001").await;
        let (_, nip) = patient(&pool, "+22670490002", "900000000049002").await;
        let mut taches = Vec::new();
        for _ in 0..10 {
            let (pool, nip) = (pool.clone(), nip.clone());
            taches.push(tokio::spawn(async move {
                let mut a = adhesion(&nip, "M-1/2026", especes());
                a.date_naissance = d(1999, 12, 31);
                adherer(&pool, ag, &a).await
            }));
        }
        let mut inconnus = 0;
        let mut limites = 0;
        for tache in taches {
            match tache.await.unwrap() {
                Err(Erreur::PatientInconnu) => inconnus += 1,
                Err(Erreur::TropDEchecs { .. }) => limites += 1,
                autre => panic!("{autre:?}"),
            }
        }
        assert_eq!(
            (inconnus, limites),
            (ECHECS_ADHESION_MAX, 10 - ECHECS_ADHESION_MAX)
        );
        let traces: i64 = sqlx::query_scalar("SELECT count(*) FROM echec_adhesion_munaseb")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(traces, ECHECS_ADHESION_MAX);
    }

    /// Garanties de la base (migration 0028), sans passer par le code :
    /// numéro de carte obligatoire et unique sur une nouvelle période,
    /// référence selon le mode, date déclarée jamais future.
    #[sqlx::test]
    #[ignore]
    async fn garanties_de_la_base(pool: PgPool) {
        let ag = agent(&pool, "+22670440001").await;
        let (p, _) = patient(&pool, "+22670440002", "900000000044002").await;
        let c = contrat_avec_periode(&pool, p, "G-1/2026", -300, -200).await;
        let inserer_periode = |carte: Option<&'static str>, debut: i32| {
            let pool = pool.clone();
            async move {
                sqlx::query(
                    "INSERT INTO periode_adhesion_munaseb (contrat_id, date_paiement, date_debut, date_fin, numero_carte) \
                     VALUES ($1, CURRENT_DATE + $2, CURRENT_DATE + $2, CURRENT_DATE + $2 + 10, $3)",
                )
                .bind(c)
                .bind(debut)
                .bind(carte)
                .execute(&pool)
                .await
            }
        };
        assert!(
            inserer_periode(None, -100).await.is_err(),
            "numéro obligatoire"
        );
        assert!(
            inserer_periode(Some(" G-2 "), -100).await.is_err(),
            "espaces autour"
        );
        assert!(
            inserer_periode(Some("G-1/2026"), -100).await.is_err(),
            "déjà pris"
        );
        inserer_periode(Some("G-2/2026"), -100).await.unwrap();

        let periode: Uuid = sqlx::query_scalar(
            "SELECT id FROM periode_adhesion_munaseb WHERE numero_carte = 'G-2/2026'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let inserer_paiement = |mode: &'static str,
                                reference: Option<&'static str>,
                                decalage: i32| {
            let pool = pool.clone();
            async move {
                sqlx::query(
                    "INSERT INTO paiement_cotisation_munaseb \
                     (periode_id, mode, montant_cotisation_fcfa, montant_fiche_fcfa, reference_transaction, date_declaree, \
                      agent_affectation_id, agent_structure_id, agent_role) \
                     VALUES ($1, $2, 5000, 100, $3, CURRENT_DATE + $4, $5, $6, 'agent_assurance_munaseb')",
                )
                .bind(periode)
                .bind(mode)
                .bind(reference)
                .bind(decalage)
                .bind(ag.affectation_id)
                .bind(ag.structure_id)
                .execute(&pool)
                .await
            }
        };
        assert!(
            inserer_paiement("mobile_money", None, 0).await.is_err(),
            "référence exigée"
        );
        assert!(
            inserer_paiement("especes", Some("OM-1"), 0).await.is_err(),
            "référence refusée"
        );
        assert!(
            inserer_paiement("especes", None, 2).await.is_err(),
            "date future"
        );
        inserer_paiement("especes", None, -1).await.unwrap();
        assert!(
            inserer_paiement("especes", None, -1).await.is_err(),
            "un paiement par période"
        );
    }
}
