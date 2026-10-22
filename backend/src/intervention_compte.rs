//! Interventions de l'équipe LaafiCare sur un compte compromis (lot « avant
//! le pilote », P2, décisions IC1 à IC8 du porteur, 2026-10-22 ; J11, DS10).
//! Outil local `intervenir_compte` (`src/bin/`), jamais une route HTTP.
//!
//! Trois actions, chacune tracée dans `intervention_compte` (migration
//! 0027 : jamais modifiée ni supprimée, jamais sur ses propres comptes,
//! garanti par la base), puis signalée à la personne par email après
//! l'enregistrement (NIST SP 800-63B rév. 4, §4.6) :
//! - retirer une affectation (même effet que le retrait par un responsable,
//!   `membres.rs`, mais l'équipe peut retirer le dernier responsable d'une
//!   structure, après confirmation) ;
//! - bloquer un compte : statut `desactive`, sessions fermées, liens par
//!   email et code SMS en cours annulés, mot de passe remplacé par
//!   l'empreinte d'une valeur que personne ne connaît (celui qui a volé
//!   l'ancien ne peut plus s'en servir) ;
//! - réactiver un compte bloqué, après vérification de la CNIB : le mot de
//!   passe est de nouveau remplacé (pendant le blocage, quelqu'un a pu le
//!   changer par SMS), et la personne en choisit un nouveau par « Mot de
//!   passe oublié ? ». Le second facteur n'est jamais touché.
//!
//! Ordres des verrous (CLAUDE.md §12) : retrait, structure → affectation →
//! sessions (DS6) ; blocage et réactivation, compte → liens par email →
//! sessions (comme la réinitialisation, 3.12).

use sqlx::PgPool;
use sqlx::types::Uuid;

use crate::alerte_securite::{self, Evenement};
use crate::auth_professionnel::nature_sqlx;
use crate::email::EmailSender;
use crate::mot_de_passe;
use crate::outils_administrateur::{CompteCible, Operateur};
use crate::session::{self, MotifRevocation};

/// Même garde-fou que le motif d'un rejet ou d'un refus (CHECK de la
/// migration 0027), en caractères.
pub const MOTIF_LONGUEUR_MAX: usize = 1000;

#[derive(Debug, thiserror::Error)]
pub enum ErreurIntervention {
    #[error("Le motif est obligatoire.")]
    MotifVide,
    #[error("Le motif dépasse 1 000 caractères.")]
    MotifTropLong,
    #[error("La vérification de la CNIB est obligatoire pour réactiver un compte.")]
    CnibNonVerifiee,
    #[error("Un administrateur n'intervient sur aucun de ses propres comptes.")]
    PropreCompte,
    #[error("Ce compte est déjà bloqué.")]
    DejaBloque,
    #[error("Ce compte n'est pas bloqué.")]
    PasBloque,
    #[error("Affectation inconnue, déjà close, ou d'un autre compte.")]
    AffectationInconnue,
    /// Le dernier responsable actif : l'outil demande une confirmation
    /// explicite, puis rappelle avec `dernier_responsable_confirme`.
    #[error("C'est le dernier responsable actif de cette structure.")]
    DernierResponsable,
    /// Seulement l'étape en échec, un texte fixe : jamais une saisie.
    #[error("erreur interne : {0}")]
    Interne(&'static str),
}

fn interne(etape: &'static str, erreur: &sqlx::Error) -> ErreurIntervention {
    tracing::error!(etape, nature = %nature_sqlx(erreur), "intervention sur un compte : erreur interne");
    ErreurIntervention::Interne(etape)
}

fn controler_motif(motif: &str) -> Result<&str, ErreurIntervention> {
    let motif = motif.trim();
    if motif.is_empty() {
        return Err(ErreurIntervention::MotifVide);
    }
    if motif.chars().count() > MOTIF_LONGUEUR_MAX {
        return Err(ErreurIntervention::MotifTropLong);
    }
    Ok(motif)
}

/// Écrit la trace. Le trigger de la migration 0027 refuse les propres
/// comptes de l'opérateur par `RAISE EXCEPTION` (SQLSTATE P0001, doc
/// PostgreSQL 18, annexe A) : seul trigger de cette table qui réagit à un
/// INSERT.
async fn tracer(
    conn: &mut sqlx::PgConnection,
    compte_id: Uuid,
    affectation_id: Option<Uuid>,
    action: &str,
    operateur: Operateur,
    motif: &str,
    cnib_verifiee: bool,
) -> Result<(), ErreurIntervention> {
    sqlx::query(
        "INSERT INTO intervention_compte (compte_id, affectation_id, action, administrateur_compte_id, motif, cnib_verifiee) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(compte_id)
    .bind(affectation_id)
    .bind(action)
    .bind(operateur.compte_id())
    .bind(motif)
    .bind(cnib_verifiee)
    .execute(conn)
    .await
    .map(|_| ())
    .map_err(|e| match e.as_database_error().and_then(|d| d.code()).as_deref() {
        Some("P0001") => ErreurIntervention::PropreCompte,
        _ => interne("trace de l'intervention", &e),
    })
}

/// Empreinte Argon2id d'une valeur aléatoire de 256 bits, jamais affichée
/// ni gardée : aucun mot de passe ne peut plus correspondre (la même
/// fonction que pour un vrai mot de passe, donc la même forme en base).
fn empreinte_inutilisable() -> Result<String, ErreurIntervention> {
    let (_, valeur) = session::octets_aleatoires_hexadecimaux();
    mot_de_passe::hacher(&valeur).map_err(|_| ErreurIntervention::Interne("empreinte du mot de passe"))
}

/// Statut actuel du compte (`actif` ou `desactive`), pour l'affichage.
pub async fn statut(pool: &PgPool, compte_id: Uuid) -> Result<String, ErreurIntervention> {
    sqlx::query_scalar("SELECT statut FROM compte WHERE id = $1")
        .bind(compte_id)
        .fetch_one(pool)
        .await
        .map_err(|e| interne("lecture du statut", &e))
}

/// Nombre d'administrateurs actifs avec un second facteur actif : l'outil
/// avertit avant de bloquer un administrateur s'il en resterait moins de
/// deux (règle d'organisation, CLAUDE.md §14).
pub async fn administrateurs_operationnels(pool: &PgPool) -> Result<i64, ErreurIntervention> {
    sqlx::query_scalar(
        "SELECT count(*) FROM compte c WHERE c.type_compte = 'administrateur_laaficare' AND c.statut = 'actif' \
           AND EXISTS (SELECT 1 FROM second_facteur_totp t WHERE t.compte_id = c.id AND t.statut = 'actif')",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| interne("comptage des administrateurs", &e))
}

// ---------------------------------------------------------------------
// Retrait d'une affectation
// ---------------------------------------------------------------------

/// Une affectation active du compte, telle que l'outil l'affiche.
#[derive(Debug, sqlx::FromRow)]
pub struct AffectationVisee {
    pub affectation_id: Uuid,
    pub structure_nom: String,
    pub role: String,
}

pub async fn affectations_actives(pool: &PgPool, compte_id: Uuid) -> Result<Vec<AffectationVisee>, ErreurIntervention> {
    sqlx::query_as(
        "SELECT a.id AS affectation_id, s.nom AS structure_nom, a.role \
         FROM affectation a JOIN structure s ON s.id = a.structure_id \
         WHERE a.compte_id = $1 AND a.statut = 'active' ORDER BY a.date_creation, a.id",
    )
    .bind(compte_id)
    .fetch_all(pool)
    .await
    .map_err(|e| interne("liste des affectations", &e))
}

/// Retire une affectation du compte visé (IC3, action 1). L'auteur de la
/// clôture est le compte administrateur de l'opérateur. Renvoie vrai si la
/// personne a été prévenue par email.
#[allow(clippy::too_many_arguments)]
pub async fn retirer_affectation(
    pool: &PgPool,
    email_sender: &dyn EmailSender,
    contact_securite: &str,
    operateur: Operateur,
    compte_id: Uuid,
    affectation_id: Uuid,
    motif: &str,
    dernier_responsable_confirme: bool,
) -> Result<bool, ErreurIntervention> {
    let motif = controler_motif(motif)?;
    let mut tx = pool.begin().await.map_err(|e| interne("transaction", &e))?;
    // 1. La structure de l'affectation, lue sans verrou puis verrouillée :
    // l'ordre structure → affectation (DS6) reste celui de `membres.rs`.
    let structure_id: Option<Uuid> =
        sqlx::query_scalar("SELECT structure_id FROM affectation WHERE id = $1 AND compte_id = $2")
            .bind(affectation_id)
            .bind(compte_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| interne("lecture de l'affectation", &e))?;
    let structure_id = structure_id.ok_or(ErreurIntervention::AffectationInconnue)?;
    sqlx::query("SELECT id FROM structure WHERE id = $1 FOR UPDATE")
        .bind(structure_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| interne("verrou de la structure", &e))?;
    // 2. L'affectation.
    let ligne: Option<(String, String)> =
        sqlx::query_as("SELECT role, statut FROM affectation WHERE id = $1 FOR UPDATE")
            .bind(affectation_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| interne("verrou de l'affectation", &e))?;
    let Some((role, statut)) = ligne else {
        return Err(ErreurIntervention::AffectationInconnue);
    };
    if statut != "active" {
        return Err(ErreurIntervention::AffectationInconnue);
    }
    if role == "responsable" && !dernier_responsable_confirme {
        let responsables: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM affectation WHERE structure_id = $1 AND role = 'responsable' AND statut = 'active'",
        )
        .bind(structure_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| interne("comptage des responsables", &e))?;
        if responsables <= 1 {
            return Err(ErreurIntervention::DernierResponsable);
        }
    }
    sqlx::query(
        "UPDATE affectation SET statut = 'desactivee', desactivee_le = now(), desactivee_par_compte_id = $2 WHERE id = $1",
    )
    .bind(affectation_id)
    .bind(operateur.compte_id())
    .execute(&mut *tx)
    .await
    .map_err(|e| interne("clôture de l'affectation", &e))?;
    tracer(&mut tx, compte_id, Some(affectation_id), "retrait_affectation", operateur, motif, false).await?;
    // 3. Les sessions ouvertes sur cette affectation.
    sqlx::query(
        "UPDATE session SET revoquee_le = now(), motif_revocation = $2 WHERE affectation_id = $1 AND revoquee_le IS NULL",
    )
    .bind(affectation_id)
    .bind(MotifRevocation::DesactivationAffectation.as_str())
    .execute(&mut *tx)
    .await
    .map_err(|e| interne("fermeture des sessions", &e))?;
    tx.commit().await.map_err(|e| interne("validation de la transaction", &e))?;

    Ok(alerte_securite::prevenir(pool, email_sender, contact_securite, compte_id, Evenement::AccesRetire).await)
}

// ---------------------------------------------------------------------
// Blocage et réactivation
// ---------------------------------------------------------------------

/// Bloque le compte visé (IC3, action 2). Renvoie vrai si la personne a été
/// prévenue par email.
pub async fn bloquer(
    pool: &PgPool,
    email_sender: &dyn EmailSender,
    contact_securite: &str,
    operateur: Operateur,
    cible: &CompteCible,
    motif: &str,
) -> Result<bool, ErreurIntervention> {
    let motif = controler_motif(motif)?;
    // Argon2id avant la transaction : le hachage ne doit pas garder le
    // verrou du compte ouvert.
    let empreinte = empreinte_inutilisable()?;
    let mut tx = pool.begin().await.map_err(|e| interne("transaction", &e))?;
    // 1. Le compte.
    let statut: String = sqlx::query_scalar("SELECT statut FROM compte WHERE id = $1 FOR UPDATE")
        .bind(cible.compte_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| interne("verrou du compte", &e))?;
    if statut != "actif" {
        return Err(ErreurIntervention::DejaBloque);
    }
    sqlx::query("UPDATE compte SET statut = 'desactive', mot_de_passe_hash = $2 WHERE id = $1")
        .bind(cible.compte_id)
        .bind(&empreinte)
        .execute(&mut *tx)
        .await
        .map_err(|e| interne("blocage du compte", &e))?;
    // Un code SMS en cours (compte patient) ne sert plus ; la demande d'un
    // nouveau code ne donne plus rien (`otp.rs`).
    sqlx::query(
        "UPDATE utilisateur SET otp_code_hash = NULL, otp_expire_a = NULL \
         WHERE id = (SELECT utilisateur_id FROM compte WHERE id = $1) \
           AND EXISTS (SELECT 1 FROM compte WHERE id = $1 AND type_compte = 'patient')",
    )
    .bind(cible.compte_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| interne("annulation du code SMS", &e))?;
    tracer(&mut tx, cible.compte_id, None, "blocage_compte", operateur, motif, false).await?;
    // 2. Les liens par email en attente (vérification et réinitialisation).
    sqlx::query(
        "UPDATE jeton_email SET annule_le = now() WHERE compte_id = $1 AND utilise_le IS NULL AND annule_le IS NULL",
    )
    .bind(cible.compte_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| interne("annulation des liens", &e))?;
    // 3. Toutes les sessions, `version_jeton` augmentée.
    session::fermer_toutes(&mut tx, cible.compte_id, MotifRevocation::DesactivationCompte, true)
        .await
        .map_err(|e| interne("fermeture des sessions", &e))?;
    tx.commit().await.map_err(|e| interne("validation de la transaction", &e))?;

    Ok(alerte_securite::prevenir(pool, email_sender, contact_securite, cible.compte_id, Evenement::CompteBloque).await)
}

/// Réactive un compte bloqué (IC3, action 3), CNIB vérifiée. Renvoie vrai si
/// la personne a été prévenue par email.
pub async fn reactiver(
    pool: &PgPool,
    email_sender: &dyn EmailSender,
    contact_securite: &str,
    operateur: Operateur,
    cible: &CompteCible,
    motif: &str,
    cnib_verifiee: bool,
) -> Result<bool, ErreurIntervention> {
    let motif = controler_motif(motif)?;
    if !cnib_verifiee {
        return Err(ErreurIntervention::CnibNonVerifiee);
    }
    let empreinte = empreinte_inutilisable()?;
    let mut tx = pool.begin().await.map_err(|e| interne("transaction", &e))?;
    let statut: String = sqlx::query_scalar("SELECT statut FROM compte WHERE id = $1 FOR UPDATE")
        .bind(cible.compte_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| interne("verrou du compte", &e))?;
    if statut != "desactive" {
        return Err(ErreurIntervention::PasBloque);
    }
    // Verrouillage remis à zéro : la personne doit pouvoir demander tout de
    // suite son nouveau mot de passe.
    sqlx::query(
        "UPDATE compte SET statut = 'actif', mot_de_passe_hash = $2, tentatives_echouees = 0, verrouille_jusqua = NULL \
         WHERE id = $1",
    )
    .bind(cible.compte_id)
    .bind(&empreinte)
    .execute(&mut *tx)
    .await
    .map_err(|e| interne("réactivation du compte", &e))?;
    tracer(&mut tx, cible.compte_id, None, "reactivation_compte", operateur, motif, true).await?;
    // Aucune session ne peut être ouverte sur un compte bloqué ; la version
    // augmente quand même, par précaution.
    session::fermer_toutes(&mut tx, cible.compte_id, MotifRevocation::DesactivationCompte, true)
        .await
        .map_err(|e| interne("fermeture des sessions", &e))?;
    tx.commit().await.map_err(|e| interne("validation de la transaction", &e))?;

    Ok(alerte_securite::prevenir(pool, email_sender, contact_securite, cible.compte_id, Evenement::CompteReactive).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_patient::{self, ErreurAuthPatient};
    use crate::email::EmailsRecus;
    use crate::jeton_email;
    use crate::jwt::JwtService;
    use crate::outils_administrateur::TypeCompte;
    use chrono::NaiveDate;

    // Base temporaire par test ; données fictives.

    const CONTACT: &str = "securite@example.org";
    const PATIENT: &str = "Patient#2026";

    /// Administrateur opérateur, d'une autre identité que les comptes visés.
    async fn operateur(pool: &PgPool, telephone: &str) -> Operateur {
        let u: Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (nom, prenom, telephone, email) VALUES ('SOME', 'Admin', $1, $2) RETURNING id",
        )
        .bind(telephone)
        .bind(format!("admin{}@example.org", &telephone[4..]))
        .fetch_one(pool)
        .await
        .unwrap();
        let c: Uuid = sqlx::query_scalar(
            "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) \
             VALUES ($1, 'administrateur_laaficare', 'x') RETURNING id",
        )
        .bind(u)
        .fetch_one(pool)
        .await
        .unwrap();
        Operateur::pour_test(c)
    }

    /// Patient inscrit par le vrai parcours (code SMS), avec un email vérifié.
    async fn patient(pool: &PgPool, telephone: &str) -> CompteCible {
        let jwt = JwtService::new(&"a".repeat(32));
        let (code, tel) = auth_patient::demander_otp(pool, telephone).await.unwrap();
        auth_patient::creer_compte(
            pool,
            &jwt,
            &tel,
            &code,
            "OUEDRAOGO",
            "Awa",
            NaiveDate::from_ymd_opt(2000, 1, 1).unwrap(),
            "Ouagadougou",
            PATIENT,
        )
        .await
        .unwrap();
        sqlx::query("UPDATE utilisateur SET email = $2 WHERE telephone = $1")
            .bind(&tel)
            .bind(format!("p{}@example.org", &tel[4..]))
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("UPDATE utilisateur SET email_verifie_le = now() WHERE telephone = $1")
            .bind(&tel)
            .execute(pool)
            .await
            .unwrap();
        let compte_id: Uuid = sqlx::query_scalar(
            "SELECT c.id FROM compte c JOIN utilisateur u ON u.id = c.utilisateur_id \
             WHERE u.telephone = $1 AND c.type_compte = 'patient'",
        )
        .bind(&tel)
        .fetch_one(pool)
        .await
        .unwrap();
        CompteCible { compte_id, type_compte: TypeCompte::Patient, nom: None, prenom: None }
    }

    /// Antidate les envois de code déjà faits : sinon le délai de 60 s entre
    /// deux codes (O1) refuserait la demande suivante du test.
    async fn passer_le_delai_otp(pool: &PgPool) {
        sqlx::query("UPDATE envoi_otp SET envoye_le = envoye_le - interval '2 minutes'").execute(pool).await.unwrap();
    }

    async fn sessions_ouvertes(pool: &PgPool, compte_id: Uuid) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM session WHERE compte_id = $1 AND revoquee_le IS NULL")
            .bind(compte_id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /// IC3 : blocage d'un patient (statut, sessions, version du jeton, code
    /// SMS effacé, plus aucun code ensuite, ancien mot de passe inutilisable,
    /// alerte) ; réactivation sans CNIB refusée ; réactivation (mot de passe
    /// de nouveau inutilisable), puis réinitialisation par SMS et connexion.
    #[sqlx::test]
    #[ignore]
    async fn blocage_puis_reactivation_d_un_patient(pool: PgPool) {
        let emails = EmailsRecus::default();
        let jwt = JwtService::new(&"a".repeat(32));
        let op = operateur(&pool, "+22670050001").await;
        let cible = patient(&pool, "+22670050002").await;
        auth_patient::se_connecter(&pool, &jwt, "+22670050002", PATIENT).await.unwrap();
        assert_eq!(sessions_ouvertes(&pool, cible.compte_id).await, 2);
        // Un code SMS en cours au moment du blocage.
        passer_le_delai_otp(&pool).await;
        auth_patient::demander_otp(&pool, "+22670050002").await.unwrap();
        let version_avant: i32 = sqlx::query_scalar("SELECT version_jeton FROM compte WHERE id = $1")
            .bind(cible.compte_id)
            .fetch_one(&pool)
            .await
            .unwrap();

        assert!(bloquer(&pool, &emails, CONTACT, op, &cible, "  Mot de passe volé (fictif)  ").await.unwrap());
        let (statut, version, code): (String, i32, Option<String>) = sqlx::query_as(
            "SELECT c.statut, c.version_jeton, u.otp_code_hash FROM compte c JOIN utilisateur u ON u.id = c.utilisateur_id \
             WHERE c.id = $1",
        )
        .bind(cible.compte_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!((statut.as_str(), version > version_avant, code), ("desactive", true, None));
        assert_eq!(sessions_ouvertes(&pool, cible.compte_id).await, 0);
        let motifs: Vec<String> = sqlx::query_scalar("SELECT DISTINCT motif_revocation FROM session WHERE compte_id = $1")
            .bind(cible.compte_id)
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(motifs, ["desactivation_compte"]);
        assert!(matches!(
            auth_patient::se_connecter(&pool, &jwt, "+22670050002", PATIENT).await,
            Err(ErreurAuthPatient::IdentifiantsInvalides)
        ));
        // Plus aucun code SMS pour ce compte.
        passer_le_delai_otp(&pool).await;
        assert!(matches!(auth_patient::demander_otp(&pool, "+22670050002").await, Err(ErreurAuthPatient::CompteBloque)));
        let code: Option<String> =
            sqlx::query_scalar("SELECT otp_code_hash FROM utilisateur WHERE telephone = '+22670050002'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(code.is_none());
        let (sujet_corps, nb) = {
            let recus = emails.0.lock().unwrap();
            (recus.last().unwrap().1.clone(), recus.len())
        };
        assert_eq!(nb, 1);
        assert!(sujet_corps.contains("bloqué") && sujet_corps.contains(CONTACT) && !sujet_corps.contains("volé"));
        assert!(matches!(bloquer(&pool, &emails, CONTACT, op, &cible, "encore").await, Err(ErreurIntervention::DejaBloque)));

        // Réactivation : CNIB obligatoire ; ancien mot de passe toujours
        // inutilisable ; réinitialisation par SMS, puis connexion.
        assert!(matches!(
            reactiver(&pool, &emails, CONTACT, op, &cible, "Identité vérifiée (fictif)", false).await,
            Err(ErreurIntervention::CnibNonVerifiee)
        ));
        assert!(reactiver(&pool, &emails, CONTACT, op, &cible, "Identité vérifiée (fictif)", true).await.unwrap());
        assert!(emails.0.lock().unwrap().last().unwrap().1.contains("Mot de passe oublié"));
        assert!(matches!(
            auth_patient::se_connecter(&pool, &jwt, "+22670050002", PATIENT).await,
            Err(ErreurAuthPatient::IdentifiantsInvalides)
        ));
        passer_le_delai_otp(&pool).await;
        let (code, _) = auth_patient::demander_otp(&pool, "+22670050002").await.unwrap();
        auth_patient::reinitialiser_mot_de_passe(&pool, "+22670050002", &code, "Nouveau#2026").await.unwrap();
        auth_patient::se_connecter(&pool, &jwt, "+22670050002", "Nouveau#2026").await.unwrap();
        assert!(matches!(reactiver(&pool, &emails, CONTACT, op, &cible, "x", true).await, Err(ErreurIntervention::PasBloque)));

        // Traces : blocage puis réactivation, CNIB seulement à la seconde ;
        // jamais modifiables.
        let traces: Vec<(String, bool, String)> = sqlx::query_as(
            "SELECT action, cnib_verifiee, motif FROM intervention_compte WHERE compte_id = $1 ORDER BY date_intervention, id",
        )
        .bind(cible.compte_id)
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            traces,
            [
                ("blocage_compte".into(), false, "Mot de passe volé (fictif)".into()),
                ("reactivation_compte".into(), true, "Identité vérifiée (fictif)".into())
            ]
        );
        assert!(sqlx::query("UPDATE intervention_compte SET motif = 'autre'").execute(&pool).await.is_err());
        assert!(sqlx::query("DELETE FROM intervention_compte").execute(&pool).await.is_err());
    }

    /// IC3 : blocage d'un professionnel ; ses liens par email en attente
    /// (réinitialisation) sont annulés.
    #[sqlx::test]
    #[ignore]
    async fn blocage_annule_les_liens_par_email(pool: PgPool) {
        let emails = EmailsRecus::default();
        let op = operateur(&pool, "+22670050011").await;
        let u: Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (nom, prenom, telephone, email) VALUES ('ZONGO', 'Paul', '+22670050012', 'pro50012@example.org') \
             RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query("UPDATE utilisateur SET email_verifie_le = now() WHERE id = $1").bind(u).execute(&pool).await.unwrap();
        let compte_id: Uuid = sqlx::query_scalar(
            "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'professionnel', 'x') RETURNING id",
        )
        .bind(u)
        .fetch_one(&pool)
        .await
        .unwrap();
        let mut conn = pool.acquire().await.unwrap();
        assert!(jeton_email::emettre_reinitialisation(&mut conn, compte_id).await.unwrap().is_some());
        drop(conn);
        let cible = CompteCible { compte_id, type_compte: TypeCompte::Professionnel, nom: None, prenom: None };

        bloquer(&pool, &emails, CONTACT, op, &cible, "Compte compromis (fictif)").await.unwrap();
        let en_attente: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM jeton_email WHERE compte_id = $1 AND utilise_le IS NULL AND annule_le IS NULL",
        )
        .bind(compte_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(en_attente, 0);
    }

    /// IC3 : retrait d'une affectation par l'équipe ; le dernier responsable
    /// demande une confirmation, puis est retiré ; auteur = l'opérateur.
    #[sqlx::test]
    #[ignore]
    async fn retrait_du_dernier_responsable_apres_confirmation(pool: PgPool) {
        let emails = EmailsRecus::default();
        let op = operateur(&pool, "+22670050021").await;
        let u: Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (nom, prenom, telephone, email) VALUES ('ZONGO', 'Paul', '+22670050022', 'pro50022@example.org') \
             RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query("UPDATE utilisateur SET email_verifie_le = now() WHERE id = $1").bind(u).execute(&pool).await.unwrap();
        let compte_id: Uuid = sqlx::query_scalar(
            "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'professionnel', 'x') RETURNING id",
        )
        .bind(u)
        .fetch_one(&pool)
        .await
        .unwrap();
        let structure_id: Uuid = sqlx::query_scalar(
            "INSERT INTO structure (type_structure, nom, telephone, telephone_urgence, region, email, commune, province, \
                                    latitude, longitude, createur_compte_id, statut) \
             VALUES ('munaseb', 'MUNASEB (test)', '+22670000000', '+22670000000', 'Région fictive', 'structure@example.org', \
                     'Commune fictive', 'Province fictive', 12.371400, -1.519700, $1, 'validee') RETURNING id",
        )
        .bind(compte_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let affectation_id: Uuid = sqlx::query_scalar(
            "INSERT INTO affectation (compte_id, structure_id, role) VALUES ($1, $2, 'responsable') RETURNING id",
        )
        .bind(compte_id)
        .bind(structure_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        session::ouvrir(&pool, compte_id, Some(affectation_id)).await.unwrap();

        assert!(matches!(
            retirer_affectation(&pool, &emails, CONTACT, op, compte_id, affectation_id, "Compte compromis (fictif)", false)
                .await,
            Err(ErreurIntervention::DernierResponsable)
        ));
        assert_eq!(affectations_actives(&pool, compte_id).await.unwrap().len(), 1, "rien n'a changé");
        assert!(
            retirer_affectation(&pool, &emails, CONTACT, op, compte_id, affectation_id, "Compte compromis (fictif)", true)
                .await
                .unwrap()
        );
        let par: Option<Uuid> = sqlx::query_scalar("SELECT desactivee_par_compte_id FROM affectation WHERE id = $1")
            .bind(affectation_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(par, Some(op.compte_id()));
        assert_eq!(sessions_ouvertes(&pool, compte_id).await, 0);
        let trace: Option<Uuid> = sqlx::query_scalar(
            "SELECT affectation_id FROM intervention_compte WHERE compte_id = $1 AND action = 'retrait_affectation'",
        )
        .bind(compte_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(trace, Some(affectation_id));
        // Déjà close.
        assert!(matches!(
            retirer_affectation(&pool, &emails, CONTACT, op, compte_id, affectation_id, "encore", true).await,
            Err(ErreurIntervention::AffectationInconnue)
        ));
    }

    /// IC2 : jamais sur ses propres comptes, refusé par la base ; rien ne
    /// change. Motif vide ou trop long refusé avant la base.
    #[sqlx::test]
    #[ignore]
    async fn jamais_sur_ses_propres_comptes(pool: PgPool) {
        let emails = EmailsRecus::default();
        let cible = patient(&pool, "+22670050031").await;
        // Le compte administrateur de la même personne.
        let admin: Uuid = sqlx::query_scalar(
            "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) \
             SELECT utilisateur_id, 'administrateur_laaficare', 'x' FROM compte WHERE id = $1 RETURNING id",
        )
        .bind(cible.compte_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let op = Operateur::pour_test(admin);
        assert!(matches!(
            bloquer(&pool, &emails, CONTACT, op, &cible, "Essai (fictif)").await,
            Err(ErreurIntervention::PropreCompte)
        ));
        assert_eq!(statut(&pool, cible.compte_id).await.unwrap(), "actif");
        assert!(emails.0.lock().unwrap().is_empty());

        let autre = operateur(&pool, "+22670050032").await;
        assert!(matches!(bloquer(&pool, &emails, CONTACT, autre, &cible, "   ").await, Err(ErreurIntervention::MotifVide)));
        let long = "x".repeat(MOTIF_LONGUEUR_MAX + 1);
        assert!(matches!(bloquer(&pool, &emails, CONTACT, autre, &cible, &long).await, Err(ErreurIntervention::MotifTropLong)));
    }
}
