//! Membres d'une structure et désactivation d'une affectation (étape 3.11
//! réduite, décisions DS1 à DS9 du porteur, 2026-10-21). Section 14 du
//! CLAUDE.md : la désactivation porte sur l'affectation, jamais sur le
//! compte, et ne supprime rien (la base n'admet que la clôture `active` →
//! `desactivee`, datée et attribuée) ; « celui qui peut inviter à un rôle
//! peut le désactiver » ; « le dernier responsable actif d'une structure ne
//! peut pas être désactivé ».
//!
//! Ordre des verrous (CLAUDE.md §12, DS6) : **structure → affectation →
//! sessions**, à la suite de « comptes → structure → invitation →
//! affectation ». Aucun cycle avec `session::renouveler`, qui ne verrouille
//! ni structure ni affectation.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use sqlx::types::Uuid;

use crate::alerte_securite::{self, Evenement};
use crate::auth_professionnel::nature_sqlx;
use crate::email::EmailSender;
use crate::session::MotifRevocation;

/// Un membre actif, tel que le responsable le voit (DS7) : ni téléphone ni
/// email, inutiles pour gérer les accès (même principe que la liste des
/// demandes pour l'agent, qui ne montre ni NIP ni téléphone).
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Membre {
    pub affectation_id: Uuid,
    pub nom: Option<String>,
    pub prenom: Option<String>,
    pub role: String,
    pub date_creation: DateTime<Utc>,
}

/// `GET /api/professionnels/membres` : affectations actives de la
/// structure, dans l'ordre de création.
pub async fn lister(pool: &PgPool, structure_id: Uuid) -> Result<Vec<Membre>, sqlx::Error> {
    sqlx::query_as(
        "SELECT a.id AS affectation_id, u.nom, u.prenom, a.role, a.date_creation \
         FROM affectation a JOIN compte c ON c.id = a.compte_id JOIN utilisateur u ON u.id = c.utilisateur_id \
         WHERE a.structure_id = $1 AND a.statut = 'active' ORDER BY a.date_creation, a.id",
    )
    .bind(structure_id)
    .fetch_all(pool)
    .await
}

#[derive(Debug, thiserror::Error)]
pub enum ErreurDesactivation {
    /// Inexistante, ou d'une autre structure : jamais distinguées (DS1).
    #[error("Affectation inconnue.")]
    AffectationInconnue,
    #[error("Cette affectation est déjà désactivée.")]
    TransitionInvalide { statut_actuel: String },
    /// §14 : la structure garde toujours un responsable actif.
    #[error("Le dernier responsable actif d'une structure ne peut pas être désactivé.")]
    DernierResponsable,
    #[error("erreur interne : {0}")]
    Interne(&'static str),
}

fn interne(etape: &'static str, erreur: &sqlx::Error) -> ErreurDesactivation {
    tracing::error!(etape, nature = %nature_sqlx(erreur), "désactivation d'une affectation : erreur interne");
    ErreurDesactivation::Interne(etape)
}

/// `POST /api/professionnels/membres/{id}/desactiver` (DS1 à DS6, DS8).
/// `responsable_compte_id` et `structure_id` viennent de l'affectation
/// `responsable` en exercice, jamais de l'adresse. Rôles de la MUNASEB :
/// ceux qu'il peut proposer (DS2), soit tous ceux qu'une MUNASEB admet. Un
/// responsable peut désactiver sa propre affectation, s'il n'est pas le
/// dernier (DS4).
pub async fn desactiver(
    pool: &PgPool,
    email_sender: &dyn EmailSender,
    contact_securite: &str,
    responsable_compte_id: Uuid,
    structure_id: Uuid,
    affectation_id: Uuid,
) -> Result<(), ErreurDesactivation> {
    let mut tx = pool.begin().await.map_err(|e| interne("transaction", &e))?;
    // 1. La structure : deux désactivations simultanées dans une même
    // structure se suivent ; le comptage des responsables (DS3) voit donc
    // toujours l'état à jour.
    sqlx::query("SELECT id FROM structure WHERE id = $1 FOR UPDATE")
        .bind(structure_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| interne("verrou de la structure", &e))?;
    // 2. L'affectation, de cette structure seulement.
    let ligne: Option<(Uuid, String, String)> = sqlx::query_as(
        "SELECT compte_id, role, statut FROM affectation WHERE id = $1 AND structure_id = $2 FOR UPDATE",
    )
    .bind(affectation_id)
    .bind(structure_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| interne("verrou de l'affectation", &e))?;
    let Some((compte_vise, role, statut)) = ligne else {
        return Err(ErreurDesactivation::AffectationInconnue);
    };
    if statut != "active" {
        return Err(ErreurDesactivation::TransitionInvalide { statut_actuel: statut });
    }
    if role == "responsable" {
        let responsables: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM affectation WHERE structure_id = $1 AND role = 'responsable' AND statut = 'active'",
        )
        .bind(structure_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| interne("comptage des responsables", &e))?;
        if responsables <= 1 {
            return Err(ErreurDesactivation::DernierResponsable);
        }
    }
    // Clôture datée et attribuée (seule modification que la base admet).
    sqlx::query(
        "UPDATE affectation SET statut = 'desactivee', desactivee_le = now(), desactivee_par_compte_id = $2 WHERE id = $1",
    )
    .bind(affectation_id)
    .bind(responsable_compte_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| interne("clôture de l'affectation", &e))?;
    // 3. Les sessions ouvertes sur cette affectation (DS5) : fermées tout de
    // suite, sans attendre leurs 12 h ; les autres sessions de la personne
    // ne changent pas, ni son compte.
    sqlx::query(
        "UPDATE session SET revoquee_le = now(), motif_revocation = $2 WHERE affectation_id = $1 AND revoquee_le IS NULL",
    )
    .bind(affectation_id)
    .bind(MotifRevocation::DesactivationAffectation.as_str())
    .execute(&mut *tx)
    .await
    .map_err(|e| interne("fermeture des sessions", &e))?;
    tx.commit().await.map_err(|e| interne("validation de la transaction", &e))?;

    // DS8, après la validation en base ; un échec n'annule rien.
    alerte_securite::prevenir(pool, email_sender, contact_securite, compte_vise, Evenement::AccesRetire).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::affectation;
    use crate::email::EmailsRecus;
    use crate::jwt::JwtService;
    use crate::session;

    // Base temporaire par test ; données fictives.

    const CONTACT: &str = "securite@example.org";

    /// Compte professionnel, email vérifié.
    async fn professionnel(pool: &PgPool, telephone: &str) -> Uuid {
        let u: Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (nom, prenom, telephone, email) VALUES ('ZONGO', 'Paul', $1, $2) RETURNING id",
        )
        .bind(telephone)
        .bind(format!("p{}@example.org", &telephone[4..]))
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query("UPDATE utilisateur SET email_verifie_le = now() WHERE id = $1").bind(u).execute(pool).await.unwrap();
        sqlx::query_scalar("INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'professionnel', 'x') RETURNING id")
            .bind(u)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /// MUNASEB validée (la base n'en admet qu'une).
    async fn structure(pool: &PgPool, createur: Uuid) -> Uuid {
        structure_au_statut(pool, createur, "validee").await
    }

    async fn structure_au_statut(pool: &PgPool, createur: Uuid, statut: &str) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO structure (type_structure, nom, telephone, telephone_urgence, region, email, commune, province, \
                                    latitude, longitude, createur_compte_id, statut) \
             VALUES ('munaseb', 'MUNASEB (test)', '+22670000000', '+22670000000', 'Région fictive', 'structure@example.org', \
                     'Commune fictive', 'Province fictive', 12.371400, -1.519700, $1, $2) RETURNING id",
        )
        .bind(createur)
        .bind(statut)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn affecter(pool: &PgPool, compte_id: Uuid, structure_id: Uuid, role: &str) -> Uuid {
        sqlx::query_scalar("INSERT INTO affectation (compte_id, structure_id, role) VALUES ($1, $2, $3) RETURNING id")
            .bind(compte_id)
            .bind(structure_id)
            .bind(role)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    async fn statut(pool: &PgPool, affectation_id: Uuid) -> String {
        sqlx::query_scalar("SELECT statut FROM affectation WHERE id = $1").bind(affectation_id).fetch_one(pool).await.unwrap()
    }

    /// Désactivation d'un agent : affectation close (date, auteur), session
    /// de cette affectation fermée, autre session intacte, alerte envoyée ;
    /// la liste ne le montre plus, sans téléphone ni email.
    #[sqlx::test]
    #[ignore]
    async fn desactivation_d_un_agent(pool: PgPool) {
        let emails = EmailsRecus::default();
        let resp = professionnel(&pool, "+22670400001").await;
        let s = structure(&pool, resp).await;
        affecter(&pool, resp, s, "responsable").await;
        let agent = professionnel(&pool, "+22670400002").await;
        let aff_agent = affecter(&pool, agent, s, "agent_assurance_munaseb").await;
        let session_agent = session::ouvrir(&pool, agent, Some(aff_agent)).await.unwrap();
        let autre_session = session::ouvrir(&pool, agent, None).await.unwrap();
        assert!(matches!(
            affectation::controler_exercice(&pool, agent, session_agent.version_jeton, session_agent.session_id, aff_agent)
                .await
                .unwrap(),
            affectation::ControleExercice::Valide(_)
        ));

        let membres = lister(&pool, s).await.unwrap();
        assert_eq!(membres.len(), 2);
        let json = serde_json::to_string(&membres).unwrap();
        assert!(!json.contains("+2267") && !json.contains("@example.org"), "ni téléphone ni email : {json}");

        desactiver(&pool, &emails, CONTACT, resp, s, aff_agent).await.unwrap();
        assert_eq!(statut(&pool, aff_agent).await, "desactivee");
        let par: Option<Uuid> = sqlx::query_scalar("SELECT desactivee_par_compte_id FROM affectation WHERE id = $1")
            .bind(aff_agent)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(par, Some(resp));

        // Session de l'affectation fermée ; l'autre intacte.
        let motifs: Vec<Option<String>> = sqlx::query_scalar("SELECT motif_revocation FROM session WHERE compte_id = $1 ORDER BY creee_le")
            .bind(agent)
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(motifs, [Some("desactivation_affectation".to_string()), None]);
        // Le jeton d'accès déjà émis ne sert plus (D4, session fermée) ; la
        // session sans affectation reste ouverte.
        assert!(matches!(
            affectation::controler_exercice(&pool, agent, session_agent.version_jeton, session_agent.session_id, aff_agent)
                .await
                .unwrap(),
            affectation::ControleExercice::JetonInvalide
        ));
        let autre_ouverte: bool = sqlx::query_scalar("SELECT revoquee_le IS NULL FROM session WHERE id = $1")
            .bind(autre_session.session_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(autre_ouverte);
        assert!(affectation::lister(&pool, agent).await.unwrap().is_empty(), "plus aucune affectation active");

        assert_eq!(lister(&pool, s).await.unwrap().len(), 1);
        let (dest, corps) = emails.0.lock().unwrap()[0].clone();
        assert_eq!(dest, "p70400002@example.org");
        assert!(corps.contains("(heure du Burkina Faso)") && corps.contains(CONTACT) && !corps.contains('\u{2014}'), "{corps}");
        assert!(!corps.contains("MUNASEB (test)"), "sans le nom de la structure");

        // Déjà désactivée.
        assert!(matches!(
            desactiver(&pool, &emails, CONTACT, resp, s, aff_agent).await,
            Err(ErreurDesactivation::TransitionInvalide { ref statut_actuel }) if statut_actuel == "desactivee"
        ));
    }

    /// DS3, DS4 : le dernier responsable reste ; un responsable se désactive
    /// lui-même quand il en reste un autre ; autre structure → inconnue.
    #[sqlx::test]
    #[ignore]
    async fn dernier_responsable_et_autre_structure(pool: PgPool) {
        let emails = EmailsRecus::default();
        let r1 = professionnel(&pool, "+22670400011").await;
        let s = structure(&pool, r1).await;
        let a1 = affecter(&pool, r1, s, "responsable").await;
        assert!(matches!(desactiver(&pool, &emails, CONTACT, r1, s, a1).await, Err(ErreurDesactivation::DernierResponsable)));

        let r2 = professionnel(&pool, "+22670400012").await;
        affecter(&pool, r2, s, "responsable").await;
        desactiver(&pool, &emails, CONTACT, r1, s, a1).await.unwrap();
        assert_eq!(statut(&pool, a1).await, "desactivee");

        // Une affectation d'une autre structure n'existe pas pour celle-ci.
        let r3 = professionnel(&pool, "+22670400013").await;
        let autre = structure_au_statut(&pool, r3, "en_attente").await;
        let a3 = affecter(&pool, r3, autre, "responsable").await;
        assert!(matches!(desactiver(&pool, &emails, CONTACT, r2, s, a3).await, Err(ErreurDesactivation::AffectationInconnue)));
        assert_eq!(statut(&pool, a3).await, "active");
    }

    /// DS3 sous concurrence : deux désactivations simultanées des deux
    /// derniers responsables ; une seule réussit, sans interblocage.
    #[sqlx::test]
    #[ignore]
    async fn deux_derniers_responsables_simultanes(pool: PgPool) {
        let r1 = professionnel(&pool, "+22670400021").await;
        let s = structure(&pool, r1).await;
        let a1 = affecter(&pool, r1, s, "responsable").await;
        let r2 = professionnel(&pool, "+22670400022").await;
        let a2 = affecter(&pool, r2, s, "responsable").await;
        let (p1, p2) = (pool.clone(), pool.clone());
        let t1 = tokio::spawn(async move { desactiver(&p1, &EmailsRecus::default(), CONTACT, r1, s, a2).await });
        let t2 = tokio::spawn(async move { desactiver(&p2, &EmailsRecus::default(), CONTACT, r2, s, a1).await });
        let (x, y) = (t1.await.unwrap(), t2.await.unwrap());
        assert!(x.is_ok() != y.is_ok(), "{x:?} / {y:?}");
        for e in [&x, &y].into_iter().filter_map(|r| r.as_ref().err()) {
            assert!(matches!(e, ErreurDesactivation::DernierResponsable), "{e:?}");
        }
    }

    /// DS6 : une désactivation et un renouvellement de la session de cette
    /// affectation, en même temps : pas d'interblocage (aucune erreur
    /// interne) ; après coup, la session est fermée.
    #[sqlx::test]
    #[ignore]
    async fn desactivation_et_renouvellement_simultanes(pool: PgPool) {
        let jwt = JwtService::new(&"a".repeat(32));
        let resp = professionnel(&pool, "+22670400031").await;
        let s = structure(&pool, resp).await;
        affecter(&pool, resp, s, "responsable").await;
        let agent = professionnel(&pool, "+22670400032").await;
        let aff = affecter(&pool, agent, s, "agent_assurance_munaseb").await;
        let paire = session::ouvrir(&pool, agent, Some(aff)).await.unwrap().en_paire(&jwt).unwrap();
        let jeton = paire.jeton_renouvellement.en_clair().to_string();

        let (p1, p2) = (pool.clone(), pool.clone());
        let t1 = tokio::spawn(async move { desactiver(&p1, &EmailsRecus::default(), CONTACT, resp, s, aff).await });
        let t2 = tokio::spawn(async move { session::renouveler(&p2, &jeton).await });
        t1.await.unwrap().unwrap();
        // Renouvelé avant la fermeture, ou refusé après : jamais une erreur
        // interne (un interblocage en serait une).
        match t2.await.unwrap() {
            Ok(_) | Err(session::ErreurSession::JetonInvalide) => {}
            Err(e) => panic!("{e:?}"),
        }
        let ouvertes: i64 = sqlx::query_scalar("SELECT count(*) FROM session WHERE affectation_id = $1 AND revoquee_le IS NULL")
            .bind(aff)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(ouvertes, 0, "aucune session ouverte sur l'affectation désactivée");
    }
}
