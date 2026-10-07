//! `AuthPatientService` (sections 11 et 14 du CLAUDE.md) : inscription et
//! mot de passe oublié (les deux flux où l'OTP intervient), et connexion
//! par téléphone + mot de passe. Le mot de passe et le verrouillage sont
//! sur le **compte patient** (table `compte`, migration 0011), jamais sur
//! l'identité ; l'OTP, lui, reste sur l'identité (`utilisateur`), puisqu'il
//! est demandé avant que le compte existe.
//!
//! Aucune information sur un compte (existence, nom) n'est jamais révélée
//! avant authentification réussie (décision actée) :
//! - dans les deux fonctions qui acceptent un code OTP, celui-ci est
//!   **toujours vérifié avant** de distinguer les cas d'erreur (compte déjà
//!   existant / inconnu) ;
//! - la connexion répond toujours `IdentifiantsInvalides`, que le numéro
//!   soit inconnu, l'inscription inachevée, le compte verrouillé ou le mot
//!   de passe faux (décisions P1 et L1, via `verrouillage.rs`).

use chrono::NaiveDate;
use sqlx::PgPool;
use sqlx::types::Uuid;

use crate::jwt::JwtService;
use crate::mot_de_passe::RegleMotDePasse;
use crate::verrouillage::{self, ErreurConnexion, Verification};
use crate::{mot_de_passe, nip, otp, telephone};

#[derive(Debug, thiserror::Error)]
pub enum ErreurAuthPatient {
    /// Numéro impossible à ramener au format international (décision T3).
    /// Une erreur de format ne dit rien sur l'existence d'un compte : elle
    /// respecte P1 et aide à corriger la saisie.
    #[error("numéro de téléphone invalide")]
    TelephoneInvalide,

    #[error("code invalide ou expiré")]
    CodeInvalide,

    /// `creer_compte` uniquement : un compte patient existait déjà pour ce
    /// téléphone au moment de la vérification de l'OTP.
    #[error("un compte existe déjà pour ce numéro")]
    TelephoneDejaUtilise,

    /// `reinitialiser_mot_de_passe` uniquement : l'identité existe (un OTP a
    /// donc pu être généré) mais aucun compte patient n'a jamais été créé
    /// pour ce numéro -- rien à réinitialiser.
    #[error("aucun compte associé à ce numéro")]
    TelephoneInconnu,

    /// Toutes les règles non respectées (`mot_de_passe::controler`).
    #[error("mot de passe non conforme")]
    MotDePasseNonConforme(Vec<RegleMotDePasse>),

    /// Connexion refusée, quelle qu'en soit la raison (décisions P1 et L1).
    /// Texte court (décision du 2026-10-03) : les actions passent par les
    /// boutons de l'app, « Mot de passe oublié ? » (qui couvre aussi le
    /// compte verrouillé, puisque la réinitialisation lève le verrouillage)
    /// et « Créer un compte ».
    #[error("Numéro ou mot de passe incorrect.")]
    IdentifiantsInvalides,

    #[error("erreur interne : {0}")]
    Interne(String),
}

impl From<telephone::TelephoneInvalide> for ErreurAuthPatient {
    fn from(_: telephone::TelephoneInvalide) -> Self {
        ErreurAuthPatient::TelephoneInvalide
    }
}

impl From<mot_de_passe::MotDePasseNonConforme> for ErreurAuthPatient {
    fn from(erreur: mot_de_passe::MotDePasseNonConforme) -> Self {
        ErreurAuthPatient::MotDePasseNonConforme(erreur.regles_non_respectees)
    }
}

impl From<sqlx::Error> for ErreurAuthPatient {
    fn from(erreur: sqlx::Error) -> Self {
        ErreurAuthPatient::Interne(erreur.to_string())
    }
}

/// Identité et compte patient éventuel pour ce téléphone.
async fn identite_et_compte_patient(
    pool: &PgPool,
    telephone: &str,
) -> Result<Option<(Uuid, Option<Uuid>)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT u.id, c.id FROM utilisateur u \
         LEFT JOIN compte c ON c.utilisateur_id = u.id AND c.type_compte = 'patient' \
         WHERE u.telephone = $1",
    )
    .bind(telephone)
    .fetch_optional(pool)
    .await
}

/// Trouve la ligne `utilisateur` pour ce téléphone, ou en crée une "en
/// attente" (téléphone seul, nom et prénom NULL, aucun compte) si le numéro est
/// inconnu -- décision actée : la création de compte ne stocke aucune
/// donnée du formulaire avant validation de l'OTP, mais l'OTP doit
/// pourtant s'accrocher à un `utilisateur_id` existant (`otp::generer_et_enregistrer`
/// l'exige).
///
/// ⚠️ Dette assumée, non codée dans cette itération : une ligne "en
/// attente" jamais complétée (inscription abandonnée) doit être purgée
/// après 24h, sinon son téléphone reste bloqué indéfiniment par la
/// contrainte `UNIQUE` -- décision actée avec le porteur, pas de nettoyage
/// automatique pour l'instant.
async fn trouver_ou_creer_utilisateur(pool: &PgPool, telephone: &str) -> Result<Uuid, ErreurAuthPatient> {
    if let Some(id) = sqlx::query_scalar::<_, Uuid>("SELECT id FROM utilisateur WHERE telephone = $1")
        .bind(telephone)
        .fetch_optional(pool)
        .await?
    {
        return Ok(id);
    }

    let id: Uuid = sqlx::query_scalar("INSERT INTO utilisateur (telephone) VALUES ($1) RETURNING id")
        .bind(telephone)
        .fetch_one(pool)
        .await?;
    Ok(id)
}

/// Demande un code OTP pour un téléphone -- sert à la fois à la création de
/// compte (numéro inconnu ou jamais finalisé) et à la récupération de mot
/// de passe (compte déjà complet). Ne sait pas laquelle des deux c'est, n'a
/// pas besoin de le savoir (voir `trouver_ou_creer_utilisateur`).
///
/// N'envoie pas le SMS elle-même : renvoie le code en clair et le **numéro
/// normalisé**, vers lequel l'appelant envoie le SMS via `SmsSender` --
/// jamais vers le numéro tel que saisi (décision T4). Ce module reste
/// indépendant du canal d'envoi.
pub async fn demander_otp(pool: &PgPool, telephone_saisi: &str) -> Result<(String, String), ErreurAuthPatient> {
    let telephone = telephone::normaliser(telephone_saisi)?;
    let utilisateur_id = trouver_ou_creer_utilisateur(pool, &telephone).await?;
    let code = otp::generer_et_enregistrer(pool, utilisateur_id).await?;
    Ok((code, telephone))
}

/// Flux de création de compte patient (section 11) : un seul appel,
/// atomique -- si l'OTP ou une étape échoue, rien n'est écrit (hors le
/// code OTP lui-même, déjà invalidé par `otp::verifier` en cas de succès,
/// avant même que cette transaction ne débute -- limite acceptée : un échec
/// très tardif de cette fonction oblige à redemander un OTP, mais ne laisse
/// jamais de compte à moitié créé).
///
/// Un code valide est consommé même quand l'appel échoue ensuite pour une
/// raison métier (`TelephoneDejaUtilise`) : la vérification a prouvé le
/// contrôle du téléphone, ce qui est son seul rôle -- si l'appelant se
/// trompait de flux (compte déjà complet), redemander un code neuf pour le
/// bon flux (`reinitialiser_mot_de_passe`) est le comportement attendu,
/// pas une régression.
#[allow(clippy::too_many_arguments)]
pub async fn creer_compte(
    pool: &PgPool,
    jwt: &JwtService,
    telephone_saisi: &str,
    code_otp: &str,
    nom: &str,
    prenom: &str,
    date_naissance: NaiveDate,
    lieu_naissance: &str,
    mot_de_passe: &str,
) -> Result<String, ErreurAuthPatient> {
    let telephone = telephone::normaliser(telephone_saisi)?;
    // Contrôlé avant l'OTP : un mot de passe refusé ne consomme pas le code.
    mot_de_passe::controler(mot_de_passe)?;

    // Aucune identité : aucun OTP n'a pu être généré pour ce téléphone
    // (`otp::generer_et_enregistrer` exige un `utilisateur_id` existant),
    // donc `code_otp` ne peut de toute façon pas être valide.
    let (utilisateur_id, compte_patient) =
        identite_et_compte_patient(pool, &telephone).await?.ok_or(ErreurAuthPatient::CodeInvalide)?;

    // OTP vérifié AVANT de regarder si le compte existe déjà : sinon un
    // appelant sans code valide pourrait sonder n'importe quel numéro pour
    // savoir s'il est déjà inscrit, sans jamais prouver qu'il le contrôle.
    if !otp::verifier(pool, utilisateur_id, code_otp).await? {
        return Err(ErreurAuthPatient::CodeInvalide);
    }

    if compte_patient.is_some() {
        return Err(ErreurAuthPatient::TelephoneDejaUtilise);
    }

    let hash = mot_de_passe::hacher(mot_de_passe).map_err(|e| ErreurAuthPatient::Interne(e.to_string()))?;
    let nip_genere = nip::generer(pool).await?;

    // Identité complétée, compte patient et ligne `patient` dans une seule
    // transaction : jamais de compte sans dossier patient, ni l'inverse.
    let mut tx = pool.begin().await?;

    sqlx::query("UPDATE utilisateur SET nom = $1, prenom = $2 WHERE id = $3")
        .bind(nom)
        .bind(prenom)
        .bind(utilisateur_id)
        .execute(&mut *tx)
        .await?;

    sqlx::query("INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'patient', $2)")
        .bind(utilisateur_id)
        .bind(&hash)
        .execute(&mut *tx)
        .await?;

    sqlx::query(
        "INSERT INTO patient (utilisateur_id, nip, date_naissance, lieu_naissance) VALUES ($1, $2, $3, $4)",
    )
    .bind(utilisateur_id)
    .bind(&nip_genere)
    .bind(date_naissance)
    .bind(lieu_naissance)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    jwt.emettre(&utilisateur_id.to_string(), "patient")
        .map_err(|e| ErreurAuthPatient::Interne(e.to_string()))
}

/// Flux mot de passe oublié -- Patient (section 11) : demande d'OTP déjà
/// faite séparément (`demander_otp`), cette fonction valide le code et fixe
/// le nouveau mot de passe.
pub async fn reinitialiser_mot_de_passe(
    pool: &PgPool,
    telephone_saisi: &str,
    code_otp: &str,
    nouveau_mot_de_passe: &str,
) -> Result<(), ErreurAuthPatient> {
    let telephone = telephone::normaliser(telephone_saisi)?;
    mot_de_passe::controler(nouveau_mot_de_passe)?;

    let (utilisateur_id, compte_patient) =
        identite_et_compte_patient(pool, &telephone).await?.ok_or(ErreurAuthPatient::CodeInvalide)?;

    // Même ordre que `creer_compte`, même raison : prouver le contrôle du
    // numéro avant de révéler quoi que ce soit sur l'état du compte.
    if !otp::verifier(pool, utilisateur_id, code_otp).await? {
        return Err(ErreurAuthPatient::CodeInvalide);
    }

    let compte_id = compte_patient.ok_or(ErreurAuthPatient::TelephoneInconnu)?;

    let hash =
        mot_de_passe::hacher(nouveau_mot_de_passe).map_err(|e| ErreurAuthPatient::Interne(e.to_string()))?;

    // - Verrouillage levé : prouver la possession du téléphone par l'OTP
    //   justifie le déblocage.
    // - `version_jeton` augmentée (décision C2) : quand le jeton portera
    //   cette version (étape 3.6), un changement de mot de passe
    //   déconnectera aussitôt les sessions déjà ouvertes, par exemple celles
    //   d'un voleur. Jusque-là, l'augmentation est enregistrée mais pas
    //   encore vérifiée.
    sqlx::query(
        "UPDATE compte SET mot_de_passe_hash = $1, tentatives_echouees = 0, verrouille_jusqua = NULL, \
         version_jeton = version_jeton + 1 WHERE id = $2",
    )
    .bind(&hash)
    .bind(compte_id)
    .execute(pool)
    .await?;

    Ok(())
}

/// Connexion patient : téléphone + mot de passe, verrouillage BF-01 (section
/// 14). Toute erreur donne `IdentifiantsInvalides` : numéro inconnu,
/// inscription inachevée (identité sans compte patient), compte verrouillé,
/// désactivé ou mauvais mot de passe (décisions P1 et L1). `verrouillage::verifier`
/// vérifie toujours un hachage, vrai ou faux, pour un temps de réponse
/// identique dans tous les cas.
pub async fn se_connecter(
    pool: &PgPool,
    jwt: &JwtService,
    telephone_saisi: &str,
    mot_de_passe_saisi: &str,
) -> Result<String, ErreurAuthPatient> {
    // Un format invalide ne correspond à aucun compte possible : le dire
    // ne révèle rien (décision T3), inutile de vérifier un faux hachage.
    let telephone = telephone::normaliser(telephone_saisi)?;
    let compte: Option<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT c.id, c.utilisateur_id FROM compte c JOIN utilisateur u ON u.id = c.utilisateur_id \
         WHERE u.telephone = $1 AND c.type_compte = 'patient'",
    )
    .bind(&telephone)
    .fetch_optional(pool)
    .await?;

    let verification = verrouillage::verifier(pool, compte.map(|(compte_id, _)| compte_id), mot_de_passe_saisi)
        .await
        .map_err(|erreur| match erreur {
            ErreurConnexion::IdentifiantsInvalides => ErreurAuthPatient::IdentifiantsInvalides,
            ErreurConnexion::Interne(detail) => ErreurAuthPatient::Interne(detail),
        })?;

    // Décision V2 : jamais de jeton complet sans le code TOTP. Le jeton
    // intermédiaire qui mène à la saisie du code arrive à l'étape 3.6 ;
    // d'ici là, aucune route ne permet d'activer un TOTP, ce cas ne peut
    // donc pas se produire, mais il est refusé par sécurité.
    if let Verification::SecondFacteurRequis(_) = verification {
        return Err(ErreurAuthPatient::IdentifiantsInvalides);
    }

    // `verifier` n'accepte qu'un compte existant : `compte` est forcément
    // présent ici.
    let (_, utilisateur_id) =
        compte.ok_or_else(|| ErreurAuthPatient::Interne("compte vérifié introuvable".into()))?;

    jwt.emettre(&utilisateur_id.to_string(), "patient")
        .map_err(|e| ErreurAuthPatient::Interne(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Base temporaire par test, jamais la base de développement : voir
    // « Tests et environnement », section 12 du CLAUDE.md.
    #[sqlx::test]
    #[ignore]
    async fn les_trois_flux_en_base(pool: PgPool) {
        let jwt = JwtService::new(&"a".repeat(32));

        let telephone = "+22670001001";
        let telephone_abandonne = "+22670001002";

        // --- création de compte ---
        let (code, _) = demander_otp(&pool, telephone).await.unwrap();

        assert!(matches!(
            creer_compte(&pool, &jwt, telephone, "000000", "Test", "Auth", chrono::NaiveDate::from_ymd_opt(1990, 1, 1).unwrap(), "Ouagadougou", "Motdepasse-123").await,
            Err(ErreurAuthPatient::CodeInvalide)
        ), "un mauvais code doit être refusé");

        assert!(matches!(
            creer_compte(&pool, &jwt, telephone, &code, "Test", "Auth", chrono::NaiveDate::from_ymd_opt(1990, 1, 1).unwrap(), "Ouagadougou", "court").await,
            Err(ErreurAuthPatient::MotDePasseNonConforme(_))
        ), "un mot de passe non conforme doit être refusé sans consommer le code");

        let jeton = creer_compte(&pool, &jwt, telephone, &code, "Test", "Auth", chrono::NaiveDate::from_ymd_opt(1990, 1, 1).unwrap(), "Ouagadougou", "Motdepasse-123")
            .await
            .expect("la création doit réussir avec un code et un mot de passe valides");
        assert!(!jeton.is_empty());

        assert!(matches!(
            creer_compte(&pool, &jwt, telephone, &code, "Test", "Auth", chrono::NaiveDate::from_ymd_opt(1990, 1, 1).unwrap(), "Ouagadougou", "Motdepasse-123").await,
            Err(ErreurAuthPatient::CodeInvalide)
        ), "le code déjà utilisé ne doit pas être réutilisable (règle NIST, otp.rs)");

        // --- mot de passe oublié, sur le compte qu'on vient de créer ---
        let (code2, _) = demander_otp(&pool, telephone).await.unwrap();

        assert!(matches!(
            creer_compte(&pool, &jwt, telephone, &code2, "Test", "Auth", chrono::NaiveDate::from_ymd_opt(1990, 1, 1).unwrap(), "Ouagadougou", "Motdepasse-123").await,
            Err(ErreurAuthPatient::TelephoneDejaUtilise)
        ), "creer_compte sur un compte déjà complet doit être refusé");
        // code2 vient d'être consommé par l'appel ci-dessus (vérifié avec
        // succès avant l'échec métier) : il en faut un nouveau, comme un
        // vrai client redemanderait pour le bon flux.
        let (code2b, _) = demander_otp(&pool, telephone).await.unwrap();

        reinitialiser_mot_de_passe(&pool, telephone, &code2b, "Nouveau-mdp-2")
            .await
            .expect("la réinitialisation doit réussir avec un code valide");

        // --- mot de passe oublié sur une inscription jamais finalisée ---
        let (code3, _) = demander_otp(&pool, telephone_abandonne).await.unwrap();
        assert!(matches!(
            reinitialiser_mot_de_passe(&pool, telephone_abandonne, &code3, "Motdepasse-123").await,
            Err(ErreurAuthPatient::TelephoneInconnu)
        ), "aucun compte complet sous ce numéro : rien à réinitialiser");
    }

    fn refuse(resultat: Result<String, ErreurAuthPatient>) -> bool {
        matches!(resultat, Err(ErreurAuthPatient::IdentifiantsInvalides))
    }

    #[sqlx::test]
    #[ignore]
    async fn connexion_en_base(pool: PgPool) {
        let jwt = JwtService::new(&"a".repeat(32));
        let telephone = "+22670016001";
        let bon = "Motdepasse-123";
        let naissance = NaiveDate::from_ymd_opt(1990, 1, 1).unwrap();

        let (code, _) = demander_otp(&pool, telephone).await.unwrap();
        creer_compte(&pool, &jwt, telephone, &code, "Test", "Connexion", naissance, "Ouagadougou", bon)
            .await
            .unwrap();

        // --- succès : jeton patient pour cette identité ---
        let jeton = se_connecter(&pool, &jwt, telephone, bon).await.unwrap();
        let claims = jwt.verifier(&jeton).unwrap();
        assert_eq!(claims.role, "patient");
        let utilisateur_id: Uuid = sqlx::query_scalar("SELECT id FROM utilisateur WHERE telephone = $1")
            .bind(telephone)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(claims.sub, utilisateur_id.to_string());

        // --- le même numéro saisi sous d'autres formes (décision C3) ---
        for saisie in ["70 01 60 01", "70-01-60-01", "70.01.60.01", "00226 70016001", "226 70 01 60 01"] {
            se_connecter(&pool, &jwt, saisie, bon).await.unwrap_or_else(|e| panic!("{saisie:?} : {e:?}"));
        }
        // L'OTP renvoie le numéro normalisé, vers lequel part le SMS.
        let (_, normalise) = demander_otp(&pool, "70 01 60 01").await.unwrap();
        assert_eq!(normalise, telephone);
        // Format invalide : erreur distincte, qui ne dit rien d'un compte (T3).
        assert!(matches!(
            se_connecter(&pool, &jwt, "70 01 60", bon).await,
            Err(ErreurAuthPatient::TelephoneInvalide)
        ));
        assert!(matches!(demander_otp(&pool, "(70) 01 60 01").await, Err(ErreurAuthPatient::TelephoneInvalide)));

        // --- même réponse pour un mauvais mot de passe, un numéro inconnu,
        // une inscription inachevée (OTP demandé, aucun compte) ---
        assert!(refuse(se_connecter(&pool, &jwt, telephone, "Mauvais-mdp-1").await));
        assert!(refuse(se_connecter(&pool, &jwt, "+22670016999", bon).await));
        let telephone_inacheve = "+22670016002";
        demander_otp(&pool, telephone_inacheve).await.unwrap();
        assert!(refuse(se_connecter(&pool, &jwt, telephone_inacheve, bon).await));

        // Texte court, identique dans tous les cas (décisions P1, L1).
        assert_eq!(ErreurAuthPatient::IdentifiantsInvalides.to_string(), "Numéro ou mot de passe incorrect.");

        // --- verrouillage : 5 échecs, puis le bon mot de passe est refusé ---
        se_connecter(&pool, &jwt, telephone, bon).await.unwrap(); // compteur remis à zéro
        for _ in 0..5 {
            assert!(refuse(se_connecter(&pool, &jwt, telephone, "Mauvais-mdp-1").await));
        }
        assert!(refuse(se_connecter(&pool, &jwt, telephone, bon).await), "refusé pendant le verrouillage");

        // --- réinitialisation par OTP : nouveau mot de passe, verrouillage
        // levé, version de jeton augmentée (C2) ---
        let version_avant: i32 = sqlx::query_scalar(
            "SELECT version_jeton FROM compte WHERE utilisateur_id = $1 AND type_compte = 'patient'",
        )
        .bind(utilisateur_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let (code_reinit, _) = demander_otp(&pool, telephone).await.unwrap();
        reinitialiser_mot_de_passe(&pool, telephone, &code_reinit, "Nouveau-mdp-2").await.unwrap();
        se_connecter(&pool, &jwt, telephone, "Nouveau-mdp-2").await.unwrap();
        assert!(refuse(se_connecter(&pool, &jwt, telephone, bon).await), "l'ancien mot de passe ne marche plus");
        let version_apres: i32 = sqlx::query_scalar(
            "SELECT version_jeton FROM compte WHERE utilisateur_id = $1 AND type_compte = 'patient'",
        )
        .bind(utilisateur_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(version_apres, version_avant + 1);

        // --- mot de passe non conforme : toutes les règles listées ---
        let (code_faible, _) = demander_otp(&pool, telephone).await.unwrap();
        match reinitialiser_mot_de_passe(&pool, telephone, &code_faible, "abc").await {
            Err(ErreurAuthPatient::MotDePasseNonConforme(regles)) => assert_eq!(
                regles,
                vec![RegleMotDePasse::LongueurMin, RegleMotDePasse::Chiffre, RegleMotDePasse::CaractereSpecial]
            ),
            autre => panic!("MotDePasseNonConforme attendu, obtenu {autre:?}"),
        }
    }
}
