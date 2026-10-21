//! Obtenir ou créer le compte professionnel d'une personne, depuis son
//! compte patient (section 14 : « Tout professionnel a d'abord un compte
//! patient »). Commun à la création d'une structure (3.7) et à
//! l'acceptation d'une invitation (3.10, décision I13) : mêmes règles, un
//! seul endroit.
//!
//! - Sans compte professionnel : email (contrôle commun `adresse_email.rs`)
//!   et mot de passe (règles de `mot_de_passe.rs`, le même que le compte
//!   patient permis, décision du porteur du 2026-10-21) ; l'email de
//!   l'identité est enregistré s'il manque, gardé s'il est le même
//!   (majuscules près, J6), refusé s'il est différent ; un lien de
//!   vérification est émis (R0).
//! - Avec un compte professionnel : son mot de passe, vérifié comme une
//!   connexion et compté dans son verrouillage (BF-01). Un TOTP actif n'est
//!   pas encore pris en charge ici (aucun professionnel n'en a avant 3.11).
//!
//! En deux temps : `preparer` fait tout ce qui ne demande aucun verrou
//! (lecture, mot de passe, hachage Argon2id, long) ; `ecrire` agit dans la
//! transaction de l'appelant et verrouille **les comptes en premier**
//! (règle de la section 12 : le compte avant le reste).

use sqlx::types::Uuid;
use sqlx::{PgConnection, PgPool};

use crate::adresse_email;
use crate::auth_professionnel::nature_sqlx;
use crate::jeton_email::{self, VerificationEmise};
use crate::mot_de_passe::{self, MotDePasseNonConforme};
use crate::verrouillage::{self, ErreurConnexion, Verification};

/// Pas de `Debug` : le mot de passe est un secret.
pub struct Saisie<'a> {
    /// Exigé seulement sans compte professionnel ; ignoré sinon.
    pub email_professionnel: Option<&'a str>,
    pub mot_de_passe: &'a str,
}

#[derive(Debug, thiserror::Error)]
pub enum ErreurComptePro {
    #[error("Le champ « email_professionnel » est obligatoire.")]
    EmailObligatoire,
    #[error("Le champ « email_professionnel » n'est pas une adresse email valide.")]
    EmailInvalide,
    #[error("Mot de passe non conforme.")]
    MotDePasseNonConforme(MotDePasseNonConforme),
    /// Compte existant : mot de passe faux, verrouillé ou désactivé (L1).
    #[error("Mot de passe professionnel incorrect.")]
    MotDePasseIncorrect,
    #[error("Votre compte professionnel a un second facteur : cette opération ne le prend pas encore en charge.")]
    SecondFacteurRequis,
    #[error("Votre identité a déjà un autre email : il ne peut pas être changé ici.")]
    EmailDifferent,
    #[error("Cet email est déjà utilisé par une autre personne.")]
    EmailDejaUtilise,
    /// Une autre requête de la même personne vient de créer son compte
    /// professionnel ; chaque appelant traduit selon son contexte.
    #[error("Le compte professionnel vient d'être créé par une autre requête.")]
    CompteDejaCree,
    #[error("erreur interne : {0}")]
    Interne(&'static str),
}

fn interne(etape: &'static str, erreur: &sqlx::Error) -> ErreurComptePro {
    tracing::error!(etape, nature = %nature_sqlx(erreur), "compte professionnel : erreur interne");
    ErreurComptePro::Interne(etape)
}

enum Compte {
    Existant(Uuid),
    Nouveau { email: String, hash: String, enregistrer_email: bool },
}

/// Résultat de `preparer`, à passer à `ecrire`.
pub struct Preparation {
    utilisateur_id: Uuid,
    compte: Compte,
    email_verifie: bool,
}

impl Preparation {
    /// L'email de l'identité est déjà vérifié (aucun lien à envoyer).
    pub fn email_verifie(&self) -> bool {
        self.email_verifie
    }
}

/// Compte professionnel obtenu, et le lien de vérification à envoyer après
/// le commit (compte qui vient d'être créé avec un email non vérifié).
pub struct CompteObtenu {
    pub compte_id: Uuid,
    pub verification: Option<VerificationEmise>,
}

/// Premier temps, sans verrou ni transaction.
pub async fn preparer(pool: &PgPool, patient_compte_id: Uuid, saisie: &Saisie<'_>) -> Result<Preparation, ErreurComptePro> {
    let identite: Option<(Uuid, Option<String>, bool, Option<Uuid>)> = sqlx::query_as(
        "SELECT u.id, u.email, u.email_verifie_le IS NOT NULL, pro.id \
         FROM compte pat JOIN utilisateur u ON u.id = pat.utilisateur_id \
         LEFT JOIN compte pro ON pro.utilisateur_id = u.id AND pro.type_compte = 'professionnel' \
         WHERE pat.id = $1 AND pat.type_compte = 'patient'",
    )
    .bind(patient_compte_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| interne("recherche de l'identité", &e))?;
    let (utilisateur_id, email_actuel, email_verifie, professionnel) =
        identite.ok_or(ErreurComptePro::Interne("compte patient introuvable"))?;

    let compte = match professionnel {
        Some(compte_id) => match verrouillage::verifier(pool, Some(compte_id), saisie.mot_de_passe).await {
            Ok(Verification::Complete(_)) => Compte::Existant(compte_id),
            Ok(Verification::SecondFacteurRequis(_)) => return Err(ErreurComptePro::SecondFacteurRequis),
            Err(ErreurConnexion::IdentifiantsInvalides) => return Err(ErreurComptePro::MotDePasseIncorrect),
            Err(ErreurConnexion::Interne(_)) => {
                tracing::error!(etape = "mot de passe professionnel", "compte professionnel : erreur interne");
                return Err(ErreurComptePro::Interne("vérification du mot de passe professionnel"));
            }
        },
        None => {
            let email = saisie
                .email_professionnel
                .map(str::trim)
                .filter(|e| !e.is_empty())
                .ok_or(ErreurComptePro::EmailObligatoire)?;
            adresse_email::controler(email).map_err(|_| ErreurComptePro::EmailInvalide)?;
            mot_de_passe::controler(saisie.mot_de_passe).map_err(ErreurComptePro::MotDePasseNonConforme)?;
            let enregistrer_email = match &email_actuel {
                None => true,
                Some(actuel) if actuel.to_lowercase() == email.to_lowercase() => false,
                Some(_) => return Err(ErreurComptePro::EmailDifferent),
            };
            // Haché avant la transaction : Argon2id ne garde pas de verrous.
            let hash = mot_de_passe::hacher(saisie.mot_de_passe)
                .map_err(|_| ErreurComptePro::Interne("hachage du mot de passe"))?;
            Compte::Nouveau { email: email.to_string(), hash, enregistrer_email }
        }
    };
    Ok(Preparation { utilisateur_id, compte, email_verifie })
}

/// Second temps, dans la transaction de l'appelant, **avant tout autre
/// verrou** : le compte patient est verrouillé, puis le compte
/// professionnel créé s'il le faut (avec son lien de vérification).
pub async fn ecrire(
    conn: &mut PgConnection,
    patient_compte_id: Uuid,
    preparation: Preparation,
) -> Result<CompteObtenu, ErreurComptePro> {
    // Deux opérations simultanées de la même personne se suivent ; la
    // seconde bute ensuite sur l'index unique du compte.
    sqlx::query("SELECT id FROM compte WHERE id = $1 FOR UPDATE")
        .bind(patient_compte_id)
        .fetch_one(&mut *conn)
        .await
        .map_err(|e| interne("verrou du compte patient", &e))?;

    let (email, hash, enregistrer_email) = match preparation.compte {
        Compte::Existant(compte_id) => return Ok(CompteObtenu { compte_id, verification: None }),
        Compte::Nouveau { email, hash, enregistrer_email } => (email, hash, enregistrer_email),
    };
    if enregistrer_email {
        // `email IS NULL` : un email posé entre-temps n'est jamais écrasé.
        let ecrit = sqlx::query("UPDATE utilisateur SET email = $2 WHERE id = $1 AND email IS NULL")
            .bind(preparation.utilisateur_id)
            .bind(&email)
            .execute(&mut *conn)
            .await
            .map_err(|e| match e.as_database_error().and_then(|d| d.constraint()) {
                Some("utilisateur_email_unique_idx") => ErreurComptePro::EmailDejaUtilise,
                _ => interne("enregistrement de l'email", &e),
            })?;
        if ecrit.rows_affected() == 0 {
            return Err(ErreurComptePro::EmailDifferent);
        }
    }
    let compte_id: Uuid = sqlx::query_scalar(
        "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'professionnel', $2) RETURNING id",
    )
    .bind(preparation.utilisateur_id)
    .bind(&hash)
    .fetch_one(&mut *conn)
    .await
    .map_err(|e| match e.as_database_error().and_then(|d| d.constraint()) {
        Some("un_compte_par_type") => ErreurComptePro::CompteDejaCree,
        _ => interne("création du compte professionnel", &e),
    })?;

    // Lien seulement pour un compte qui vient d'être créé avec un email non
    // vérifié ; un compte existant non vérifié passe par la route de renvoi.
    let verification = if preparation.email_verifie {
        None
    } else {
        jeton_email::emettre_verification(&mut *conn, compte_id)
            .await
            .map_err(|e| interne("émission du jeton d'email", &e))?
    };
    Ok(CompteObtenu { compte_id, verification })
}
