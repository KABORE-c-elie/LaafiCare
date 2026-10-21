//! Invitations (étape 3.10 réduite, décisions I1 à I17 et du porteur du
//! 2026-10-21) : le responsable d'une structure validée invite au numéro ;
//! la personne répond depuis son compte patient. Section 14 du CLAUDE.md :
//! invitation enregistrée **au numéro, sans rien chercher**, réponse et SMS
//! identiques dans tous les cas ; 7 jours ; la première acceptation crée le
//! compte professionnel.
//!
//! **Ordre des verrous, le même pour toutes les opérations** (décision du
//! porteur, 2026-10-21 ; CLAUDE.md §12) : **comptes** (patient, puis
//! professionnel) → **structure** → **invitation** → affectation créée. Il
//! prolonge la règle « le compte avant le reste » (§12) et la 3.8 (la
//! structure verrouillée par chaque décision) : une décision sur la
//! structure, une invitation, une annulation ou une acceptation se suivent
//! sur la structure, et aucune ne prend jamais une invitation avant sa
//! structure. La liste ne verrouille rien.
//!
//! Une invitation expirée n'est jamais écrite comme telle (M4) : elle reste
//! `en_attente` en base et se lit `expiree` dès que `expire_le` est passée.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::types::Uuid;
use sqlx::{PgConnection, PgPool};

use crate::auth_professionnel::nature_sqlx;
use crate::compte_professionnel::{self, ErreurComptePro};
use crate::email::EmailSender;
use crate::sms::SmsSender;
use crate::telephone;
use crate::verification_email;

/// Durée de validité d'une invitation (section 14, constante ; décidée par
/// le porteur). Écrite à la création : la changer ne touche jamais les
/// invitations existantes.
pub const VALIDITE_INVITATION_JOURS: i32 = 7;
/// Invitations créées par structure sur 24 heures glissantes (I9 a ;
/// décision du porteur, 2026-10-21 ; aucune source ne donne de valeur).
pub const INVITATIONS_PAR_STRUCTURE_PAR_24H: i64 = 20;
/// Invitations en attente, non expirées, par structure (I9 b ; décision du
/// porteur, 2026-10-21).
pub const INVITATIONS_EN_ATTENTE_MAX: i64 = 50;
/// SMS d'invitation par numéro, toutes structures confondues, sur 24 heures
/// glissantes (I9 c ; décision du porteur, 2026-10-21). Au-delà,
/// l'invitation est enregistrée et la réponse est la même, sans SMS : le
/// numéro n'est pas inondé et l'invitant n'apprend rien. Même raisonnement
/// que les limites de l'OTP (O1, O2 ; OWASP *Multifactor Authentication
/// Cheat Sheet*, limites d'envoi et coût des SMS).
pub const SMS_INVITATION_PAR_NUMERO_24H: i64 = 3;

/// Rôles auxquels on peut inviter (I3). Liste fermée par type de structure
/// dans `roles_permis` (section 14 : seule la ligne `munaseb` est utilisable
/// dans cette itération).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleInvitation {
    Responsable,
    AgentAssuranceMunaseb,
}

impl RoleInvitation {
    pub fn as_str(self) -> &'static str {
        match self {
            RoleInvitation::Responsable => "responsable",
            RoleInvitation::AgentAssuranceMunaseb => "agent_assurance_munaseb",
        }
    }

    /// Libellé du SMS.
    fn libelle(self) -> &'static str {
        match self {
            RoleInvitation::Responsable => "responsable",
            RoleInvitation::AgentAssuranceMunaseb => "agent MUNASEB",
        }
    }
}

fn roles_permis(type_structure: &str) -> &'static [RoleInvitation] {
    match type_structure {
        "munaseb" => &[RoleInvitation::Responsable, RoleInvitation::AgentAssuranceMunaseb],
        _ => &[],
    }
}

/// Texte du SMS (I6, décision du porteur, 2026-10-21), identique que le
/// numéro ait un compte ou non ; aucun lien ni code (un lien serait un appât
/// facile à imiter).
pub fn texte_sms(structure_nom: &str, role: RoleInvitation, expire_le: &str) -> String {
    format!(
        "LaafiCare : vous êtes invité(e) à rejoindre « {structure_nom} » comme {}. Ouvrez l'application LaafiCare \
         avec ce numéro (ou créez votre compte) pour répondre avant le {expire_le}. LaafiCare ne vous demandera \
         jamais votre mot de passe.",
        role.libelle()
    )
}

#[derive(Debug, thiserror::Error)]
pub enum ErreurInvitation {
    #[error("Numéro de téléphone invalide.")]
    TelephoneInvalide,
    #[error("Ce rôle ne peut pas être proposé dans cette structure.")]
    RoleNonPermis,
    /// I11 : séparation des tâches (NIST SP 800-53 rév. 5, AC-5).
    #[error("Vous ne pouvez pas vous inviter vous-même.")]
    AutoInvitation,
    #[error("Trop d'invitations envoyées aujourd'hui pour cette structure. Réessayez plus tard.")]
    TropDeDemandes,
    #[error("Trop d'invitations en attente pour cette structure : annulez-en ou attendez leurs réponses.")]
    TropDInvitationsEnAttente,
    #[error("Une invitation est déjà en attente pour ce numéro et ce rôle.")]
    InvitationDejaEnAttente,
    /// Inexistante, ou qui n'est pas à vous : jamais distinguées.
    #[error("Invitation inconnue.")]
    InvitationInconnue,
    #[error("Cette invitation a expiré.")]
    InvitationExpiree,
    #[error("Cette invitation n'est plus en attente.")]
    TransitionInvalide { statut_actuel: String },
    #[error("Cette structure n'est plus validée : l'invitation ne peut pas être acceptée.")]
    StructureNonValidee,
    #[error("Vous avez déjà ce rôle dans cette structure.")]
    AffectationDejaActive,
    #[error("{0}")]
    ComptePro(ErreurComptePro),
    #[error("erreur interne : {0}")]
    Interne(&'static str),
}

impl From<ErreurComptePro> for ErreurInvitation {
    fn from(erreur: ErreurComptePro) -> Self {
        match erreur {
            ErreurComptePro::Interne(etape) => ErreurInvitation::Interne(etape),
            autre => ErreurInvitation::ComptePro(autre),
        }
    }
}

fn interne(etape: &'static str, erreur: &sqlx::Error) -> ErreurInvitation {
    tracing::error!(etape, nature = %nature_sqlx(erreur), "invitation : erreur interne");
    ErreurInvitation::Interne(etape)
}

/// Statut lu, `expiree` déduite (M4). Macro et `concat!` : la requête reste
/// une chaîne littérale, exigée par sqlx 0.9 (trait `SqlSafeStr`, contre
/// les injections), comme `select_resume!` ailleurs.
macro_rules! statut_lu {
    () => {
        "CASE WHEN i.statut = 'en_attente' AND i.expire_le <= now() THEN 'expiree' ELSE i.statut END"
    };
}

/// Verrouille la structure (2ᵉ rang de l'ordre des verrous).
async fn verrouiller_structure(conn: &mut PgConnection, structure_id: Uuid) -> Result<(String, String, String), ErreurInvitation> {
    let ligne: Option<(String, String, String)> =
        sqlx::query_as("SELECT statut, type_structure, nom FROM structure WHERE id = $1 FOR UPDATE")
            .bind(structure_id)
            .fetch_optional(&mut *conn)
            .await
            .map_err(|e| interne("verrou de la structure", &e))?;
    ligne.ok_or(ErreurInvitation::InvitationInconnue)
}

// ---------------------------------------------------------------------
// Côté responsable (I1, I4, I9 à I11, I15)
// ---------------------------------------------------------------------

/// Le responsable qui invite, tel que l'extracteur l'a contrôlé (affectation
/// active `responsable`, structure validée sous licence).
#[derive(Debug, Clone, Copy)]
pub struct Responsable<'a> {
    pub compte_id: Uuid,
    pub structure_id: Uuid,
    pub type_structure: &'a str,
}

#[derive(Debug, Serialize)]
pub struct InvitationCreee {
    pub invitation_id: Uuid,
    pub expire_le: DateTime<Utc>,
}

/// `POST /api/professionnels/invitations`. Réponse identique que le numéro
/// ait un compte ou non (I4) : rien n'est cherché à partir du numéro.
pub async fn creer(
    pool: &PgPool,
    sms: &dyn SmsSender,
    responsable: Responsable<'_>,
    telephone_saisi: &str,
    role: RoleInvitation,
) -> Result<InvitationCreee, ErreurInvitation> {
    let numero = telephone::normaliser(telephone_saisi).map_err(|_| ErreurInvitation::TelephoneInvalide)?;
    if !roles_permis(responsable.type_structure).contains(&role) {
        return Err(ErreurInvitation::RoleNonPermis);
    }
    // I11 : son propre numéro (celui de son identité, vérifié par l'OTP).
    // Ne révèle rien : c'est le numéro de l'invitant lui-même.
    let propre: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM compte c JOIN utilisateur u ON u.id = c.utilisateur_id WHERE c.id = $1 AND u.telephone = $2)",
    )
    .bind(responsable.compte_id)
    .bind(&numero)
    .fetch_one(pool)
    .await
    .map_err(|e| interne("contrôle de l'auto-invitation", &e))?;
    if propre {
        return Err(ErreurInvitation::AutoInvitation);
    }

    let mut tx = pool.begin().await.map_err(|e| interne("transaction", &e))?;
    // Structure verrouillée : les limites sont comptées sans que deux
    // créations simultanées les dépassent.
    let (statut, _, nom) = verrouiller_structure(&mut tx, responsable.structure_id).await?;
    if statut != "validee" {
        return Err(ErreurInvitation::StructureNonValidee);
    }

    let (par_24h, en_attente, doublon, sms_numero): (i64, i64, bool, i64) = sqlx::query_as(
        "SELECT \
           (SELECT count(*) FROM invitation WHERE structure_id = $1 AND date_creation > now() - interval '24 hours'), \
           (SELECT count(*) FROM invitation WHERE structure_id = $1 AND statut = 'en_attente' AND expire_le > now()), \
           EXISTS (SELECT 1 FROM invitation WHERE structure_id = $1 AND telephone = $2 AND role = $3 \
                   AND statut = 'en_attente' AND expire_le > now()), \
           (SELECT count(*) FROM invitation WHERE telephone = $2 AND date_creation > now() - interval '24 hours')",
    )
    .bind(responsable.structure_id)
    .bind(&numero)
    .bind(role.as_str())
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| interne("comptage des invitations", &e))?;
    if par_24h >= INVITATIONS_PAR_STRUCTURE_PAR_24H {
        return Err(ErreurInvitation::TropDeDemandes);
    }
    if en_attente >= INVITATIONS_EN_ATTENTE_MAX {
        return Err(ErreurInvitation::TropDInvitationsEnAttente);
    }
    // I10 : contrôlé par le code sous le verrou de la structure ; un index
    // unique bloquerait la réinvitation après une expiration (M4).
    if doublon {
        return Err(ErreurInvitation::InvitationDejaEnAttente);
    }

    // Date d'expiration calculée par PostgreSQL (une seule horloge), et son
    // affichage au format du Burkina pour le SMS.
    let (invitation_id, expire_le, expire_texte): (Uuid, DateTime<Utc>, String) = sqlx::query_as(
        "INSERT INTO invitation (structure_id, role, telephone, invite_par_compte_id, expire_le) \
         VALUES ($1, $2, $3, $4, now() + make_interval(days => $5)) \
         RETURNING id, expire_le, to_char(expire_le AT TIME ZONE 'Africa/Ouagadougou', 'DD/MM/YYYY')",
    )
    .bind(responsable.structure_id)
    .bind(role.as_str())
    .bind(&numero)
    .bind(responsable.compte_id)
    .bind(VALIDITE_INVITATION_JOURS)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| interne("création de l'invitation", &e))?;
    tx.commit().await.map_err(|e| interne("validation de la transaction", &e))?;

    // Après le commit : un échec d'envoi n'annule jamais l'invitation, qui
    // reste visible dans l'application (I7). Journaux sans le numéro.
    if sms_numero < SMS_INVITATION_PAR_NUMERO_24H {
        if sms.envoyer(&numero, &texte_sms(&nom, role, &expire_texte)).is_err() {
            tracing::error!("invitation : échec d'envoi du SMS");
        }
    } else {
        tracing::warn!("invitation : SMS non envoyé, limite par numéro atteinte");
    }
    Ok(InvitationCreee { invitation_id, expire_le })
}

/// Une invitation, vue par la structure (I4) : jamais de nom avant
/// l'acceptation.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct InvitationDeLaStructure {
    pub id: Uuid,
    pub telephone: String,
    pub role: String,
    /// `en_attente`, `expiree`, `acceptee`, `refusee` ou `annulee`.
    pub statut: String,
    pub date_creation: DateTime<Utc>,
    pub expire_le: DateTime<Utc>,
    pub date_reponse: Option<DateTime<Utc>>,
    /// Après l'acceptation seulement (la personne a consenti).
    pub accepte_par_nom: Option<String>,
    pub accepte_par_prenom: Option<String>,
}

/// `GET /api/professionnels/invitations` : les plus récentes d'abord,
/// pagination par curseur (`avant` : identifiant de la dernière ligne de
/// la page précédente).
pub async fn lister_pour_structure(
    pool: &PgPool,
    structure_id: Uuid,
    avant: Option<Uuid>,
    limite: i64,
) -> Result<Vec<InvitationDeLaStructure>, sqlx::Error> {
    sqlx::query_as(concat!(
        "SELECT i.id, i.telephone, i.role, ",
        statut_lu!(),
        " AS statut, i.date_creation, i.expire_le, i.date_reponse, \
                u.nom AS accepte_par_nom, u.prenom AS accepte_par_prenom \
         FROM invitation i \
         LEFT JOIN affectation a ON a.id = i.affectation_id \
         LEFT JOIN compte c ON c.id = a.compte_id \
         LEFT JOIN utilisateur u ON u.id = c.utilisateur_id \
         WHERE i.structure_id = $1 AND ($2::uuid IS NULL OR i.id < $2) \
         ORDER BY i.id DESC LIMIT $3"
    ))
    .bind(structure_id)
    .bind(avant)
    .bind(limite)
    .fetch_all(pool)
    .await
}

/// `POST /api/professionnels/invitations/{id}/annuler` (I15) : par tout
/// responsable actif de la structure (l'invitant en est un).
pub async fn annuler(pool: &PgPool, structure_id: Uuid, invitation_id: Uuid) -> Result<(), ErreurInvitation> {
    let mut tx = pool.begin().await.map_err(|e| interne("transaction", &e))?;
    verrouiller_structure(&mut tx, structure_id).await?;
    let ligne: Option<String> = sqlx::query_scalar(concat!(
        "SELECT ",
        statut_lu!(),
        " FROM invitation i WHERE i.id = $1 AND i.structure_id = $2 FOR UPDATE"
    ))
    .bind(invitation_id)
    .bind(structure_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| interne("verrou de l'invitation", &e))?;
    controler_en_attente(ligne)?;
    sqlx::query("UPDATE invitation SET statut = 'annulee', date_reponse = now() WHERE id = $1")
        .bind(invitation_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| interne("annulation", &e))?;
    tx.commit().await.map_err(|e| interne("validation de la transaction", &e))
}

/// Statut lu sous verrou → erreur claire (I16).
fn controler_en_attente(statut: Option<String>) -> Result<(), ErreurInvitation> {
    match statut.as_deref() {
        None => Err(ErreurInvitation::InvitationInconnue),
        Some("en_attente") => Ok(()),
        Some("expiree") => Err(ErreurInvitation::InvitationExpiree),
        Some(_) => Err(ErreurInvitation::TransitionInvalide { statut_actuel: statut.unwrap_or_default() }),
    }
}

// ---------------------------------------------------------------------
// Côté invité, depuis le compte patient (I2, I5, I12 à I14)
// ---------------------------------------------------------------------

/// Une invitation reçue (I5, complété par le porteur le 2026-10-21 : le nom
/// et le prénom de l'invitant, pour juger si l'invitation est légitime).
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct InvitationRecue {
    pub id: Uuid,
    pub structure_nom: String,
    pub structure_type: String,
    pub role: String,
    pub expire_le: DateTime<Utc>,
    pub invitant_nom: Option<String>,
    pub invitant_prenom: Option<String>,
}

/// `GET /api/patients/invitations` : invitations au numéro vérifié du
/// compte patient, en attente, non expirées, d'une structure validée.
pub async fn lister_pour_patient(pool: &PgPool, patient_compte_id: Uuid) -> Result<Vec<InvitationRecue>, sqlx::Error> {
    sqlx::query_as(
        "SELECT i.id, s.nom AS structure_nom, s.type_structure AS structure_type, i.role, i.expire_le, \
                ui.nom AS invitant_nom, ui.prenom AS invitant_prenom \
         FROM compte pat JOIN utilisateur u ON u.id = pat.utilisateur_id \
         JOIN invitation i ON i.telephone = u.telephone \
         JOIN structure s ON s.id = i.structure_id \
         JOIN compte ci ON ci.id = i.invite_par_compte_id JOIN utilisateur ui ON ui.id = ci.utilisateur_id \
         WHERE pat.id = $1 AND pat.type_compte = 'patient' \
           AND i.statut = 'en_attente' AND i.expire_le > now() AND s.statut = 'validee' \
         ORDER BY i.id",
    )
    .bind(patient_compte_id)
    .fetch_all(pool)
    .await
}

/// Structure d'une invitation adressée au numéro du compte patient (sans
/// verrou : sert à connaître la structure à verrouiller ensuite).
async fn structure_de_l_invitation(
    pool: &PgPool,
    patient_compte_id: Uuid,
    invitation_id: Uuid,
) -> Result<Uuid, ErreurInvitation> {
    let structure: Option<Uuid> = sqlx::query_scalar(
        "SELECT i.structure_id FROM invitation i \
         JOIN utilisateur u ON u.telephone = i.telephone JOIN compte pat ON pat.utilisateur_id = u.id \
         WHERE i.id = $1 AND pat.id = $2 AND pat.type_compte = 'patient' AND i.structure_id IS NOT NULL",
    )
    .bind(invitation_id)
    .bind(patient_compte_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| interne("recherche de l'invitation", &e))?;
    structure.ok_or(ErreurInvitation::InvitationInconnue)
}

/// Invitation verrouillée (3ᵉ rang), adressée au numéro du compte patient.
async fn verrouiller_invitation_du_patient(
    conn: &mut PgConnection,
    patient_compte_id: Uuid,
    invitation_id: Uuid,
) -> Result<String, ErreurInvitation> {
    let ligne: Option<(String, String)> = sqlx::query_as(concat!(
        "SELECT ",
        statut_lu!(),
        ", i.role FROM invitation i \
         JOIN utilisateur u ON u.telephone = i.telephone JOIN compte pat ON pat.utilisateur_id = u.id \
         WHERE i.id = $1 AND pat.id = $2 AND pat.type_compte = 'patient' \
         FOR UPDATE OF i"
    ))
    .bind(invitation_id)
    .bind(patient_compte_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(|e| interne("verrou de l'invitation", &e))?;
    let (statut, role) = match ligne {
        Some((statut, role)) => (Some(statut), role),
        None => (None, String::new()),
    };
    controler_en_attente(statut)?;
    Ok(role)
}

/// Résultat d'une acceptation.
#[derive(Debug)]
pub struct Acceptation {
    pub affectation_id: Uuid,
    /// Compte professionnel créé avec un email non vérifié : l'affectation
    /// n'est exerçable qu'après le lien (R0, D4).
    pub email_a_verifier: bool,
}

/// `POST /api/patients/invitations/{id}/accepter` (I13).
pub async fn accepter(
    pool: &PgPool,
    email_sender: &dyn EmailSender,
    url_application_web: &str,
    patient_compte_id: Uuid,
    invitation_id: Uuid,
    saisie: &compte_professionnel::Saisie<'_>,
) -> Result<Acceptation, ErreurInvitation> {
    // Lecture d'abord : une invitation inconnue ne fait compter aucun essai
    // de mot de passe.
    let structure_id = structure_de_l_invitation(pool, patient_compte_id, invitation_id).await?;
    let preparation = compte_professionnel::preparer(pool, patient_compte_id, saisie).await?;
    let email_a_verifier = !preparation.email_verifie();

    let mut tx = pool.begin().await.map_err(|e| interne("transaction", &e))?;
    // Ordre des verrous : comptes, structure, invitation.
    let compte = compte_professionnel::ecrire(&mut tx, patient_compte_id, preparation).await?;
    let (statut_structure, _, _) = verrouiller_structure(&mut tx, structure_id).await?;
    let role = verrouiller_invitation_du_patient(&mut tx, patient_compte_id, invitation_id).await?;
    if statut_structure != "validee" {
        return Err(ErreurInvitation::StructureNonValidee);
    }

    // I12 : déjà ce rôle, actif, dans cette structure (index unique de la
    // 0012).
    let affectation_id: Uuid = sqlx::query_scalar(
        "INSERT INTO affectation (compte_id, structure_id, role) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(compte.compte_id)
    .bind(structure_id)
    .bind(&role)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| match e.as_database_error().and_then(|d| d.constraint()) {
        Some("affectation_une_active_par_role") => ErreurInvitation::AffectationDejaActive,
        _ => interne("création de l'affectation", &e),
    })?;
    sqlx::query("UPDATE invitation SET statut = 'acceptee', date_reponse = now(), affectation_id = $2 WHERE id = $1")
        .bind(invitation_id)
        .bind(affectation_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| interne("acceptation", &e))?;
    tx.commit().await.map_err(|e| interne("validation de la transaction", &e))?;

    if let Some(emise) = &compte.verification {
        verification_email::envoyer_lien(email_sender, url_application_web, emise);
    }
    Ok(Acceptation { affectation_id, email_a_verifier })
}

/// `POST /api/patients/invitations/{id}/refuser` (I14).
pub async fn refuser(pool: &PgPool, patient_compte_id: Uuid, invitation_id: Uuid) -> Result<(), ErreurInvitation> {
    let structure_id = structure_de_l_invitation(pool, patient_compte_id, invitation_id).await?;
    let mut tx = pool.begin().await.map_err(|e| interne("transaction", &e))?;
    verrouiller_structure(&mut tx, structure_id).await?;
    verrouiller_invitation_du_patient(&mut tx, patient_compte_id, invitation_id).await?;
    sqlx::query("UPDATE invitation SET statut = 'refusee', date_reponse = now() WHERE id = $1")
        .bind(invitation_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| interne("refus", &e))?;
    tx.commit().await.map_err(|e| interne("validation de la transaction", &e))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::affectation;
    use crate::email::{EmailsRecus, jeton_du_lien};
    use crate::mot_de_passe;
    use crate::sms::ErreurSms;

    // Base temporaire par test ; données fictives.

    const PRO: &str = "Professionnel#2026";

    #[derive(Default)]
    struct SmsRecus(Mutex<Vec<(String, String)>>);

    impl SmsSender for SmsRecus {
        fn envoyer(&self, telephone: &str, message: &str) -> Result<(), ErreurSms> {
            self.0.lock().unwrap().push((telephone.to_string(), message.to_string()));
            Ok(())
        }
        fn simule(&self) -> bool {
            true
        }
    }

    impl SmsRecus {
        fn nombre(&self) -> usize {
            self.0.lock().unwrap().len()
        }
    }

    async fn identite(pool: &PgPool, telephone: &str, email: Option<&str>) -> Uuid {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (nom, prenom, telephone, email) VALUES ('SAWADOGO', 'Issa', $1, $2) RETURNING id",
        )
        .bind(telephone)
        .bind(email)
        .fetch_one(pool)
        .await
        .unwrap();
        if email.is_some() {
            sqlx::query("UPDATE utilisateur SET email_verifie_le = now() WHERE id = $1").bind(id).execute(pool).await.unwrap();
        }
        id
    }

    async fn compte(pool: &PgPool, utilisateur_id: Uuid, type_compte: &str) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(utilisateur_id)
        .bind(type_compte)
        .bind(mot_de_passe::hacher(PRO).unwrap())
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// MUNASEB validée et son responsable (compte professionnel). Renvoie
    /// (structure, compte du responsable).
    async fn munaseb(pool: &PgPool, telephone_responsable: &str) -> (Uuid, Uuid) {
        let u = identite(pool, telephone_responsable, Some(&format!("r{}@example.org", &telephone_responsable[4..]))).await;
        let pro = compte(pool, u, "professionnel").await;
        let structure_id: Uuid = sqlx::query_scalar(
            "INSERT INTO structure (type_structure, nom, telephone, telephone_urgence, region, email, commune, province, \
                                    latitude, longitude, createur_compte_id, statut) \
             VALUES ('munaseb', 'MUNASEB (test)', '+22670000000', '+22670000000', 'Région fictive', 'structure@example.org', \
                     'Commune fictive', 'Province fictive', 12.371400, -1.519700, $1, 'validee') RETURNING id",
        )
        .bind(pro)
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO affectation (compte_id, structure_id, role) VALUES ($1, $2, 'responsable')")
            .bind(pro)
            .bind(structure_id)
            .execute(pool)
            .await
            .unwrap();
        (structure_id, pro)
    }

    fn responsable(structure_id: Uuid, compte_id: Uuid) -> Responsable<'static> {
        Responsable { compte_id, structure_id, type_structure: "munaseb" }
    }

    /// Compte patient (sans compte professionnel) au numéro donné.
    async fn patient(pool: &PgPool, telephone: &str) -> Uuid {
        let u = identite(pool, telephone, None).await;
        compte(pool, u, "patient").await
    }

    fn saisie_nouveau<'a>(email: &'a str) -> compte_professionnel::Saisie<'a> {
        compte_professionnel::Saisie { email_professionnel: Some(email), mot_de_passe: PRO }
    }

    /// I4, I6 : même réponse et même SMS, que le numéro ait un compte ou
    /// non ; ni lien ni code ; phrase de sécurité à la fin.
    #[sqlx::test]
    #[ignore]
    async fn reponse_et_sms_identiques_avec_ou_sans_compte(pool: PgPool) {
        let sms = SmsRecus::default();
        let (structure_id, pro) = munaseb(&pool, "+22670200001").await;
        patient(&pool, "+22670200002").await;

        let avec = creer(&pool, &sms, responsable(structure_id, pro), "70 20 00 02", RoleInvitation::AgentAssuranceMunaseb).await.unwrap();
        let sans = creer(&pool, &sms, responsable(structure_id, pro), "70200003", RoleInvitation::AgentAssuranceMunaseb).await.unwrap();
        assert!(avec.expire_le > Utc::now() + chrono::Duration::days(6));
        assert!(sans.expire_le > Utc::now() + chrono::Duration::days(6));

        let recus = sms.0.lock().unwrap().clone();
        assert_eq!(recus.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(), ["+22670200002", "+22670200003"]);
        assert_eq!(recus[0].1, recus[1].1, "SMS identique");
        assert!(recus[0].1.contains("« MUNASEB (test) » comme agent MUNASEB"), "{}", recus[0].1);
        assert!(recus[0].1.ends_with("LaafiCare ne vous demandera jamais votre mot de passe."));
        assert!(!recus[0].1.contains("http"));
    }

    #[sqlx::test]
    #[ignore]
    async fn refus_a_la_creation(pool: PgPool) {
        let sms = SmsRecus::default();
        let (structure_id, pro) = munaseb(&pool, "+22670200011").await;
        let r = responsable(structure_id, pro);

        assert!(matches!(creer(&pool, &sms, r, "12", RoleInvitation::Responsable).await, Err(ErreurInvitation::TelephoneInvalide)));
        // I11 : son propre numéro.
        assert!(matches!(
            creer(&pool, &sms, r, "70 20 00 11", RoleInvitation::AgentAssuranceMunaseb).await,
            Err(ErreurInvitation::AutoInvitation)
        ));
        // I3 : rôle non permis pour ce type de structure.
        let clinique = Responsable { type_structure: "clinique", ..r };
        assert!(matches!(
            creer(&pool, &sms, clinique, "70200012", RoleInvitation::AgentAssuranceMunaseb).await,
            Err(ErreurInvitation::RoleNonPermis)
        ));
        // I10 : doublon ; un autre rôle reste possible.
        creer(&pool, &sms, r, "70200012", RoleInvitation::AgentAssuranceMunaseb).await.unwrap();
        assert!(matches!(
            creer(&pool, &sms, r, "70200012", RoleInvitation::AgentAssuranceMunaseb).await,
            Err(ErreurInvitation::InvitationDejaEnAttente)
        ));
        creer(&pool, &sms, r, "70200012", RoleInvitation::Responsable).await.unwrap();
        // Structure qui n'est plus validée.
        sqlx::query("UPDATE structure SET statut = 'suspendue' WHERE id = $1").bind(structure_id).execute(&pool).await.unwrap();
        assert!(matches!(
            creer(&pool, &sms, r, "70200013", RoleInvitation::AgentAssuranceMunaseb).await,
            Err(ErreurInvitation::StructureNonValidee)
        ));
    }

    /// I9 a, b, c.
    #[sqlx::test]
    #[ignore]
    async fn limites(pool: PgPool) {
        let sms = SmsRecus::default();
        let (structure_id, pro) = munaseb(&pool, "+22670200021").await;
        let r = responsable(structure_id, pro);

        // c : 3 SMS par numéro en 24 h, puis la même réponse sans SMS.
        for role in [RoleInvitation::AgentAssuranceMunaseb, RoleInvitation::Responsable] {
            creer(&pool, &sms, r, "70200099", role).await.unwrap();
        }
        let (autre, autre_pro) = (structure_id, pro);
        sqlx::query("UPDATE invitation SET statut = 'annulee', date_reponse = now() WHERE telephone = '+22670200099'")
            .execute(&pool)
            .await
            .unwrap();
        creer(&pool, &sms, responsable(autre, autre_pro), "70200099", RoleInvitation::AgentAssuranceMunaseb).await.unwrap();
        assert_eq!(sms.nombre(), 3);
        sqlx::query("UPDATE invitation SET statut = 'annulee', date_reponse = now() WHERE telephone = '+22670200099'")
            .execute(&pool)
            .await
            .unwrap();
        creer(&pool, &sms, r, "70200099", RoleInvitation::Responsable).await.unwrap();
        assert_eq!(sms.nombre(), 3, "4ᵉ invitation au même numéro : enregistrée, sans SMS");

        // a : 20 créations par structure en 24 h (4 déjà faites).
        for i in 0..16 {
            creer(&pool, &sms, r, &format!("7021{i:04}"), RoleInvitation::AgentAssuranceMunaseb).await.unwrap();
        }
        assert!(matches!(
            creer(&pool, &sms, r, "70219999", RoleInvitation::AgentAssuranceMunaseb).await,
            Err(ErreurInvitation::TropDeDemandes)
        ));

        // b : 50 en attente ; les créations anciennes ne comptent plus dans
        // la limite a.
        sqlx::query("UPDATE invitation SET date_creation = now() - interval '2 days' WHERE structure_id = $1")
            .bind(structure_id)
            .execute(&pool)
            .await
            .unwrap();
        let mut n = 0;
        let resultat = loop {
            let r2 = creer(&pool, &sms, r, &format!("7022{n:04}"), RoleInvitation::AgentAssuranceMunaseb).await;
            if r2.is_err() {
                break r2;
            }
            n += 1;
            sqlx::query("UPDATE invitation SET date_creation = now() - interval '2 days' WHERE structure_id = $1")
                .bind(structure_id)
                .execute(&pool)
                .await
                .unwrap();
        };
        assert!(matches!(resultat, Err(ErreurInvitation::TropDInvitationsEnAttente)), "{resultat:?}");
        let en_attente: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM invitation WHERE structure_id = $1 AND statut = 'en_attente' AND expire_le > now()",
        )
        .bind(structure_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(en_attente, INVITATIONS_EN_ATTENTE_MAX);
    }

    /// Parcours : invitation, liste côté invité (avec l'invitant), première
    /// acceptation (compte professionnel créé, lien envoyé), affectation
    /// exerçable après la vérification ; liste côté structure avec le nom
    /// après acceptation.
    #[sqlx::test]
    #[ignore]
    async fn acceptation_cree_le_compte_professionnel(pool: PgPool) {
        let sms = SmsRecus::default();
        let emails = EmailsRecus::default();
        let (structure_id, pro) = munaseb(&pool, "+22670200031").await;
        let invite = patient(&pool, "+22670200032").await;
        let creee = creer(&pool, &sms, responsable(structure_id, pro), "70200032", RoleInvitation::AgentAssuranceMunaseb).await.unwrap();

        // La liste de la structure ne nomme personne avant l'acceptation.
        let avant = lister_pour_structure(&pool, structure_id, None, 50).await.unwrap();
        assert_eq!((avant[0].statut.as_str(), avant[0].accepte_par_nom.as_deref()), ("en_attente", None));

        let recues = lister_pour_patient(&pool, invite).await.unwrap();
        assert_eq!(recues.len(), 1);
        assert_eq!(
            (recues[0].structure_nom.as_str(), recues[0].role.as_str(), recues[0].invitant_nom.as_deref()),
            ("MUNASEB (test)", "agent_assurance_munaseb", Some("SAWADOGO"))
        );

        let acceptation = accepter(&pool, &emails, "http://localhost:4200", invite, creee.invitation_id, &saisie_nouveau("agent.invite@example.org"))
            .await
            .unwrap();
        assert!(acceptation.email_a_verifier);
        let compte_pro: Uuid = sqlx::query_scalar("SELECT compte_id FROM affectation WHERE id = $1")
            .bind(acceptation.affectation_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        let liste = affectation::lister(&pool, compte_pro).await.unwrap();
        assert_eq!(liste[0].role, "agent_assurance_munaseb");

        // Le lien vérifie l'email ; l'affectation devient exerçable (D4).
        assert!(liste[0].exercable, "structure validée sous licence");
        // Connexion refusée tant que l'email n'est pas vérifié (R0), puis
        // possible : l'agent pourra activer son affectation.
        let jwt = crate::jwt::JwtService::new(&"a".repeat(32));
        assert!(matches!(
            crate::auth_professionnel::se_connecter(&pool, &jwt, "agent.invite@example.org", PRO).await,
            Err(crate::auth_professionnel::ErreurAuthProfessionnel::EmailNonVerifie)
        ));
        let (_, corps) = emails.0.lock().unwrap()[0].clone();
        assert!(verification_email::verifier(&pool, &jeton_du_lien(&corps)).await.unwrap());
        assert!(matches!(
            crate::auth_professionnel::se_connecter(&pool, &jwt, "agent.invite@example.org", PRO).await,
            Ok(crate::auth_professionnel::ConnexionProfessionnelle::Complete(_))
        ));

        let apres = lister_pour_structure(&pool, structure_id, None, 50).await.unwrap();
        assert_eq!((apres[0].statut.as_str(), apres[0].accepte_par_nom.as_deref()), ("acceptee", Some("SAWADOGO")));
        assert!(lister_pour_patient(&pool, invite).await.unwrap().is_empty());
    }

    /// I12, I13 : avec un compte professionnel, le mot de passe est exigé
    /// et compté ; un rôle déjà actif est refusé.
    #[sqlx::test]
    #[ignore]
    async fn acceptation_avec_compte_existant(pool: PgPool) {
        let sms = SmsRecus::default();
        let emails = EmailsRecus::default();
        let (structure_id, pro) = munaseb(&pool, "+22670200041").await;
        let u = identite(&pool, "+22670200042", Some("deja.pro@example.org")).await;
        let invite = compte(&pool, u, "patient").await;
        let pro_invite = compte(&pool, u, "professionnel").await;
        let r = responsable(structure_id, pro);

        let premiere = creer(&pool, &sms, r, "70200042", RoleInvitation::AgentAssuranceMunaseb).await.unwrap();
        let mauvais = compte_professionnel::Saisie { email_professionnel: None, mot_de_passe: "Mauvais#2026" };
        assert!(matches!(
            accepter(&pool, &emails, "http://localhost:4200", invite, premiere.invitation_id, &mauvais).await,
            Err(ErreurInvitation::ComptePro(ErreurComptePro::MotDePasseIncorrect))
        ));
        let tentatives: i16 = sqlx::query_scalar("SELECT tentatives_echouees FROM compte WHERE id = $1")
            .bind(pro_invite)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(tentatives, 1, "compté dans le verrouillage");

        let bon = compte_professionnel::Saisie { email_professionnel: None, mot_de_passe: PRO };
        let acceptation = accepter(&pool, &emails, "http://localhost:4200", invite, premiere.invitation_id, &bon).await.unwrap();
        assert!(!acceptation.email_a_verifier);
        assert!(emails.0.lock().unwrap().is_empty(), "aucun lien pour un compte existant");

        // Même rôle, déjà actif : refusé, l'invitation reste en attente.
        let seconde = creer(&pool, &sms, r, "70200042", RoleInvitation::AgentAssuranceMunaseb).await.unwrap();
        assert!(matches!(
            accepter(&pool, &emails, "http://localhost:4200", invite, seconde.invitation_id, &bon).await,
            Err(ErreurInvitation::AffectationDejaActive)
        ));
    }

    /// I2 : l'invitation d'un autre numéro est inconnue ; structure plus
    /// validée ; refus par l'invité.
    #[sqlx::test]
    #[ignore]
    async fn autre_numero_structure_suspendue_et_refus(pool: PgPool) {
        let sms = SmsRecus::default();
        let emails = EmailsRecus::default();
        let (structure_id, pro) = munaseb(&pool, "+22670200051").await;
        let invite = patient(&pool, "+22670200052").await;
        let intrus = patient(&pool, "+22670200053").await;
        let creee = creer(&pool, &sms, responsable(structure_id, pro), "70200052", RoleInvitation::AgentAssuranceMunaseb).await.unwrap();

        assert!(lister_pour_patient(&pool, intrus).await.unwrap().is_empty());
        assert!(matches!(
            accepter(&pool, &emails, "http://localhost:4200", intrus, creee.invitation_id, &saisie_nouveau("intrus@example.org")).await,
            Err(ErreurInvitation::InvitationInconnue)
        ));
        assert!(matches!(refuser(&pool, intrus, creee.invitation_id).await, Err(ErreurInvitation::InvitationInconnue)));

        sqlx::query("UPDATE structure SET statut = 'suspendue' WHERE id = $1").bind(structure_id).execute(&pool).await.unwrap();
        assert!(matches!(
            accepter(&pool, &emails, "http://localhost:4200", invite, creee.invitation_id, &saisie_nouveau("invite@example.org")).await,
            Err(ErreurInvitation::StructureNonValidee)
        ));
        let comptes_pro: i64 = sqlx::query_scalar("SELECT count(*) FROM compte WHERE type_compte = 'professionnel'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(comptes_pro, 1, "rien n'est créé pour l'invité");

        sqlx::query("UPDATE structure SET statut = 'validee' WHERE id = $1").bind(structure_id).execute(&pool).await.unwrap();
        refuser(&pool, invite, creee.invitation_id).await.unwrap();
        let liste = lister_pour_structure(&pool, structure_id, None, 50).await.unwrap();
        assert_eq!((liste[0].statut.as_str(), liste[0].accepte_par_nom.as_deref()), ("refusee", None));
    }

    /// I15, I16 : annulation ; une invitation expirée se lit `expiree` et
    /// n'a plus de suite.
    #[sqlx::test]
    #[ignore]
    async fn annulation_et_expiration(pool: PgPool) {
        let sms = SmsRecus::default();
        let emails = EmailsRecus::default();
        let (structure_id, pro) = munaseb(&pool, "+22670200061").await;
        let invite = patient(&pool, "+22670200062").await;
        let r = responsable(structure_id, pro);

        let a_annuler = creer(&pool, &sms, r, "70200062", RoleInvitation::AgentAssuranceMunaseb).await.unwrap();
        annuler(&pool, structure_id, a_annuler.invitation_id).await.unwrap();
        assert!(matches!(
            annuler(&pool, structure_id, a_annuler.invitation_id).await,
            Err(ErreurInvitation::TransitionInvalide { ref statut_actuel }) if statut_actuel == "annulee"
        ));
        // L'invitation d'une autre structure n'existe pas pour celle-ci.
        assert!(matches!(annuler(&pool, Uuid::nil(), a_annuler.invitation_id).await, Err(ErreurInvitation::InvitationInconnue)));

        let a_expirer = creer(&pool, &sms, r, "70200062", RoleInvitation::AgentAssuranceMunaseb).await.unwrap();
        sqlx::query("UPDATE invitation SET date_creation = now() - interval '8 days', expire_le = now() - interval '1 day' WHERE id = $1")
            .bind(a_expirer.invitation_id)
            .execute(&pool)
            .await
            .unwrap();
        let liste = lister_pour_structure(&pool, structure_id, None, 50).await.unwrap();
        assert_eq!(liste.iter().find(|i| i.id == a_expirer.invitation_id).unwrap().statut, "expiree");
        assert!(lister_pour_patient(&pool, invite).await.unwrap().is_empty());
        assert!(matches!(
            accepter(&pool, &emails, "http://localhost:4200", invite, a_expirer.invitation_id, &saisie_nouveau("expire@example.org")).await,
            Err(ErreurInvitation::InvitationExpiree)
        ));
        assert!(matches!(refuser(&pool, invite, a_expirer.invitation_id).await, Err(ErreurInvitation::InvitationExpiree)));
        assert!(matches!(annuler(&pool, structure_id, a_expirer.invitation_id).await, Err(ErreurInvitation::InvitationExpiree)));
    }

    /// Ordre des verrous : une acceptation et une annulation lancées en même
    /// temps sur la même invitation ; une seule réussit, sans interblocage
    /// (aucune erreur interne, PostgreSQL signalerait un interblocage par
    /// le SQLSTATE 40P01).
    #[sqlx::test]
    #[ignore]
    async fn acceptation_et_annulation_simultanees(pool: PgPool) {
        let sms = SmsRecus::default();
        let (structure_id, pro) = munaseb(&pool, "+22670200071").await;
        let invite = patient(&pool, "+22670200072").await;

        for essai in 0..5 {
            let creee = creer(&pool, &sms, responsable(structure_id, pro), "70200072", RoleInvitation::AgentAssuranceMunaseb)
                .await
                .unwrap();
            let (p1, p2) = (pool.clone(), pool.clone());
            let id = creee.invitation_id;
            let email = format!("simultane{essai}@example.org");
            let acceptation = tokio::spawn(async move {
                let emails = EmailsRecus::default();
                accepter(&p1, &emails, "http://localhost:4200", invite, id, &saisie_nouveau(&email)).await.map(|_| ())
            });
            let annulation = tokio::spawn(async move { annuler(&p2, structure_id, id).await });
            let (a, b) = (acceptation.await.unwrap(), annulation.await.unwrap());
            assert!(a.is_ok() != b.is_ok(), "une seule réussit : {a:?} / {b:?}");
            for perdant in [&a, &b] {
                if let Err(e) = perdant {
                    assert!(matches!(e, ErreurInvitation::TransitionInvalide { .. }), "pas d'interblocage : {e:?}");
                }
            }
            // Si l'acceptation a gagné, l'affectation existe : on la clôt
            // pour l'essai suivant (même rôle).
            if a.is_ok() {
                sqlx::query(
                    "UPDATE affectation SET statut = 'desactivee', desactivee_le = now(), desactivee_par_compte_id = $1 \
                     WHERE structure_id = $2 AND role = 'agent_assurance_munaseb' AND statut = 'active'",
                )
                .bind(pro)
                .bind(structure_id)
                .execute(&pool)
                .await
                .unwrap();
            }
        }
    }

    #[test]
    fn texte_du_sms() {
        let texte = texte_sms("MUNASEB", RoleInvitation::AgentAssuranceMunaseb, "28/10/2026");
        assert!(texte.starts_with("LaafiCare : vous êtes invité(e) à rejoindre « MUNASEB » comme agent MUNASEB."));
        assert!(texte.contains("avant le 28/10/2026."));
        assert!(texte.ends_with("LaafiCare ne vous demandera jamais votre mot de passe."));
    }
}
