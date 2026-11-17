//! Accès aux données d'un patient par un professionnel (module médecin,
//! étape 2 ; décisions du porteur du 2026-11-17, « vues par rôle » ;
//! migration 0036).
//!
//! - **Journal des accès** : chaque scan du NIP, chaque lecture du dossier
//!   et chaque affichage du téléphone est écrit **dans la même
//!   transaction, avant** que les données soient renvoyées. Si l'écriture
//!   ou la validation de la transaction échoue, rien n'est montré. Le
//!   journal n'est jamais modifié ni effacé (triggers et droits, migration
//!   0036). Le patient le lit (`journal_du_patient`).
//! - **Scan du NIP** : il n'ouvre **jamais** le dossier. Il renvoie une
//!   fiche d'identité dont les champs dépendent du rôle de l'affectation en
//!   exercice (loi n° 001-2021/AN : données « adéquates, pertinentes et
//!   non excessives » ; principe de minimisation, docs/points-ouverts.md).
//!   Le filtrage est fait ici, côté serveur : un champ que le rôle ne doit
//!   pas voir est **absent** de la réponse.
//! - **Limites** : 5 essais sans résultat par affectation en 15 minutes
//!   (valeur du verrouillage BF-01, comme l'adhésion MUNASEB ; OWASP
//!   *Authentication Cheat Sheet*, « Protect Against Automated Attacks »),
//!   et au plus `SCANS_MAX` scans réussis par affectation dans la même
//!   fenêtre (OWASP API Security Top 10 2023, API4 *Unrestricted Resource
//!   Consumption* et API3 : limiter la collecte en masse). Au-delà : 429
//!   avec le délai d'attente.
//! - **NIP inconnu et NIP mal formé** : exactement la même réponse
//!   (OWASP API3:2023) ; les deux comptent comme un échec.

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use serde_json::{Map, Value, json};
use sqlx::types::Uuid;
use sqlx::{PgPool, Postgres, Transaction};

use crate::affectation::Exercice;
use crate::demande_remboursement_munaseb::AffectationAgent;
use crate::espace_patient_munaseb;
use crate::nip;
use crate::patient::Sexe;

/// Essais sans résultat permis par affectation dans la fenêtre (valeur du
/// verrouillage BF-01, 5 en 15 minutes, déjà reprise pour l'adhésion
/// MUNASEB ; aucune source n'en donne une autre).
pub const ECHECS_MAX: i64 = 5;
pub const FENETRE_ECHECS_MINUTES: i32 = 15;

/// Scans réussis permis par affectation dans une fenêtre glissante de
/// 15 minutes : **60**, soit un patient toutes les 15 secondes pendant un
/// quart d'heure. **Valeur proposée, sans source chiffrée** (OWASP API4:2023
/// demande une limite sans en donner) : un accueil ne peut pas recevoir
/// plus vite ; au-delà, c'est une collecte automatique. À valider par le
/// porteur (docs/points-ouverts.md).
pub const SCANS_MAX: i64 = 60;
pub const FENETRE_SCANS_MINUTES: i32 = 15;

/// Nature d'une ligne du journal (CHECK de la migration 0036).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NatureAcces {
    Scan,
    DetailContrat,
    RechercheIdentite,
    OuverturePriseEnCharge,
    ProlongationPriseEnCharge,
    BrisDeGlace,
    LectureDossier,
    EcritureDossier,
}

impl NatureAcces {
    pub fn as_str(self) -> &'static str {
        match self {
            NatureAcces::Scan => "scan",
            NatureAcces::DetailContrat => "detail_contrat",
            NatureAcces::RechercheIdentite => "recherche_identite",
            NatureAcces::OuverturePriseEnCharge => "ouverture_prise_en_charge",
            NatureAcces::ProlongationPriseEnCharge => "prolongation_prise_en_charge",
            NatureAcces::BrisDeGlace => "bris_de_glace",
            NatureAcces::LectureDossier => "lecture_dossier",
            NatureAcces::EcritureDossier => "ecriture_dossier",
        }
    }
}

/// Nature d'un essai sans résultat (CHECK de la migration 0036).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NatureEchec {
    Scan,
    RechercheIdentite,
    CodeSoin,
}

impl NatureEchec {
    fn as_str(self) -> &'static str {
        match self {
            NatureEchec::Scan => "scan",
            NatureEchec::RechercheIdentite => "recherche_identite",
            NatureEchec::CodeSoin => "code_soin",
        }
    }
}

/// Celui qui agit : l'affectation en exercice (qui, avec quel rôle, dans
/// quel établissement).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Acteur {
    pub affectation_id: Uuid,
    pub structure_id: Uuid,
    pub role: String,
}

impl From<&Exercice> for Acteur {
    fn from(exercice: &Exercice) -> Self {
        Acteur {
            affectation_id: exercice.affectation_id,
            structure_id: exercice.structure_id,
            role: exercice.role.clone(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ErreurAcces {
    /// Rôle ou type d'établissement sans droit sur cette action : 403.
    #[error("accès refusé")]
    RoleNonAutorise,
    /// NIP inconnu, NIP mal formé, identité sans résultat unique : jamais
    /// distingués.
    #[error("Aucun patient ne correspond.")]
    PatientInconnu,
    #[error("Trop d'essais. Réessayez plus tard.")]
    TropDEssais { reessayer_dans_secs: u64 },
    #[error("erreur interne : {0}")]
    Interne(String),
}

impl From<sqlx::Error> for ErreurAcces {
    fn from(erreur: sqlx::Error) -> Self {
        ErreurAcces::Interne(erreur.to_string())
    }
}

// ---------------------------------------------------------------------
// Briques communes (aussi pour la prise en charge et le dossier)
// ---------------------------------------------------------------------

/// Écrit une ligne du journal dans la transaction de l'appelant. L'appelant
/// ne renvoie les données qu'**après** la validation de cette transaction.
pub async fn ecrire_journal(
    tx: &mut Transaction<'_, Postgres>,
    acteur: &Acteur,
    patient_id: Uuid,
    nature: NatureAcces,
    telephone_affiche: bool,
    prise_en_charge_id: Option<Uuid>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO journal_acces (patient_id, affectation_id, structure_id, role, nature, telephone_affiche, prise_en_charge_id) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(patient_id)
    .bind(acteur.affectation_id)
    .bind(acteur.structure_id)
    .bind(&acteur.role)
    .bind(nature.as_str())
    .bind(telephone_affiche)
    .bind(prise_en_charge_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Trace d'un essai sans résultat, **validée avant la réponse** : sinon
/// l'annulation de la transaction l'effacerait et la limite ne compterait
/// rien (même règle que l'adhésion MUNASEB).
pub async fn enregistrer_echec(
    mut tx: Transaction<'_, Postgres>,
    acteur: &Acteur,
    nature: NatureEchec,
    patient_id: Option<Uuid>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO echec_acces_patient (affectation_id, structure_id, role, nature, patient_id) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(acteur.affectation_id)
    .bind(acteur.structure_id)
    .bind(&acteur.role)
    .bind(nature.as_str())
    .bind(patient_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

/// Nombre de lignes dans la fenêtre et secondes avant que la plus ancienne
/// en sorte (horloge de PostgreSQL ; au moins 1 s).
fn trop(nombre: i64, attente: Option<i64>, max: i64) -> Result<(), ErreurAcces> {
    if nombre >= max {
        return Err(ErreurAcces::TropDEssais {
            reessayer_dans_secs: attente.unwrap_or(1).max(1) as u64,
        });
    }
    Ok(())
}

/// Verrouille l'affectation (les essais simultanés d'un même acteur passent
/// l'un après l'autre : la limite ne peut pas être dépassée), puis contrôle
/// ses essais sans résultat de cette nature dans la fenêtre.
pub async fn controler_echecs(
    tx: &mut Transaction<'_, Postgres>,
    acteur: &Acteur,
    nature: NatureEchec,
) -> Result<(), ErreurAcces> {
    sqlx::query("SELECT id FROM affectation WHERE id = $1 FOR UPDATE")
        .bind(acteur.affectation_id)
        .fetch_one(&mut **tx)
        .await?;
    let (nombre, attente): (i64, Option<i64>) = sqlx::query_as(
        "SELECT count(*), \
                ceil(extract(epoch FROM min(date_echec) + make_interval(mins => $3) - now()))::bigint \
         FROM echec_acces_patient \
         WHERE affectation_id = $1 AND nature = $2 AND date_echec > now() - make_interval(mins => $3)",
    )
    .bind(acteur.affectation_id)
    .bind(nature.as_str())
    .bind(FENETRE_ECHECS_MINUTES)
    .fetch_one(&mut **tx)
    .await?;
    trop(nombre, attente, ECHECS_MAX)
}

/// Patient du NIP, `None` pour un NIP mal formé ou inconnu (jamais
/// distingués). Une identité sans nom (inscription inachevée) n'est pas un
/// patient.
pub async fn patient_du_nip(
    tx: &mut Transaction<'_, Postgres>,
    nip_saisi: &str,
) -> Result<Option<Uuid>, sqlx::Error> {
    let nip_saisi = nip_saisi.trim();
    if !nip::valider(nip_saisi) {
        return Ok(None);
    }
    sqlx::query_scalar(
        "SELECT p.id FROM patient p JOIN utilisateur u ON u.id = p.utilisateur_id \
         WHERE p.nip = $1 AND u.nom IS NOT NULL AND u.prenom IS NOT NULL",
    )
    .bind(nip_saisi)
    .fetch_optional(&mut **tx)
    .await
}

// ---------------------------------------------------------------------
// Scan du NIP : fiche d'identité par rôle
// ---------------------------------------------------------------------

/// Ce que chaque rôle voit au scan (décisions du porteur du 2026-11-17).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VueScan {
    /// `agent_accueil` (fait aussi la caisse) : nom, prénom, sexe, date et
    /// lieu de naissance.
    Accueil,
    /// `medecin`, `infirmier` : même fiche que l'accueil.
    Soignant,
    /// `agent_assurance_munaseb` : la même, **plus le téléphone**, pour
    /// enregistrer sans erreur d'identité et rappeler l'adhérent.
    AgentMunaseb,
    /// `pharmacien` : la même, plus l'état des droits MUNASEB (D12).
    Pharmacien,
    /// `auxiliaire_pharmacie` : nom, prénom, état des droits MUNASEB.
    Auxiliaire,
}

/// Vue du rôle de l'affectation, dans son type d'établissement ; `None` :
/// 403 (tout autre rôle, responsable compris, ou un rôle hors de son type
/// d'établissement).
pub fn vue_scan(exercice: &Exercice) -> Option<VueScan> {
    match (exercice.role.as_str(), exercice.type_structure.as_str()) {
        ("agent_accueil", "clinique") => Some(VueScan::Accueil),
        ("medecin" | "infirmier", "clinique") => Some(VueScan::Soignant),
        ("agent_assurance_munaseb", "munaseb") => Some(VueScan::AgentMunaseb),
        ("pharmacien", "pharmacie") => Some(VueScan::Pharmacien),
        ("auxiliaire_pharmacie", "pharmacie") => Some(VueScan::Auxiliaire),
        _ => None,
    }
}

/// Identité complète lue en base ; seule une partie sort, selon la vue.
struct Identite {
    nom: String,
    prenom: String,
    sexe: Option<Sexe>,
    date_naissance: NaiveDate,
    lieu_naissance: String,
    telephone: String,
}

async fn identite(
    tx: &mut Transaction<'_, Postgres>,
    patient_id: Uuid,
) -> Result<Identite, sqlx::Error> {
    type Ligne = (String, String, Option<String>, NaiveDate, String, String);
    let (nom, prenom, sexe, date_naissance, lieu_naissance, telephone): Ligne = sqlx::query_as(
        "SELECT u.nom, u.prenom, p.sexe, p.date_naissance, p.lieu_naissance, u.telephone \
         FROM patient p JOIN utilisateur u ON u.id = p.utilisateur_id WHERE p.id = $1",
    )
    .bind(patient_id)
    .fetch_one(&mut **tx)
    .await?;
    Ok(Identite {
        nom,
        prenom,
        sexe: Sexe::depuis_base(sexe.as_deref()),
        date_naissance,
        lieu_naissance,
        telephone,
    })
}

/// La fiche, construite champ par champ : un champ que la vue ne prévoit
/// pas n'est jamais ajouté. `droits` : état des droits MUNASEB, pour les
/// vues de la pharmacie seulement.
fn fiche(vue: VueScan, identite: &Identite, droits: Option<&str>) -> Map<String, Value> {
    let mut fiche = Map::new();
    fiche.insert("nom".into(), json!(identite.nom));
    fiche.insert("prenom".into(), json!(identite.prenom));
    if vue != VueScan::Auxiliaire {
        fiche.insert("sexe".into(), json!(identite.sexe));
        fiche.insert("date_naissance".into(), json!(identite.date_naissance));
        fiche.insert("lieu_naissance".into(), json!(identite.lieu_naissance));
    }
    if vue == VueScan::AgentMunaseb {
        fiche.insert("telephone".into(), json!(identite.telephone));
    }
    if matches!(vue, VueScan::Pharmacien | VueScan::Auxiliaire) {
        fiche.insert("droits_munaseb".into(), json!({ "etat": droits }));
    }
    fiche
}

/// Scan du NIP. Ordre : rôle (403 avant toute lecture), verrou de
/// l'affectation, limites (429), patient (404, échec tracé), journal,
/// lecture, validation, réponse.
pub async fn scanner(
    pool: &PgPool,
    exercice: &Exercice,
    nip_saisi: &str,
) -> Result<Map<String, Value>, ErreurAcces> {
    let vue = vue_scan(exercice).ok_or(ErreurAcces::RoleNonAutorise)?;
    let acteur = Acteur::from(exercice);
    let mut tx = pool.begin().await?;
    controler_echecs(&mut tx, &acteur, NatureEchec::Scan).await?;
    let (scans, attente): (i64, Option<i64>) = sqlx::query_as(
        "SELECT count(*), \
                ceil(extract(epoch FROM min(date_acces) + make_interval(mins => $2) - now()))::bigint \
         FROM journal_acces \
         WHERE affectation_id = $1 AND nature = 'scan' AND date_acces > now() - make_interval(mins => $2)",
    )
    .bind(acteur.affectation_id)
    .bind(FENETRE_SCANS_MINUTES)
    .fetch_one(&mut *tx)
    .await?;
    trop(scans, attente, SCANS_MAX)?;

    let Some(patient_id) = patient_du_nip(&mut tx, nip_saisi).await? else {
        enregistrer_echec(tx, &acteur, NatureEchec::Scan, None).await?;
        return Err(ErreurAcces::PatientInconnu);
    };
    ecrire_journal(
        &mut tx,
        &acteur,
        patient_id,
        NatureAcces::Scan,
        vue == VueScan::AgentMunaseb,
        None,
    )
    .await?;
    let identite = identite(&mut tx, patient_id).await?;
    let droits = if matches!(vue, VueScan::Pharmacien | VueScan::Auxiliaire) {
        let droits = espace_patient_munaseb::droits_par_patient(pool, patient_id)
            .await
            .map_err(|e| ErreurAcces::Interne(e.to_string()))?;
        Some(
            serde_json::to_value(droits.etat)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .ok_or_else(|| ErreurAcces::Interne("état des droits illisible".into()))?,
        )
    } else {
        None
    };
    // Le journal est validé avant que la fiche ne quitte le serveur.
    tx.commit().await?;
    Ok(fiche(vue, &identite, droits.as_deref()))
}

// ---------------------------------------------------------------------
// Téléphone de l'adhérent dans le détail d'un contrat MUNASEB
// ---------------------------------------------------------------------

/// Téléphone de l'adhérent d'un contrat, pour l'agent (décision du porteur
/// du 2026-11-17, qui remplace « l'agent ne voit jamais le téléphone ») :
/// chaque affichage est écrit dans le journal, validé avant la réponse.
/// `None` si le contrat n'existe pas.
pub async fn telephone_pour_agent(
    pool: &PgPool,
    agent: &AffectationAgent,
    contrat_id: Uuid,
) -> Result<Option<String>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let ligne: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT p.id, u.telephone FROM contrat_assurance_munaseb c \
         JOIN patient p ON p.id = c.patient_id JOIN utilisateur u ON u.id = p.utilisateur_id \
         WHERE c.id = $1",
    )
    .bind(contrat_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((patient_id, telephone)) = ligne else {
        return Ok(None);
    };
    let acteur = Acteur {
        affectation_id: agent.affectation_id,
        structure_id: agent.structure_id,
        role: "agent_assurance_munaseb".into(),
    };
    ecrire_journal(
        &mut tx,
        &acteur,
        patient_id,
        NatureAcces::DetailContrat,
        true,
        None,
    )
    .await?;
    tx.commit().await?;
    Ok(Some(telephone))
}

// ---------------------------------------------------------------------
// Journal vu par le patient (décisions MD4 et MD5)
// ---------------------------------------------------------------------

/// Une ligne du journal telle que le patient la voit : l'établissement, le
/// rôle, le nom du professionnel, la date et le motif (MD5).
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct AccesVu {
    pub id: Uuid,
    pub date_acces: DateTime<Utc>,
    pub nature: String,
    pub structure_nom: String,
    pub role: String,
    pub professionnel_nom: Option<String>,
    pub professionnel_prenom: Option<String>,
    pub telephone_affiche: bool,
    /// Pour un accès sous une prise en charge : comment elle a été ouverte
    /// (`code_soin`, `cnib_verifiee`, `bris_de_glace`).
    pub moyen: Option<String>,
    /// Pour un bris de glace : la catégorie et le texte du motif.
    pub motif_categorie: Option<String>,
    pub motif: Option<String>,
}

/// Journal du patient du compte, le plus récent d'abord (curseur `avant`
/// sur l'identifiant `uuidv7`). Le patient est celui du jeton, jamais un
/// identifiant de la requête (OWASP API1:2023).
pub async fn journal_du_patient(
    pool: &PgPool,
    compte_id: Uuid,
    avant: Option<Uuid>,
    limite: i64,
) -> Result<Vec<AccesVu>, sqlx::Error> {
    sqlx::query_as(
        "SELECT j.id, j.date_acces, j.nature, s.nom AS structure_nom, j.role, \
                u.nom AS professionnel_nom, u.prenom AS professionnel_prenom, j.telephone_affiche, \
                NULL::text AS moyen, NULL::text AS motif_categorie, NULL::text AS motif \
         FROM compte cp \
         JOIN patient p ON p.utilisateur_id = cp.utilisateur_id \
         JOIN journal_acces j ON j.patient_id = p.id \
         JOIN structure s ON s.id = j.structure_id \
         JOIN affectation a ON a.id = j.affectation_id \
         JOIN compte c ON c.id = a.compte_id \
         JOIN utilisateur u ON u.id = c.utilisateur_id \
         WHERE cp.id = $1 AND cp.type_compte = 'patient' AND ($2::uuid IS NULL OR j.id < $2) \
         ORDER BY j.id DESC LIMIT $3",
    )
    .bind(compte_id)
    .bind(avant)
    .bind(limite)
    .fetch_all(pool)
    .await
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::fixtures_medicales::{exercice, patient, structure};

    fn cles(fiche: &Map<String, Value>) -> BTreeSet<&str> {
        fiche.keys().map(String::as_str).collect()
    }

    async fn lignes_du_journal(pool: &PgPool, patient_id: Uuid) -> Vec<(String, String, bool)> {
        sqlx::query_as(
            "SELECT role, nature, telephone_affiche FROM journal_acces WHERE patient_id = $1 ORDER BY id",
        )
        .bind(patient_id)
        .fetch_all(pool)
        .await
        .unwrap()
    }

    /// Chaque rôle reçoit exactement ses champs au scan, et rien de plus ;
    /// seul l'agent MUNASEB reçoit le téléphone. Tout autre rôle : refus.
    /// Chaque scan est écrit dans le journal, avec le téléphone montré pour
    /// l'agent seulement.
    #[sqlx::test]
    #[ignore]
    async fn scan_par_role(pool: PgPool) {
        let p = patient(&pool, "+22670360001").await;
        let clinique = structure(&pool, "clinique", "Clinique fictive").await;
        let pharmacie = structure(&pool, "pharmacie", "Pharmacie fictive").await;
        let munaseb = structure(&pool, "munaseb", "MUNASEB").await;

        let identite = ["nom", "prenom", "sexe", "date_naissance", "lieu_naissance"];
        let attendu = |plus: &[&'static str]| -> BTreeSet<&'static str> {
            identite.iter().chain(plus).copied().collect()
        };
        for (structure_id, role, champs) in [
            (clinique, "agent_accueil", attendu(&[])),
            (clinique, "medecin", attendu(&[])),
            (clinique, "infirmier", attendu(&[])),
            (munaseb, "agent_assurance_munaseb", attendu(&["telephone"])),
            (pharmacie, "pharmacien", attendu(&["droits_munaseb"])),
            (
                pharmacie,
                "auxiliaire_pharmacie",
                ["nom", "prenom", "droits_munaseb"].into_iter().collect(),
            ),
        ] {
            let ex = exercice(&pool, structure_id, role).await;
            let fiche = scanner(&pool, &ex, &p.nip).await.unwrap();
            assert_eq!(cles(&fiche), champs, "{role}");
            assert_eq!(fiche["nom"], "OUEDRAOGO");
            if role == "agent_assurance_munaseb" {
                assert!(fiche["telephone"].as_str().unwrap().starts_with("+226"));
            }
            if role == "pharmacien" {
                assert_eq!(fiche["sexe"], "feminin");
                assert_eq!(fiche["droits_munaseb"]["etat"], "aucun_contrat");
            }
            let texte = Value::Object(fiche).to_string();
            assert!(!texte.contains(&p.nip), "NIP renvoyé à {role}");
        }

        // Tout autre rôle, ou un rôle hors de son type d'établissement :
        // refusé avant toute lecture, rien dans le journal.
        for (structure_id, role) in [
            (clinique, "responsable"),
            (pharmacie, "responsable"),
            (munaseb, "responsable"),
            (pharmacie, "medecin"),
            (clinique, "pharmacien"),
        ] {
            let ex = exercice(&pool, structure_id, role).await;
            assert!(matches!(
                scanner(&pool, &ex, &p.nip).await,
                Err(ErreurAcces::RoleNonAutorise)
            ));
        }

        let journal = lignes_du_journal(&pool, p.patient_id).await;
        assert_eq!(journal.len(), 6);
        assert!(journal.iter().all(|(_, nature, _)| nature == "scan"));
        assert_eq!(
            journal
                .iter()
                .filter(|(_, _, telephone)| *telephone)
                .map(|(role, _, _)| role.as_str())
                .collect::<Vec<_>>(),
            ["agent_assurance_munaseb"]
        );
    }

    /// NIP inconnu et NIP mal formé : même réponse, chacun compte comme un
    /// échec ; au 5ᵉ, 429 avec un délai, même avec un bon NIP. Limite des
    /// scans réussis par fenêtre.
    #[sqlx::test]
    #[ignore]
    async fn nip_inconnu_et_limites(pool: PgPool) {
        let p = patient(&pool, "+22670360002").await;
        let clinique = structure(&pool, "clinique", "Clinique fictive").await;
        let accueil = exercice(&pool, clinique, "agent_accueil").await;

        // Même NIP, chiffre de contrôle faux ; NIP bien formé inconnu.
        let faux_controle = format!("{}{}", &p.nip[..15], (p.nip.as_bytes()[15] - b'0' + 1) % 10);
        let inconnu = nip::generer(&pool).await.unwrap();
        for saisi in [
            "123",
            "abcdefghijklmnop",
            faux_controle.as_str(),
            inconnu.as_str(),
        ] {
            assert!(matches!(
                scanner(&pool, &accueil, saisi).await,
                Err(ErreurAcces::PatientInconnu)
            ));
        }
        // Espaces autour acceptées.
        scanner(&pool, &accueil, &format!(" {} ", p.nip))
            .await
            .unwrap();
        assert!(matches!(
            scanner(&pool, &accueil, &inconnu).await,
            Err(ErreurAcces::PatientInconnu)
        ));
        match scanner(&pool, &accueil, &p.nip).await {
            Err(ErreurAcces::TropDEssais {
                reessayer_dans_secs,
            }) => assert!((1..=15 * 60).contains(&reessayer_dans_secs)),
            autre => panic!("{autre:?}"),
        }
        // Rien de ce qui a été saisi n'est gardé.
        let colonnes: Vec<String> = sqlx::query_scalar(
            "SELECT column_name::text FROM information_schema.columns WHERE table_name = 'echec_acces_patient'",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert!(!colonnes.iter().any(|c| c.contains("nip")), "{colonnes:?}");

        // Limite des scans réussis : SCANS_MAX lignes récentes.
        let medecin = exercice(&pool, clinique, "medecin").await;
        sqlx::query(
            "INSERT INTO journal_acces (patient_id, affectation_id, structure_id, role, nature) \
             SELECT $1, $2, $3, 'medecin', 'scan' FROM generate_series(1, $4)",
        )
        .bind(p.patient_id)
        .bind(medecin.affectation_id)
        .bind(clinique)
        .bind(SCANS_MAX as i32)
        .execute(&pool)
        .await
        .unwrap();
        assert!(matches!(
            scanner(&pool, &medecin, &p.nip).await,
            Err(ErreurAcces::TropDEssais { .. })
        ));
    }

    /// Le journal ne se modifie ni ne s'efface, même par le propriétaire des
    /// tables (triggers) ; le patient voit son journal (structure, rôle,
    /// nom du professionnel), et seulement le sien.
    #[sqlx::test]
    #[ignore]
    async fn journal_immuable_et_vu_par_le_patient(pool: PgPool) {
        let p = patient(&pool, "+22670360003").await;
        let autre = patient(&pool, "+22670360004").await;
        let clinique = structure(&pool, "clinique", "Clinique fictive").await;
        let medecin = exercice(&pool, clinique, "medecin").await;
        scanner(&pool, &medecin, &p.nip).await.unwrap();
        // Un échec aussi, pour que les triggers de ligne aient une ligne.
        assert!(scanner(&pool, &medecin, "0").await.is_err());

        for sql in [
            "UPDATE journal_acces SET nature = 'scan'",
            "DELETE FROM journal_acces",
            "TRUNCATE journal_acces",
            "UPDATE echec_acces_patient SET nature = 'scan'",
            "DELETE FROM echec_acces_patient",
        ] {
            assert!(sqlx::query(sql).execute(&pool).await.is_err(), "{sql}");
        }

        let vu = journal_du_patient(&pool, p.compte_id, None, 50)
            .await
            .unwrap();
        assert_eq!(vu.len(), 1);
        assert_eq!(
            (
                vu[0].structure_nom.as_str(),
                vu[0].role.as_str(),
                vu[0].nature.as_str(),
                vu[0].professionnel_nom.as_deref()
            ),
            ("Clinique fictive", "medecin", "scan", Some("SAWADOGO"))
        );
        assert!(
            journal_du_patient(&pool, autre.compte_id, None, 50)
                .await
                .unwrap()
                .is_empty()
        );
    }

    /// Le téléphone de l'adhérent est rendu à l'agent avec le détail d'un
    /// contrat, et chaque affichage est tracé.
    #[sqlx::test]
    #[ignore]
    async fn telephone_du_contrat_trace(pool: PgPool) {
        let p = patient(&pool, "+22670360005").await;
        let munaseb = structure(&pool, "munaseb", "MUNASEB").await;
        let agent = exercice(&pool, munaseb, "agent_assurance_munaseb").await;
        let contrat_id: Uuid = sqlx::query_scalar(
            "INSERT INTO contrat_assurance_munaseb (patient_id, numero_carte, ufr, universite, num_matricule) \
             VALUES ($1, '0001/2026', 'UFR fictive', 'Université fictive', 'MAT-1') RETURNING id",
        )
        .bind(p.patient_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let agent = AffectationAgent {
            affectation_id: agent.affectation_id,
            structure_id: agent.structure_id,
        };
        assert_eq!(
            telephone_pour_agent(&pool, &agent, contrat_id)
                .await
                .unwrap()
                .as_deref(),
            Some("+22670360005")
        );
        assert_eq!(
            telephone_pour_agent(&pool, &agent, Uuid::max())
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            lignes_du_journal(&pool, p.patient_id).await,
            [(
                "agent_assurance_munaseb".to_string(),
                "detail_contrat".to_string(),
                true
            )]
        );
    }
}
