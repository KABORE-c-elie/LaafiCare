//! DemandeRemboursement MUNASEB, circuit tiers payant (section 12 du
//! CLAUDE.md, "Décisions actées -- DemandeRemboursement (2026-09-24)").
//!
//! `creer_depuis_acte` est l'unique point d'entrée métier pour générer une
//! demande depuis un acte enregistré par une structure partenaire. Les
//! futurs modules Pharmacie/Laboratoire/Hôpital l'appelleront directement,
//! la route de test `simuler-acte` aussi -- sans réécriture de cette
//! logique.
//!
//! Chaque demande est rattachée dès sa création à la période d'adhésion
//! qui contient la date de l'acte (section 12) : le plafond se calcule par
//! période.
//!
//! Transitions de statut (décisions T1-T3, H, V1-V4 du 2026-09-25) :
//! `prendre_en_charge`, `valider`, `rejeter`, `marquer_payee`. Chacune
//! écrit une ligne dans `historique_statut_demande` dans la même
//! transaction, puis notifie le patient après l'enregistrement.
//!
//! ORDRE DES VERROUS (règle permanente, section 12 du CLAUDE.md) : tout code
//! qui verrouille à la fois une demande et sa période verrouille TOUJOURS
//! la demande d'abord, puis la période. Sinon, deux transactions peuvent se
//! bloquer mutuellement.

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use sqlx::PgPool;
use sqlx::types::Uuid;

use crate::contrat_assurance_munaseb::PLAFOND_ANNUEL_FCFA;
use crate::nip;
use crate::notification::{self, TypeNotification};

/// Liste fermée, définie par le code : même liste que le CHECK de
/// `tarif_acte_munaseb.type_acte` (migration 0007). Les VALEURS (taux ou
/// forfait) ne sont jamais ici : elles sont en base, modifiables par les
/// agents.
///
/// `Deserialize` en snake_case (décision S2) : les valeurs JSON acceptées
/// sont exactement celles de `as_str`, un type inconnu est refusé à la
/// lecture du corps de la requête.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TypeActeMunaseb {
    Consultation,
    Hospitalisation,
    Pharmacie,
    Laboratoire,
    Lunetterie,
}

impl TypeActeMunaseb {
    pub const TOUS: [TypeActeMunaseb; 5] = [
        TypeActeMunaseb::Consultation,
        TypeActeMunaseb::Hospitalisation,
        TypeActeMunaseb::Pharmacie,
        TypeActeMunaseb::Laboratoire,
        TypeActeMunaseb::Lunetterie,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            TypeActeMunaseb::Consultation => "consultation",
            TypeActeMunaseb::Hospitalisation => "hospitalisation",
            TypeActeMunaseb::Pharmacie => "pharmacie",
            TypeActeMunaseb::Laboratoire => "laboratoire",
            TypeActeMunaseb::Lunetterie => "lunetterie",
        }
    }
}

/// Cycle En attente -> En cours -> Validé / Rejeté -> Payé (mémoire l. 815).
///
/// `Deserialize` en snake_case (décision L7) : le filtre `?statut=` de la
/// liste accepte exactement les valeurs de `as_str`, celles de la base.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatutDemande {
    EnAttente,
    EnCours,
    Valide,
    Rejete,
    Paye,
}

impl StatutDemande {
    pub fn as_str(self) -> &'static str {
        match self {
            StatutDemande::EnAttente => "en_attente",
            StatutDemande::EnCours => "en_cours",
            StatutDemande::Valide => "valide",
            StatutDemande::Rejete => "rejete",
            StatutDemande::Paye => "paye",
        }
    }

    /// Texte de la notification de changement de statut. Volontairement
    /// sans type d'acte, montant, nom du partenaire ni motif de rejet : un
    /// push futur s'afficherait sur l'écran verrouillé (section 12).
    pub fn message_notification(self) -> &'static str {
        match self {
            StatutDemande::EnAttente => "Une demande de remboursement a été enregistrée à votre nom.",
            StatutDemande::EnCours => "Votre demande de remboursement est en cours de traitement.",
            StatutDemande::Valide => "Votre demande de remboursement a été validée.",
            StatutDemande::Rejete => "Votre demande de remboursement a été rejetée.",
            StatutDemande::Paye => "Votre demande de remboursement a été payée.",
        }
    }
}

pub const TITRE_NOTIFICATION: &str = "Remboursement MUNASEB";

/// Texte d'une validation à 0 FCFA (plafond de la période épuisé) : le
/// patient ne doit pas croire qu'il est remboursé (décision du porteur).
pub const MESSAGE_VALIDEE_PLAFOND_ATTEINT: &str =
    "Votre demande a été traitée : le plafond de votre période est atteint, aucun montant ne sera versé.";

/// Garde-fou technique sur le motif de rejet (décision V4), en caractères.
pub const MOTIF_LONGUEUR_MAX: usize = 1000;

/// Acte transmis par une structure partenaire. Le patient est désigné par
/// son NIP, clé d'interopérabilité entre les applications (section 5).
#[derive(Debug, Clone)]
pub struct Acte {
    pub nip: String,
    pub partenaire_id: Uuid,
    pub type_acte: TypeActeMunaseb,
    /// Identifiant de l'acte chez le partenaire : un second appel avec le
    /// même couple (partenaire, référence) renvoie la demande existante.
    pub reference_acte: String,
    pub date_acte: NaiveDate,
    pub montant_acte_fcfa: i32,
}

#[derive(Debug, thiserror::Error)]
pub enum ErreurDemandeRemboursement {
    #[error("le montant de l'acte doit être strictement positif")]
    MontantInvalide,

    /// Format ou chiffre de contrôle Luhn incorrect : faute de frappe ou QR
    /// code corrompu, détectée sans requête en base (voir nip.rs).
    #[error("NIP invalide")]
    NipInvalide,

    /// NIP bien formé mais inconnu. Distinct de `NonCouvert` (validé, à
    /// condition que cette fonction ne soit jamais exposée sans
    /// authentification : l'erreur révèle si un NIP existe).
    #[error("aucun patient pour ce NIP")]
    PatientInconnu,

    /// Même référence d'acte chez le même partenaire, mais NIP, type,
    /// montant ou date différents. Renvoyer la demande existante ferait
    /// perdre l'acte réel en silence (patient jamais remboursé) : on refuse
    /// explicitement.
    #[error("cette référence d'acte est déjà utilisée par ce partenaire pour un acte différent")]
    ReferenceActeDejaUtilisee,

    #[error("partenaire inconnu")]
    PartenaireInconnu,

    /// Distinct de `NonCouvert` (décision actée) : le patient reste couvert,
    /// c'est le partenaire qui ne l'est plus. Une erreur commune ferait
    /// croire au patient qu'il n'est pas assuré.
    #[error("partenaire suspendu")]
    PartenaireSuspendu,

    /// Pas de contrat, contrat suspendu, ou date de l'acte dans aucune
    /// période (y compris les jours non couverts avant un paiement en
    /// retard : pas d'effet rétroactif). L'appelant doit traiter ce cas sans bloquer le
    /// patient : MUNASEB n'est jamais une dépendance dure (section 12).
    #[error("acte non couvert par un contrat MUNASEB actif")]
    NonCouvert,

    /// Acte fait entre le paiement d'une période et son début (décision
    /// R1). Distinct de `NonCouvert`, pour la même raison que
    /// `PartenaireSuspendu` : le patient a payé, il ne doit pas croire qu'il
    /// n'est pas assuré.
    #[error("en carence, couverture à partir du {debut_couverture}")]
    EnCarence { debut_couverture: NaiveDate },

    #[error("erreur interne : {0}")]
    Interne(String),
}

impl From<sqlx::Error> for ErreurDemandeRemboursement {
    fn from(erreur: sqlx::Error) -> Self {
        ErreurDemandeRemboursement::Interne(erreur.to_string())
    }
}

/// Tarif d'un type d'acte : soit un taux, soit un forfait (contrainte
/// `taux_ou_forfait`, migration 0007).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tarif {
    TauxPercent(i16),
    ForfaitFcfa(i32),
}

/// Part MUNASEB d'un acte, avant tout plafonnement au solde restant.
///
/// - Taux : arrondi à l'entier FCFA le plus proche (décision actée). Les
///   montants étant positifs, `(montant × taux + 50) / 100` en division
///   entière arrondit au plus proche, les demis vers le haut. Calcul en
///   i64 : `montant × taux` peut dépasser i32 pour un gros montant.
/// - Forfait : `min(coût, forfait)` (décision actée, mémoire l. 502 : "ne
///   rembourse que 15000 francs CFA") -- jamais plus que le coût réel.
pub fn calculer_montant_demande(montant_acte_fcfa: i32, tarif: Tarif) -> i32 {
    match tarif {
        Tarif::TauxPercent(taux) => {
            let arrondi = (i64::from(montant_acte_fcfa) * i64::from(taux) + 50) / 100;
            // taux <= 100 (CHECK en base) : le résultat ne dépasse jamais
            // montant_acte_fcfa, donc tient dans un i32.
            arrondi as i32
        }
        Tarif::ForfaitFcfa(forfait) => montant_acte_fcfa.min(forfait),
    }
}

/// Génère la demande de remboursement d'un acte (tiers payant) au statut
/// En attente, puis notifie le patient. Renvoie l'id de la demande, ou
/// celui de la demande déjà créée pour le même acte.
pub async fn creer_depuis_acte(
    pool: &PgPool,
    acte: &Acte,
) -> Result<Uuid, ErreurDemandeRemboursement> {
    if acte.montant_acte_fcfa <= 0 {
        return Err(ErreurDemandeRemboursement::MontantInvalide);
    }
    // Avant toute requête : c'est la raison d'être du chiffre de contrôle.
    if !nip::valider(&acte.nip) {
        return Err(ErreurDemandeRemboursement::NipInvalide);
    }

    let statut_partenaire: Option<String> =
        sqlx::query_scalar("SELECT statut FROM partenaire_sante_munaseb WHERE id = $1")
            .bind(acte.partenaire_id)
            .fetch_optional(pool)
            .await?;
    let statut_partenaire = statut_partenaire.ok_or(ErreurDemandeRemboursement::PartenaireInconnu)?;

    // Rejeu d'un acte déjà transmis (ex. après un timeout côté appelant) :
    // renvoyer la demande existante, avant tout autre contrôle, pour que le
    // rejeu donne le même résultat que le premier appel même si le contrat
    // ou le partenaire a changé d'état entre-temps.
    if let Some(id) = rejeu(pool, acte).await? {
        return Ok(id);
    }

    if statut_partenaire != "actif" {
        return Err(ErreurDemandeRemboursement::PartenaireSuspendu);
    }

    let ligne: Option<(Uuid, Option<Uuid>, Option<String>)> = sqlx::query_as(
        "SELECT p.utilisateur_id, c.id, c.statut \
         FROM patient p LEFT JOIN contrat_assurance_munaseb c ON c.patient_id = p.id \
         WHERE p.nip = $1",
    )
    .bind(&acte.nip)
    .fetch_optional(pool)
    .await?;
    let (utilisateur_id, contrat_id, statut_contrat) = ligne.ok_or(ErreurDemandeRemboursement::PatientInconnu)?;

    let contrat_id = match (contrat_id, statut_contrat.as_deref()) {
        (Some(id), Some("actif")) => id,
        _ => return Err(ErreurDemandeRemboursement::NonCouvert),
    };

    // La couverture s'apprécie à la date de l'acte, pas à la date de
    // l'appel : un acte de fin de période transmis après un renouvellement
    // retombe bien dans l'ancienne période.
    let periode_id = periode_de_l_acte(pool, contrat_id, acte.date_acte).await?;

    let tarif = lire_tarif(pool, acte.type_acte).await?;
    let montant_demande = calculer_montant_demande(acte.montant_acte_fcfa, tarif);

    // Demande et ligne de création de l'historique dans une même
    // transaction (décision H, "création comprise") ; la notification ne
    // part qu'après le commit. ON CONFLICT couvre deux appels simultanés
    // pour le même acte, que le contrôle `rejeu` plus haut ne peut pas
    // exclure seul.
    let mut tx = pool.begin().await?;
    let inseree: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO demande_remboursement_munaseb \
         (periode_id, partenaire_id, type_acte, reference_acte, date_acte, montant_acte_fcfa, montant_demande_fcfa) \
         VALUES ($1, $2, $3, $4, $5, $6, $7) \
         ON CONFLICT (partenaire_id, reference_acte) DO NOTHING RETURNING id",
    )
    .bind(periode_id)
    .bind(acte.partenaire_id)
    .bind(acte.type_acte.as_str())
    .bind(&acte.reference_acte)
    .bind(acte.date_acte)
    .bind(acte.montant_acte_fcfa)
    .bind(montant_demande)
    .fetch_optional(&mut *tx)
    .await?;

    let Some(demande_id) = inseree else {
        // Perdu la course contre un appel simultané : la demande existe,
        // et c'est cet autre appel qui a notifié. Rien n'a été écrit ici.
        // Même comparaison des champs que pour un rejeu ordinaire.
        tx.rollback().await?;
        return rejeu(pool, acte)
            .await?
            .ok_or_else(|| ErreurDemandeRemboursement::Interne("conflit sans demande existante".into()));
    };

    historiser(&mut tx, demande_id, None, StatutDemande::EnAttente, None).await?;
    tx.commit().await?;

    // Un échec de notification est journalisé et n'annule jamais la demande
    // (décision actée). Ni téléphone ni nom dans le journal.
    if let Err(erreur) = notification::creer(
        pool,
        utilisateur_id,
        TypeNotification::RemboursementStatut,
        TITRE_NOTIFICATION,
        StatutDemande::EnAttente.message_notification(),
        Some(demande_id),
    )
    .await
    {
        tracing::error!(%demande_id, %erreur, "échec de création de la notification de demande de remboursement");
    }

    Ok(demande_id)
}

/// Demande déjà enregistrée pour ce couple (partenaire, référence) :
/// `Some(id)` si l'acte est identique (NIP, type, montant, date),
/// `ReferenceActeDejaUtilisee` s'il diffère, `None` s'il n'y en a pas.
async fn rejeu(pool: &PgPool, acte: &Acte) -> Result<Option<Uuid>, ErreurDemandeRemboursement> {
    let ligne: Option<(Uuid, String, String, i32, NaiveDate)> = sqlx::query_as(
        "SELECT d.id, p.nip, d.type_acte, d.montant_acte_fcfa, d.date_acte \
         FROM demande_remboursement_munaseb d \
         JOIN periode_adhesion_munaseb pe ON pe.id = d.periode_id \
         JOIN contrat_assurance_munaseb c ON c.id = pe.contrat_id \
         JOIN patient p ON p.id = c.patient_id \
         WHERE d.partenaire_id = $1 AND d.reference_acte = $2",
    )
    .bind(acte.partenaire_id)
    .bind(&acte.reference_acte)
    .fetch_optional(pool)
    .await?;

    match ligne {
        None => Ok(None),
        Some((id, nip, type_acte, montant, date))
            if nip == acte.nip
                && type_acte == acte.type_acte.as_str()
                && montant == acte.montant_acte_fcfa
                && date == acte.date_acte =>
        {
            Ok(Some(id))
        }
        Some(_) => Err(ErreurDemandeRemboursement::ReferenceActeDejaUtilisee),
    }
}

/// Période du contrat qui contient la date de l'acte. Sans elle :
/// `EnCarence` si l'acte tombe entre le paiement d'une période et son
/// début, `NonCouvert` sinon (jours non couverts avant un paiement en
/// retard, ou acte hors de toute période).
async fn periode_de_l_acte(
    pool: &PgPool,
    contrat_id: Uuid,
    date_acte: NaiveDate,
) -> Result<Uuid, ErreurDemandeRemboursement> {
    // Les périodes d'un contrat ne se chevauchent pas (contrainte
    // d'exclusion, migration 0009) : au plus une ligne.
    let periode: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM periode_adhesion_munaseb \
         WHERE contrat_id = $1 AND $2 BETWEEN date_debut AND date_fin",
    )
    .bind(contrat_id)
    .bind(date_acte)
    .fetch_optional(pool)
    .await?;
    if let Some(id) = periode {
        return Ok(id);
    }

    // date_paiement est conservée sur la période (décision C3) : c'est ce
    // qui distingue la carence (acte après le paiement) des jours non
    // couverts (acte avant le paiement).
    let debut_couverture: Option<NaiveDate> = sqlx::query_scalar(
        "SELECT date_debut FROM periode_adhesion_munaseb \
         WHERE contrat_id = $1 AND $2 >= date_paiement AND $2 < date_debut",
    )
    .bind(contrat_id)
    .bind(date_acte)
    .fetch_optional(pool)
    .await?;

    Err(match debut_couverture {
        Some(debut_couverture) => ErreurDemandeRemboursement::EnCarence { debut_couverture },
        None => ErreurDemandeRemboursement::NonCouvert,
    })
}

async fn lire_tarif(pool: &PgPool, type_acte: TypeActeMunaseb) -> Result<Tarif, ErreurDemandeRemboursement> {
    let ligne: Option<(Option<i16>, Option<i32>)> =
        sqlx::query_as("SELECT taux_percent, forfait_fcfa FROM tarif_acte_munaseb WHERE type_acte = $1")
            .bind(type_acte.as_str())
            .fetch_optional(pool)
            .await?;
    // Chaque variante de l'enum a sa ligne (migration 0007) et la contrainte
    // `taux_ou_forfait` garantit exactement une des deux valeurs : les autres
    // cas signalent une base incohérente avec le code, pas une erreur métier.
    match ligne {
        Some((Some(taux), None)) => Ok(Tarif::TauxPercent(taux)),
        Some((None, Some(forfait))) => Ok(Tarif::ForfaitFcfa(forfait)),
        _ => Err(ErreurDemandeRemboursement::Interne(format!(
            "tarif absent ou incohérent pour {}",
            type_acte.as_str()
        ))),
    }
}

// ---------------------------------------------------------------------
// Transitions de statut
// ---------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum ErreurTransition {
    #[error("demande inconnue")]
    DemandeInconnue,

    /// Le statut actuel est renvoyé (décision V2) : si un autre agent vient
    /// de traiter la demande, l'application affiche son nouvel état.
    #[error("transition impossible depuis le statut {statut_actuel}")]
    TransitionInvalide { statut_actuel: String },

    #[error("le motif de rejet est obligatoire")]
    MotifVide,

    #[error("le motif de rejet dépasse {MOTIF_LONGUEUR_MAX} caractères")]
    MotifTropLong,

    #[error("erreur interne : {0}")]
    Interne(String),
}

impl From<sqlx::Error> for ErreurTransition {
    fn from(erreur: sqlx::Error) -> Self {
        ErreurTransition::Interne(erreur.to_string())
    }
}

/// Demande verrouillée pour la durée de la transaction.
struct DemandeVerrouillee {
    statut: String,
    periode_id: Uuid,
    montant_demande_fcfa: i32,
    montant_rembourse_fcfa: Option<i32>,
    /// Destinataire de la notification (identité du patient).
    utilisateur_id: Uuid,
}

/// Verrouille la ligne de la demande (premier verrou, voir l'ordre des
/// verrous en tête de fichier). `FOR UPDATE OF d` : seule la demande est
/// verrouillée ; sans `OF`, la jointure verrouillerait aussi la période,
/// le contrat et le patient.
async fn verrouiller_demande(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    demande_id: Uuid,
) -> Result<DemandeVerrouillee, ErreurTransition> {
    let ligne: Option<(String, Uuid, i32, Option<i32>, Uuid)> = sqlx::query_as(
        "SELECT d.statut, d.periode_id, d.montant_demande_fcfa, d.montant_rembourse_fcfa, p.utilisateur_id \
         FROM demande_remboursement_munaseb d \
         JOIN periode_adhesion_munaseb pe ON pe.id = d.periode_id \
         JOIN contrat_assurance_munaseb c ON c.id = pe.contrat_id \
         JOIN patient p ON p.id = c.patient_id \
         WHERE d.id = $1 \
         FOR UPDATE OF d",
    )
    .bind(demande_id)
    .fetch_optional(&mut **tx)
    .await?;
    let (statut, periode_id, montant_demande_fcfa, montant_rembourse_fcfa, utilisateur_id) =
        ligne.ok_or(ErreurTransition::DemandeInconnue)?;
    Ok(DemandeVerrouillee { statut, periode_id, montant_demande_fcfa, montant_rembourse_fcfa, utilisateur_id })
}

fn exiger_statut(demande: &DemandeVerrouillee, autorises: &[StatutDemande]) -> Result<(), ErreurTransition> {
    if autorises.iter().any(|statut| statut.as_str() == demande.statut) {
        Ok(())
    } else {
        Err(ErreurTransition::TransitionInvalide { statut_actuel: demande.statut.clone() })
    }
}

/// Ligne d'historique, dans la transaction de l'appelant. Création :
/// `precedent` et `agent` à `None` (contrainte `agent_si_transition`).
async fn historiser(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    demande_id: Uuid,
    precedent: Option<&str>,
    nouveau: StatutDemande,
    agent_utilisateur_id: Option<Uuid>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO historique_statut_demande (demande_id, statut_precedent, statut_nouveau, agent_utilisateur_id) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(demande_id)
    .bind(precedent)
    .bind(nouveau.as_str())
    .bind(agent_utilisateur_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Notification après le commit. Un échec est journalisé et n'annule jamais
/// la transition (décision actée). Ni téléphone ni nom dans le journal.
async fn notifier(pool: &PgPool, utilisateur_id: Uuid, message: &str, demande_id: Uuid) {
    if let Err(erreur) = notification::creer(
        pool,
        utilisateur_id,
        TypeNotification::RemboursementStatut,
        TITRE_NOTIFICATION,
        message,
        Some(demande_id),
    )
    .await
    {
        tracing::error!(%demande_id, %erreur, "échec de création de la notification de changement de statut");
    }
}

/// En attente -> En cours.
pub async fn prendre_en_charge(
    pool: &PgPool,
    demande_id: Uuid,
    agent_utilisateur_id: Uuid,
) -> Result<(), ErreurTransition> {
    let mut tx = pool.begin().await?;
    let demande = verrouiller_demande(&mut tx, demande_id).await?;
    exiger_statut(&demande, &[StatutDemande::EnAttente])?;

    sqlx::query("UPDATE demande_remboursement_munaseb SET statut = 'en_cours' WHERE id = $1")
        .bind(demande_id)
        .execute(&mut *tx)
        .await?;
    historiser(&mut tx, demande_id, Some(&demande.statut), StatutDemande::EnCours, Some(agent_utilisateur_id)).await?;
    tx.commit().await?;

    notifier(pool, demande.utilisateur_id, StatutDemande::EnCours.message_notification(), demande_id).await;
    Ok(())
}

/// En cours -> Validé. Le montant remboursé est `montant_demande` plafonné
/// au solde restant de la période de la demande, 0 si ce solde est épuisé
/// (décision T1 a : jamais de rejet pour ce seul motif). Renvoie le montant
/// remboursé (décision V3).
pub async fn valider(
    pool: &PgPool,
    demande_id: Uuid,
    agent_utilisateur_id: Uuid,
) -> Result<i32, ErreurTransition> {
    let mut tx = pool.begin().await?;
    // Ordre des verrous : la demande d'abord...
    let demande = verrouiller_demande(&mut tx, demande_id).await?;
    exiger_statut(&demande, &[StatutDemande::EnCours])?;

    // ... puis sa période (décision C5). Deux validations de demandes de la
    // même période passent l'une après l'autre : le solde lu ci-dessous
    // tient compte de la validation précédente, jamais accordé deux fois.
    sqlx::query("SELECT id FROM periode_adhesion_munaseb WHERE id = $1 FOR UPDATE")
        .bind(demande.periode_id)
        .execute(&mut *tx)
        .await?;

    let solde = solde_periode(&mut *tx, demande.periode_id).await?;
    let montant_rembourse = demande.montant_demande_fcfa.min(solde);

    sqlx::query(
        "UPDATE demande_remboursement_munaseb SET statut = 'valide', montant_rembourse_fcfa = $2 WHERE id = $1",
    )
    .bind(demande_id)
    .bind(montant_rembourse)
    .execute(&mut *tx)
    .await?;
    historiser(&mut tx, demande_id, Some(&demande.statut), StatutDemande::Valide, Some(agent_utilisateur_id)).await?;
    tx.commit().await?;

    let message = if montant_rembourse == 0 {
        MESSAGE_VALIDEE_PLAFOND_ATTEINT
    } else {
        StatutDemande::Valide.message_notification()
    };
    notifier(pool, demande.utilisateur_id, message, demande_id).await;
    Ok(montant_rembourse)
}

/// En attente ou En cours -> Rejeté (décision T2). Le motif est enregistré
/// sur la demande, jamais repris dans la notification.
pub async fn rejeter(
    pool: &PgPool,
    demande_id: Uuid,
    agent_utilisateur_id: Uuid,
    motif: &str,
) -> Result<(), ErreurTransition> {
    let motif = motif.trim();
    if motif.is_empty() {
        return Err(ErreurTransition::MotifVide);
    }
    // En caractères, pas en octets : un accent compte pour un.
    if motif.chars().count() > MOTIF_LONGUEUR_MAX {
        return Err(ErreurTransition::MotifTropLong);
    }

    let mut tx = pool.begin().await?;
    let demande = verrouiller_demande(&mut tx, demande_id).await?;
    exiger_statut(&demande, &[StatutDemande::EnAttente, StatutDemande::EnCours])?;

    sqlx::query("UPDATE demande_remboursement_munaseb SET statut = 'rejete', motif_rejet = $2 WHERE id = $1")
        .bind(demande_id)
        .bind(motif)
        .execute(&mut *tx)
        .await?;
    historiser(&mut tx, demande_id, Some(&demande.statut), StatutDemande::Rejete, Some(agent_utilisateur_id)).await?;
    tx.commit().await?;

    notifier(pool, demande.utilisateur_id, StatutDemande::Rejete.message_notification(), demande_id).await;
    Ok(())
}

/// Validé -> Payé (marquage manuel, sans Mobile Money pour l'instant). Une
/// demande validée à 0 FCFA passe aussi à Payé (décision T3, "dossier
/// clos"), mais sans notification : rien n'est versé.
pub async fn marquer_payee(
    pool: &PgPool,
    demande_id: Uuid,
    agent_utilisateur_id: Uuid,
) -> Result<(), ErreurTransition> {
    let mut tx = pool.begin().await?;
    let demande = verrouiller_demande(&mut tx, demande_id).await?;
    exiger_statut(&demande, &[StatutDemande::Valide])?;

    sqlx::query("UPDATE demande_remboursement_munaseb SET statut = 'paye' WHERE id = $1")
        .bind(demande_id)
        .execute(&mut *tx)
        .await?;
    historiser(&mut tx, demande_id, Some(&demande.statut), StatutDemande::Paye, Some(agent_utilisateur_id)).await?;
    tx.commit().await?;

    // Toujours Some en statut Validé (contrainte montant_rembourse_si_valide_ou_paye).
    if demande.montant_rembourse_fcfa.unwrap_or(0) > 0 {
        notifier(pool, demande.utilisateur_id, StatutDemande::Paye.message_notification(), demande_id).await;
    }
    Ok(())
}

/// Solde restant d'une période : plafond moins la somme des montants
/// Validé et Payé (décisions actées, section 12). Utilisé sous verrou par
/// `valider`, et sans verrou par la lecture du détail (valeur indicative).
async fn solde_periode<'e, E>(executeur: E, periode_id: Uuid) -> Result<i32, sqlx::Error>
where
    E: sqlx::PgExecutor<'e>,
{
    let consomme: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(montant_rembourse_fcfa), 0)::BIGINT FROM demande_remboursement_munaseb \
         WHERE periode_id = $1 AND statut IN ('valide', 'paye')",
    )
    .bind(periode_id)
    .fetch_one(executeur)
    .await?;
    // Le `max(0)` couvre une baisse future du plafond ; le résultat reste
    // <= PLAFOND_ANNUEL_FCFA, il tient dans un i32.
    Ok((i64::from(PLAFOND_ANNUEL_FCFA) - consomme).max(0) as i32)
}

// ---------------------------------------------------------------------
// Lectures pour les agents
// ---------------------------------------------------------------------

/// Une demande telle que l'agent la voit dans la liste. Patient identifié
/// par nom, prénom et numéro de carte MUNASEB, ce que contient le dossier
/// de la mutuelle (décision L2) ; jamais le NIP ni le téléphone, inutiles
/// à l'instruction du dossier.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ResumeDemande {
    pub id: Uuid,
    pub statut: String,
    pub type_acte: String,
    pub date_acte: NaiveDate,
    pub montant_acte_fcfa: i32,
    pub montant_demande_fcfa: i32,
    pub montant_rembourse_fcfa: Option<i32>,
    pub date_demande: DateTime<Utc>,
    pub partenaire: String,
    // Nullables en base (migration 0004 : compte en cours de création).
    pub patient_nom: Option<String>,
    pub patient_prenom: Option<String>,
    pub numero_carte: String,
}

// Macro plutôt que `const` + `format!` : sqlx 0.9 n'accepte que des
// chaînes SQL littérales (trait `SqlSafeStr`, "prefer literal SQL strings
// with bind parameters") ; `concat!` produit une littérale à la
// compilation, aucune donnée ne peut s'y glisser.
macro_rules! select_resume {
    () => {
        "SELECT d.id, d.statut, d.type_acte, d.date_acte, d.montant_acte_fcfa, \
     d.montant_demande_fcfa, d.montant_rembourse_fcfa, d.date_demande, pa.nom AS partenaire, \
     u.nom AS patient_nom, u.prenom AS patient_prenom, c.numero_carte \
     FROM demande_remboursement_munaseb d \
     JOIN partenaire_sante_munaseb pa ON pa.id = d.partenaire_id \
     JOIN periode_adhesion_munaseb pe ON pe.id = d.periode_id \
     JOIN contrat_assurance_munaseb c ON c.id = pe.contrat_id \
     JOIN patient p ON p.id = c.patient_id \
     JOIN utilisateur u ON u.id = p.utilisateur_id"
    };
}

/// Page de demandes, filtrée par statut si demandé, dans l'ordre des
/// identifiants (décision P1). Pagination par curseur (décision L1 b) :
/// `apres` est l'identifiant de la dernière demande de la page précédente.
/// Contrairement à un décalage, une demande qui change de statut entre deux
/// pages ne décale pas la suite.
///
/// Les identifiants sont des `uuidv7()`, « time-ordered » selon la doc
/// PostgreSQL 18 : l'ordre suit l'ordre d'arrivée. La doc ne garantit pas
/// un ordre strict entre deux connexions à la même milliseconde : une
/// demande créée pendant le parcours peut n'apparaître qu'au rechargement
/// (limite acceptée par le porteur).
pub async fn lister_demandes(
    pool: &PgPool,
    statut: Option<StatutDemande>,
    apres: Option<Uuid>,
    limite: i64,
) -> Result<Vec<ResumeDemande>, sqlx::Error> {
    sqlx::query_as::<_, ResumeDemande>(concat!(
        select_resume!(),
        " WHERE ($1::text IS NULL OR d.statut = $1) AND ($2::uuid IS NULL OR d.id > $2) ORDER BY d.id LIMIT $3"
    ))
    .bind(statut.map(StatutDemande::as_str))
    .bind(apres)
    .bind(limite)
    .fetch_all(pool)
    .await
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct LigneHistorique {
    pub statut_precedent: Option<String>,
    pub statut_nouveau: String,
    pub date_changement: DateTime<Utc>,
    /// Nom et prénom de l'agent (décision L4) ; absents pour la création,
    /// qui vient du partenaire.
    pub agent_nom: Option<String>,
    pub agent_prenom: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DetailDemande {
    #[serde(flatten)]
    pub demande: ResumeDemande,
    pub motif_rejet: Option<String>,
    pub periode_debut: NaiveDate,
    pub periode_fin: NaiveDate,
    /// Indicatif (décision L3) : calculé à la lecture, sans verrou. La
    /// validation le recalcule sous verrou.
    pub solde_restant_periode_fcfa: i32,
    pub historique: Vec<LigneHistorique>,
}

/// Détail d'une demande avec son historique complet, `None` si elle
/// n'existe pas.
pub async fn detail_demande(pool: &PgPool, demande_id: Uuid) -> Result<Option<DetailDemande>, sqlx::Error> {
    let Some(demande) = sqlx::query_as::<_, ResumeDemande>(concat!(select_resume!(), " WHERE d.id = $1"))
        .bind(demande_id)
        .fetch_optional(pool)
        .await?
    else {
        return Ok(None);
    };

    let (motif_rejet, periode_id, periode_debut, periode_fin): (Option<String>, Uuid, NaiveDate, NaiveDate) =
        sqlx::query_as(
            "SELECT d.motif_rejet, pe.id, pe.date_debut, pe.date_fin \
             FROM demande_remboursement_munaseb d JOIN periode_adhesion_munaseb pe ON pe.id = d.periode_id \
             WHERE d.id = $1",
        )
        .bind(demande_id)
        .fetch_one(pool)
        .await?;

    let historique = sqlx::query_as::<_, LigneHistorique>(
        "SELECT h.statut_precedent, h.statut_nouveau, h.date_changement, u.nom AS agent_nom, u.prenom AS agent_prenom \
         FROM historique_statut_demande h LEFT JOIN utilisateur u ON u.id = h.agent_utilisateur_id \
         WHERE h.demande_id = $1 ORDER BY h.date_changement, h.id",
    )
    .bind(demande_id)
    .fetch_all(pool)
    .await?;

    Ok(Some(DetailDemande {
        demande,
        motif_rejet,
        periode_debut,
        periode_fin,
        solde_restant_periode_fcfa: solde_periode(pool, periode_id).await?,
        historique,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn taux_arrondi_a_l_entier_le_plus_proche() {
        // 80 % de 1 501 = 1 200,8 -> 1 201.
        assert_eq!(calculer_montant_demande(1501, Tarif::TauxPercent(80)), 1201);
        // 80 % de 1 499 = 1 199,2 -> 1 199.
        assert_eq!(calculer_montant_demande(1499, Tarif::TauxPercent(80)), 1199);
        // Demi exact : 50 % de 3 = 1,5 -> 2.
        assert_eq!(calculer_montant_demande(3, Tarif::TauxPercent(50)), 2);
        assert_eq!(calculer_montant_demande(1000, Tarif::TauxPercent(100)), 1000);
    }

    #[test]
    fn taux_sans_debordement_sur_un_gros_montant() {
        assert_eq!(calculer_montant_demande(i32::MAX, Tarif::TauxPercent(100)), i32::MAX);
    }

    #[test]
    fn forfait_jamais_superieur_au_cout_reel() {
        assert_eq!(calculer_montant_demande(20_000, Tarif::ForfaitFcfa(15_000)), 15_000);
        assert_eq!(calculer_montant_demande(10_000, Tarif::ForfaitFcfa(15_000)), 10_000);
    }

    #[test]
    fn messages_de_notification_sans_information_de_sante() {
        for statut in [
            StatutDemande::EnAttente,
            StatutDemande::EnCours,
            StatutDemande::Valide,
            StatutDemande::Rejete,
            StatutDemande::Paye,
        ] {
            let message = statut.message_notification().to_lowercase();
            for type_acte in TypeActeMunaseb::TOUS {
                assert!(!message.contains(type_acte.as_str()), "{message}");
            }
        }
    }

    /// Complète 15 chiffres par le seul chiffre de contrôle que
    /// `nip::valider` accepte. Préfixe 9 : hors de portée de la séquence
    /// réelle, qui démarre à 1.
    fn nip_valide(payload: &str) -> String {
        (0..=9)
            .map(|d| format!("{payload}{d}"))
            .find(|nip| nip::valider(nip))
            .unwrap()
    }

    fn acte(nip: &str, partenaire_id: Uuid, reference: &str, type_acte: TypeActeMunaseb, montant: i32) -> Acte {
        Acte {
            nip: nip.to_string(),
            partenaire_id,
            type_acte,
            reference_acte: reference.to_string(),
            date_acte: Utc::now().date_naive(),
            montant_acte_fcfa: montant,
        }
    }

    // #[sqlx::test] (doc sqlx 0.9, attr.test) : "a new test database is
    // created" pour ce test, migrations appliquées automatiquement, base
    // supprimée si le test réussit (gardée pour analyse s'il échoue). La
    // base de développement n'est jamais touchée, et aucun nettoyage n'est
    // nécessaire -- indispensable depuis que l'historique des statuts ne
    // peut plus être supprimé (migration 0010). `#[ignore]` : exige un
    // PostgreSQL démarré, comme les autres tests en base.
    #[sqlx::test]
    #[ignore]
    async fn creer_depuis_acte_en_base(pool: PgPool) {

        // La liste de l'enum et les lignes de la grille tarifaire doivent
        // rester identiques.
        let mut types_en_base: Vec<String> =
            sqlx::query_scalar("SELECT type_acte FROM tarif_acte_munaseb").fetch_all(&pool).await.unwrap();
        types_en_base.sort();
        let mut types_du_code: Vec<&str> = TypeActeMunaseb::TOUS.iter().map(|t| t.as_str()).collect();
        types_du_code.sort();
        assert_eq!(types_en_base, types_du_code);

        let aujourdhui = Utc::now().date_naive();
        let nip_couvert = nip_valide("901010101010101");
        let nip_sans_contrat = nip_valide("902020202020202");
        let nip_en_carence = nip_valide("903030303030303");
        let nip_inconnu = nip_valide("909999999999999");
        let nip_couvert = nip_couvert.as_str();
        let nip_sans_contrat = nip_sans_contrat.as_str();
        let nip_en_carence = nip_en_carence.as_str();

        let mut utilisateurs = Vec::new();
        let mut patients = Vec::new();
        for (telephone, nip) in [
            ("+22670010010", nip_couvert),
            ("+22670010011", nip_sans_contrat),
            ("+22670010012", nip_en_carence),
        ] {
            let u: Uuid = sqlx::query_scalar(
                "INSERT INTO utilisateur (nom, prenom, telephone) VALUES ('Test', 'Remboursement', $1) RETURNING id",
            )
            .bind(telephone)
            .fetch_one(&pool)
            .await
            .unwrap();
            let p: Uuid = sqlx::query_scalar(
                "INSERT INTO patient (utilisateur_id, nip, date_naissance, lieu_naissance) \
                 VALUES ($1, $2, '2000-01-01', 'Ouagadougou') RETURNING id",
            )
            .bind(u)
            .bind(nip)
            .fetch_one(&pool)
            .await
            .unwrap();
            utilisateurs.push(u);
            patients.push(p);
        }

        // Périodes posées directement en base : creer/renouveler prennent
        // la date du jour (avec carence), elles ne permettent pas de fixer
        // une période déjà en cours.
        let jours = |n: i64| aujourdhui + chrono::Duration::days(n);
        let debut_couverture_carence = jours(25);
        let mut contrats = Vec::new();
        for (patient_id, carte, paiement, debut, fin) in [
            // Période en cours, payée il y a 30 jours.
            (patients[0], "TEST-REMB-0001", jours(-30), jours(-30), jours(300)),
            // Payée il y a 5 jours, couverture dans 25 jours : en carence.
            (patients[2], "TEST-REMB-0003", jours(-5), debut_couverture_carence, jours(25 + 364)),
        ] {
            let contrat_id: Uuid = sqlx::query_scalar(
                "INSERT INTO contrat_assurance_munaseb (patient_id, numero_carte, ufr, universite, num_matricule) \
                 VALUES ($1, $2, 'UFR', 'Université', 'MAT') RETURNING id",
            )
            .bind(patient_id)
            .bind(carte)
            .fetch_one(&pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO periode_adhesion_munaseb (contrat_id, date_paiement, date_debut, date_fin) \
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(contrat_id)
            .bind(paiement)
            .bind(debut)
            .bind(fin)
            .execute(&pool)
            .await
            .unwrap();
            contrats.push(contrat_id);
        }

        let mut partenaires = Vec::new();
        for (nom, statut) in [("Pharmacie Test Actif", "actif"), ("Pharmacie Test Suspendu", "suspendu")] {
            let id: Uuid = sqlx::query_scalar(
                "INSERT INTO partenaire_sante_munaseb (nom, type, ville, telephone, statut) \
                 VALUES ($1, 'pharmacie', 'Ouagadougou', '+22670010099', $2) RETURNING id",
            )
            .bind(nom)
            .bind(statut)
            .fetch_one(&pool)
            .await
            .unwrap();
            partenaires.push(id);
        }
        let (actif, suspendu) = (partenaires[0], partenaires[1]);

        // --- cas nominal : taux 80 %, arrondi ---
        let pharmacie = acte(nip_couvert, actif, "REF-PHARMA-1", TypeActeMunaseb::Pharmacie, 1501);
        let id = creer_depuis_acte(&pool, &pharmacie).await.unwrap();
        let (statut, montant_demande, montant_rembourse): (String, i32, Option<i32>) = sqlx::query_as(
            "SELECT statut, montant_demande_fcfa, montant_rembourse_fcfa FROM demande_remboursement_munaseb WHERE id = $1",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(statut, "en_attente");
        assert_eq!(montant_demande, 1201);
        assert_eq!(montant_rembourse, None, "aucun plafonnement à la création");

        let (message, lien): (String, Option<Uuid>) = sqlx::query_as(
            "SELECT message, demande_remboursement_id FROM notification WHERE utilisateur_id = $1",
        )
        .bind(utilisateurs[0])
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(message, StatutDemande::EnAttente.message_notification());
        assert_eq!(lien, Some(id));

        // --- rejeu du même acte : même demande, pas de seconde notification ---
        assert_eq!(creer_depuis_acte(&pool, &pharmacie).await.unwrap(), id);
        let nb_notifications: i64 =
            sqlx::query_scalar("SELECT count(*) FROM notification WHERE utilisateur_id = $1")
                .bind(utilisateurs[0])
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(nb_notifications, 1);

        // --- même référence, acte différent : erreur explicite, jamais la
        // demande existante (sinon l'acte réel serait perdu en silence) ---
        let mut autre_montant = pharmacie.clone();
        autre_montant.montant_acte_fcfa = 3000;
        assert!(matches!(
            creer_depuis_acte(&pool, &autre_montant).await,
            Err(ErreurDemandeRemboursement::ReferenceActeDejaUtilisee)
        ));
        let mut autre_type = pharmacie.clone();
        autre_type.type_acte = TypeActeMunaseb::Laboratoire;
        assert!(matches!(
            creer_depuis_acte(&pool, &autre_type).await,
            Err(ErreurDemandeRemboursement::ReferenceActeDejaUtilisee)
        ));
        let mut autre_date = pharmacie.clone();
        autre_date.date_acte = aujourdhui - chrono::Duration::days(1);
        assert!(matches!(
            creer_depuis_acte(&pool, &autre_date).await,
            Err(ErreurDemandeRemboursement::ReferenceActeDejaUtilisee)
        ));
        let mut autre_patient = pharmacie.clone();
        autre_patient.nip = nip_sans_contrat.to_string();
        assert!(matches!(
            creer_depuis_acte(&pool, &autre_patient).await,
            Err(ErreurDemandeRemboursement::ReferenceActeDejaUtilisee)
        ));
        let nb_demandes: i64 =
            sqlx::query_scalar("SELECT count(*) FROM demande_remboursement_munaseb WHERE partenaire_id = $1")
                .bind(actif)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(nb_demandes, 1, "aucune demande ne doit avoir été créée ou modifiée");

        // --- forfait plafonné au forfait ---
        let lunettes = acte(nip_couvert, actif, "REF-LUN-1", TypeActeMunaseb::Lunetterie, 20_000);
        let id_lunettes = creer_depuis_acte(&pool, &lunettes).await.unwrap();
        let montant_lunettes: i32 =
            sqlx::query_scalar("SELECT montant_demande_fcfa FROM demande_remboursement_munaseb WHERE id = $1")
                .bind(id_lunettes)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(montant_lunettes, 15_000);

        // --- erreurs ---
        assert!(matches!(
            creer_depuis_acte(&pool, &acte(nip_couvert, actif, "REF-0", TypeActeMunaseb::Pharmacie, 0)).await,
            Err(ErreurDemandeRemboursement::MontantInvalide)
        ));
        // Faute de frappe sur le dernier chiffre : refusée par Luhn, avant
        // toute requête.
        let mut nip_faute = nip_couvert.to_string();
        let dernier = nip_faute.pop().unwrap().to_digit(10).unwrap();
        nip_faute.push(char::from_digit((dernier + 1) % 10, 10).unwrap());
        assert!(matches!(
            creer_depuis_acte(&pool, &acte(&nip_faute, actif, "REF-X", TypeActeMunaseb::Pharmacie, 1000)).await,
            Err(ErreurDemandeRemboursement::NipInvalide)
        ));
        assert!(matches!(
            creer_depuis_acte(&pool, &acte(&nip_inconnu, actif, "REF-X", TypeActeMunaseb::Pharmacie, 1000)).await,
            Err(ErreurDemandeRemboursement::PatientInconnu)
        ));
        assert!(matches!(
            creer_depuis_acte(&pool, &acte(nip_couvert, Uuid::nil(), "REF-X", TypeActeMunaseb::Pharmacie, 1000)).await,
            Err(ErreurDemandeRemboursement::PartenaireInconnu)
        ));
        assert!(matches!(
            creer_depuis_acte(&pool, &acte(nip_couvert, suspendu, "REF-X", TypeActeMunaseb::Pharmacie, 1000)).await,
            Err(ErreurDemandeRemboursement::PartenaireSuspendu)
        ));
        assert!(matches!(
            creer_depuis_acte(&pool, &acte(nip_sans_contrat, actif, "REF-X", TypeActeMunaseb::Pharmacie, 1000)).await,
            Err(ErreurDemandeRemboursement::NonCouvert)
        ));

        // Acte antérieur au paiement de la seule période : non couvert.
        let mut hors_periode = acte(nip_couvert, actif, "REF-VIEUX", TypeActeMunaseb::Pharmacie, 1000);
        hors_periode.date_acte = jours(-60);
        assert!(matches!(
            creer_depuis_acte(&pool, &hors_periode).await,
            Err(ErreurDemandeRemboursement::NonCouvert)
        ));

        // --- carence : acte après le paiement, avant le début ---
        match creer_depuis_acte(&pool, &acte(nip_en_carence, actif, "REF-CARENCE", TypeActeMunaseb::Pharmacie, 1000)).await {
            Err(ErreurDemandeRemboursement::EnCarence { debut_couverture }) => {
                assert_eq!(debut_couverture, debut_couverture_carence)
            }
            autre => panic!("EnCarence attendu, obtenu {autre:?}"),
        }
        // Acte avant le paiement : jours non couverts, pas de carence.
        let mut avant_paiement = acte(nip_en_carence, actif, "REF-AVANT-PAIEMENT", TypeActeMunaseb::Pharmacie, 1000);
        avant_paiement.date_acte = jours(-10);
        assert!(matches!(
            creer_depuis_acte(&pool, &avant_paiement).await,
            Err(ErreurDemandeRemboursement::NonCouvert)
        ));

        // --- la demande est rattachée à la période de l'acte ---
        let periode_rattachee: Uuid =
            sqlx::query_scalar("SELECT periode_id FROM demande_remboursement_munaseb WHERE id = $1")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap();
        let periode_du_contrat: Uuid =
            sqlx::query_scalar("SELECT id FROM periode_adhesion_munaseb WHERE contrat_id = $1")
                .bind(contrats[0])
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(periode_rattachee, periode_du_contrat);

        sqlx::query("UPDATE contrat_assurance_munaseb SET statut = 'suspendu' WHERE patient_id = $1")
            .bind(patients[0])
            .execute(&pool)
            .await
            .unwrap();
        assert!(matches!(
            creer_depuis_acte(&pool, &acte(nip_couvert, actif, "REF-SUSP", TypeActeMunaseb::Pharmacie, 1000)).await,
            Err(ErreurDemandeRemboursement::NonCouvert)
        ));
        // Le rejeu d'un acte déjà enregistré reste idempotent même après la
        // suspension du contrat.
        assert_eq!(creer_depuis_acte(&pool, &pharmacie).await.unwrap(), id);

        // --- historique non modifiable (migration 0010) ---
        // Ligne posée à la main : l'écriture de l'historique par
        // creer_depuis_acte et les transitions arrive à l'étape suivante.
        let agent: Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (nom, prenom, telephone) VALUES ('Test', 'Agent', '+22670010013') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO historique_statut_demande (demande_id, statut_precedent, statut_nouveau, agent_utilisateur_id) \
             VALUES ($1, 'en_attente', 'en_cours', $2)",
        )
        .bind(id)
        .bind(agent)
        .execute(&pool)
        .await
        .unwrap();
        for requete in [
            "UPDATE historique_statut_demande SET statut_nouveau = 'rejete'",
            "DELETE FROM historique_statut_demande",
            "TRUNCATE historique_statut_demande",
        ] {
            let erreur = sqlx::query(requete).execute(&pool).await.unwrap_err().to_string();
            assert!(erreur.contains("jamais modifiées ni supprimées"), "{requete} : {erreur}");
        }
        // Une transition sans agent est refusée (contrainte agent_si_transition).
        assert!(
            sqlx::query(
                "INSERT INTO historique_statut_demande (demande_id, statut_precedent, statut_nouveau) \
                 VALUES ($1, 'en_cours', 'valide')",
            )
            .bind(id)
            .execute(&pool)
            .await
            .is_err()
        );
        // Une demande qui a un historique ne peut plus être supprimée. Ses
        // notifications sont retirées d'abord : sinon le refus viendrait de
        // leur clé étrangère, et ne prouverait rien sur l'historique.
        sqlx::query("DELETE FROM notification WHERE demande_remboursement_id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
        let erreur = sqlx::query("DELETE FROM demande_remboursement_munaseb WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap_err()
            .to_string();
        assert!(erreur.contains("historique_statut_demande"), "{erreur}");
    }

    /// Patient avec un contrat et une période en cours (posée en base :
    /// `creer` impose une carence). Renvoie (NIP, utilisateur_id du patient).
    async fn patient_couvert(pool: &PgPool, payload_nip: &str, telephone: &str, carte: &str) -> (String, Uuid) {
        let nip = nip_valide(payload_nip);
        let utilisateur_id: Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (nom, prenom, telephone) VALUES ('Test', 'Transitions', $1) RETURNING id",
        )
        .bind(telephone)
        .fetch_one(pool)
        .await
        .unwrap();
        let patient_id: Uuid = sqlx::query_scalar(
            "INSERT INTO patient (utilisateur_id, nip, date_naissance, lieu_naissance) \
             VALUES ($1, $2, '2000-01-01', 'Ouagadougou') RETURNING id",
        )
        .bind(utilisateur_id)
        .bind(&nip)
        .fetch_one(pool)
        .await
        .unwrap();
        let contrat_id: Uuid = sqlx::query_scalar(
            "INSERT INTO contrat_assurance_munaseb (patient_id, numero_carte, ufr, universite, num_matricule) \
             VALUES ($1, $2, 'UFR', 'Université', 'MAT') RETURNING id",
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
        .bind(contrat_id)
        .execute(pool)
        .await
        .unwrap();
        (nip, utilisateur_id)
    }

    async fn messages(pool: &PgPool, demande_id: Uuid) -> Vec<String> {
        sqlx::query_scalar("SELECT message FROM notification WHERE demande_remboursement_id = $1 ORDER BY date_creation, id")
            .bind(demande_id)
            .fetch_all(pool)
            .await
            .unwrap()
    }

    async fn historique(pool: &PgPool, demande_id: Uuid) -> Vec<(Option<String>, String, Option<Uuid>)> {
        sqlx::query_as(
            "SELECT statut_precedent, statut_nouveau, agent_utilisateur_id FROM historique_statut_demande \
             WHERE demande_id = $1 ORDER BY date_changement, id",
        )
        .bind(demande_id)
        .fetch_all(pool)
        .await
        .unwrap()
    }

    // Même principe que le test précédent : base temporaire créée par
    // #[sqlx::test], jamais la base de développement.
    #[sqlx::test]
    #[ignore]
    async fn transitions_en_base(pool: PgPool) {
        let (nip, _) = patient_couvert(&pool, "904040404040404", "+22670012001", "TEST-TR-0001").await;
        let partenaire: Uuid = sqlx::query_scalar(
            "INSERT INTO partenaire_sante_munaseb (nom, type, ville, telephone) \
             VALUES ('Pharmacie Test', 'pharmacie', 'Ouagadougou', '+22670012099') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let agent: Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (nom, prenom, telephone) VALUES ('Test', 'Agent', '+22670012098') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        let creer = |reference: &'static str, montant: i32| {
            let pool = pool.clone();
            let nip = nip.clone();
            async move {
                creer_depuis_acte(&pool, &acte(&nip, partenaire, reference, TypeActeMunaseb::Pharmacie, montant))
                    .await
                    .unwrap()
            }
        };

        // Pharmacie à 80 % : 112 500 -> 90 000 ; 25 000 -> 20 000 ;
        // 1 000 -> 800.
        let a = creer("TR-A", 112_500).await;
        let b = creer("TR-B", 25_000).await;
        let c = creer("TR-C", 1_000).await;
        let d = creer("TR-D", 1_000).await;

        // --- création : ligne d'historique sans statut précédent ni agent ---
        assert_eq!(historique(&pool, a).await, vec![(None, "en_attente".to_string(), None)]);

        // --- transitions interdites ---
        match valider(&pool, a, agent).await {
            Err(ErreurTransition::TransitionInvalide { statut_actuel }) => assert_eq!(statut_actuel, "en_attente"),
            autre => panic!("TransitionInvalide attendu, obtenu {autre:?}"),
        }
        assert!(matches!(marquer_payee(&pool, a, agent).await, Err(ErreurTransition::TransitionInvalide { .. })));
        assert!(matches!(prendre_en_charge(&pool, Uuid::nil(), agent).await, Err(ErreurTransition::DemandeInconnue)));

        // --- plafond : 90 000 puis 10 000 (reste du solde) puis 0 ---
        for demande in [a, b, c] {
            prendre_en_charge(&pool, demande, agent).await.unwrap();
        }
        assert_eq!(valider(&pool, a, agent).await.unwrap(), 90_000);
        assert_eq!(valider(&pool, b, agent).await.unwrap(), 10_000, "plafonné au solde restant");
        assert_eq!(valider(&pool, c, agent).await.unwrap(), 0, "solde épuisé : validé à 0, jamais rejeté");
        assert!(matches!(prendre_en_charge(&pool, a, agent).await, Err(ErreurTransition::TransitionInvalide { .. })));

        // --- textes de notification ---
        assert_eq!(
            messages(&pool, b).await,
            vec![
                StatutDemande::EnAttente.message_notification(),
                StatutDemande::EnCours.message_notification(),
                StatutDemande::Valide.message_notification(),
            ]
        );
        assert_eq!(messages(&pool, c).await.last().unwrap(), MESSAGE_VALIDEE_PLAFOND_ATTEINT);

        // --- paiement : notification si montant > 0, aucune à 0 FCFA ---
        marquer_payee(&pool, b, agent).await.unwrap();
        assert_eq!(messages(&pool, b).await.last().unwrap(), StatutDemande::Paye.message_notification());
        let avant = messages(&pool, c).await.len();
        marquer_payee(&pool, c, agent).await.unwrap();
        assert_eq!(messages(&pool, c).await.len(), avant, "aucune notification pour un paiement de 0 FCFA");

        // --- historique complet d'une demande ---
        assert_eq!(
            historique(&pool, b).await,
            vec![
                (None, "en_attente".to_string(), None),
                (Some("en_attente".to_string()), "en_cours".to_string(), Some(agent)),
                (Some("en_cours".to_string()), "valide".to_string(), Some(agent)),
                (Some("valide".to_string()), "paye".to_string(), Some(agent)),
            ]
        );

        // --- rejet : motif obligatoire, borné, jamais dans la notification ---
        assert!(matches!(rejeter(&pool, d, agent, "   ").await, Err(ErreurTransition::MotifVide)));
        let trop_long = "é".repeat(MOTIF_LONGUEUR_MAX + 1);
        assert!(matches!(rejeter(&pool, d, agent, &trop_long).await, Err(ErreurTransition::MotifTropLong)));
        // Exactement 1 000 caractères accentués (2 000 octets) : accepté.
        let motif = "é".repeat(MOTIF_LONGUEUR_MAX);
        rejeter(&pool, d, agent, &motif).await.unwrap();
        let (statut, motif_enregistre): (String, Option<String>) =
            sqlx::query_as("SELECT statut, motif_rejet FROM demande_remboursement_munaseb WHERE id = $1")
                .bind(d)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!((statut.as_str(), motif_enregistre.as_deref()), ("rejete", Some(motif.as_str())));
        let dernier = messages(&pool, d).await.pop().unwrap();
        assert_eq!(dernier, StatutDemande::Rejete.message_notification());
        assert!(!dernier.contains("éé"), "le motif n'apparaît pas");
    }

    /// Deux validations simultanées de deux demandes de la même période,
    /// avec un solde qui ne suffit que pour une : grâce au verrou sur la
    /// période, la seconde voit la première, et le plafond n'est jamais
    /// dépassé.
    #[sqlx::test]
    #[ignore]
    async fn validations_simultanees_en_base(pool: PgPool) {
        let (nip, _) = patient_couvert(&pool, "905050505050505", "+22670013001", "TEST-TR-0002").await;
        let partenaire: Uuid = sqlx::query_scalar(
            "INSERT INTO partenaire_sante_munaseb (nom, type, ville, telephone) \
             VALUES ('Pharmacie Test', 'pharmacie', 'Ouagadougou', '+22670013099') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let agent: Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (nom, prenom, telephone) VALUES ('Test', 'Agent', '+22670013098') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        // 100 000 x 80 % = 80 000 chacune ; ensemble 160 000 > plafond.
        let mut demandes = Vec::new();
        for reference in ["SIM-1", "SIM-2"] {
            let id = creer_depuis_acte(&pool, &acte(&nip, partenaire, reference, TypeActeMunaseb::Pharmacie, 100_000))
                .await
                .unwrap();
            prendre_en_charge(&pool, id, agent).await.unwrap();
            demandes.push(id);
        }

        let (premier, second) = tokio::join!(valider(&pool, demandes[0], agent), valider(&pool, demandes[1], agent));
        let mut montants = [premier.unwrap(), second.unwrap()];
        montants.sort();
        assert_eq!(montants, [20_000, 80_000]);

        let total: i64 = sqlx::query_scalar(
            "SELECT SUM(montant_rembourse_fcfa)::BIGINT FROM demande_remboursement_munaseb WHERE id = ANY($1)",
        )
        .bind(&demandes)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(total, i64::from(PLAFOND_ANNUEL_FCFA));
    }

    #[sqlx::test]
    #[ignore]
    async fn lectures_en_base(pool: PgPool) {
        let (nip, _) = patient_couvert(&pool, "906060606060606", "+22670014001", "TEST-LEC-0001").await;
        let partenaire: Uuid = sqlx::query_scalar(
            "INSERT INTO partenaire_sante_munaseb (nom, type, ville, telephone) \
             VALUES ('Pharmacie Lecture', 'pharmacie', 'Ouagadougou', '+22670014099') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let agent: Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (nom, prenom, telephone) VALUES ('Ouedraogo', 'Awa', '+22670014098') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        let mut ids = Vec::new();
        for reference in ["LEC-1", "LEC-2", "LEC-3"] {
            ids.push(
                creer_depuis_acte(&pool, &acte(&nip, partenaire, reference, TypeActeMunaseb::Pharmacie, 10_000))
                    .await
                    .unwrap(),
            );
        }
        // Ordre des identifiants = ordre d'arrivée (uuidv7), sur une même
        // connexion ici.
        let mut tries = ids.clone();
        tries.sort();
        assert_eq!(tries, ids);

        // --- pagination par curseur : 2 puis 1 ---
        let page1 = lister_demandes(&pool, Some(StatutDemande::EnAttente), None, 2).await.unwrap();
        assert_eq!(page1.iter().map(|d| d.id).collect::<Vec<_>>(), ids[..2]);
        let page2 = lister_demandes(&pool, Some(StatutDemande::EnAttente), Some(page1[1].id), 2).await.unwrap();
        assert_eq!(page2.iter().map(|d| d.id).collect::<Vec<_>>(), ids[2..]);

        // --- une demande prise en charge ne décale pas la suite (P2) ---
        prendre_en_charge(&pool, ids[0], agent).await.unwrap();
        let apres_page1 = lister_demandes(&pool, Some(StatutDemande::EnAttente), Some(page1[1].id), 2).await.unwrap();
        assert_eq!(apres_page1.iter().map(|d| d.id).collect::<Vec<_>>(), ids[2..]);
        let en_cours = lister_demandes(&pool, Some(StatutDemande::EnCours), None, 50).await.unwrap();
        assert_eq!(en_cours.iter().map(|d| d.id).collect::<Vec<_>>(), [ids[0]]);
        assert_eq!(lister_demandes(&pool, None, None, 50).await.unwrap().len(), 3, "sans filtre : toutes");

        // --- détail : solde, historique, nom de l'agent ---
        valider(&pool, ids[0], agent).await.unwrap();
        let detail = detail_demande(&pool, ids[0]).await.unwrap().unwrap();
        assert_eq!(detail.demande.statut, "valide");
        assert_eq!(detail.demande.montant_rembourse_fcfa, Some(8_000));
        assert_eq!(detail.demande.numero_carte, "TEST-LEC-0001");
        assert_eq!(detail.solde_restant_periode_fcfa, PLAFOND_ANNUEL_FCFA - 8_000);
        let etapes: Vec<_> = detail
            .historique
            .iter()
            .map(|l| (l.statut_nouveau.as_str(), l.agent_prenom.as_deref()))
            .collect();
        assert_eq!(etapes, [("en_attente", None), ("en_cours", Some("Awa")), ("valide", Some("Awa"))]);

        // --- ni NIP ni téléphone dans ce que voit l'agent ---
        let json = serde_json::to_string(&detail).unwrap();
        assert!(!json.contains(&nip), "{json}");
        assert!(!json.contains("+22670014001"), "{json}");

        assert!(detail_demande(&pool, Uuid::nil()).await.unwrap().is_none());
    }
}
