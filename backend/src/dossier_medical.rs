//! Dossier médical (module médecin, étape 4 ; décisions MD12 à MD17 et
//! « vues par rôle » du porteur, 2026-11-17 ; migration 0038).
//!
//! **Accès** : toujours sous une prise en charge **en cours** de
//! l'établissement de l'affectation (`prise_en_charge::en_cours`), jamais
//! par le NIP. Chaque lecture et chaque écriture est écrite dans le journal
//! des accès, dans la même transaction, **avant** la réponse.
//!
//! **Vues par rôle** (filtrage côté serveur, champs absents) :
//! - `medecin`, `infirmier` d'une clinique : **dossier complet** ;
//! - `pharmacien` d'une pharmacie : **vue pharmacie** : identité, allergies,
//!   ordonnances (traitements) et antécédents médicamenteux ; **jamais** les
//!   consultations, les diagnostics ni les examens sensibles ;
//! - tout autre rôle (accueil, auxiliaire, agent MUNASEB, responsable) :
//!   jamais le dossier (403).
//!
//! **Écritures** : allergies et antécédents par le médecin ou l'infirmier ;
//! retrait d'une allergie, consultation, ajout à une consultation et
//! ordonnance par le médecin seulement. Rien ne se modifie ni ne s'efface
//! (CDC, BF-02).
//!
//! **Contrôle d'allergie** (CDC, BF-03 ; MD15, MD16) : une ligne d'un
//! médicament du catalogue dont une DCI est celle d'une allergie active du
//! patient bloque l'ordonnance (409 avec la liste des conflits), sauf si le
//! médecin donne un motif de dépassement, tracé ligne par ligne. Une ligne
//! hors catalogue et une allergie sans DCI ne peuvent pas être comparées :
//! elles sont signalées dans `avertissements`.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sqlx::types::Uuid;
use sqlx::{PgPool, Postgres, Transaction};

use crate::acces_patient::{self, Acteur, ErreurAcces, NatureAcces};
use crate::affectation::Exercice;
use crate::patient::Sexe;
use crate::prise_en_charge;

/// Lignes d'une ordonnance (garde-fou, comme la vente de la pharmacie).
pub const LIGNES_MAX: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VueDossier {
    Complete,
    Pharmacie,
}

/// Vue du rôle, dans son type d'établissement ; `None` : 403.
pub fn vue(exercice: &Exercice) -> Option<VueDossier> {
    match (exercice.role.as_str(), exercice.type_structure.as_str()) {
        ("medecin" | "infirmier", "clinique") => Some(VueDossier::Complete),
        ("pharmacien", "pharmacie") => Some(VueDossier::Pharmacie),
        _ => None,
    }
}

fn medecin(exercice: &Exercice) -> bool {
    exercice.role == "medecin" && exercice.type_structure == "clinique"
}

fn soignant(exercice: &Exercice) -> bool {
    vue(exercice) == Some(VueDossier::Complete)
}

/// Une allergie dont la DCI se trouve dans un médicament prescrit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConflitAllergie {
    pub rang: i16,
    pub medicament_id: Uuid,
    pub allergie_id: Uuid,
    pub substance: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ErreurDossier {
    #[error(transparent)]
    Acces(#[from] ErreurAcces),
    #[error("{0}")]
    DonneesInvalides(String),
    #[error("Allergie inconnue.")]
    AllergieInconnue,
    #[error("Cette allergie est déjà retirée.")]
    AllergieDejaRetiree,
    #[error("Consultation inconnue.")]
    ConsultationInconnue,
    #[error("Médicament inconnu.")]
    MedicamentInconnu,
    /// CDC, BF-03 : alerte bloquante ; le médecin peut passer outre avec un
    /// motif (MD16).
    #[error("Allergie connue du patient pour un médicament prescrit.")]
    AlerteAllergie(Vec<ConflitAllergie>),
}

impl From<sqlx::Error> for ErreurDossier {
    fn from(erreur: sqlx::Error) -> Self {
        ErreurDossier::Acces(ErreurAcces::from(erreur))
    }
}

fn texte(champ: &str, valeur: &str, max: usize) -> Result<String, ErreurDossier> {
    let valeur = valeur.trim();
    if valeur.is_empty() || valeur.chars().count() > max {
        return Err(ErreurDossier::DonneesInvalides(format!(
            "Le champ « {champ} » est obligatoire, {max} caractères au plus."
        )));
    }
    Ok(valeur.to_string())
}

fn texte_facultatif(
    champ: &str,
    valeur: Option<&str>,
    max: usize,
) -> Result<Option<String>, ErreurDossier> {
    match valeur.map(str::trim) {
        None | Some("") => Ok(None),
        Some(v) => texte(champ, v, max).map(Some),
    }
}

/// Ouvre la transaction d'un accès au dossier : rôle, prise en charge en
/// cours de l'établissement. Rend (transaction, acteur, patient).
async fn ouvrir_acces<'a>(
    pool: &'a PgPool,
    exercice: &Exercice,
    prise_en_charge_id: Uuid,
    autorise: bool,
) -> Result<(Transaction<'a, Postgres>, Acteur, Uuid), ErreurDossier> {
    if !autorise {
        return Err(ErreurAcces::RoleNonAutorise.into());
    }
    let acteur = Acteur::from(exercice);
    let mut tx = pool.begin().await?;
    let (patient_id, _) = prise_en_charge::en_cours(&mut tx, &acteur, prise_en_charge_id).await?;
    Ok((tx, acteur, patient_id))
}

async fn journaliser(
    tx: &mut Transaction<'_, Postgres>,
    acteur: &Acteur,
    patient_id: Uuid,
    nature: NatureAcces,
    prise_en_charge_id: Uuid,
) -> Result<(), sqlx::Error> {
    acces_patient::ecrire_journal(
        tx,
        acteur,
        patient_id,
        nature,
        false,
        Some(prise_en_charge_id),
    )
    .await
}

// ---------------------------------------------------------------------
// Lecture : vues par rôle
// ---------------------------------------------------------------------

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Allergie {
    pub id: Uuid,
    pub substance: String,
    pub dci_id: Option<Uuid>,
    pub dci_nom: Option<String>,
    pub reaction: Option<String>,
    pub date_saisie: DateTime<Utc>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct AllergieRetiree {
    pub id: Uuid,
    pub substance: String,
    pub dci_id: Option<Uuid>,
    pub reaction: Option<String>,
    pub date_saisie: DateTime<Utc>,
    pub motif_retrait: String,
    pub date_retrait: DateTime<Utc>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Antecedent {
    pub id: Uuid,
    pub type_antecedent: String,
    pub description: String,
    pub date_debut: Option<NaiveDate>,
    pub date_saisie: DateTime<Utc>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct AjoutConsultation {
    pub texte: String,
    pub date_ajout: DateTime<Utc>,
    pub medecin_nom: Option<String>,
    pub medecin_prenom: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Consultation {
    pub id: Uuid,
    pub motif: String,
    pub examen: Option<String>,
    pub diagnostic: Option<String>,
    pub date_consultation: DateTime<Utc>,
    pub structure_nom: String,
    pub medecin_nom: Option<String>,
    pub medecin_prenom: Option<String>,
    pub ajouts: Vec<AjoutConsultation>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct LigneOrdonnance {
    #[serde(skip)]
    pub id: Uuid,
    pub rang: i16,
    pub medicament_id: Option<Uuid>,
    /// Dénomination du catalogue, ou nom saisi hors catalogue.
    pub denomination: String,
    pub forme: Option<String>,
    pub dosage: Option<String>,
    pub hors_catalogue: bool,
    pub posologie: String,
    pub quantite: i32,
    pub duree_jours: Option<i16>,
    /// Prescrit malgré une alerte d'allergie (MD16).
    pub alerte_allergie_depassee: bool,
}

#[derive(Debug, Serialize)]
pub struct Ordonnance {
    pub id: Uuid,
    /// Absente de la vue pharmacie (jamais de lien vers une consultation).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consultation_id: Option<Uuid>,
    /// Toujours rendue (MD17 : aucune durée de validité imposée).
    pub date_emission: DateTime<Utc>,
    pub structure_nom: String,
    pub medecin_nom: Option<String>,
    pub medecin_prenom: Option<String>,
    pub lignes: Vec<LigneOrdonnance>,
}

async fn identite(
    tx: &mut Transaction<'_, Postgres>,
    patient_id: Uuid,
) -> Result<Map<String, Value>, sqlx::Error> {
    type Ligne = (String, String, Option<String>, NaiveDate, String);
    let (nom, prenom, sexe, date_naissance, lieu_naissance): Ligne = sqlx::query_as(
        "SELECT u.nom, u.prenom, p.sexe, p.date_naissance, p.lieu_naissance \
         FROM patient p JOIN utilisateur u ON u.id = p.utilisateur_id WHERE p.id = $1",
    )
    .bind(patient_id)
    .fetch_one(&mut **tx)
    .await?;
    let mut fiche = Map::new();
    fiche.insert("nom".into(), json!(nom));
    fiche.insert("prenom".into(), json!(prenom));
    fiche.insert("sexe".into(), json!(Sexe::depuis_base(sexe.as_deref())));
    fiche.insert("date_naissance".into(), json!(date_naissance));
    fiche.insert("lieu_naissance".into(), json!(lieu_naissance));
    Ok(fiche)
}

async fn allergies_actives(
    tx: &mut Transaction<'_, Postgres>,
    patient_id: Uuid,
) -> Result<Vec<Allergie>, sqlx::Error> {
    sqlx::query_as(
        "SELECT a.id, a.substance, a.dci_id, d.nom AS dci_nom, a.reaction, a.date_saisie \
         FROM allergie a LEFT JOIN dci d ON d.id = a.dci_id \
         WHERE a.patient_id = $1 AND NOT EXISTS (SELECT 1 FROM retrait_allergie r WHERE r.allergie_id = a.id) \
         ORDER BY a.id",
    )
    .bind(patient_id)
    .fetch_all(&mut **tx)
    .await
}

async fn antecedents(
    tx: &mut Transaction<'_, Postgres>,
    patient_id: Uuid,
    seulement_medicamenteux: bool,
) -> Result<Vec<Antecedent>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, type_antecedent, description, date_debut, date_saisie FROM antecedent \
         WHERE patient_id = $1 AND (NOT $2 OR type_antecedent = 'medicamenteux') ORDER BY id",
    )
    .bind(patient_id)
    .bind(seulement_medicamenteux)
    .fetch_all(&mut **tx)
    .await
}

/// Ordonnances du patient, les plus récentes d'abord. `avec_consultation` :
/// le lien vers la consultation n'est rendu que dans la vue complète.
async fn ordonnances(
    tx: &mut Transaction<'_, Postgres>,
    patient_id: Uuid,
    seulement: Option<Uuid>,
    avec_consultation: bool,
) -> Result<Vec<Ordonnance>, sqlx::Error> {
    type Entete = (
        Uuid,
        Option<Uuid>,
        DateTime<Utc>,
        String,
        Option<String>,
        Option<String>,
    );
    let entetes: Vec<Entete> = sqlx::query_as(
        "SELECT o.id, o.consultation_id, o.date_emission, s.nom, u.nom, u.prenom \
         FROM ordonnance o JOIN structure s ON s.id = o.structure_id \
         JOIN affectation a ON a.id = o.affectation_id JOIN compte c ON c.id = a.compte_id \
         JOIN utilisateur u ON u.id = c.utilisateur_id \
         WHERE o.patient_id = $1 AND ($2::uuid IS NULL OR o.id = $2) ORDER BY o.id DESC",
    )
    .bind(patient_id)
    .bind(seulement)
    .fetch_all(&mut **tx)
    .await?;
    let mut resultat = Vec::with_capacity(entetes.len());
    for (id, consultation_id, date_emission, structure_nom, medecin_nom, medecin_prenom) in entetes
    {
        let lignes: Vec<LigneOrdonnance> = sqlx::query_as(
            "SELECT l.id, l.rang, l.medicament_id, COALESCE(m.denomination, l.denomination_libre) AS denomination, \
                    m.forme, m.dosage, l.medicament_id IS NULL AS hors_catalogue, l.posologie, l.quantite, l.duree_jours, \
                    EXISTS (SELECT 1 FROM depassement_alerte_allergie d WHERE d.ligne_id = l.id) AS alerte_allergie_depassee \
             FROM ligne_ordonnance l LEFT JOIN medicament m ON m.id = l.medicament_id \
             WHERE l.ordonnance_id = $1 ORDER BY l.rang",
        )
        .bind(id)
        .fetch_all(&mut **tx)
        .await?;
        resultat.push(Ordonnance {
            id,
            consultation_id: consultation_id.filter(|_| avec_consultation),
            date_emission,
            structure_nom,
            medecin_nom,
            medecin_prenom,
            lignes,
        });
    }
    Ok(resultat)
}

async fn consultations(
    tx: &mut Transaction<'_, Postgres>,
    patient_id: Uuid,
) -> Result<Vec<Consultation>, sqlx::Error> {
    type Ligne = (
        Uuid,
        String,
        Option<String>,
        Option<String>,
        DateTime<Utc>,
        String,
        Option<String>,
        Option<String>,
    );
    let lignes: Vec<Ligne> = sqlx::query_as(
        "SELECT co.id, co.motif, co.examen, co.diagnostic, co.date_consultation, s.nom, u.nom, u.prenom \
         FROM consultation co JOIN structure s ON s.id = co.structure_id \
         JOIN affectation a ON a.id = co.affectation_id JOIN compte c ON c.id = a.compte_id \
         JOIN utilisateur u ON u.id = c.utilisateur_id \
         WHERE co.patient_id = $1 ORDER BY co.id DESC",
    )
    .bind(patient_id)
    .fetch_all(&mut **tx)
    .await?;
    let mut resultat = Vec::with_capacity(lignes.len());
    for (id, motif, examen, diagnostic, date_consultation, structure_nom, nom, prenom) in lignes {
        let ajouts: Vec<AjoutConsultation> = sqlx::query_as(
            "SELECT aj.texte, aj.date_ajout, u.nom AS medecin_nom, u.prenom AS medecin_prenom \
             FROM ajout_consultation aj JOIN affectation a ON a.id = aj.affectation_id \
             JOIN compte c ON c.id = a.compte_id JOIN utilisateur u ON u.id = c.utilisateur_id \
             WHERE aj.consultation_id = $1 ORDER BY aj.id",
        )
        .bind(id)
        .fetch_all(&mut **tx)
        .await?;
        resultat.push(Consultation {
            id,
            motif,
            examen,
            diagnostic,
            date_consultation,
            structure_nom,
            medecin_nom: nom,
            medecin_prenom: prenom,
            ajouts,
        });
    }
    Ok(resultat)
}

fn valeur<T: Serialize>(objet: T) -> Result<Value, ErreurDossier> {
    serde_json::to_value(objet)
        .map_err(|e| ErreurDossier::Acces(ErreurAcces::Interne(e.to_string())))
}

/// Le dossier, dans la vue du rôle. La réponse est construite champ par
/// champ : ce que la vue ne prévoit pas n'y entre jamais. Les allergies
/// viennent en premier (règle du modèle : affichées en priorité).
pub async fn lire(
    pool: &PgPool,
    exercice: &Exercice,
    prise_en_charge_id: Uuid,
) -> Result<Map<String, Value>, ErreurDossier> {
    let vue = vue(exercice);
    let (mut tx, acteur, patient_id) =
        ouvrir_acces(pool, exercice, prise_en_charge_id, vue.is_some()).await?;
    journaliser(
        &mut tx,
        &acteur,
        patient_id,
        NatureAcces::LectureDossier,
        prise_en_charge_id,
    )
    .await?;
    let mut dossier = Map::new();
    dossier.insert(
        "allergies".into(),
        valeur(allergies_actives(&mut tx, patient_id).await?)?,
    );
    dossier.insert(
        "patient".into(),
        Value::Object(identite(&mut tx, patient_id).await?),
    );
    match vue {
        Some(VueDossier::Complete) => {
            let retirees: Vec<AllergieRetiree> = sqlx::query_as(
                "SELECT a.id, a.substance, a.dci_id, a.reaction, a.date_saisie, r.motif AS motif_retrait, r.date_retrait \
                 FROM allergie a JOIN retrait_allergie r ON r.allergie_id = a.id WHERE a.patient_id = $1 ORDER BY a.id",
            )
            .bind(patient_id)
            .fetch_all(&mut *tx)
            .await?;
            dossier.insert("allergies_retirees".into(), valeur(retirees)?);
            dossier.insert(
                "antecedents".into(),
                valeur(antecedents(&mut tx, patient_id, false).await?)?,
            );
            dossier.insert(
                "consultations".into(),
                valeur(consultations(&mut tx, patient_id).await?)?,
            );
            dossier.insert(
                "ordonnances".into(),
                valeur(ordonnances(&mut tx, patient_id, None, true).await?)?,
            );
        }
        Some(VueDossier::Pharmacie) => {
            dossier.insert(
                "antecedents_medicamenteux".into(),
                valeur(antecedents(&mut tx, patient_id, true).await?)?,
            );
            dossier.insert(
                "ordonnances".into(),
                valeur(ordonnances(&mut tx, patient_id, None, false).await?)?,
            );
        }
        // Déjà refusé par `ouvrir_acces` ; jamais de panique dans le serveur.
        None => return Err(ErreurAcces::RoleNonAutorise.into()),
    }
    // Le journal est validé avant que le dossier ne quitte le serveur.
    tx.commit().await?;
    Ok(dossier)
}

// ---------------------------------------------------------------------
// Écritures
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct NouvelleAllergie {
    pub substance: String,
    pub dci_id: Option<Uuid>,
    pub reaction: Option<String>,
}

/// Allergie déclarée par un médecin ou un infirmier (MD12).
pub async fn ajouter_allergie(
    pool: &PgPool,
    exercice: &Exercice,
    prise_en_charge_id: Uuid,
    saisie: &NouvelleAllergie,
) -> Result<Allergie, ErreurDossier> {
    let substance = texte("substance", &saisie.substance, 200)?;
    let reaction = texte_facultatif("reaction", saisie.reaction.as_deref(), 1000)?;
    let (mut tx, acteur, patient_id) =
        ouvrir_acces(pool, exercice, prise_en_charge_id, soignant(exercice)).await?;
    if let Some(dci_id) = saisie.dci_id {
        let existe: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM dci WHERE id = $1)")
            .bind(dci_id)
            .fetch_one(&mut *tx)
            .await?;
        if !existe {
            return Err(ErreurDossier::DonneesInvalides("DCI inconnue.".into()));
        }
    }
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO allergie (patient_id, substance, dci_id, reaction, prise_en_charge_id, affectation_id, structure_id, role) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING id",
    )
    .bind(patient_id)
    .bind(&substance)
    .bind(saisie.dci_id)
    .bind(&reaction)
    .bind(prise_en_charge_id)
    .bind(acteur.affectation_id)
    .bind(acteur.structure_id)
    .bind(&acteur.role)
    .fetch_one(&mut *tx)
    .await?;
    journaliser(
        &mut tx,
        &acteur,
        patient_id,
        NatureAcces::EcritureDossier,
        prise_en_charge_id,
    )
    .await?;
    let allergie: Allergie = sqlx::query_as(
        "SELECT a.id, a.substance, a.dci_id, d.nom AS dci_nom, a.reaction, a.date_saisie \
         FROM allergie a LEFT JOIN dci d ON d.id = a.dci_id WHERE a.id = $1",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(allergie)
}

/// Retrait d'une allergie du patient (désactivation réversible : la
/// redéclarer crée une nouvelle ligne), par un médecin, avec un motif.
pub async fn retirer_allergie(
    pool: &PgPool,
    exercice: &Exercice,
    prise_en_charge_id: Uuid,
    allergie_id: Uuid,
    motif: &str,
) -> Result<(), ErreurDossier> {
    let motif = texte("motif", motif, 1000)?;
    let (mut tx, acteur, patient_id) =
        ouvrir_acces(pool, exercice, prise_en_charge_id, medecin(exercice)).await?;
    let etat: Option<bool> = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM retrait_allergie r WHERE r.allergie_id = a.id) \
         FROM allergie a WHERE a.id = $1 AND a.patient_id = $2",
    )
    .bind(allergie_id)
    .bind(patient_id)
    .fetch_optional(&mut *tx)
    .await?;
    match etat {
        None => return Err(ErreurDossier::AllergieInconnue),
        Some(true) => return Err(ErreurDossier::AllergieDejaRetiree),
        Some(false) => {}
    }
    sqlx::query(
        "INSERT INTO retrait_allergie (allergie_id, motif, prise_en_charge_id, affectation_id, structure_id, role) \
         VALUES ($1, $2, $3, $4, $5, 'medecin')",
    )
    .bind(allergie_id)
    .bind(&motif)
    .bind(prise_en_charge_id)
    .bind(acteur.affectation_id)
    .bind(acteur.structure_id)
    .execute(&mut *tx)
    .await?;
    journaliser(
        &mut tx,
        &acteur,
        patient_id,
        NatureAcces::EcritureDossier,
        prise_en_charge_id,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Types d'antécédent (liste fermée proposée, CHECK de la migration 0038).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TypeAntecedent {
    Medical,
    Chirurgical,
    Familial,
    Medicamenteux,
}

impl TypeAntecedent {
    fn as_str(self) -> &'static str {
        match self {
            TypeAntecedent::Medical => "medical",
            TypeAntecedent::Chirurgical => "chirurgical",
            TypeAntecedent::Familial => "familial",
            TypeAntecedent::Medicamenteux => "medicamenteux",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct NouvelAntecedent {
    pub type_antecedent: TypeAntecedent,
    pub description: String,
    pub date_debut: Option<NaiveDate>,
}

pub async fn ajouter_antecedent(
    pool: &PgPool,
    exercice: &Exercice,
    prise_en_charge_id: Uuid,
    saisie: &NouvelAntecedent,
) -> Result<Antecedent, ErreurDossier> {
    let description = texte("description", &saisie.description, 2000)?;
    let (mut tx, acteur, patient_id) =
        ouvrir_acces(pool, exercice, prise_en_charge_id, soignant(exercice)).await?;
    let antecedent: Antecedent = sqlx::query_as(
        "INSERT INTO antecedent (patient_id, type_antecedent, description, date_debut, prise_en_charge_id, affectation_id, structure_id, role) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
         RETURNING id, type_antecedent, description, date_debut, date_saisie",
    )
    .bind(patient_id)
    .bind(saisie.type_antecedent.as_str())
    .bind(&description)
    .bind(saisie.date_debut)
    .bind(prise_en_charge_id)
    .bind(acteur.affectation_id)
    .bind(acteur.structure_id)
    .bind(&acteur.role)
    .fetch_one(&mut *tx)
    .await?;
    journaliser(
        &mut tx,
        &acteur,
        patient_id,
        NatureAcces::EcritureDossier,
        prise_en_charge_id,
    )
    .await?;
    tx.commit().await?;
    Ok(antecedent)
}

#[derive(Debug, Clone, Deserialize)]
pub struct NouvelleConsultation {
    pub motif: String,
    pub examen: Option<String>,
    pub diagnostic: Option<String>,
}

/// Consultation par un médecin (MD13 : diagnostic en texte libre). Rend
/// son identifiant et sa date.
pub async fn creer_consultation(
    pool: &PgPool,
    exercice: &Exercice,
    prise_en_charge_id: Uuid,
    saisie: &NouvelleConsultation,
) -> Result<(Uuid, DateTime<Utc>), ErreurDossier> {
    let motif = texte("motif", &saisie.motif, 1000)?;
    let examen = texte_facultatif("examen", saisie.examen.as_deref(), 5000)?;
    let diagnostic = texte_facultatif("diagnostic", saisie.diagnostic.as_deref(), 2000)?;
    let (mut tx, acteur, patient_id) =
        ouvrir_acces(pool, exercice, prise_en_charge_id, medecin(exercice)).await?;
    let cree: (Uuid, DateTime<Utc>) = sqlx::query_as(
        "INSERT INTO consultation (patient_id, motif, examen, diagnostic, prise_en_charge_id, affectation_id, structure_id, role) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'medecin') RETURNING id, date_consultation",
    )
    .bind(patient_id)
    .bind(&motif)
    .bind(&examen)
    .bind(&diagnostic)
    .bind(prise_en_charge_id)
    .bind(acteur.affectation_id)
    .bind(acteur.structure_id)
    .fetch_one(&mut *tx)
    .await?;
    journaliser(
        &mut tx,
        &acteur,
        patient_id,
        NatureAcces::EcritureDossier,
        prise_en_charge_id,
    )
    .await?;
    tx.commit().await?;
    Ok(cree)
}

/// Ajout à une consultation du patient (MD14 : une consultation ne se
/// modifie jamais), par un médecin.
pub async fn ajouter_a_consultation(
    pool: &PgPool,
    exercice: &Exercice,
    prise_en_charge_id: Uuid,
    consultation_id: Uuid,
    texte_ajout: &str,
) -> Result<(), ErreurDossier> {
    let texte_ajout = texte("texte", texte_ajout, 2000)?;
    let (mut tx, acteur, patient_id) =
        ouvrir_acces(pool, exercice, prise_en_charge_id, medecin(exercice)).await?;
    let existe: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM consultation WHERE id = $1 AND patient_id = $2)",
    )
    .bind(consultation_id)
    .bind(patient_id)
    .fetch_one(&mut *tx)
    .await?;
    if !existe {
        return Err(ErreurDossier::ConsultationInconnue);
    }
    sqlx::query(
        "INSERT INTO ajout_consultation (consultation_id, texte, prise_en_charge_id, affectation_id, structure_id, role) \
         VALUES ($1, $2, $3, $4, $5, 'medecin')",
    )
    .bind(consultation_id)
    .bind(&texte_ajout)
    .bind(prise_en_charge_id)
    .bind(acteur.affectation_id)
    .bind(acteur.structure_id)
    .execute(&mut *tx)
    .await?;
    journaliser(
        &mut tx,
        &acteur,
        patient_id,
        NatureAcces::EcritureDossier,
        prise_en_charge_id,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

// ---------------------------------------------------------------------
// Ordonnance et contrôle d'allergie
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct NouvelleLigne {
    /// Un médicament du catalogue national…
    pub medicament_id: Option<Uuid>,
    /// … ou un nom hors catalogue (MD15 : permis, marqué, sans contrôle
    /// d'allergie possible).
    pub denomination: Option<String>,
    pub posologie: String,
    pub quantite: i32,
    pub duree_jours: Option<i16>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NouvelleOrdonnance {
    pub consultation_id: Option<Uuid>,
    pub lignes: Vec<NouvelleLigne>,
    /// Motif pour prescrire malgré une alerte d'allergie (MD16).
    pub depassement_allergie_motif: Option<String>,
}

/// Ce qui n'a pas pu être contrôlé : lignes hors catalogue, allergies
/// sans DCI (texte libre).
#[derive(Debug, Default, Serialize)]
pub struct Avertissements {
    pub lignes_hors_catalogue: Vec<i16>,
    pub allergies_non_comparees: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct OrdonnanceCreee {
    #[serde(flatten)]
    pub ordonnance: Ordonnance,
    pub avertissements: Avertissements,
}

/// Ordonnance par un médecin. Ordre : contrôles de la saisie (422), rôle
/// et prise en charge, consultation du patient, médicaments du catalogue,
/// conflits d'allergie (409 sans motif, la lecture des allergies étant
/// écrite au journal), puis l'ordonnance, ses lignes, les dépassements et
/// le journal.
pub async fn prescrire(
    pool: &PgPool,
    exercice: &Exercice,
    prise_en_charge_id: Uuid,
    saisie: &NouvelleOrdonnance,
) -> Result<OrdonnanceCreee, ErreurDossier> {
    if saisie.lignes.is_empty() || saisie.lignes.len() > LIGNES_MAX {
        return Err(ErreurDossier::DonneesInvalides(format!(
            "lignes : entre 1 et {LIGNES_MAX}."
        )));
    }
    let mut lignes = Vec::with_capacity(saisie.lignes.len());
    for (i, ligne) in saisie.lignes.iter().enumerate() {
        let rang = (i + 1) as i16;
        let denomination = texte_facultatif("denomination", ligne.denomination.as_deref(), 200)?;
        if ligne.medicament_id.is_some() == denomination.is_some() {
            return Err(ErreurDossier::DonneesInvalides(format!(
                "ligne {rang} : donnez soit « medicament_id », soit « denomination »."
            )));
        }
        let posologie = texte("posologie", &ligne.posologie, 500)?;
        if !(1..=10_000).contains(&ligne.quantite)
            || ligne.duree_jours.is_some_and(|d| !(1..=365).contains(&d))
        {
            return Err(ErreurDossier::DonneesInvalides(format!(
                "ligne {rang} : quantité de 1 à 10 000, durée de 1 à 365 jours."
            )));
        }
        lignes.push((rang, ligne.medicament_id, denomination, posologie, ligne));
    }
    let motif = texte_facultatif(
        "depassement_allergie_motif",
        saisie.depassement_allergie_motif.as_deref(),
        1000,
    )?;

    let (mut tx, acteur, patient_id) =
        ouvrir_acces(pool, exercice, prise_en_charge_id, medecin(exercice)).await?;
    if let Some(consultation_id) = saisie.consultation_id {
        let existe: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM consultation WHERE id = $1 AND patient_id = $2)",
        )
        .bind(consultation_id)
        .bind(patient_id)
        .fetch_one(&mut *tx)
        .await?;
        if !existe {
            return Err(ErreurDossier::ConsultationInconnue);
        }
    }
    let medicaments: Vec<Uuid> = lignes.iter().filter_map(|l| l.1).collect();
    let connus: i64 =
        sqlx::query_scalar("SELECT count(DISTINCT id) FROM medicament WHERE id = ANY($1)")
            .bind(&medicaments)
            .fetch_one(&mut *tx)
            .await?;
    let distincts = medicaments
        .iter()
        .collect::<std::collections::BTreeSet<_>>()
        .len() as i64;
    if connus != distincts {
        return Err(ErreurDossier::MedicamentInconnu);
    }

    // Conflits : allergie active du patient dont la DCI est une des DCI d'un
    // médicament prescrit (comparaison par DCI, MD12 et MD16).
    let paires: Vec<(Uuid, Uuid, String)> = sqlx::query_as(
        "SELECT md.medicament_id, a.id, a.substance \
         FROM allergie a JOIN medicament_dci md ON md.dci_id = a.dci_id \
         WHERE a.patient_id = $1 AND md.medicament_id = ANY($2) \
           AND NOT EXISTS (SELECT 1 FROM retrait_allergie r WHERE r.allergie_id = a.id) \
         ORDER BY a.id",
    )
    .bind(patient_id)
    .bind(&medicaments)
    .fetch_all(&mut *tx)
    .await?;
    let conflits: Vec<ConflitAllergie> = lignes
        .iter()
        .filter_map(|l| l.1.map(|m| (l.0, m)))
        .flat_map(|(rang, medicament_id)| {
            paires
                .iter()
                .filter(move |(m, _, _)| *m == medicament_id)
                .map(move |(_, allergie_id, substance)| ConflitAllergie {
                    rang,
                    medicament_id,
                    allergie_id: *allergie_id,
                    substance: substance.clone(),
                })
        })
        .collect();
    if !conflits.is_empty() && motif.is_none() {
        // L'alerte montre des allergies du dossier : c'est une lecture,
        // écrite au journal et validée avant la réponse.
        journaliser(
            &mut tx,
            &acteur,
            patient_id,
            NatureAcces::LectureDossier,
            prise_en_charge_id,
        )
        .await?;
        tx.commit().await?;
        return Err(ErreurDossier::AlerteAllergie(conflits));
    }

    let ordonnance_id: Uuid = sqlx::query_scalar(
        "INSERT INTO ordonnance (patient_id, consultation_id, prise_en_charge_id, affectation_id, structure_id, role) \
         VALUES ($1, $2, $3, $4, $5, 'medecin') RETURNING id",
    )
    .bind(patient_id)
    .bind(saisie.consultation_id)
    .bind(prise_en_charge_id)
    .bind(acteur.affectation_id)
    .bind(acteur.structure_id)
    .fetch_one(&mut *tx)
    .await?;
    for (rang, medicament_id, denomination, posologie, ligne) in &lignes {
        let ligne_id: Uuid = sqlx::query_scalar(
            "INSERT INTO ligne_ordonnance (ordonnance_id, rang, medicament_id, denomination_libre, posologie, quantite, duree_jours) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id",
        )
        .bind(ordonnance_id)
        .bind(rang)
        .bind(medicament_id)
        .bind(denomination)
        .bind(posologie)
        .bind(ligne.quantite)
        .bind(ligne.duree_jours)
        .fetch_one(&mut *tx)
        .await?;
        for conflit in conflits.iter().filter(|c| c.rang == *rang) {
            sqlx::query(
                "INSERT INTO depassement_alerte_allergie (ordonnance_id, ligne_id, allergie_id, motif) VALUES ($1, $2, $3, $4)",
            )
            .bind(ordonnance_id)
            .bind(ligne_id)
            .bind(conflit.allergie_id)
            .bind(&motif)
            .execute(&mut *tx)
            .await?;
        }
    }
    let allergies_non_comparees: Vec<String> = sqlx::query_scalar(
        "SELECT substance FROM allergie a WHERE a.patient_id = $1 AND a.dci_id IS NULL \
           AND NOT EXISTS (SELECT 1 FROM retrait_allergie r WHERE r.allergie_id = a.id) ORDER BY a.id",
    )
    .bind(patient_id)
    .fetch_all(&mut *tx)
    .await?;
    journaliser(
        &mut tx,
        &acteur,
        patient_id,
        NatureAcces::EcritureDossier,
        prise_en_charge_id,
    )
    .await?;
    let ordonnance = ordonnances(&mut tx, patient_id, Some(ordonnance_id), true)
        .await?
        .pop()
        .ok_or_else(|| ErreurAcces::Interne("ordonnance créée introuvable".into()))?;
    tx.commit().await?;
    Ok(OrdonnanceCreee {
        avertissements: Avertissements {
            lignes_hors_catalogue: lignes
                .iter()
                .filter(|l| l.1.is_none())
                .map(|l| l.0)
                .collect(),
            allergies_non_comparees,
        },
        ordonnance,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::fixtures_medicales::{compte_pro, exercice, patient, structure};
    use crate::prise_en_charge::tests::prise_terminee;
    use crate::prise_en_charge::{self as pec, Preuve};

    const CLE: [u8; 32] = [9; 32];

    /// Catalogue fictif : X et Y de molécule A, Z de molécule B. Rend
    /// (X, Z, DCI de A).
    async fn catalogue(pool: &PgPool) -> (Uuid, Uuid, Uuid) {
        let admin: Uuid = sqlx::query_scalar(
            "WITH u AS (INSERT INTO utilisateur (nom, prenom, telephone) VALUES ('ADMIN', 'Fictif', '+22670409999') RETURNING id) \
             INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) SELECT id, 'administrateur_laaficare', 'x' FROM u RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        let fichier = "denomination;dci;forme;dosage\n\
                       Produit X fictif;Molécule-A fictive;comprimé;500 mg\n\
                       Produit Y fictif;Molécule-A fictive;sirop;250 mg\n\
                       Produit Z fictif;Molécule-B fictive;comprimé;100 mg\n";
        crate::catalogue::importer(pool, admin, "Liste fictive", fichier.as_bytes())
            .await
            .unwrap();
        let id = |sql: &'static str| async move {
            sqlx::query_scalar::<_, Uuid>(sql)
                .fetch_one(pool)
                .await
                .unwrap()
        };
        (
            id("SELECT id FROM medicament WHERE denomination = 'Produit X fictif'").await,
            id("SELECT id FROM medicament WHERE denomination = 'Produit Z fictif'").await,
            id("SELECT id FROM dci WHERE nom = 'Molécule-A fictive'").await,
        )
    }

    fn ligne(medicament_id: Option<Uuid>, denomination: Option<&str>) -> NouvelleLigne {
        NouvelleLigne {
            medicament_id,
            denomination: denomination.map(str::to_string),
            posologie: "1 comprimé matin et soir".into(),
            quantite: 10,
            duree_jours: Some(5),
        }
    }

    fn cles(dossier: &Map<String, Value>) -> BTreeSet<&str> {
        dossier.keys().map(String::as_str).collect()
    }

    /// Clés présentes à toute profondeur.
    fn toutes_les_cles(valeur: &Value, cles: &mut BTreeSet<String>) {
        match valeur {
            Value::Object(m) => {
                for (k, v) in m {
                    cles.insert(k.clone());
                    toutes_les_cles(v, cles);
                }
            }
            Value::Array(a) => a.iter().for_each(|v| toutes_les_cles(v, cles)),
            _ => {}
        }
    }

    async fn lectures(pool: &PgPool, patient_id: Uuid) -> i64 {
        sqlx::query_scalar(
            "SELECT count(*) FROM journal_acces WHERE patient_id = $1 AND nature = 'lecture_dossier'",
        )
        .bind(patient_id)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// Vues par rôle : dossier complet pour le médecin et l'infirmier ; vue
    /// pharmacie sans consultation, diagnostic ni examen ; refus de
    /// l'accueil, de l'auxiliaire et de l'agent MUNASEB. Chaque lecture au
    /// journal.
    #[sqlx::test]
    #[ignore]
    async fn vues_par_role(pool: PgPool) {
        let (x, _, dci_a) = catalogue(&pool).await;
        let p = patient(&pool, "+22670400001").await;
        let clinique = structure(&pool, "clinique", "Clinique fictive").await;
        let pharmacie = structure(&pool, "pharmacie", "Pharmacie fictive").await;
        let munaseb = structure(&pool, "munaseb", "MUNASEB").await;
        let medecin = exercice(&pool, clinique, "medecin").await;
        let infirmier = exercice(&pool, clinique, "infirmier").await;
        let prise = pec::ouvrir(&pool, &CLE, &medecin, &p.nip, Preuve::CnibVerifiee)
            .await
            .unwrap();

        ajouter_allergie(
            &pool,
            &infirmier,
            prise.id,
            &NouvelleAllergie {
                substance: "Molécule-A".into(),
                dci_id: Some(dci_a),
                reaction: Some("Éruption".into()),
            },
        )
        .await
        .unwrap();
        for (type_antecedent, description) in [
            (TypeAntecedent::Medical, "Antécédent médical fictif"),
            (TypeAntecedent::Medicamenteux, "Traitement ancien fictif"),
        ] {
            ajouter_antecedent(
                &pool,
                &medecin,
                prise.id,
                &NouvelAntecedent {
                    type_antecedent,
                    description: description.into(),
                    date_debut: None,
                },
            )
            .await
            .unwrap();
        }
        let (consultation_id, _) = creer_consultation(
            &pool,
            &medecin,
            prise.id,
            &NouvelleConsultation {
                motif: "Fièvre fictive".into(),
                examen: Some("Examen clinique fictif".into()),
                diagnostic: Some("Diagnostic fictif".into()),
            },
        )
        .await
        .unwrap();
        prescrire(
            &pool,
            &medecin,
            prise.id,
            &NouvelleOrdonnance {
                consultation_id: Some(consultation_id),
                lignes: vec![ligne(None, Some("Produit hors catalogue fictif"))],
                depassement_allergie_motif: None,
            },
        )
        .await
        .unwrap();

        let complet = [
            "allergies",
            "patient",
            "allergies_retirees",
            "antecedents",
            "consultations",
            "ordonnances",
        ]
        .into_iter()
        .collect::<BTreeSet<_>>();
        for ex in [&medecin, &infirmier] {
            let dossier = lire(&pool, ex, prise.id).await.unwrap();
            assert_eq!(cles(&dossier), complet);
            assert_eq!(dossier.keys().next().map(String::as_str), Some("allergies"));
            assert_eq!(
                dossier["consultations"][0]["diagnostic"],
                "Diagnostic fictif"
            );
            assert_eq!(
                dossier["ordonnances"][0]["consultation_id"],
                consultation_id.to_string()
            );
        }

        // Vue pharmacie : prise en charge ouverte par la pharmacie.
        let pharmacien = exercice(&pool, pharmacie, "pharmacien").await;
        let prise_pharmacie = pec::ouvrir(&pool, &CLE, &pharmacien, &p.nip, Preuve::CnibVerifiee)
            .await
            .unwrap();
        let vue = lire(&pool, &pharmacien, prise_pharmacie.id).await.unwrap();
        assert_eq!(
            cles(&vue),
            [
                "allergies",
                "patient",
                "antecedents_medicamenteux",
                "ordonnances"
            ]
            .into_iter()
            .collect()
        );
        let json = Value::Object(vue.clone());
        let mut toutes = BTreeSet::new();
        toutes_les_cles(&json, &mut toutes);
        for interdit in [
            "consultations",
            "consultation_id",
            "diagnostic",
            "examen",
            "motif",
        ] {
            assert!(
                !toutes.contains(interdit),
                "{interdit} montré au pharmacien"
            );
        }
        let texte = json.to_string();
        for interdit in [
            "Diagnostic fictif",
            "Examen clinique fictif",
            "Fièvre fictive",
            "Antécédent médical fictif",
        ] {
            assert!(!texte.contains(interdit), "{interdit} montré au pharmacien");
        }
        assert_eq!(vue["allergies"][0]["substance"], "Molécule-A");
        assert_eq!(
            vue["antecedents_medicamenteux"].as_array().map(Vec::len),
            Some(1)
        );
        assert_eq!(vue["ordonnances"][0]["lignes"][0]["hors_catalogue"], true);

        // Jamais le dossier : accueil, auxiliaire, agent MUNASEB, responsable.
        for (ex, prise_id) in [
            (exercice(&pool, clinique, "agent_accueil").await, prise.id),
            (exercice(&pool, clinique, "responsable").await, prise.id),
            (
                exercice(&pool, pharmacie, "auxiliaire_pharmacie").await,
                prise_pharmacie.id,
            ),
            (
                exercice(&pool, munaseb, "agent_assurance_munaseb").await,
                prise.id,
            ),
        ] {
            assert!(
                matches!(
                    lire(&pool, &ex, prise_id).await,
                    Err(ErreurDossier::Acces(ErreurAcces::RoleNonAutorise))
                ),
                "{}",
                ex.role
            );
        }
        // Le pharmacien ne prescrit pas et n'ajoute rien.
        assert!(matches!(
            ajouter_allergie(
                &pool,
                &pharmacien,
                prise_pharmacie.id,
                &NouvelleAllergie {
                    substance: "x".into(),
                    dci_id: None,
                    reaction: None
                }
            )
            .await,
            Err(ErreurDossier::Acces(ErreurAcces::RoleNonAutorise))
        ));
        assert_eq!(lectures(&pool, p.patient_id).await, 3);
        assert!(x != Uuid::nil());
    }

    /// Le dossier ne s'ouvre que sous une prise en charge en cours de son
    /// établissement : autre établissement, prise en charge terminée ou
    /// inconnue, refusés (lecture et écriture) ; rien n'est écrit au journal.
    #[sqlx::test]
    #[ignore]
    async fn hors_prise_en_charge(pool: PgPool) {
        let p = patient(&pool, "+22670400002").await;
        let clinique = structure(&pool, "clinique", "Clinique fictive").await;
        let autre = structure(&pool, "clinique", "Autre clinique").await;
        let medecin = exercice(&pool, clinique, "medecin").await;
        let medecin_ailleurs = exercice(&pool, autre, "medecin").await;
        let prise = pec::ouvrir(&pool, &CLE, &medecin, &p.nip, Preuve::CnibVerifiee)
            .await
            .unwrap();
        let terminee = prise_terminee(&pool, &medecin, p.patient_id).await;

        assert!(matches!(
            lire(&pool, &medecin_ailleurs, prise.id).await,
            Err(ErreurDossier::Acces(ErreurAcces::PriseEnChargeInconnue))
        ));
        assert!(matches!(
            lire(&pool, &medecin, Uuid::max()).await,
            Err(ErreurDossier::Acces(ErreurAcces::PriseEnChargeInconnue))
        ));
        assert!(matches!(
            lire(&pool, &medecin, terminee).await,
            Err(ErreurDossier::Acces(ErreurAcces::PriseEnChargeExpiree))
        ));
        assert!(matches!(
            creer_consultation(
                &pool,
                &medecin_ailleurs,
                prise.id,
                &NouvelleConsultation {
                    motif: "x".into(),
                    examen: None,
                    diagnostic: None
                }
            )
            .await,
            Err(ErreurDossier::Acces(ErreurAcces::PriseEnChargeInconnue))
        ));
        assert_eq!(lectures(&pool, p.patient_id).await, 0);
        // Un compte quelconque n'y change rien.
        let _ = compte_pro(&pool).await;
    }

    /// Contrôle d'allergie (BF-03, MD15, MD16) : alerte bloquante par DCI,
    /// dépassement avec motif tracé, avertissements, retrait d'allergie ;
    /// droits et immuabilité.
    #[sqlx::test]
    #[ignore]
    async fn ordonnance_et_allergie(pool: PgPool) {
        let (x, z, dci_a) = catalogue(&pool).await;
        let p = patient(&pool, "+22670400003").await;
        let autre_patient = patient(&pool, "+22670400004").await;
        let clinique = structure(&pool, "clinique", "Clinique fictive").await;
        let medecin = exercice(&pool, clinique, "medecin").await;
        let infirmier = exercice(&pool, clinique, "infirmier").await;
        let prise = pec::ouvrir(&pool, &CLE, &medecin, &p.nip, Preuve::CnibVerifiee)
            .await
            .unwrap();
        let prise_autre = pec::ouvrir(
            &pool,
            &CLE,
            &medecin,
            &autre_patient.nip,
            Preuve::CnibVerifiee,
        )
        .await
        .unwrap();
        let allergie = ajouter_allergie(
            &pool,
            &medecin,
            prise.id,
            &NouvelleAllergie {
                substance: "Molécule-A".into(),
                dci_id: Some(dci_a),
                reaction: None,
            },
        )
        .await
        .unwrap();
        ajouter_allergie(
            &pool,
            &infirmier,
            prise.id,
            &NouvelleAllergie {
                substance: "Arachide".into(),
                dci_id: None,
                reaction: None,
            },
        )
        .await
        .unwrap();
        let ordonnance = |lignes: Vec<NouvelleLigne>, motif: Option<&str>| NouvelleOrdonnance {
            consultation_id: None,
            lignes,
            depassement_allergie_motif: motif.map(str::to_string),
        };
        let compter = |requete: &'static str| {
            let pool = pool.clone();
            async move {
                sqlx::query_scalar::<_, i64>(requete)
                    .fetch_one(&pool)
                    .await
                    .unwrap()
            }
        };

        // Alerte bloquante, rien n'est créé, la lecture est tracée.
        match prescrire(
            &pool,
            &medecin,
            prise.id,
            &ordonnance(vec![ligne(Some(z), None), ligne(Some(x), None)], None),
        )
        .await
        {
            Err(ErreurDossier::AlerteAllergie(conflits)) => assert_eq!(
                conflits,
                [ConflitAllergie {
                    rang: 2,
                    medicament_id: x,
                    allergie_id: allergie.id,
                    substance: "Molécule-A".into()
                }]
            ),
            autre => panic!("{autre:?}"),
        }
        assert_eq!(compter("SELECT count(*) FROM ordonnance").await, 0);
        assert_eq!(lectures(&pool, p.patient_id).await, 1);

        // Dépassement avec motif : créé, tracé ligne par ligne.
        let creee = prescrire(
            &pool,
            &medecin,
            prise.id,
            &ordonnance(
                vec![
                    ligne(Some(x), None),
                    ligne(None, Some("Hors catalogue fictif")),
                ],
                Some("Bénéfice supérieur au risque, surveillance"),
            ),
        )
        .await
        .unwrap();
        assert!(creee.ordonnance.lignes[0].alerte_allergie_depassee);
        assert!(!creee.ordonnance.lignes[1].alerte_allergie_depassee);
        assert_eq!(creee.avertissements.lignes_hors_catalogue, [2]);
        assert_eq!(creee.avertissements.allergies_non_comparees, ["Arachide"]);
        assert_eq!(
            compter("SELECT count(*) FROM depassement_alerte_allergie").await,
            1
        );
        // L'autre patient n'a pas cette allergie : aucune alerte.
        prescrire(
            &pool,
            &medecin,
            prise_autre.id,
            &ordonnance(vec![ligne(Some(x), None)], None),
        )
        .await
        .unwrap();

        // Retrait de l'allergie par un médecin : plus d'alerte.
        assert!(matches!(
            retirer_allergie(&pool, &infirmier, prise.id, allergie.id, "Erreur").await,
            Err(ErreurDossier::Acces(ErreurAcces::RoleNonAutorise))
        ));
        assert!(matches!(
            retirer_allergie(&pool, &medecin, prise_autre.id, allergie.id, "Erreur").await,
            Err(ErreurDossier::AllergieInconnue)
        ));
        retirer_allergie(
            &pool,
            &medecin,
            prise.id,
            allergie.id,
            "Allergie écartée par test fictif",
        )
        .await
        .unwrap();
        assert!(matches!(
            retirer_allergie(&pool, &medecin, prise.id, allergie.id, "Encore").await,
            Err(ErreurDossier::AllergieDejaRetiree)
        ));
        prescrire(
            &pool,
            &medecin,
            prise.id,
            &ordonnance(vec![ligne(Some(x), None)], None),
        )
        .await
        .unwrap();
        let dossier = lire(&pool, &medecin, prise.id).await.unwrap();
        assert_eq!(dossier["allergies"].as_array().map(Vec::len), Some(1));
        assert_eq!(dossier["allergies_retirees"][0]["substance"], "Molécule-A");

        // Saisies refusées ; l'infirmier ne prescrit pas.
        assert!(matches!(
            prescrire(
                &pool,
                &infirmier,
                prise.id,
                &ordonnance(vec![ligne(Some(z), None)], None)
            )
            .await,
            Err(ErreurDossier::Acces(ErreurAcces::RoleNonAutorise))
        ));
        for lignes in [
            vec![],
            vec![ligne(Some(z), Some("Les deux"))],
            vec![ligne(None, None)],
        ] {
            assert!(matches!(
                prescrire(&pool, &medecin, prise.id, &ordonnance(lignes, None)).await,
                Err(ErreurDossier::DonneesInvalides(_))
            ));
        }
        assert!(matches!(
            prescrire(
                &pool,
                &medecin,
                prise.id,
                &ordonnance(vec![ligne(Some(Uuid::max()), None)], None)
            )
            .await,
            Err(ErreurDossier::MedicamentInconnu)
        ));

        // Consultation : ajout du même patient seulement.
        let (consultation_id, _) = creer_consultation(
            &pool,
            &medecin,
            prise.id,
            &NouvelleConsultation {
                motif: "Contrôle fictif".into(),
                examen: None,
                diagnostic: None,
            },
        )
        .await
        .unwrap();
        ajouter_a_consultation(
            &pool,
            &medecin,
            prise.id,
            consultation_id,
            "Correction fictive",
        )
        .await
        .unwrap();
        assert!(matches!(
            ajouter_a_consultation(&pool, &medecin, prise_autre.id, consultation_id, "x").await,
            Err(ErreurDossier::ConsultationInconnue)
        ));
        assert!(matches!(
            creer_consultation(
                &pool,
                &infirmier,
                prise.id,
                &NouvelleConsultation {
                    motif: "x".into(),
                    examen: None,
                    diagnostic: None
                }
            )
            .await,
            Err(ErreurDossier::Acces(ErreurAcces::RoleNonAutorise))
        ));

        // Rien ne se modifie ni ne s'efface.
        for sql in [
            "UPDATE consultation SET diagnostic = 'autre'",
            "DELETE FROM allergie",
            "UPDATE ligne_ordonnance SET quantite = 1",
            "DELETE FROM ordonnance",
            "DELETE FROM retrait_allergie",
            "DELETE FROM ajout_consultation",
            "DELETE FROM depassement_alerte_allergie",
        ] {
            assert!(sqlx::query(sql).execute(&pool).await.is_err(), "{sql}");
        }
    }
}
