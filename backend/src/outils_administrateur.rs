//! Logique des outils locaux de l'équipe LaafiCare (étape 3.6.5, décisions
//! B1 à B10 du 2026-10-18) : premier administrateur, administrateurs
//! suivants, reprise d'un administrateur qui n'a jamais eu de TOTP actif,
//! désactivation d'un TOTP. Jamais une route HTTP.
//!
//! Les programmes de `src/bin/` ne font que poser les questions et afficher
//! les réponses (B4) : tout ce qui décide est ici, testé en base.
//!
//! Règles communes :
//! - un administrateur enregistre lui-même son TOTP, pendant une fenêtre de
//!   10 minutes ouverte par l'outil (Y4 révisé), par les routes de la
//!   3.6.4 ; aucun secret TOTP n'est jamais affiché par un outil ;
//! - son email est vérifié par un jeton envoyé par email, que la personne
//!   colle dans l'outil (R0) ;
//! - chaque création, reprise ou désactivation est tracée, sans jamais être
//!   modifiée ni supprimée (migrations 0013 et 0020).

use sqlx::types::Uuid;
use sqlx::{PgConnection, PgPool};

use crate::adresse_email;
use crate::auth_administrateur::FENETRE_REENROLEMENT_MINUTES;
use crate::auth_professionnel::{self, ErreurAuthProfessionnel};
use crate::config::CleTotp;
use crate::email::EmailSender;
use crate::jeton_email::{self, VerificationEmise};
use crate::mot_de_passe::{self, MotDePasseNonConforme};
use crate::session::{self, MotifRevocation};
use crate::telephone;
use crate::totp::{self, ErreurTotp};
use crate::verrouillage::{self, ErreurConnexion};

#[derive(Debug, thiserror::Error)]
pub enum ErreurOutil {
    #[error("Un administrateur existe déjà : utilisez l'outil creer_administrateur.")]
    AdministrateurExisteDeja,
    #[error("Cette personne a déjà un compte administrateur.")]
    DejaAdministrateur,
    #[error("Numéro de téléphone invalide.")]
    TelephoneInvalide,
    #[error("Aucun compte patient actif pour ce numéro : la personne doit d'abord s'inscrire dans l'application patient.")]
    PasDeComptePatient,
    #[error("Email vide.")]
    EmailVide,
    #[error("Adresse email invalide.")]
    EmailInvalide,
    #[error("Cette identité a déjà un autre email : l'outil ne le change pas (outil « changer l'email », à concevoir).")]
    EmailDifferent,
    #[error("Cet email est déjà utilisé par une autre personne.")]
    EmailDejaUtilise,
    #[error("Mot de passe non conforme.")]
    MotDePasseNonConforme(MotDePasseNonConforme),
    #[error("Le mot de passe administrateur doit être différent de celui du compte patient.")]
    MotDePasseIdentiquePatient,
    #[error("Numéro de CNIB invalide : 8 chiffres attendus après « B ».")]
    CnibInvalide,
    #[error("Ce numéro de CNIB est déjà enregistré pour un autre administrateur.")]
    CnibDejaUtilisee,
    #[error("Authentification refusée.")]
    AuthentificationRefusee,
    #[error("Code de vérification incorrect, expiré ou déjà utilisé.")]
    JetonInvalide,
    #[error("Reprise impossible : l'administrateur doit n'avoir jamais eu de second facteur actif.")]
    RepriseImpossible,
    #[error("Compte introuvable.")]
    CompteIntrouvable,
    #[error("Ce compte n'a aucun second facteur actif à désactiver.")]
    AucunSecondFacteur,
    #[error("Un administrateur ne désactive le second facteur d'aucun de ses propres comptes.")]
    PropreCompte,
    #[error("La vérification de la CNIB est obligatoire.")]
    CnibNonVerifiee,
    #[error("Le motif est obligatoire.")]
    MotifVide,
    #[error("L'email n'a pas pu être envoyé ; relancez l'outil en mode reprise.")]
    EnvoiEmail,
    /// Seulement l'étape en échec, un texte fixe : jamais une saisie.
    #[error("erreur interne : {0}")]
    Interne(&'static str),
}

fn interne_sqlx(etape: &'static str, erreur: &sqlx::Error) -> ErreurOutil {
    tracing::error!(etape, nature = %auth_professionnel::nature_sqlx(erreur), "outil administrateur : erreur interne");
    ErreurOutil::Interne(etape)
}

fn contrainte(erreur: &sqlx::Error) -> Option<&str> {
    erreur.as_database_error().and_then(|e| e.constraint())
}

// ---------------------------------------------------------------------
// Saisies
// ---------------------------------------------------------------------

/// Ce que l'outil demande pour un nouvel administrateur. Pas de `Debug` :
/// le mot de passe est un secret.
pub struct NouvelAdministrateur<'a> {
    pub telephone: &'a str,
    pub email: &'a str,
    /// Les 8 chiffres saisis après « B » ; vide : pas de CNIB (facultative,
    /// décision du porteur du 2026-10-18, K3 révisé).
    pub cnib_chiffres: &'a str,
    pub mot_de_passe: &'a str,
}

/// Format de la CNIB : B suivi de 8 chiffres (décision du porteur, d'après
/// le format de la CNIB burkinabè ; aucune source officielle). La base le
/// contrôle aussi (migration 0013). Vide : `None`, aucun profil créé.
fn numero_cnib(chiffres: &str) -> Result<Option<String>, ErreurOutil> {
    let chiffres = chiffres.trim();
    if chiffres.is_empty() {
        return Ok(None);
    }
    if chiffres.len() != 8 || !chiffres.bytes().all(|o| o.is_ascii_digit()) {
        return Err(ErreurOutil::CnibInvalide);
    }
    Ok(Some(format!("B{chiffres}")))
}

/// Administrateur créé ou repris : l'outil attend ensuite le jeton reçu par
/// email (si `verification_envoyee`), puis ouvre la fenêtre.
#[derive(Debug)]
pub struct AdministrateurEnCours {
    pub compte_id: Uuid,
    pub email: String,
    pub verification_envoyee: bool,
}

/// Membre de l'équipe authentifié par email, mot de passe et code TOTP (Z9).
/// Ne se construit que par `authentifier_operateur`.
#[derive(Debug, Clone, Copy)]
pub struct Operateur {
    compte_id: Uuid,
}

impl Operateur {
    pub fn compte_id(&self) -> Uuid {
        self.compte_id
    }
}

// ---------------------------------------------------------------------
// Email de vérification et fenêtre de réenrôlement
// ---------------------------------------------------------------------

fn envoyer_verification(email_sender: &dyn EmailSender, emise: &VerificationEmise) -> Result<(), ErreurOutil> {
    let corps = format!(
        "Voici votre code de vérification LaafiCare. Collez-le dans l'outil qui vous l'a demandé.\n\
         Il est valable {} heures et ne sert qu'une fois.\n\n{}\n\n\
         Si vous n'êtes pas à l'origine de cette demande, ignorez ce message.",
        jeton_email::DUREE_VERIFICATION_HEURES,
        emise.jeton.en_clair()
    );
    email_sender
        .envoyer(&emise.email, "LaafiCare : vérification de votre adresse email", &corps)
        .map_err(|_| ErreurOutil::EnvoiEmail)
}

/// Vérifie l'email du compte avec le jeton collé (R0). Seul un jeton de ce
/// compte est accepté.
pub async fn verifier_email(pool: &PgPool, compte_id: Uuid, jeton: &str) -> Result<(), ErreurOutil> {
    match jeton_email::verifier_email(pool, jeton.trim(), Some(compte_id)).await {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(ErreurOutil::JetonInvalide),
        Err(e) => Err(interne_sqlx("vérification de l'email", &e)),
    }
}

/// Ouvre la fenêtre de réenrôlement de 10 minutes (migration 0019), pour un
/// compte administrateur actif, à l'email vérifié et sans TOTP actif.
/// Horloge de PostgreSQL. `false` si le compte ne remplit pas ces
/// conditions.
async fn ouvrir_fenetre_dans(conn: &mut PgConnection, compte_id: Uuid) -> Result<bool, sqlx::Error> {
    let ouverte: Option<Uuid> = sqlx::query_scalar(
        "UPDATE compte c SET reenrolement_autorise_jusqu_au = now() + make_interval(mins => $2) \
         FROM utilisateur u \
         WHERE c.id = $1 AND u.id = c.utilisateur_id AND c.type_compte = 'administrateur_laaficare' \
           AND c.statut = 'actif' AND u.email_verifie_le IS NOT NULL \
           AND NOT EXISTS (SELECT 1 FROM second_facteur_totp t WHERE t.compte_id = c.id AND t.statut = 'actif') \
         RETURNING c.id",
    )
    .bind(compte_id)
    .bind(FENETRE_REENROLEMENT_MINUTES)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(ouverte.is_some())
}

/// Ouvre la fenêtre, une fois l'email vérifié. Refus si les conditions ne
/// sont pas remplies (email pas encore vérifié, TOTP déjà actif).
pub async fn ouvrir_fenetre(pool: &PgPool, compte_id: Uuid) -> Result<(), ErreurOutil> {
    let mut conn = pool.acquire().await.map_err(|e| interne_sqlx("connexion", &e))?;
    match ouvrir_fenetre_dans(&mut conn, compte_id).await {
        Ok(true) => Ok(()),
        Ok(false) => Err(ErreurOutil::RepriseImpossible),
        Err(e) => Err(interne_sqlx("ouverture de la fenêtre", &e)),
    }
}

// ---------------------------------------------------------------------
// Authentification de l'opérateur (Z9)
// ---------------------------------------------------------------------

/// Email, mot de passe et code TOTP du membre de l'équipe : chaque essai
/// compte dans son verrouillage (BF-01), comme à la connexion. Un
/// administrateur sans TOTP actif est refusé (Y4). Une seule réponse pour
/// tous les refus.
pub async fn authentifier_operateur(
    pool: &PgPool,
    cle: &CleTotp,
    email: &str,
    mot_de_passe: &str,
    code: &str,
) -> Result<Operateur, ErreurOutil> {
    let compte = auth_professionnel::verifier_identifiants(pool, "administrateur_laaficare", email, mot_de_passe)
        .await
        .map_err(|erreur| match erreur {
            ErreurAuthProfessionnel::Interne(etape) => ErreurOutil::Interne(etape),
            _ => ErreurOutil::AuthentificationRefusee,
        })?;
    if !compte.second_facteur {
        return Err(ErreurOutil::AuthentificationRefusee);
    }
    totp::verifier_code(pool, cle.octets(), compte.compte_id, code.trim())
        .await
        .map_err(|erreur| match erreur {
            ErreurTotp::Refuse => ErreurOutil::AuthentificationRefusee,
            ErreurTotp::Interne(_) => {
                tracing::error!(etape = "code de l'opérateur", "outil administrateur : erreur interne");
                ErreurOutil::Interne("code de l'opérateur")
            }
        })?;
    Ok(Operateur { compte_id: compte.compte_id })
}

// ---------------------------------------------------------------------
// Création d'un administrateur (premier ou suivants)
// ---------------------------------------------------------------------

/// Nombre de comptes administrateur : l'outil du premier administrateur
/// choisit entre création (aucun) et reprise (B6).
pub async fn nombre_administrateurs(pool: &PgPool) -> Result<i64, ErreurOutil> {
    sqlx::query_scalar("SELECT count(*) FROM compte WHERE type_compte = 'administrateur_laaficare'")
        .fetch_one(pool)
        .await
        .map_err(|e| interne_sqlx("recherche d'un administrateur", &e))
}

/// Premier administrateur (outil `creer_premier_administrateur`). Refusé dès
/// qu'un administrateur existe ; deux outils lancés en même temps n'en
/// créent qu'un (index unique de la migration 0020).
pub async fn creer_premier_administrateur(
    pool: &PgPool,
    email_sender: &dyn EmailSender,
    saisie: &NouvelAdministrateur<'_>,
) -> Result<AdministrateurEnCours, ErreurOutil> {
    let existe: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM compte WHERE type_compte = 'administrateur_laaficare')")
        .fetch_one(pool)
        .await
        .map_err(|e| interne_sqlx("recherche d'un administrateur", &e))?;
    if existe {
        return Err(ErreurOutil::AdministrateurExisteDeja);
    }
    creer(pool, email_sender, saisie, None).await
}

/// Administrateur supplémentaire (outil `creer_administrateur`), créé par un
/// administrateur authentifié, qui est enregistré dans la trace.
pub async fn creer_administrateur(
    pool: &PgPool,
    email_sender: &dyn EmailSender,
    operateur: Operateur,
    saisie: &NouvelAdministrateur<'_>,
) -> Result<AdministrateurEnCours, ErreurOutil> {
    creer(pool, email_sender, saisie, Some(operateur.compte_id)).await
}

async fn creer(
    pool: &PgPool,
    email_sender: &dyn EmailSender,
    saisie: &NouvelAdministrateur<'_>,
    cree_par: Option<Uuid>,
) -> Result<AdministrateurEnCours, ErreurOutil> {
    // Contrôles sans base d'abord.
    let telephone = telephone::normaliser(saisie.telephone).map_err(|_| ErreurOutil::TelephoneInvalide)?;
    let email = saisie.email.trim();
    if email.is_empty() {
        return Err(ErreurOutil::EmailVide);
    }
    // Contrôle commun de toutes les adresses (`adresse_email.rs`, décisions
    // du porteur du 2026-10-21).
    adresse_email::controler(email).map_err(|_| ErreurOutil::EmailInvalide)?;
    mot_de_passe::controler(saisie.mot_de_passe).map_err(ErreurOutil::MotDePasseNonConforme)?;
    let cnib = numero_cnib(saisie.cnib_chiffres)?;

    // L'identité doit avoir un compte patient actif : tout professionnel et
    // tout administrateur a d'abord un compte patient (section 14).
    let identite: Option<(Uuid, Option<String>, String)> = sqlx::query_as(
        "SELECT u.id, u.email, c.mot_de_passe_hash FROM utilisateur u \
         JOIN compte c ON c.utilisateur_id = u.id AND c.type_compte = 'patient' AND c.statut = 'actif' \
         WHERE u.telephone = $1",
    )
    .bind(&telephone)
    .fetch_optional(pool)
    .await
    .map_err(|e| interne_sqlx("recherche du compte patient", &e))?;
    let Some((utilisateur_id, email_actuel, hash_patient)) = identite else {
        return Err(ErreurOutil::PasDeComptePatient);
    };

    // Décision du porteur (2026-10-18) : le mot de passe administrateur doit
    // différer de celui du compte patient de la même identité. Vérifié par
    // Argon2 sur le hachage du compte patient, comme une connexion.
    let identique = mot_de_passe::verifier(saisie.mot_de_passe, &hash_patient)
        .map_err(|_| ErreurOutil::Interne("comparaison avec le mot de passe patient"))?;
    if identique {
        return Err(ErreurOutil::MotDePasseIdentiquePatient);
    }
    // Haché avant la transaction : Argon2id ne doit pas garder de verrous
    // ouverts.
    let hash = mot_de_passe::hacher(saisie.mot_de_passe).map_err(|_| ErreurOutil::Interne("hachage du mot de passe"))?;

    let mut tx = pool.begin().await.map_err(|e| interne_sqlx("transaction", &e))?;

    // Email : enregistré s'il manque, gardé s'il est le même (majuscules
    // près, J6), refusé s'il est différent (changer l'email d'une personne
    // relève d'un autre outil, à concevoir).
    match email_actuel {
        None => {
            sqlx::query("UPDATE utilisateur SET email = $2 WHERE id = $1")
                .bind(utilisateur_id)
                .bind(email)
                .execute(&mut *tx)
                .await
                .map_err(|e| match contrainte(&e) {
                    Some("utilisateur_email_unique_idx") => ErreurOutil::EmailDejaUtilise,
                    _ => interne_sqlx("enregistrement de l'email", &e),
                })?;
        }
        Some(actuel) if actuel.to_lowercase() == email.to_lowercase() => {}
        Some(_) => return Err(ErreurOutil::EmailDifferent),
    }

    let compte_id: Uuid = sqlx::query_scalar(
        "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) \
         VALUES ($1, 'administrateur_laaficare', $2) RETURNING id",
    )
    .bind(utilisateur_id)
    .bind(&hash)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| match contrainte(&e) {
        Some("un_compte_par_type") => ErreurOutil::DejaAdministrateur,
        _ => interne_sqlx("création du compte", &e),
    })?;

    // CNIB facultative (K3 révisé) : sans numéro, aucun profil.
    if let Some(numero) = &cnib {
        sqlx::query("INSERT INTO profil_administrateur (compte_id, numero_cnib) VALUES ($1, $2)")
            .bind(compte_id)
            .bind(numero)
            .execute(&mut *tx)
            .await
            .map_err(|e| match contrainte(&e) {
                Some("profil_administrateur_cnib_unique") => ErreurOutil::CnibDejaUtilisee,
                Some("profil_administrateur_cnib_format") => ErreurOutil::CnibInvalide,
                _ => interne_sqlx("enregistrement de la CNIB", &e),
            })?;
    }

    sqlx::query("INSERT INTO creation_administrateur (compte_id, cree_par_compte_id) VALUES ($1, $2)")
        .bind(compte_id)
        .bind(cree_par)
        .execute(&mut *tx)
        .await
        .map_err(|e| match contrainte(&e) {
            // Un autre outil a créé le premier administrateur entre-temps.
            Some("creation_administrateur_un_seul_premier") => ErreurOutil::AdministrateurExisteDeja,
            _ => interne_sqlx("trace de la création", &e),
        })?;

    let emise = jeton_email::emettre_verification(&mut tx, compte_id)
        .await
        .map_err(|e| interne_sqlx("émission du jeton d'email", &e))?
        .ok_or(ErreurOutil::Interne("compte sans email"))?;
    tx.commit().await.map_err(|e| interne_sqlx("validation de la transaction", &e))?;

    // Après le commit : un échec d'envoi laisse le compte créé, et la reprise
    // renvoie un nouveau jeton.
    envoyer_verification(email_sender, &emise)?;
    Ok(AdministrateurEnCours { compte_id, email: emise.email.clone(), verification_envoyee: true })
}

// ---------------------------------------------------------------------
// Reprise d'un administrateur qui n'a jamais eu de TOTP actif
// ---------------------------------------------------------------------

/// Compte administrateur actif trouvé par son email : identifiant, email
/// vérifié, et s'il n'a **jamais eu de TOTP actif** (aucun TOTP actif, et
/// aucune désactivation tracée : un TOTP désactivé l'a forcément été après
/// avoir été actif).
async fn administrateur_par_email(pool: &PgPool, email: &str) -> Result<Option<(Uuid, bool, bool)>, ErreurOutil> {
    sqlx::query_as(
        "SELECT c.id, u.email_verifie_le IS NOT NULL, \
                NOT EXISTS (SELECT 1 FROM second_facteur_totp t WHERE t.compte_id = c.id AND t.statut = 'actif') \
                AND NOT EXISTS (SELECT 1 FROM desactivation_totp d WHERE d.compte_id = c.id) \
         FROM compte c JOIN utilisateur u ON u.id = c.utilisateur_id \
         WHERE lower(u.email) = lower(btrim($1)) AND c.type_compte = 'administrateur_laaficare' AND c.statut = 'actif'",
    )
    .bind(email)
    .fetch_optional(pool)
    .await
    .map_err(|e| interne_sqlx("recherche de l'administrateur à reprendre", &e))
}

/// Trace, puis nouveau jeton si l'email n'est pas encore vérifié (il annule
/// le précédent, B1).
async fn reprendre(
    pool: &PgPool,
    email_sender: &dyn EmailSender,
    compte_id: Uuid,
    email_verifie: bool,
    repris_par: Option<Uuid>,
) -> Result<AdministrateurEnCours, ErreurOutil> {
    let mut tx = pool.begin().await.map_err(|e| interne_sqlx("transaction", &e))?;
    sqlx::query("INSERT INTO reprise_administrateur (compte_id, repris_par_compte_id) VALUES ($1, $2)")
        .bind(compte_id)
        .bind(repris_par)
        .execute(&mut *tx)
        .await
        .map_err(|e| interne_sqlx("trace de la reprise", &e))?;
    let emise = if email_verifie {
        None
    } else {
        Some(
            jeton_email::emettre_verification(&mut tx, compte_id)
                .await
                .map_err(|e| interne_sqlx("émission du jeton d'email", &e))?
                .ok_or(ErreurOutil::Interne("compte sans email"))?,
        )
    };
    let email: String = sqlx::query_scalar("SELECT u.email FROM compte c JOIN utilisateur u ON u.id = c.utilisateur_id WHERE c.id = $1")
        .bind(compte_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| interne_sqlx("lecture de l'email", &e))?;
    tx.commit().await.map_err(|e| interne_sqlx("validation de la transaction", &e))?;

    if let Some(emise) = &emise {
        envoyer_verification(email_sender, emise)?;
    }
    Ok(AdministrateurEnCours { compte_id, email, verification_envoyee: emise.is_some() })
}

/// Reprise du premier administrateur par lui-même (B6), quand il est le seul
/// administrateur : son mot de passe est demandé, et compte dans son
/// verrouillage. Trace sans « repris par ».
pub async fn reprendre_premier_administrateur(
    pool: &PgPool,
    email_sender: &dyn EmailSender,
    email: &str,
    mot_de_passe: &str,
) -> Result<AdministrateurEnCours, ErreurOutil> {
    let (nombre, premier): (i64, Option<Uuid>) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM compte WHERE type_compte = 'administrateur_laaficare'), \
                (SELECT compte_id FROM creation_administrateur WHERE cree_par_compte_id IS NULL)",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| interne_sqlx("recherche du premier administrateur", &e))?;
    let cible = administrateur_par_email(pool, email).await?;

    // Mot de passe vérifié d'abord (compté dans le verrouillage), puis les
    // conditions : seul administrateur, le premier, jamais de TOTP actif.
    // Un mot de passe faux ou un email inconnu donnent le même refus.
    let verification = verrouillage::verifier(pool, cible.map(|(id, _, _)| id), mot_de_passe).await;
    match verification {
        Err(ErreurConnexion::Interne(_)) => return Err(ErreurOutil::Interne("vérification du mot de passe")),
        Err(ErreurConnexion::IdentifiantsInvalides) => return Err(ErreurOutil::AuthentificationRefusee),
        Ok(_) => {}
    }
    let Some((compte_id, email_verifie, jamais_totp)) = cible else {
        return Err(ErreurOutil::AuthentificationRefusee);
    };
    if nombre != 1 || premier != Some(compte_id) || !jamais_totp {
        return Err(ErreurOutil::RepriseImpossible);
    }
    reprendre(pool, email_sender, compte_id, email_verifie, None).await
}

/// Reprise par n'importe quel administrateur authentifié (décision du
/// porteur, 2026-10-18), d'un administrateur qui n'a jamais eu de TOTP
/// actif. Trace avec l'administrateur qui reprend.
pub async fn reprendre_administrateur(
    pool: &PgPool,
    email_sender: &dyn EmailSender,
    operateur: Operateur,
    email: &str,
) -> Result<AdministrateurEnCours, ErreurOutil> {
    let Some((compte_id, email_verifie, jamais_totp)) = administrateur_par_email(pool, email).await? else {
        return Err(ErreurOutil::CompteIntrouvable);
    };
    if !jamais_totp || compte_id == operateur.compte_id {
        return Err(ErreurOutil::RepriseImpossible);
    }
    reprendre(pool, email_sender, compte_id, email_verifie, Some(operateur.compte_id)).await
}

// ---------------------------------------------------------------------
// Désactivation d'un TOTP (Z9)
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeCompte {
    Patient,
    Professionnel,
    Administrateur,
}

impl TypeCompte {
    fn valeur(self) -> &'static str {
        match self {
            TypeCompte::Patient => "patient",
            TypeCompte::Professionnel => "professionnel",
            TypeCompte::Administrateur => "administrateur_laaficare",
        }
    }
}

/// Compte visé, avec ce que l'outil affiche pour confirmation.
#[derive(Debug)]
pub struct CompteCible {
    pub compte_id: Uuid,
    pub type_compte: TypeCompte,
    pub nom: Option<String>,
    pub prenom: Option<String>,
}

/// Par le téléphone pour un patient, par l'email sinon (section 14).
pub async fn trouver_compte(pool: &PgPool, type_compte: TypeCompte, identifiant: &str) -> Result<CompteCible, ErreurOutil> {
    let trouve: Option<(Uuid, Option<String>, Option<String>)> = if type_compte == TypeCompte::Patient {
        let telephone = telephone::normaliser(identifiant).map_err(|_| ErreurOutil::TelephoneInvalide)?;
        sqlx::query_as(
            "SELECT c.id, u.nom, u.prenom FROM compte c JOIN utilisateur u ON u.id = c.utilisateur_id \
             WHERE u.telephone = $1 AND c.type_compte = 'patient'",
        )
        .bind(telephone)
        .fetch_optional(pool)
        .await
    } else {
        sqlx::query_as(
            "SELECT c.id, u.nom, u.prenom FROM compte c JOIN utilisateur u ON u.id = c.utilisateur_id \
             WHERE lower(u.email) = lower(btrim($1)) AND c.type_compte = $2",
        )
        .bind(identifiant)
        .bind(type_compte.valeur())
        .fetch_optional(pool)
        .await
    }
    .map_err(|e| interne_sqlx("recherche du compte", &e))?;
    let (compte_id, nom, prenom) = trouve.ok_or(ErreurOutil::CompteIntrouvable)?;
    Ok(CompteCible { compte_id, type_compte, nom, prenom })
}

/// Résultat d'une désactivation.
#[derive(Debug)]
pub struct Desactivation {
    /// Administrateur visé : fenêtre de 10 minutes ouverte pour un nouveau
    /// TOTP.
    pub fenetre_ouverte: bool,
    /// La personne a été prévenue par email (B8 a) ; faux si elle n'a pas
    /// d'email ou si l'envoi a échoué (la désactivation reste faite).
    pub personne_prevenue: bool,
}

/// Désactive le TOTP d'un compte (Z9), en une transaction, dans l'ordre des
/// verrous de la section 12 (le compte avant le reste) :
/// 1. le compte visé est verrouillé ;
/// 2. son TOTP est supprimé, actif et en attente ; ses codes de secours
///    partent avec lui (`ON DELETE CASCADE`, migration 0013, Y1) ;
/// 3. la trace est écrite ; la base refuse les propres comptes de
///    l'opérateur (trigger Y3-bis) ;
/// 4. toutes ses sessions sont fermées, `version_jeton` augmentée : les
///    jetons intermédiaires déjà émis tombent aussi ;
/// 5. pour un administrateur, la fenêtre de réenrôlement est ouverte.
/// Puis, après le commit, la personne est prévenue par email (NIST SP
/// 800-63B rév. 4 §4.2.3 : « account recovery SHALL cause a notification to
/// be sent to the subscriber »).
pub async fn desactiver_totp(
    pool: &PgPool,
    email_sender: &dyn EmailSender,
    operateur: Operateur,
    cible: &CompteCible,
    motif: &str,
    cnib_verifiee: bool,
) -> Result<Desactivation, ErreurOutil> {
    let motif = motif.trim();
    if motif.is_empty() {
        return Err(ErreurOutil::MotifVide);
    }
    // Y2 : attestation obligatoire (le contrôle de la carte, pas le numéro).
    if !cnib_verifiee {
        return Err(ErreurOutil::CnibNonVerifiee);
    }

    let mut tx = pool.begin().await.map_err(|e| interne_sqlx("transaction", &e))?;
    sqlx::query("SELECT id FROM compte WHERE id = $1 FOR UPDATE")
        .bind(cible.compte_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| interne_sqlx("verrou du compte", &e))?;

    let actif: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM second_facteur_totp WHERE compte_id = $1 AND statut = 'actif')",
    )
    .bind(cible.compte_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| interne_sqlx("recherche du TOTP", &e))?;
    if !actif {
        return Err(ErreurOutil::AucunSecondFacteur);
    }

    sqlx::query("DELETE FROM second_facteur_totp WHERE compte_id = $1")
        .bind(cible.compte_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| interne_sqlx("suppression du TOTP", &e))?;

    sqlx::query(
        "INSERT INTO desactivation_totp (compte_id, administrateur_compte_id, motif, cnib_verifiee) \
         VALUES ($1, $2, $3, true)",
    )
    .bind(cible.compte_id)
    .bind(operateur.compte_id)
    .bind(motif)
    .execute(&mut *tx)
    .await
    .map_err(|e| match e.as_database_error().and_then(|d| d.code()).as_deref() {
        // `RAISE EXCEPTION` du trigger Y3-bis (code SQLSTATE P0001, doc
        // PostgreSQL 18, annexe A) : le seul trigger de cette table qui
        // réagit à un INSERT.
        Some("P0001") => ErreurOutil::PropreCompte,
        _ => interne_sqlx("trace de la désactivation", &e),
    })?;

    session::fermer_toutes(&mut tx, cible.compte_id, MotifRevocation::DesactivationTotp, true)
        .await
        .map_err(|e| interne_sqlx("fermeture des sessions", &e))?;

    let fenetre_ouverte = if cible.type_compte == TypeCompte::Administrateur {
        ouvrir_fenetre_dans(&mut tx, cible.compte_id)
            .await
            .map_err(|e| interne_sqlx("ouverture de la fenêtre", &e))?
    } else {
        false
    };

    let email: Option<String> =
        sqlx::query_scalar("SELECT u.email FROM compte c JOIN utilisateur u ON u.id = c.utilisateur_id WHERE c.id = $1")
            .bind(cible.compte_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| interne_sqlx("lecture de l'email", &e))?;
    tx.commit().await.map_err(|e| interne_sqlx("validation de la transaction", &e))?;

    // Texte minimal : ni motif, ni nom de l'opérateur.
    let personne_prevenue = match email {
        Some(adresse) => email_sender
            .envoyer(
                &adresse,
                "LaafiCare : second facteur désactivé",
                "Le second facteur (application d'authentification) de l'un de vos comptes LaafiCare \
                 a été désactivé par l'équipe LaafiCare, à la suite d'une demande vérifiée.\n\n\
                 Si vous n'êtes pas à l'origine de cette demande, contactez l'équipe LaafiCare immédiatement.",
            )
            .is_ok(),
        None => false,
    };
    Ok(Desactivation { fenetre_ouverte, personne_prevenue })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::auth_administrateur::{self, ConnexionAdministrateur};
    use crate::email::ErreurEmail;
    use crate::jwt::JwtService;

    // Base temporaire par test ; données fictives.

    const PATIENT: &str = "Patient#2026";
    const ADMIN: &str = "Admin#2026x";

    /// Garde les emails envoyés, pour y lire le jeton.
    #[derive(Default)]
    struct EmailsEnvoyes(Mutex<Vec<(String, String)>>);

    impl EmailSender for EmailsEnvoyes {
        fn envoyer(&self, destinataire: &str, _sujet: &str, corps: &str) -> Result<(), ErreurEmail> {
            self.0.lock().unwrap().push((destinataire.to_owned(), corps.to_owned()));
            Ok(())
        }
        fn simule(&self) -> bool {
            true
        }
    }

    impl EmailsEnvoyes {
        fn nombre(&self) -> usize {
            self.0.lock().unwrap().len()
        }
        /// Le jeton est le seul mot de 64 caractères hexadécimaux du corps.
        fn dernier_jeton(&self) -> String {
            let emails = self.0.lock().unwrap();
            let (_, corps) = emails.last().expect("aucun email");
            corps
                .split_whitespace()
                .find(|mot| mot.len() == 64 && mot.bytes().all(|o| o.is_ascii_hexdigit()))
                .expect("aucun jeton dans l'email")
                .to_owned()
        }
    }

    fn cle() -> CleTotp {
        CleTotp::depuis_octets([7; 32])
    }

    fn maintenant() -> u64 {
        jsonwebtoken::get_current_timestamp()
    }

    async fn patient(pool: &PgPool, telephone: &str) -> Uuid {
        let utilisateur_id: Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (nom, prenom, telephone) VALUES ('OUEDRAOGO', 'Awa', $1) RETURNING id",
        )
        .bind(telephone)
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

    fn saisie<'a>(telephone: &'a str, email: &'a str, cnib: &'a str) -> NouvelAdministrateur<'a> {
        NouvelAdministrateur { telephone, email, cnib_chiffres: cnib, mot_de_passe: ADMIN }
    }

    /// Création complète par l'outil : email vérifié par le jeton envoyé,
    /// puis fenêtre ouverte.
    async fn administrateur_pret(
        pool: &PgPool,
        emails: &EmailsEnvoyes,
        telephone: &str,
        email: &str,
        operateur: Option<Operateur>,
    ) -> Uuid {
        patient(pool, telephone).await;
        let en_cours = match operateur {
            None => creer_premier_administrateur(pool, emails, &saisie(telephone, email, "")).await.unwrap(),
            Some(op) => creer_administrateur(pool, emails, op, &saisie(telephone, email, "")).await.unwrap(),
        };
        verifier_email(pool, en_cours.compte_id, &emails.dernier_jeton()).await.unwrap();
        ouvrir_fenetre(pool, en_cours.compte_id).await.unwrap();
        en_cours.compte_id
    }

    /// Le TOTP activé comme le fera l'administrateur par les routes de la
    /// 3.6.4 ; renvoie un code valable au pas suivant.
    async fn activer_totp(pool: &PgPool, compte_id: Uuid) -> String {
        let activation = auth_administrateur::commencer_reenrolement(pool, &cle(), compte_id, ADMIN).await.unwrap();
        let generateur = totp_rs::Builder::new()
            .with_secret(totp_rs::Secret::try_from_base32(&activation.cle_base32).unwrap())
            .build()
            .unwrap();
        auth_administrateur::confirmer_reenrolement(pool, &cle(), compte_id, &generateur.generate(maintenant()).to_string())
            .await
            .unwrap();
        generateur.generate(maintenant() + 30).to_string()
    }

    /// Premier administrateur prêt, avec TOTP actif, authentifié comme
    /// opérateur.
    async fn operateur(pool: &PgPool, emails: &EmailsEnvoyes) -> Operateur {
        let compte_id = administrateur_pret(pool, emails, "+22670140001", "operateur@example.org", None).await;
        let code = activer_totp(pool, compte_id).await;
        authentifier_operateur(pool, &cle(), "operateur@example.org", ADMIN, &code).await.unwrap()
    }

    #[test]
    fn cnib_facultative_et_au_format() {
        assert_eq!(numero_cnib("").unwrap(), None);
        assert_eq!(numero_cnib("  ").unwrap(), None);
        assert_eq!(numero_cnib("12345678").unwrap().as_deref(), Some("B12345678"));
        assert_eq!(numero_cnib(" 12345678 ").unwrap().as_deref(), Some("B12345678"));
        for faux in ["1234567", "123456789", "1234567a", "B1234567", "１２３４５６７８"] {
            assert!(matches!(numero_cnib(faux), Err(ErreurOutil::CnibInvalide)), "{faux}");
        }
    }

    /// Parcours complet : création, jeton collé, fenêtre, puis la connexion
    /// administrateur donne un jeton de réenrôlement. Trace sans créateur.
    #[sqlx::test]
    #[ignore]
    async fn premier_administrateur_de_bout_en_bout(pool: PgPool) {
        let emails = EmailsEnvoyes::default();
        patient(&pool, "+22670140002").await;
        let en_cours =
            creer_premier_administrateur(&pool, &emails, &saisie("70 14 00 02", "premier@example.org", "12345678"))
                .await
                .unwrap();
        assert!(en_cours.verification_envoyee);
        assert_eq!(emails.0.lock().unwrap()[0].0, "premier@example.org");

        // Avant la vérification de l'email, pas de fenêtre possible.
        assert!(matches!(ouvrir_fenetre(&pool, en_cours.compte_id).await, Err(ErreurOutil::RepriseImpossible)));
        assert!(matches!(
            verifier_email(&pool, en_cours.compte_id, &"0".repeat(64)).await,
            Err(ErreurOutil::JetonInvalide)
        ));
        verifier_email(&pool, en_cours.compte_id, &emails.dernier_jeton()).await.unwrap();
        ouvrir_fenetre(&pool, en_cours.compte_id).await.unwrap();

        let jwt = JwtService::new(&"a".repeat(32));
        assert!(matches!(
            auth_administrateur::se_connecter(&pool, &jwt, "premier@example.org", ADMIN).await,
            Ok(ConnexionAdministrateur::Reenrolement { .. })
        ));

        let (cree_par, cnib): (Option<Uuid>, String) = sqlx::query_as(
            "SELECT t.cree_par_compte_id, p.numero_cnib FROM creation_administrateur t \
             JOIN profil_administrateur p ON p.compte_id = t.compte_id WHERE t.compte_id = $1",
        )
        .bind(en_cours.compte_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!((cree_par, cnib.as_str()), (None, "B12345678"));
    }

    #[sqlx::test]
    #[ignore]
    async fn un_seul_premier_administrateur(pool: PgPool) {
        let emails = EmailsEnvoyes::default();
        administrateur_pret(&pool, &emails, "+22670140003", "seul@example.org", None).await;
        patient(&pool, "+22670140004").await;
        assert!(matches!(
            creer_premier_administrateur(&pool, &emails, &saisie("+22670140004", "deuxieme@example.org", "")).await,
            Err(ErreurOutil::AdministrateurExisteDeja)
        ));
    }

    /// Deux outils lancés en même temps : un seul premier administrateur
    /// (index unique de la migration 0020).
    #[sqlx::test]
    #[ignore]
    async fn deux_premiers_simultanes_un_seul_cree(pool: PgPool) {
        patient(&pool, "+22670140005").await;
        patient(&pool, "+22670140006").await;
        let mut taches = tokio::task::JoinSet::new();
        for (telephone, email) in [("+22670140005", "simul.a@example.org"), ("+22670140006", "simul.b@example.org")] {
            let pool = pool.clone();
            taches.spawn(async move {
                creer_premier_administrateur(&pool, &EmailsEnvoyes::default(), &saisie(telephone, email, "")).await.is_ok()
            });
        }
        let mut reussites = 0;
        while let Some(resultat) = taches.join_next().await {
            reussites += usize::from(resultat.unwrap());
        }
        assert_eq!(reussites, 1);
        let administrateurs: i64 =
            sqlx::query_scalar("SELECT count(*) FROM compte WHERE type_compte = 'administrateur_laaficare'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(administrateurs, 1);
    }

    #[sqlx::test]
    #[ignore]
    async fn refus_a_la_creation(pool: PgPool) {
        let emails = EmailsEnvoyes::default();
        // Pas de compte patient.
        assert!(matches!(
            creer_premier_administrateur(&pool, &emails, &saisie("+22670140007", "a@example.org", "")).await,
            Err(ErreurOutil::PasDeComptePatient)
        ));
        let compte_patient = patient(&pool, "+22670140007").await;
        // Mot de passe identique à celui du compte patient.
        let identique = NouvelAdministrateur { mot_de_passe: PATIENT, ..saisie("+22670140007", "a@example.org", "") };
        assert!(matches!(
            creer_premier_administrateur(&pool, &emails, &identique).await,
            Err(ErreurOutil::MotDePasseIdentiquePatient)
        ));
        // Mot de passe non conforme, email vide, CNIB mal formée, téléphone faux.
        let faible = NouvelAdministrateur { mot_de_passe: "court", ..saisie("+22670140007", "a@example.org", "") };
        assert!(matches!(
            creer_premier_administrateur(&pool, &emails, &faible).await,
            Err(ErreurOutil::MotDePasseNonConforme(_))
        ));
        assert!(matches!(
            creer_premier_administrateur(&pool, &emails, &saisie("+22670140007", "  ", "")).await,
            Err(ErreurOutil::EmailVide)
        ));
        // Contrôle commun des adresses : accent, ou nom et adresse.
        for email in ["élodie@example.org", "Awa <a@example.org>"] {
            assert!(matches!(
                creer_premier_administrateur(&pool, &emails, &saisie("+22670140007", email, "")).await,
                Err(ErreurOutil::EmailInvalide)
            ));
        }
        assert!(matches!(
            creer_premier_administrateur(&pool, &emails, &saisie("+22670140007", "a@example.org", "123")).await,
            Err(ErreurOutil::CnibInvalide)
        ));
        assert!(matches!(
            creer_premier_administrateur(&pool, &emails, &saisie("abc", "a@example.org", "")).await,
            Err(ErreurOutil::TelephoneInvalide)
        ));
        // Email déjà utilisé par une autre identité.
        sqlx::query("INSERT INTO utilisateur (telephone, email) VALUES ('+22670140008', 'pris@example.org')")
            .execute(&pool)
            .await
            .unwrap();
        assert!(matches!(
            creer_premier_administrateur(&pool, &emails, &saisie("+22670140007", "PRIS@example.org", "")).await,
            Err(ErreurOutil::EmailDejaUtilise)
        ));
        // Identité qui a déjà un autre email.
        sqlx::query("UPDATE utilisateur SET email = 'autre@example.org' FROM compte c WHERE c.id = $1 AND utilisateur.id = c.utilisateur_id")
            .bind(compte_patient)
            .execute(&pool)
            .await
            .unwrap();
        assert!(matches!(
            creer_premier_administrateur(&pool, &emails, &saisie("+22670140007", "a@example.org", "")).await,
            Err(ErreurOutil::EmailDifferent)
        ));
        // Rien n'a été créé, aucun email envoyé.
        let administrateurs: i64 =
            sqlx::query_scalar("SELECT count(*) FROM compte WHERE type_compte = 'administrateur_laaficare'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!((administrateurs, emails.nombre()), (0, 0));
    }

    /// CNIB vide : aucun profil (K3 révisé). CNIB déjà enregistrée : refus.
    #[sqlx::test]
    #[ignore]
    async fn cnib_vide_sans_profil_et_cnib_unique(pool: PgPool) {
        let emails = EmailsEnvoyes::default();
        let op = operateur(&pool, &emails).await;
        let profils: i64 = sqlx::query_scalar("SELECT count(*) FROM profil_administrateur")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(profils, 0);

        patient(&pool, "+22670140009").await;
        creer_administrateur(&pool, &emails, op, &saisie("+22670140009", "cnib.a@example.org", "11112222")).await.unwrap();
        patient(&pool, "+22670140010").await;
        assert!(matches!(
            creer_administrateur(&pool, &emails, op, &saisie("+22670140010", "cnib.b@example.org", "11112222")).await,
            Err(ErreurOutil::CnibDejaUtilisee)
        ));
    }

    /// Administrateur suivant : trace avec son créateur ; une même personne
    /// n'a qu'un compte administrateur.
    #[sqlx::test]
    #[ignore]
    async fn administrateur_suivant_trace_avec_son_createur(pool: PgPool) {
        let emails = EmailsEnvoyes::default();
        let op = operateur(&pool, &emails).await;
        let compte_id = administrateur_pret(&pool, &emails, "+22670140011", "suivant@example.org", Some(op)).await;
        let cree_par: Option<Uuid> =
            sqlx::query_scalar("SELECT cree_par_compte_id FROM creation_administrateur WHERE compte_id = $1")
                .bind(compte_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(cree_par, Some(op.compte_id()));
        assert!(matches!(
            creer_administrateur(&pool, &emails, op, &saisie("+22670140011", "suivant@example.org", "")).await,
            Err(ErreurOutil::DejaAdministrateur)
        ));
    }

    /// L'opérateur doit donner un bon code TOTP ; un administrateur sans TOTP
    /// actif ne peut pas être opérateur.
    #[sqlx::test]
    #[ignore]
    async fn operateur_refuse_sans_bon_code_ou_sans_totp(pool: PgPool) {
        let emails = EmailsEnvoyes::default();
        let compte_id = administrateur_pret(&pool, &emails, "+22670140012", "sans.totp@example.org", None).await;
        assert!(matches!(
            authentifier_operateur(&pool, &cle(), "sans.totp@example.org", ADMIN, "123456").await,
            Err(ErreurOutil::AuthentificationRefusee)
        ));
        let code = activer_totp(&pool, compte_id).await;
        let faux = if code == "000000" { "000001" } else { "000000" };
        for (mot_de_passe, code) in [(ADMIN, faux), ("Mauvais#2026", code.as_str())] {
            assert!(matches!(
                authentifier_operateur(&pool, &cle(), "sans.totp@example.org", mot_de_passe, code).await,
                Err(ErreurOutil::AuthentificationRefusee)
            ));
        }
        authentifier_operateur(&pool, &cle(), "sans.totp@example.org", ADMIN, &code).await.unwrap();
    }

    /// Reprise du premier par lui-même : seul administrateur, jamais de
    /// TOTP ; nouveau jeton qui annule le précédent ; trace sans « repris
    /// par ». Refusée une fois le TOTP actif.
    #[sqlx::test]
    #[ignore]
    async fn reprise_du_premier_administrateur(pool: PgPool) {
        let emails = EmailsEnvoyes::default();
        patient(&pool, "+22670140013").await;
        let en_cours =
            creer_premier_administrateur(&pool, &emails, &saisie("+22670140013", "reprise@example.org", "")).await.unwrap();
        let ancien = emails.dernier_jeton();

        assert!(matches!(
            reprendre_premier_administrateur(&pool, &emails, "reprise@example.org", "Mauvais#2026").await,
            Err(ErreurOutil::AuthentificationRefusee)
        ));
        let reprise = reprendre_premier_administrateur(&pool, &emails, "reprise@example.org", ADMIN).await.unwrap();
        assert_eq!(reprise.compte_id, en_cours.compte_id);
        assert!(reprise.verification_envoyee);
        assert!(matches!(verifier_email(&pool, reprise.compte_id, &ancien).await, Err(ErreurOutil::JetonInvalide)));
        verifier_email(&pool, reprise.compte_id, &emails.dernier_jeton()).await.unwrap();

        // Email vérifié : une nouvelle reprise n'envoie plus de jeton.
        let encore = reprendre_premier_administrateur(&pool, &emails, "reprise@example.org", ADMIN).await.unwrap();
        assert!(!encore.verification_envoyee);
        ouvrir_fenetre(&pool, encore.compte_id).await.unwrap();

        let traces: Vec<Option<Uuid>> =
            sqlx::query_scalar("SELECT repris_par_compte_id FROM reprise_administrateur WHERE compte_id = $1")
                .bind(en_cours.compte_id)
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(traces, [None, None]);

        activer_totp(&pool, en_cours.compte_id).await;
        assert!(matches!(
            reprendre_premier_administrateur(&pool, &emails, "reprise@example.org", ADMIN).await,
            Err(ErreurOutil::RepriseImpossible)
        ));
    }

    /// Reprise par un administrateur qui n'est pas le créateur (décision du
    /// porteur, 2026-10-18) ; trace avec lui. Le premier ne se reprend plus
    /// lui-même dès qu'un second administrateur existe.
    #[sqlx::test]
    #[ignore]
    async fn reprise_par_un_autre_administrateur(pool: PgPool) {
        let emails = EmailsEnvoyes::default();
        let createur = operateur(&pool, &emails).await;
        // Second administrateur, avec TOTP : il reprendra le troisième.
        let second = administrateur_pret(&pool, &emails, "+22670140014", "second@example.org", Some(createur)).await;
        let code = activer_totp(&pool, second).await;
        let repreneur = authentifier_operateur(&pool, &cle(), "second@example.org", ADMIN, &code).await.unwrap();

        patient(&pool, "+22670140015").await;
        let troisieme = creer_administrateur(&pool, &emails, createur, &saisie("+22670140015", "troisieme@example.org", ""))
            .await
            .unwrap();
        let reprise = reprendre_administrateur(&pool, &emails, repreneur, "troisieme@example.org").await.unwrap();
        assert!(reprise.verification_envoyee);
        let repris_par: Option<Uuid> =
            sqlx::query_scalar("SELECT repris_par_compte_id FROM reprise_administrateur WHERE compte_id = $1")
                .bind(troisieme.compte_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(repris_par, Some(repreneur.compte_id()));

        // Un administrateur qui a un TOTP actif ne se reprend pas.
        assert!(matches!(
            reprendre_administrateur(&pool, &emails, repreneur, "operateur@example.org").await,
            Err(ErreurOutil::RepriseImpossible)
        ));
    }

    async fn compter(pool: &PgPool, requete: &'static str, compte_id: Uuid) -> i64 {
        sqlx::query_scalar(requete).bind(compte_id).fetch_one(pool).await.unwrap()
    }

    /// Désactivation d'un administrateur : TOTP et codes de secours
    /// supprimés, sessions fermées (motif `desactivation_totp`), version
    /// augmentée, trace écrite, fenêtre ouverte, personne prévenue.
    #[sqlx::test]
    #[ignore]
    async fn desactivation_d_un_administrateur(pool: PgPool) {
        let emails = EmailsEnvoyes::default();
        let op = operateur(&pool, &emails).await;
        let cible_id = administrateur_pret(&pool, &emails, "+22670140016", "cible@example.org", Some(op)).await;
        activer_totp(&pool, cible_id).await;
        session::ouvrir(&pool, cible_id, None).await.unwrap();
        assert_eq!(compter(&pool, "SELECT count(*) FROM code_secours cs JOIN second_facteur_totp t ON t.id = cs.second_facteur_id WHERE t.compte_id = $1", cible_id).await, 10);
        let version_avant = compter(&pool, "SELECT version_jeton::bigint FROM compte WHERE id = $1", cible_id).await;
        let emails_avant = emails.nombre();

        let cible = trouver_compte(&pool, TypeCompte::Administrateur, " CIBLE@example.org ").await.unwrap();
        assert_eq!(cible.compte_id, cible_id);
        let resultat = desactiver_totp(&pool, &emails, op, &cible, "Téléphone perdu", true).await.unwrap();
        assert!(resultat.fenetre_ouverte && resultat.personne_prevenue);
        assert_eq!(emails.nombre(), emails_avant + 1);

        assert_eq!(compter(&pool, "SELECT count(*) FROM second_facteur_totp WHERE compte_id = $1", cible_id).await, 0);
        let codes_restants: i64 = sqlx::query_scalar("SELECT count(*) FROM code_secours").fetch_one(&pool).await.unwrap();
        assert_eq!(codes_restants, 10, "seuls ceux de l'opérateur restent");
        assert_eq!(
            compter(&pool, "SELECT count(*) FROM session WHERE compte_id = $1 AND motif_revocation = 'desactivation_totp'", cible_id).await,
            1
        );
        assert_eq!(compter(&pool, "SELECT version_jeton::bigint FROM compte WHERE id = $1", cible_id).await, version_avant + 1);
        assert_eq!(compter(&pool, "SELECT count(*) FROM desactivation_totp WHERE compte_id = $1", cible_id).await, 1);
        assert_eq!(
            compter(&pool, "SELECT count(*) FROM compte WHERE id = $1 AND reenrolement_autorise_jusqu_au > now()", cible_id).await,
            1
        );
        // Plus rien à désactiver.
        assert!(matches!(
            desactiver_totp(&pool, &emails, op, &cible, "encore", true).await,
            Err(ErreurOutil::AucunSecondFacteur)
        ));
    }

    /// Patient : pas de fenêtre ; motif et attestation obligatoires ; jamais
    /// les propres comptes de l'opérateur.
    #[sqlx::test]
    #[ignore]
    async fn desactivation_refus_et_patient_sans_fenetre(pool: PgPool) {
        let emails = EmailsEnvoyes::default();
        let op = operateur(&pool, &emails).await;
        let patient_id = patient(&pool, "+22670140017").await;
        let activation = totp::commencer_activation(&pool, cle().octets(), patient_id, PATIENT).await.unwrap();
        let generateur = totp_rs::Builder::new()
            .with_secret(totp_rs::Secret::try_from_base32(&activation.cle_base32).unwrap())
            .build()
            .unwrap();
        totp::confirmer_activation(&pool, cle().octets(), patient_id, &generateur.generate(maintenant()).to_string(), totp::Autorisation::Libre)
            .await
            .unwrap();

        let cible = trouver_compte(&pool, TypeCompte::Patient, "70 14 00 17").await.unwrap();
        assert!(matches!(desactiver_totp(&pool, &emails, op, &cible, "  ", true).await, Err(ErreurOutil::MotifVide)));
        assert!(matches!(
            desactiver_totp(&pool, &emails, op, &cible, "Téléphone perdu", false).await,
            Err(ErreurOutil::CnibNonVerifiee)
        ));
        let resultat = desactiver_totp(&pool, &emails, op, &cible, "Téléphone perdu", true).await.unwrap();
        assert!(!resultat.fenetre_ouverte);
        assert!(!resultat.personne_prevenue, "patient sans email");

        // L'opérateur ne désactive pas le TOTP de son propre compte
        // administrateur ; rien n'est modifié.
        let soi = trouver_compte(&pool, TypeCompte::Administrateur, "operateur@example.org").await.unwrap();
        assert!(matches!(
            desactiver_totp(&pool, &emails, op, &soi, "Essai", true).await,
            Err(ErreurOutil::PropreCompte)
        ));
        assert_eq!(compter(&pool, "SELECT count(*) FROM second_facteur_totp WHERE compte_id = $1 AND statut = 'actif'", op.compte_id()).await, 1);
    }
}
