//! Demande de structure par son futur responsable (étape 3.7 réduite à la
//! MUNASEB, décisions E1 à E13 et du porteur du 2026-10-20) : création,
//! soumission, suivi. La validation par l'équipe LaafiCare est l'étape 3.8.
//!
//! Tout part du **compte patient** (E4) : n'importe quel patient peut créer
//! sa structure et en devient responsable (section 14). S'il n'a pas de
//! compte professionnel, il le crée à ce moment (email et mot de passe
//! professionnels) ; sinon son mot de passe professionnel est demandé.
//!
//! Avant validation, une structure ne donne accès à rien : l'affectation
//! `responsable` créée avec elle n'est exerçable qu'une fois la structure
//! validée (`affectation::controler_exercice`).

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use sqlx::types::Uuid;

use crate::adresse_email;
use crate::auth_professionnel::nature_sqlx;
use crate::compte_professionnel::{self, ErreurComptePro};
use crate::email::EmailSender;
use crate::mot_de_passe::MotDePasseNonConforme;
use crate::telephone;
use crate::verification_email;

/// Limites du territoire, en degrés décimaux (WGS 84 pour les applications).
/// Source : Institut géographique du Burkina, *Rapport du Burkina Faso*
/// (août 2014, remis au comité d'experts des Nations unies sur la gestion de
/// l'information géospatiale, UN-GGIM) : « Son territoire est situé entre 8°
/// et 16° de latitude nord et entre les longitudes 6° ouest et 3° est par
/// rapport au méridien de Greenwich ». Ce rectangle déborde un peu sur les
/// pays voisins : il écarte les erreurs grossières (coordonnées inversées,
/// signe oublié, position ailleurs dans le monde), pas une erreur de
/// quelques kilomètres.
pub const LATITUDE_MIN: f64 = 8.0;
pub const LATITUDE_MAX: f64 = 16.0;
pub const LONGITUDE_MIN: f64 = -6.0;
pub const LONGITUDE_MAX: f64 = 3.0;

/// Types de structure (liste fermée de la migration 0012).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TypeStructure {
    Clinique,
    Hopital,
    Pharmacie,
    Laboratoire,
    Munaseb,
}

impl TypeStructure {
    fn en_base(self) -> &'static str {
        match self {
            TypeStructure::Clinique => "clinique",
            TypeStructure::Hopital => "hopital",
            TypeStructure::Pharmacie => "pharmacie",
            TypeStructure::Laboratoire => "laboratoire",
            TypeStructure::Munaseb => "munaseb",
        }
    }

    fn depuis_base(valeur: &str) -> Option<Self> {
        [
            TypeStructure::Clinique,
            TypeStructure::Hopital,
            TypeStructure::Pharmacie,
            TypeStructure::Laboratoire,
            TypeStructure::Munaseb,
        ]
        .into_iter()
        .find(|t| t.en_base() == valeur)
    }

    /// E2 : seule la MUNASEB, qui se soumet sans pièce (U6 b). Les autres
    /// types attendent la liste de leurs pièces exigées (U6 a) et le
    /// téléversement (reste de la 3.7).
    fn disponible(self) -> bool {
        self == TypeStructure::Munaseb
    }
}

/// Ce que le responsable saisit (E1, champs du porteur du 2026-10-20). Pas
/// de `Debug` : le mot de passe est un secret.
pub struct NouvelleStructure<'a> {
    pub type_structure: TypeStructure,
    pub nom: &'a str,
    /// Numéro de service.
    pub telephone: &'a str,
    pub telephone_urgence: &'a str,
    pub region: &'a str,
    pub province: &'a str,
    pub commune: &'a str,
    pub parcelle: Option<&'a str>,
    pub lot: Option<&'a str>,
    pub section: Option<&'a str>,
    /// Position envoyée par l'application, confirmée ou corrigée à l'écran.
    pub latitude: f64,
    pub longitude: f64,
    /// Email de la structure (décision du porteur, 2026-10-20) : coordonnée
    /// de l'établissement, jamais vérifiée par lien.
    pub email: &'a str,
    /// Email de connexion du responsable : exigé seulement si la personne
    /// n'a pas encore de compte professionnel ; ignoré sinon (l'email du
    /// compte ne change pas ici).
    pub email_professionnel: Option<&'a str>,
    pub mot_de_passe_professionnel: &'a str,
}

#[derive(Debug, thiserror::Error)]
pub enum ErreurStructure {
    #[error("Le champ « {0} » est obligatoire.")]
    ChampObligatoire(&'static str),
    /// Nom du champ en cause : `email` ou `email_professionnel`.
    #[error("Le champ « {0} » n'est pas une adresse email valide.")]
    EmailInvalide(&'static str),
    #[error("Ce type de structure ne peut pas encore être créé.")]
    TypeNonDisponible,
    #[error("Le nom de la structure dépasse {NOM_LONGUEUR_MAX} caractères.")]
    NomTropLong,
    #[error("Le nom de la structure contient un retour à la ligne ou un caractère invisible.")]
    NomInvalide,
    /// Deux erreurs, deux codes (décision du porteur, 2026-10-21) :
    /// l'application sait quel champ signaler sans lire le message.
    #[error("Numéro de service invalide.")]
    TelephoneServiceInvalide,
    #[error("Numéro d'urgence invalide.")]
    TelephoneUrgenceInvalide,
    #[error("La position de la structure doit se trouver au Burkina Faso.")]
    LocalisationHorsBurkina,
    #[error("Mot de passe non conforme.")]
    MotDePasseNonConforme(MotDePasseNonConforme),
    /// Compte professionnel existant : mot de passe faux, compte verrouillé
    /// ou désactivé, jamais distingués (L1).
    #[error("Mot de passe professionnel incorrect.")]
    MotDePasseIncorrect,
    /// Un TOTP est actif sur le compte professionnel : cette route ne
    /// demande pas encore le code (aucun professionnel n'a de TOTP avant
    /// l'étape 3.11, J10).
    #[error("Votre compte professionnel a un second facteur : cette opération ne le prend pas encore en charge.")]
    SecondFacteurRequis,
    #[error("Votre identité a déjà un autre email : il ne peut pas être changé ici.")]
    EmailDifferent,
    #[error("Cet email est déjà utilisé par une autre personne.")]
    EmailDejaUtilise,
    /// Limite anti-abus (M5) : une seule structure non validée par personne.
    #[error("Vous avez déjà une demande de structure en cours.")]
    DemandeDejaEnCours,
    /// Seulement l'étape en échec, un texte fixe : jamais une saisie.
    #[error("erreur interne : {0}")]
    Interne(&'static str),
}

fn interne_sqlx(etape: &'static str, erreur: &sqlx::Error) -> ErreurStructure {
    tracing::error!(etape, nature = %nature_sqlx(erreur), "structure : erreur interne");
    ErreurStructure::Interne(etape)
}

fn contrainte(erreur: &sqlx::Error) -> Option<&str> {
    erreur.as_database_error().and_then(|e| e.constraint())
}

/// Texte obligatoire, sans les espaces autour.
fn obligatoire(champ: &'static str, valeur: &str) -> Result<String, ErreurStructure> {
    let valeur = valeur.trim();
    if valeur.is_empty() {
        return Err(ErreurStructure::ChampObligatoire(champ));
    }
    Ok(valeur.to_string())
}

/// Longueur maximale du nom, en caractères (décision du porteur,
/// 2026-10-21) : le nom est repris dans le SMS d'invitation, qui tient alors
/// en 2 parties au plus (3GPP TS 23.040 §9.2.3.24.1). Garantie aussi par la
/// base (migration 0023).
pub const NOM_LONGUEUR_MAX: usize = 100;

/// Mêmes règles que la migration 0023 : 100 caractères au plus, ni
/// caractère de contrôle (`char::is_control`, catégorie Unicode Cc) ni
/// séparateur de ligne ou de paragraphe (U+2028, U+2029).
fn controler_nom(nom: &str) -> Result<(), ErreurStructure> {
    if nom.chars().count() > NOM_LONGUEUR_MAX {
        return Err(ErreurStructure::NomTropLong);
    }
    if nom.chars().any(|c| c.is_control() || c == '\u{2028}' || c == '\u{2029}') {
        return Err(ErreurStructure::NomInvalide);
    }
    Ok(())
}

/// Texte facultatif : vide ou absent devient `None`.
fn facultatif(valeur: Option<&str>) -> Option<String> {
    valeur.map(str::trim).filter(|v| !v.is_empty()).map(str::to_string)
}

fn dans_le_territoire(latitude: f64, longitude: f64) -> bool {
    // `contains` est faux pour NaN : un nombre non fini est refusé.
    (LATITUDE_MIN..=LATITUDE_MAX).contains(&latitude) && (LONGITUDE_MIN..=LONGITUDE_MAX).contains(&longitude)
}

/// Structure créée.
#[derive(Debug)]
pub struct StructureCreee {
    pub structure_id: Uuid,
    /// L'email du compte professionnel reste à vérifier avant la soumission
    /// (R0). Un lien vient d'être envoyé si le compte vient d'être créé ;
    /// sinon la route de renvoi en donne un.
    pub email_a_verifier: bool,
}

impl From<ErreurComptePro> for ErreurStructure {
    fn from(erreur: ErreurComptePro) -> Self {
        match erreur {
            ErreurComptePro::EmailObligatoire => ErreurStructure::ChampObligatoire("email_professionnel"),
            ErreurComptePro::EmailInvalide => ErreurStructure::EmailInvalide("email_professionnel"),
            ErreurComptePro::MotDePasseNonConforme(regles) => ErreurStructure::MotDePasseNonConforme(regles),
            ErreurComptePro::MotDePasseIncorrect => ErreurStructure::MotDePasseIncorrect,
            ErreurComptePro::SecondFacteurRequis => ErreurStructure::SecondFacteurRequis,
            ErreurComptePro::EmailDifferent => ErreurStructure::EmailDifferent,
            ErreurComptePro::EmailDejaUtilise => ErreurStructure::EmailDejaUtilise,
            // Une autre requête de la même personne vient de créer son
            // compte professionnel, avec une demande de structure.
            ErreurComptePro::CompteDejaCree => ErreurStructure::DemandeDejaEnCours,
            ErreurComptePro::Interne(etape) => ErreurStructure::Interne(etape),
        }
    }
}

/// `POST /api/structures` (E1). `patient_compte_id` vient du jeton patient,
/// déjà contrôlé par l'extracteur (compte actif, session ouverte).
pub async fn creer(
    pool: &PgPool,
    email_sender: &dyn EmailSender,
    url_application_web: &str,
    patient_compte_id: Uuid,
    saisie: &NouvelleStructure<'_>,
) -> Result<StructureCreee, ErreurStructure> {
    // --- Contrôles sans base ---------------------------------------------
    if !saisie.type_structure.disponible() {
        return Err(ErreurStructure::TypeNonDisponible);
    }
    let nom = obligatoire("nom", saisie.nom)?;
    controler_nom(&nom)?;
    let region = obligatoire("region", saisie.region)?;
    let province = obligatoire("province", saisie.province)?;
    let commune = obligatoire("commune", saisie.commune)?;
    // Même normalisation que tous les numéros (C3, T1 à T4) ; les deux
    // numéros peuvent être identiques (décision du porteur).
    let telephone = telephone::normaliser(saisie.telephone).map_err(|_| ErreurStructure::TelephoneServiceInvalide)?;
    let telephone_urgence =
        telephone::normaliser(saisie.telephone_urgence).map_err(|_| ErreurStructure::TelephoneUrgenceInvalide)?;
    if !dans_le_territoire(saisie.latitude, saisie.longitude) {
        return Err(ErreurStructure::LocalisationHorsBurkina);
    }
    // Contrôle commun de toutes les adresses (`adresse_email.rs`).
    let email_structure = obligatoire("email", saisie.email)?;
    adresse_email::controler(&email_structure).map_err(|_| ErreurStructure::EmailInvalide("email"))?;

    // --- Compte professionnel (commun avec l'acceptation d'une invitation) --
    let preparation = compte_professionnel::preparer(
        pool,
        patient_compte_id,
        &compte_professionnel::Saisie {
            email_professionnel: saisie.email_professionnel,
            mot_de_passe: saisie.mot_de_passe_professionnel,
        },
    )
    .await?;
    let email_verifie = preparation.email_verifie();

    // --- Écriture, en une transaction ------------------------------------
    let mut tx = pool.begin().await.map_err(|e| interne_sqlx("transaction", &e))?;
    // Les comptes d'abord (le compte avant le reste, section 12).
    let compte = compte_professionnel::ecrire(&mut tx, patient_compte_id, preparation).await?;
    let compte_pro_id = compte.compte_id;

    // Coordonnées envoyées en `double precision`, converties en
    // NUMERIC(9,6) par PostgreSQL : arrondies à 6 décimales (environ 11 cm,
    // décision M2).
    let structure_id: Uuid = sqlx::query_scalar(
        "INSERT INTO structure (type_structure, nom, telephone, telephone_urgence, region, province, commune, \
                                parcelle, lot, section_cadastrale, latitude, longitude, createur_compte_id, email) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11::numeric(9,6), $12::numeric(9,6), $13, $14) RETURNING id",
    )
    .bind(saisie.type_structure.en_base())
    .bind(&nom)
    .bind(&telephone)
    .bind(&telephone_urgence)
    .bind(&region)
    .bind(&province)
    .bind(&commune)
    .bind(facultatif(saisie.parcelle))
    .bind(facultatif(saisie.lot))
    .bind(facultatif(saisie.section))
    .bind(saisie.latitude)
    .bind(saisie.longitude)
    .bind(compte_pro_id)
    .bind(&email_structure)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| match contrainte(&e) {
        // E11 : index unique de la migration 0012.
        Some("structure_une_non_validee_par_createur") => ErreurStructure::DemandeDejaEnCours,
        _ => interne_sqlx("création de la structure", &e),
    })?;

    // E3 : responsable dès la création, dans la même transaction.
    sqlx::query("INSERT INTO affectation (compte_id, structure_id, role) VALUES ($1, $2, 'responsable')")
        .bind(compte_pro_id)
        .bind(structure_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| interne_sqlx("affectation du responsable", &e))?;

    tx.commit().await.map_err(|e| interne_sqlx("validation de la transaction", &e))?;

    // Après le commit : un échec d'envoi laisse la structure créée. Lien
    // seulement pour un compte qui vient d'être créé avec un email non
    // vérifié ; sinon, route de renvoi (E7).
    if let Some(emise) = &compte.verification {
        verification_email::envoyer_lien(email_sender, url_application_web, emise);
    }
    Ok(StructureCreee { structure_id, email_a_verifier: !email_verifie })
}

// ---------------------------------------------------------------------
// Soumission (E9)
// ---------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum ErreurSoumission {
    /// Inexistante, ou créée par une autre personne : jamais distinguées.
    #[error("Structure inconnue.")]
    StructureInconnue,
    #[error("Cette demande ne peut pas être soumise dans son état actuel.")]
    TransitionInvalide { statut_actuel: String },
    #[error("Votre email n'est pas encore vérifié. Ouvrez le lien que nous vous avons envoyé, ou demandez-en un nouveau.")]
    EmailNonVerifie,
    #[error("Votre compte professionnel est désactivé.")]
    CompteProfessionnelDesactive,
    #[error("Ce type de structure ne peut pas encore être soumis.")]
    TypeNonDisponible,
    #[error("erreur interne : {0}")]
    Interne(&'static str),
}

/// `POST /api/structures/{id}/soumettre` : `brouillon` → `en_attente`,
/// par le créateur seulement (son compte patient), avec un email
/// professionnel vérifié (R0). La MUNASEB se soumet sans pièce (U6 b).
pub async fn soumettre(pool: &PgPool, patient_compte_id: Uuid, structure_id: Uuid) -> Result<(), ErreurSoumission> {
    let interne = |etape: &'static str, e: sqlx::Error| {
        tracing::error!(etape, nature = %nature_sqlx(&e), "soumission : erreur interne");
        ErreurSoumission::Interne(etape)
    };
    let mut tx = pool.begin().await.map_err(|e| interne("transaction", e))?;
    // Verrou de la structure : deux soumissions simultanées se suivent, la
    // seconde voit `en_attente`.
    let ligne: Option<(String, String, String, bool)> = sqlx::query_as(
        "SELECT s.statut, s.type_structure, pro.statut, u.email_verifie_le IS NOT NULL \
         FROM structure s \
         JOIN compte pro ON pro.id = s.createur_compte_id \
         JOIN utilisateur u ON u.id = pro.utilisateur_id \
         JOIN compte pat ON pat.utilisateur_id = u.id AND pat.type_compte = 'patient' \
         WHERE s.id = $1 AND pat.id = $2 \
         FOR UPDATE OF s",
    )
    .bind(structure_id)
    .bind(patient_compte_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| interne("lecture de la structure", e))?;
    let (statut, type_structure, statut_pro, email_verifie) = ligne.ok_or(ErreurSoumission::StructureInconnue)?;

    if statut != "brouillon" {
        return Err(ErreurSoumission::TransitionInvalide { statut_actuel: statut });
    }
    if statut_pro != "actif" {
        return Err(ErreurSoumission::CompteProfessionnelDesactive);
    }
    if !email_verifie {
        return Err(ErreurSoumission::EmailNonVerifie);
    }
    if !TypeStructure::depuis_base(&type_structure).is_some_and(TypeStructure::disponible) {
        return Err(ErreurSoumission::TypeNonDisponible);
    }
    sqlx::query("UPDATE structure SET statut = 'en_attente' WHERE id = $1")
        .bind(structure_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| interne("soumission", e))?;
    tx.commit().await.map_err(|e| interne("validation de la transaction", e))
}

// ---------------------------------------------------------------------
// Abandon (M5 ; décisions AB1 à AB3 du porteur, 2026-10-21)
// ---------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum ErreurAbandon {
    /// Inexistante, ou créée par une autre personne : jamais distinguées.
    #[error("Structure inconnue.")]
    StructureInconnue,
    #[error("Cette demande ne peut pas être abandonnée dans son état actuel.")]
    TransitionInvalide { statut_actuel: String },
    #[error("erreur interne : {0}")]
    Interne(&'static str),
}

/// `POST /api/structures/{id}/abandonner` : `brouillon` ou `refusee` →
/// `abandonnee` (M5, état final), par le créateur seulement (son compte
/// patient, AB1, AB2). Rien n'est effacé, pièces comprises ; la structure
/// sort de la limite « une non validée par personne » (index partiel de la
/// 0012). Une demande `en_attente` n'est pas abandonnable : l'équipe
/// LaafiCare l'examine peut-être déjà.
///
/// AB3 : l'affectation `responsable` du créateur (et toute autre restée
/// active) est close dans la même transaction, datée et attribuée au compte
/// professionnel du créateur ; sinon elle resterait active sur une
/// structure morte. Ordre des verrous (§12) : structure, puis affectations.
pub async fn abandonner(pool: &PgPool, patient_compte_id: Uuid, structure_id: Uuid) -> Result<(), ErreurAbandon> {
    let interne = |etape: &'static str, e: sqlx::Error| {
        tracing::error!(etape, nature = %nature_sqlx(&e), "abandon : erreur interne");
        ErreurAbandon::Interne(etape)
    };
    let mut tx = pool.begin().await.map_err(|e| interne("transaction", e))?;
    let ligne: Option<(String, Uuid)> = sqlx::query_as(
        "SELECT s.statut, s.createur_compte_id \
         FROM structure s \
         JOIN compte pro ON pro.id = s.createur_compte_id \
         JOIN compte pat ON pat.utilisateur_id = pro.utilisateur_id AND pat.type_compte = 'patient' \
         WHERE s.id = $1 AND pat.id = $2 \
         FOR UPDATE OF s",
    )
    .bind(structure_id)
    .bind(patient_compte_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| interne("lecture de la structure", e))?;
    let (statut, createur) = ligne.ok_or(ErreurAbandon::StructureInconnue)?;
    if statut != "brouillon" && statut != "refusee" {
        return Err(ErreurAbandon::TransitionInvalide { statut_actuel: statut });
    }
    sqlx::query("UPDATE structure SET statut = 'abandonnee' WHERE id = $1")
        .bind(structure_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| interne("abandon", e))?;
    // Une structure jamais validée n'a pu inviter personne : seules les
    // affectations `responsable` existent, sans session d'exercice possible
    // (structure non validée) ; aucune session à fermer.
    sqlx::query(
        "UPDATE affectation SET statut = 'desactivee', desactivee_le = now(), desactivee_par_compte_id = $2 \
         WHERE structure_id = $1 AND statut = 'active'",
    )
    .bind(structure_id)
    .bind(createur)
    .execute(&mut *tx)
    .await
    .map_err(|e| interne("clôture des affectations", e))?;
    tx.commit().await.map_err(|e| interne("validation de la transaction", e))
}

// ---------------------------------------------------------------------
// Suivi (E10)
// ---------------------------------------------------------------------

/// Une structure créée par la personne, quel que soit son statut.
#[derive(Debug, Serialize)]
pub struct DemandeStructure {
    pub id: Uuid,
    #[serde(rename = "type")]
    pub type_structure: TypeStructure,
    pub nom: String,
    /// `brouillon`, `en_attente`, `validee`, `refusee`, `suspendue` ou
    /// `abandonnee` (migration 0012).
    pub statut: String,
    pub date_creation: DateTime<Utc>,
    pub licence_expire_le: Option<NaiveDate>,
    /// Motif de la dernière décision de refus, seulement si la structure est
    /// refusée.
    pub motif_refus: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MesDemandes {
    /// Le compte professionnel existe et son email n'est pas vérifié : la
    /// soumission sera refusée tant que le lien n'a pas servi.
    pub email_a_verifier: bool,
    /// Les plus récentes d'abord (identifiants `uuidv7`, ordonnés dans le
    /// temps).
    pub demandes: Vec<DemandeStructure>,
}

/// `GET /api/structures/mes-demandes`.
pub async fn mes_demandes(pool: &PgPool, patient_compte_id: Uuid) -> Result<MesDemandes, sqlx::Error> {
    type Ligne = (Uuid, String, String, String, DateTime<Utc>, Option<NaiveDate>, Option<String>);
    let lignes: Vec<Ligne> = sqlx::query_as(
        "SELECT s.id, s.type_structure, s.nom, s.statut, s.date_creation, s.licence_expire_le, \
                CASE WHEN s.statut = 'refusee' THEN \
                    (SELECT d.motif FROM decision_structure d \
                     WHERE d.structure_id = s.id AND d.decision = 'refusee' \
                     ORDER BY d.date_decision DESC LIMIT 1) \
                END \
         FROM structure s \
         JOIN compte pro ON pro.id = s.createur_compte_id \
         JOIN compte pat ON pat.utilisateur_id = pro.utilisateur_id AND pat.type_compte = 'patient' \
         WHERE pat.id = $1 \
         ORDER BY s.id DESC",
    )
    .bind(patient_compte_id)
    .fetch_all(pool)
    .await?;
    let email_a_verifier: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM compte pat \
                        JOIN compte pro ON pro.utilisateur_id = pat.utilisateur_id AND pro.type_compte = 'professionnel' \
                        JOIN utilisateur u ON u.id = pat.utilisateur_id \
                        WHERE pat.id = $1 AND u.email_verifie_le IS NULL)",
    )
    .bind(patient_compte_id)
    .fetch_one(pool)
    .await?;

    let mut demandes = Vec::with_capacity(lignes.len());
    for (id, type_structure, nom, statut, date_creation, licence_expire_le, motif_refus) in lignes {
        // La base n'admet que les cinq valeurs (CHECK de la 0012).
        let type_structure = TypeStructure::depuis_base(&type_structure).ok_or_else(|| {
            sqlx::Error::Decode("type de structure inconnu".into())
        })?;
        demandes.push(DemandeStructure { id, type_structure, nom, statut, date_creation, licence_expire_le, motif_refus });
    }
    Ok(MesDemandes { email_a_verifier, demandes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::email::{EmailsRecus, jeton_du_lien};
    use crate::mot_de_passe;

    // Base temporaire par test ; données fictives.

    const PATIENT: &str = "Patient#2026";
    const PRO: &str = "Professionnel#2026";
    const URL: &str = "http://localhost:4200";

    async fn patient(pool: &PgPool, telephone: &str, email: Option<&str>) -> Uuid {
        let utilisateur_id: Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (nom, prenom, telephone, email) VALUES ('OUEDRAOGO', 'Awa', $1, $2) RETURNING id",
        )
        .bind(telephone)
        .bind(email)
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query_scalar(
            "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'patient', $2) RETURNING id",
        )
        .bind(utilisateur_id)
        .bind(mot_de_passe::hacher(PATIENT).unwrap())
        .fetch_one(pool)
        .await
        .unwrap()
    }

    fn saisie<'a>(email: Option<&'a str>, mot_de_passe: &'a str) -> NouvelleStructure<'a> {
        NouvelleStructure {
            type_structure: TypeStructure::Munaseb,
            nom: "  MUNASEB (test) ",
            telephone: "70 00 00 01",
            telephone_urgence: "70000001",
            region: "Région fictive",
            province: "Province fictive",
            commune: "Commune fictive",
            parcelle: Some(" "),
            lot: None,
            section: Some("B"),
            latitude: 12.3714277,
            longitude: -1.5196603,
            email: " contact.structure@example.org ",
            email_professionnel: email,
            mot_de_passe_professionnel: mot_de_passe,
        }
    }

    async fn soumettre_ok(pool: &PgPool, patient_id: Uuid, structure_id: Uuid) -> Result<(), ErreurSoumission> {
        soumettre(pool, patient_id, structure_id).await
    }

    /// Parcours complet : création du compte professionnel et de la
    /// structure, soumission refusée tant que l'email n'est pas vérifié,
    /// lien envoyé, vérification, soumission, suivi.
    #[sqlx::test]
    #[ignore]
    async fn creation_verification_et_soumission(pool: PgPool) {
        let emails = EmailsRecus::default();
        let patient_id = patient(&pool, "+22670150001", None).await;

        let creee = creer(&pool, &emails, URL, patient_id, &saisie(Some(" Responsable@Example.org "), PRO)).await.unwrap();
        assert!(creee.email_a_verifier);

        // Ce qui a été enregistré : valeurs nettoyées, numéros normalisés,
        // coordonnées arrondies à 6 décimales, responsable affecté.
        let (nom, telephone, urgence, parcelle, section, latitude, statut): (String, String, String, Option<String>, Option<String>, String, String) =
            sqlx::query_as(
                "SELECT nom, telephone, telephone_urgence, parcelle, section_cadastrale, latitude::text, statut \
                 FROM structure WHERE id = $1",
            )
            .bind(creee.structure_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            (nom.as_str(), telephone.as_str(), urgence.as_str(), parcelle, section.as_deref(), latitude.as_str(), statut.as_str()),
            ("MUNASEB (test)", "+22670000001", "+22670000001", None, Some("B"), "12.371428", "brouillon")
        );
        let email_structure: String = sqlx::query_scalar("SELECT email FROM structure WHERE id = $1")
            .bind(creee.structure_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(email_structure, "contact.structure@example.org", "email de la structure, sans les espaces autour");
        let role: String = sqlx::query_scalar("SELECT role FROM affectation WHERE structure_id = $1")
            .bind(creee.structure_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(role, "responsable");
        let email: String = sqlx::query_scalar(
            "SELECT u.email FROM compte c JOIN utilisateur u ON u.id = c.utilisateur_id WHERE c.id = $1",
        )
        .bind(patient_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(email, "Responsable@Example.org", "enregistré sans les espaces autour");

        // Soumission refusée : email non vérifié.
        assert!(matches!(
            soumettre_ok(&pool, patient_id, creee.structure_id).await,
            Err(ErreurSoumission::EmailNonVerifie)
        ));
        let suivi = mes_demandes(&pool, patient_id).await.unwrap();
        assert!(suivi.email_a_verifier);

        // Lien envoyé à l'adresse, jeton après `#`.
        let (destinataire, corps) = emails.0.lock().unwrap()[0].clone();
        assert_eq!(destinataire, "Responsable@Example.org");
        assert!(verification_email::verifier(&pool, &jeton_du_lien(&corps)).await.unwrap());

        soumettre_ok(&pool, patient_id, creee.structure_id).await.unwrap();
        assert!(matches!(
            soumettre_ok(&pool, patient_id, creee.structure_id).await,
            Err(ErreurSoumission::TransitionInvalide { statut_actuel }) if statut_actuel == "en_attente"
        ));

        let suivi = mes_demandes(&pool, patient_id).await.unwrap();
        assert!(!suivi.email_a_verifier);
        assert_eq!(suivi.demandes.len(), 1);
        assert_eq!(
            (suivi.demandes[0].id, suivi.demandes[0].type_structure, suivi.demandes[0].statut.as_str()),
            (creee.structure_id, TypeStructure::Munaseb, "en_attente")
        );
    }

    /// E11 : une seule demande non validée par personne ; une seconde
    /// création, avec le compte professionnel désormais existant, est
    /// refusée après le mot de passe.
    #[sqlx::test]
    #[ignore]
    async fn seconde_demande_refusee_et_mot_de_passe_existant(pool: PgPool) {
        let emails = EmailsRecus::default();
        let patient_id = patient(&pool, "+22670150002", None).await;
        creer(&pool, &emails, URL, patient_id, &saisie(Some("seconde@example.org"), PRO)).await.unwrap();

        assert!(matches!(
            creer(&pool, &emails, URL, patient_id, &saisie(None, "Faux#2026")).await,
            Err(ErreurStructure::MotDePasseIncorrect)
        ));
        assert!(matches!(
            creer(&pool, &emails, URL, patient_id, &saisie(None, PRO)).await,
            Err(ErreurStructure::DemandeDejaEnCours)
        ));
        let structures: i64 = sqlx::query_scalar("SELECT count(*) FROM structure").fetch_one(&pool).await.unwrap();
        assert_eq!(structures, 1);
        assert_eq!(emails.0.lock().unwrap().len(), 1, "pas de nouveau lien pour un compte existant");
    }

    /// Une structure abandonnée libère la place (M5) ; le compte existant
    /// sert sans email.
    #[sqlx::test]
    #[ignore]
    async fn compte_professionnel_existant_reutilise(pool: PgPool) {
        let emails = EmailsRecus::default();
        let patient_id = patient(&pool, "+22670150003", None).await;
        let premiere = creer(&pool, &emails, URL, patient_id, &saisie(Some("reutilise@example.org"), PRO)).await.unwrap();
        sqlx::query("UPDATE structure SET statut = 'abandonnee' WHERE id = $1")
            .bind(premiere.structure_id)
            .execute(&pool)
            .await
            .unwrap();
        let seconde = creer(&pool, &emails, URL, patient_id, &saisie(None, PRO)).await.unwrap();
        assert_ne!(premiere.structure_id, seconde.structure_id);
        let comptes_pro: i64 = sqlx::query_scalar("SELECT count(*) FROM compte WHERE type_compte = 'professionnel'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(comptes_pro, 1);
    }

    #[sqlx::test]
    #[ignore]
    async fn saisies_refusees_sans_rien_ecrire(pool: PgPool) {
        let emails = EmailsRecus::default();
        let patient_id = patient(&pool, "+22670150004", Some("deja@example.org")).await;
        let base = || saisie(Some("deja@example.org"), PRO);
        // 101 caractères accentués : la limite compte des caractères, pas des
        // octets.
        let nom_trop_long = "é".repeat(NOM_LONGUEUR_MAX + 1);

        let cas: Vec<(NouvelleStructure, fn(&ErreurStructure) -> bool)> = vec![
            (NouvelleStructure { type_structure: TypeStructure::Clinique, ..base() }, |e| {
                matches!(e, ErreurStructure::TypeNonDisponible)
            }),
            (NouvelleStructure { nom: "  ", ..base() }, |e| matches!(e, ErreurStructure::ChampObligatoire("nom"))),
            (NouvelleStructure { region: "", ..base() }, |e| matches!(e, ErreurStructure::ChampObligatoire("region"))),
            // Migration 0023 : 100 caractères au plus, ni retour à la ligne
            // ni caractère de contrôle.
            (NouvelleStructure { nom: &nom_trop_long, ..base() }, |e| matches!(e, ErreurStructure::NomTropLong)),
            (NouvelleStructure { nom: "MUNASEB\nCentre", ..base() }, |e| matches!(e, ErreurStructure::NomInvalide)),
            (NouvelleStructure { nom: "MUNASEB\u{2028}Centre", ..base() }, |e| matches!(e, ErreurStructure::NomInvalide)),
            (NouvelleStructure { nom: "MUNASEB\u{7}", ..base() }, |e| matches!(e, ErreurStructure::NomInvalide)),
            (NouvelleStructure { telephone: "12", ..base() }, |e| {
                matches!(e, ErreurStructure::TelephoneServiceInvalide)
            }),
            (NouvelleStructure { telephone_urgence: "12", ..base() }, |e| {
                matches!(e, ErreurStructure::TelephoneUrgenceInvalide)
            }),
            // Coordonnées inversées, puis hors du rectangle (IGB 2014).
            (NouvelleStructure { latitude: -1.52, longitude: 12.37, ..base() }, |e| {
                matches!(e, ErreurStructure::LocalisationHorsBurkina)
            }),
            (NouvelleStructure { latitude: 16.5, ..base() }, |e| matches!(e, ErreurStructure::LocalisationHorsBurkina)),
            (NouvelleStructure { longitude: 3.01, ..base() }, |e| matches!(e, ErreurStructure::LocalisationHorsBurkina)),
            // Email de la structure : obligatoire, bien formé.
            (NouvelleStructure { email: "  ", ..base() }, |e| matches!(e, ErreurStructure::ChampObligatoire("email"))),
            (NouvelleStructure { email: "contact sans arobase", ..base() }, |e| {
                matches!(e, ErreurStructure::EmailInvalide("email"))
            }),
            // Contrôle commun (`adresse_email.rs`) : ASCII seulement.
            (NouvelleStructure { email: "contact@exemplé.bf", ..base() }, |e| {
                matches!(e, ErreurStructure::EmailInvalide("email"))
            }),
            // Email professionnel : exigé sans compte professionnel.
            (NouvelleStructure { email_professionnel: None, ..base() }, |e| {
                matches!(e, ErreurStructure::ChampObligatoire("email_professionnel"))
            }),
            (NouvelleStructure { email_professionnel: Some("sans-arobase"), ..base() }, |e| {
                matches!(e, ErreurStructure::EmailInvalide("email_professionnel"))
            }),
            (NouvelleStructure { email_professionnel: Some("a@b@c"), ..base() }, |e| {
                matches!(e, ErreurStructure::EmailInvalide("email_professionnel"))
            }),
            (NouvelleStructure { email_professionnel: Some("autre@example.org"), ..base() }, |e| {
                matches!(e, ErreurStructure::EmailDifferent)
            }),
            (NouvelleStructure { email_professionnel: Some("élodie@example.org"), ..base() }, |e| {
                matches!(e, ErreurStructure::EmailInvalide("email_professionnel"))
            }),
            (NouvelleStructure { mot_de_passe_professionnel: "court", ..base() }, |e| {
                matches!(e, ErreurStructure::MotDePasseNonConforme(_))
            }),
        ];
        for (saisie, attendu) in cas {
            let erreur = creer(&pool, &emails, URL, patient_id, &saisie).await.unwrap_err();
            assert!(attendu(&erreur), "{erreur:?}");
        }
        let (structures, comptes): (i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM structure), (SELECT count(*) FROM compte WHERE type_compte = 'professionnel')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!((structures, comptes), (0, 0));
        assert!(emails.0.lock().unwrap().is_empty());
    }

    /// Décision du porteur (2026-10-21) : le mot de passe professionnel peut
    /// être celui du compte patient.
    #[sqlx::test]
    #[ignore]
    async fn mot_de_passe_identique_au_patient_accepte(pool: PgPool) {
        let emails = EmailsRecus::default();
        let patient_id = patient(&pool, "+22670150011", None).await;
        creer(&pool, &emails, URL, patient_id, &saisie(Some("meme.mdp@example.org"), PATIENT)).await.unwrap();
    }

    /// Bornes comprises : un point exactement sur la limite est accepté.
    #[test]
    fn limites_du_territoire() {
        assert!(dans_le_territoire(8.0, -6.0) && dans_le_territoire(16.0, 3.0));
        assert!(dans_le_territoire(12.3714, -1.5197), "Ouagadougou");
        assert!(!dans_le_territoire(7.99, 0.0) && !dans_le_territoire(12.0, -6.01));
        assert!(!dans_le_territoire(f64::NAN, 0.0) && !dans_le_territoire(12.0, f64::INFINITY));
    }

    /// Email déjà utilisé par une autre identité.
    #[sqlx::test]
    #[ignore]
    async fn email_d_une_autre_personne_refuse(pool: PgPool) {
        let emails = EmailsRecus::default();
        patient(&pool, "+22670150005", Some("pris@example.org")).await;
        let patient_id = patient(&pool, "+22670150006", None).await;
        assert!(matches!(
            creer(&pool, &emails, URL, patient_id, &saisie(Some("PRIS@example.org"), PRO)).await,
            Err(ErreurStructure::EmailDejaUtilise)
        ));
    }

    /// La structure d'une autre personne n'existe pas pour elle ; un compte
    /// professionnel désactivé ne soumet rien.
    #[sqlx::test]
    #[ignore]
    async fn soumission_par_une_autre_personne_ou_compte_desactive(pool: PgPool) {
        let emails = EmailsRecus::default();
        let createur = patient(&pool, "+22670150007", None).await;
        let autre = patient(&pool, "+22670150008", None).await;
        let creee = creer(&pool, &emails, URL, createur, &saisie(Some("createur@example.org"), PRO)).await.unwrap();
        sqlx::query("UPDATE utilisateur SET email_verifie_le = now() WHERE email = 'createur@example.org'")
            .execute(&pool)
            .await
            .unwrap();

        assert!(matches!(soumettre_ok(&pool, autre, creee.structure_id).await, Err(ErreurSoumission::StructureInconnue)));
        assert!(matches!(soumettre_ok(&pool, createur, Uuid::nil()).await, Err(ErreurSoumission::StructureInconnue)));
        assert!(mes_demandes(&pool, autre).await.unwrap().demandes.is_empty());

        sqlx::query("UPDATE compte SET statut = 'desactive' WHERE type_compte = 'professionnel'")
            .execute(&pool)
            .await
            .unwrap();
        assert!(matches!(
            soumettre_ok(&pool, createur, creee.structure_id).await,
            Err(ErreurSoumission::CompteProfessionnelDesactive)
        ));
    }

    /// Motif du dernier refus, seulement pour une structure refusée.
    #[sqlx::test]
    #[ignore]
    async fn suivi_avec_motif_de_refus(pool: PgPool) {
        let emails = EmailsRecus::default();
        let patient_id = patient(&pool, "+22670150009", None).await;
        let creee = creer(&pool, &emails, URL, patient_id, &saisie(Some("refus@example.org"), PRO)).await.unwrap();
        let admin_utilisateur: Uuid =
            sqlx::query_scalar("INSERT INTO utilisateur (telephone) VALUES ('+22670150010') RETURNING id")
                .fetch_one(&pool)
                .await
                .unwrap();
        let admin: Uuid = sqlx::query_scalar(
            "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) \
             VALUES ($1, 'administrateur_laaficare', 'x') RETURNING id",
        )
        .bind(admin_utilisateur)
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO decision_structure (structure_id, administrateur_compte_id, decision, motif) \
             VALUES ($1, $2, 'refusee', 'Informations incomplètes (fictif)')",
        )
        .bind(creee.structure_id)
        .bind(admin)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("UPDATE structure SET statut = 'refusee' WHERE id = $1")
            .bind(creee.structure_id)
            .execute(&pool)
            .await
            .unwrap();
        let suivi = mes_demandes(&pool, patient_id).await.unwrap();
        assert_eq!(suivi.demandes[0].motif_refus.as_deref(), Some("Informations incomplètes (fictif)"));
        assert_eq!(serde_json::to_value(&suivi.demandes[0]).unwrap()["type"], "munaseb");
    }

    async fn statut_structure(pool: &PgPool, structure_id: Uuid) -> String {
        sqlx::query_scalar("SELECT statut FROM structure WHERE id = $1").bind(structure_id).fetch_one(pool).await.unwrap()
    }

    /// AB1 à AB3 : abandon d'un brouillon par son créateur ; l'affectation
    /// `responsable` est close (datée, attribuée au compte professionnel du
    /// créateur), la place se libère ; un second abandon est refusé.
    #[sqlx::test]
    #[ignore]
    async fn abandon_d_un_brouillon(pool: PgPool) {
        let emails = EmailsRecus::default();
        let createur = patient(&pool, "+22670150011", None).await;
        let autre = patient(&pool, "+22670150012", None).await;
        let creee = creer(&pool, &emails, URL, createur, &saisie(Some("abandon@example.org"), PRO)).await.unwrap();

        assert!(matches!(abandonner(&pool, autre, creee.structure_id).await, Err(ErreurAbandon::StructureInconnue)));
        assert!(matches!(abandonner(&pool, createur, Uuid::nil()).await, Err(ErreurAbandon::StructureInconnue)));
        assert_eq!(statut_structure(&pool, creee.structure_id).await, "brouillon");

        abandonner(&pool, createur, creee.structure_id).await.unwrap();
        assert_eq!(statut_structure(&pool, creee.structure_id).await, "abandonnee");
        let (statut, par, pro): (String, Option<Uuid>, Uuid) = sqlx::query_as(
            "SELECT a.statut, a.desactivee_par_compte_id, s.createur_compte_id \
             FROM affectation a JOIN structure s ON s.id = a.structure_id WHERE a.structure_id = $1",
        )
        .bind(creee.structure_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!((statut.as_str(), par), ("desactivee", Some(pro)));

        assert!(matches!(
            abandonner(&pool, createur, creee.structure_id).await,
            Err(ErreurAbandon::TransitionInvalide { ref statut_actuel }) if statut_actuel == "abandonnee"
        ));
        // La place est libre : une nouvelle demande est acceptée.
        creer(&pool, &emails, URL, createur, &saisie(None, PRO)).await.unwrap();
    }

    /// AB2 : une demande refusée s'abandonne ; une demande en attente ou
    /// validée, non.
    #[sqlx::test]
    #[ignore]
    async fn abandon_selon_le_statut(pool: PgPool) {
        let emails = EmailsRecus::default();
        let createur = patient(&pool, "+22670150013", None).await;
        let creee = creer(&pool, &emails, URL, createur, &saisie(Some("statuts@example.org"), PRO)).await.unwrap();
        for statut in ["en_attente", "validee"] {
            sqlx::query("UPDATE structure SET statut = $2 WHERE id = $1")
                .bind(creee.structure_id)
                .bind(statut)
                .execute(&pool)
                .await
                .unwrap();
            assert!(matches!(
                abandonner(&pool, createur, creee.structure_id).await,
                Err(ErreurAbandon::TransitionInvalide { ref statut_actuel }) if statut_actuel == statut
            ));
        }
        sqlx::query("UPDATE structure SET statut = 'refusee' WHERE id = $1")
            .bind(creee.structure_id)
            .execute(&pool)
            .await
            .unwrap();
        abandonner(&pool, createur, creee.structure_id).await.unwrap();
        assert_eq!(statut_structure(&pool, creee.structure_id).await, "abandonnee");
    }
}
