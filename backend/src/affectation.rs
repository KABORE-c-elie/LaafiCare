//! Affectations d'un professionnel : liste, contrôle « exercer » et
//! activation (étape 3.9, décisions D1 à D4 et D9 du 2026-10-18 ; J5).
//!
//! Après la connexion professionnelle, le jeton ne porte aucune affectation
//! (`professionnel_sans_affectation`). La personne liste ses affectations,
//! en active une, et reçoit un jeton `professionnel_en_exercice` qui la
//! porte (`aff`). Changer de structure réutilise l'activation, sans mot de
//! passe (J5).
//!
//! **Date du jour.** Une licence est en cours jusqu'à sa date de fin
//! comprise, à la date du jour **au Burkina Faso** :
//! `(now() AT TIME ZONE 'Africa/Ouagadougou')::date` (décision du porteur,
//! 2026-10-18). Doc PostgreSQL 18, §9.9.4 « AT TIME ZONE and AT LOCAL » :
//! `timestamp with time zone AT TIME ZONE zone` « Converts given time stamp
//! with time zone to time stamp without time zone, as the time would appear
//! in that zone ». Le résultat ne dépend donc pas du fuseau du serveur
//! PostgreSQL, contrairement à `current_date`.

use chrono::NaiveDate;
use serde::Serialize;
use sqlx::PgPool;
use sqlx::types::Uuid;

use crate::session::{self, ErreurSession, SessionActive};

/// Condition « l'établissement permet d'exercer » (D4) : validé, et licence
/// sans fin ou pas encore finie à la date du jour au Burkina. Alias de la
/// structure : `s`. Macro et non `const` : sqlx 0.9 n'accepte que des
/// chaînes SQL littérales (`concat!` en produit une à la compilation).
macro_rules! structure_exercable {
    () => {
        "(s.statut = 'validee' AND (s.licence_expire_le IS NULL \
          OR s.licence_expire_le >= (now() AT TIME ZONE 'Africa/Ouagadougou')::date))"
    };
}

/// Une affectation active, telle que l'application la montre (D1).
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct AffectationListee {
    pub id: Uuid,
    pub role: String,
    pub structure_id: Uuid,
    pub structure_nom: String,
    pub structure_type: String,
    pub structure_statut: String,
    pub licence_expire_le: Option<NaiveDate>,
    /// Vrai si l'établissement est validé et sa licence en cours :
    /// l'application peut alors proposer de l'activer, et sinon expliquer
    /// pourquoi (établissement suspendu, licence expirée).
    pub exercable: bool,
}

/// Les affectations **actives** du compte, dans l'ordre de création.
pub async fn lister(pool: &PgPool, compte_id: Uuid) -> Result<Vec<AffectationListee>, sqlx::Error> {
    sqlx::query_as(concat!(
        "SELECT a.id, a.role, s.id AS structure_id, s.nom AS structure_nom, s.type_structure AS structure_type, \
                s.statut AS structure_statut, s.licence_expire_le, ",
        structure_exercable!(),
        " AS exercable \
         FROM affectation a JOIN structure s ON s.id = a.structure_id \
         WHERE a.compte_id = $1 AND a.statut = 'active' \
         ORDER BY a.date_creation, a.id"
    ))
    .bind(compte_id)
    .fetch_all(pool)
    .await
}

/// Ce qu'un professionnel en exercice peut faire, et où (D4).
#[derive(Debug, Clone)]
pub struct Exercice {
    pub compte_id: Uuid,
    pub affectation_id: Uuid,
    pub structure_id: Uuid,
    pub type_structure: String,
    pub role: String,
}

/// Résultat du contrôle « exercer », fait à chaque requête.
#[derive(Debug)]
pub enum ControleExercice {
    Valide(Exercice),
    /// Jeton révoqué (RFC 6750 §3.1) : session fermée, expirée ou d'une autre
    /// affectation, compte désactivé, autre `version_jeton`, email plus
    /// vérifié (R0), affectation d'un autre compte. Réponse 401.
    JetonInvalide,
    /// Jeton valable, mais le droit a été retiré (J1) : affectation
    /// désactivée, établissement plus validé, licence expirée. Réponse 403.
    DroitRetire,
}

/// Contrôle « exercer » (D4), en une seule requête : le compte, l'email, la
/// session du `sid` (ouverte, portant la même affectation que le jeton),
/// puis l'affectation et son établissement.
pub async fn controler_exercice(
    pool: &PgPool,
    compte_id: Uuid,
    version_jeton: i32,
    session_id: Uuid,
    affectation_id: Uuid,
) -> Result<ControleExercice, sqlx::Error> {
    let ligne: Option<(bool, Option<bool>, Option<Uuid>, Option<String>, Option<String>)> = sqlx::query_as(concat!(
        // `IS NOT DISTINCT FROM` et non `=` : une session sans affectation a
        // `affectation_id` vide, et `= $4` donnerait NULL ; la colonne, lue
        // en `bool`, ferait alors une erreur (500) au lieu d'un refus (401).
        // Les autres termes de cette colonne portent sur des colonnes
        // NOT NULL (`statut`, `version_jeton`, `expire_le`) ou sont des
        // `IS [NOT] NULL` : jamais NULL.
        "SELECT c.statut = 'actif' AND c.version_jeton = $2 AND u.email_verifie_le IS NOT NULL \
                AND se.revoquee_le IS NULL AND se.expire_le > now() AND se.affectation_id IS NOT DISTINCT FROM $4, \
                a.statut = 'active' AND ",
        structure_exercable!(),
        ", s.id, s.type_structure, a.role \
         FROM compte c \
         JOIN utilisateur u ON u.id = c.utilisateur_id \
         JOIN session se ON se.id = $3 AND se.compte_id = c.id \
         LEFT JOIN affectation a ON a.id = $4 AND a.compte_id = c.id \
         LEFT JOIN structure s ON s.id = a.structure_id \
         WHERE c.id = $1 AND c.type_compte = 'professionnel'"
    ))
    .bind(compte_id)
    .bind(version_jeton)
    .bind(session_id)
    .bind(affectation_id)
    .fetch_optional(pool)
    .await?;

    let Some((jeton_valide, droit, structure_id, type_structure, role)) = ligne else {
        return Ok(ControleExercice::JetonInvalide);
    };
    // Une affectation qui n'est pas celle du compte ne donne aucune ligne
    // d'affectation : jeton invalide, comme un jeton forgé.
    let (Some(droit), Some(structure_id), Some(type_structure), Some(role)) = (droit, structure_id, type_structure, role)
    else {
        return Ok(ControleExercice::JetonInvalide);
    };
    if !jeton_valide {
        return Ok(ControleExercice::JetonInvalide);
    }
    if !droit {
        return Ok(ControleExercice::DroitRetire);
    }
    Ok(ControleExercice::Valide(Exercice { compte_id, affectation_id, structure_id, type_structure, role }))
}

#[derive(Debug, thiserror::Error)]
pub enum ErreurActivation {
    /// Affectation inexistante, ou d'un autre compte : jamais distingués
    /// (D1, 404).
    #[error("affectation inconnue")]
    AffectationInconnue,
    /// Affectation désactivée, ou établissement qui ne permet pas d'exercer
    /// (D1, 403).
    #[error("cette affectation ne permet pas d'exercer : affectation désactivée, établissement non validé ou licence expirée")]
    NonExercable,
    /// La session du jeton a été fermée entre-temps (401).
    #[error("session fermée")]
    SessionFermee,
    #[error("erreur interne : {0}")]
    Interne(String),
}

impl From<sqlx::Error> for ErreurActivation {
    fn from(erreur: sqlx::Error) -> Self {
        ErreurActivation::Interne(erreur.to_string())
    }
}

/// Active une affectation (D1, D3 a) : la session du jeton est remplacée par
/// une session de cette affectation, avec la même date de fin absolue.
/// Une transaction ; le compte est verrouillé avant les sessions (règle S2).
pub async fn activer(
    pool: &PgPool,
    compte_id: Uuid,
    session_id: Uuid,
    affectation_id: Uuid,
) -> Result<SessionActive, ErreurActivation> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT id FROM compte WHERE id = $1 FOR UPDATE")
        .bind(compte_id)
        .fetch_one(&mut *tx)
        .await?;

    let etat: Option<(bool,)> = sqlx::query_as(concat!(
        "SELECT a.statut = 'active' AND ",
        structure_exercable!(),
        " FROM affectation a JOIN structure s ON s.id = a.structure_id \
         WHERE a.id = $1 AND a.compte_id = $2"
    ))
    .bind(affectation_id)
    .bind(compte_id)
    .fetch_optional(&mut *tx)
    .await?;
    match etat {
        None => return Err(ErreurActivation::AffectationInconnue),
        Some((false,)) => return Err(ErreurActivation::NonExercable),
        Some((true,)) => {}
    }

    let nouvelle = session::remplacer(&mut tx, session_id, compte_id, affectation_id)
        .await
        .map_err(|erreur| match erreur {
            ErreurSession::JetonInvalide => ErreurActivation::SessionFermee,
            autre => ErreurActivation::Interne(autre.to_string()),
        })?;
    tx.commit().await?;
    Ok(nouvelle)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Base temporaire par test ; données fictives.

    async fn professionnel(pool: &PgPool, telephone: &str) -> Uuid {
        let utilisateur_id: Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (telephone, email) VALUES ($1, $1 || '@example.org') RETURNING id",
        )
        .bind(telephone)
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query("UPDATE utilisateur SET email_verifie_le = now() WHERE id = $1")
            .bind(utilisateur_id)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query_scalar(
            "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'professionnel', 'x') RETURNING id",
        )
        .bind(utilisateur_id)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// Clinique (plusieurs peuvent être validées) au statut donné.
    async fn structure(pool: &PgPool, createur: Uuid, nom: &str, statut: &str) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO structure (type_structure, nom, telephone, commune, province, latitude, longitude, createur_compte_id, statut) \
             VALUES ('clinique', $2, '+22670000000', 'Ouagadougou', 'Kadiogo', 12.371400, -1.519700, $1, $3) RETURNING id",
        )
        .bind(createur)
        .bind(nom)
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

    /// Fin de licence relative à la date du jour au Burkina.
    async fn licence(pool: &PgPool, structure_id: Uuid, jours: i32) {
        sqlx::query(
            "UPDATE structure SET licence_expire_le = (now() AT TIME ZONE 'Africa/Ouagadougou')::date + $2 WHERE id = $1",
        )
        .bind(structure_id)
        .bind(jours)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn desactiver(pool: &PgPool, affectation_id: Uuid, par: Uuid) {
        sqlx::query(
            "UPDATE affectation SET statut = 'desactivee', desactivee_le = now(), desactivee_par_compte_id = $2 WHERE id = $1",
        )
        .bind(affectation_id)
        .bind(par)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn controle(pool: &PgPool, session: &SessionActive, affectation_id: Uuid) -> ControleExercice {
        controler_exercice(pool, session.compte_id, session.version_jeton, session.session_id, affectation_id)
            .await
            .unwrap()
    }

    #[sqlx::test]
    #[ignore]
    async fn liste_des_affectations_actives(pool: PgPool) {
        let compte = professionnel(&pool, "+22670170001").await;
        let ouverte = structure(&pool, compte, "Clinique ouverte", "validee").await;
        let suspendue = structure(&pool, compte, "Clinique suspendue", "suspendue").await;
        let expiree = structure(&pool, compte, "Clinique expirée", "validee").await;
        licence(&pool, expiree, -1).await;
        let medecin = affecter(&pool, compte, ouverte, "medecin").await;
        affecter(&pool, compte, suspendue, "medecin").await;
        affecter(&pool, compte, expiree, "medecin").await;
        let ancienne = affecter(&pool, compte, ouverte, "directeur").await;
        desactiver(&pool, ancienne, compte).await;

        let liste = lister(&pool, compte).await.unwrap();
        let resume: Vec<_> = liste.iter().map(|a| (a.structure_nom.as_str(), a.exercable)).collect();
        assert_eq!(resume, [("Clinique ouverte", true), ("Clinique suspendue", false), ("Clinique expirée", false)]);
        assert_eq!(liste[0].id, medecin);
        assert_eq!(liste[1].structure_statut, "suspendue");
    }

    /// D3 a : l'activation remplace la session, avec la même date de fin ;
    /// changer d'affectation ensuite réutilise l'activation (J5).
    #[sqlx::test]
    #[ignore]
    async fn activation_remplace_la_session_sans_la_prolonger(pool: PgPool) {
        let compte = professionnel(&pool, "+22670170002").await;
        let a = structure(&pool, compte, "Clinique A", "validee").await;
        let b = structure(&pool, compte, "Clinique B", "validee").await;
        let dans_a = affecter(&pool, compte, a, "medecin").await;
        let dans_b = affecter(&pool, compte, b, "medecin").await;

        let initiale = session::ouvrir(&pool, compte, None).await.unwrap();
        let fin = |id: Uuid| {
            let pool = pool.clone();
            async move {
                sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>("SELECT expire_le FROM session WHERE id = $1")
                    .bind(id)
                    .fetch_one(&pool)
                    .await
                    .unwrap()
            }
        };
        let fin_initiale = fin(initiale.session_id).await;

        let en_a = activer(&pool, compte, initiale.session_id, dans_a).await.unwrap();
        assert_eq!(en_a.affectation_id, Some(dans_a));
        assert_eq!(fin(en_a.session_id).await, fin_initiale, "même date de fin absolue");
        let motif: Option<String> = sqlx::query_scalar("SELECT motif_revocation FROM session WHERE id = $1")
            .bind(initiale.session_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(motif.as_deref(), Some("changement_affectation"));
        assert!(matches!(controle(&pool, &en_a, dans_a).await, ControleExercice::Valide(_)));

        // Changer de structure, depuis la session en exercice.
        let en_b = activer(&pool, compte, en_a.session_id, dans_b).await.unwrap();
        assert_eq!(fin(en_b.session_id).await, fin_initiale);
        assert!(matches!(controle(&pool, &en_a, dans_a).await, ControleExercice::JetonInvalide), "ancienne session fermée");
        assert!(matches!(controle(&pool, &en_b, dans_b).await, ControleExercice::Valide(_)));
        // Le jeton doit porter l'affectation de sa session.
        assert!(matches!(controle(&pool, &en_b, dans_a).await, ControleExercice::JetonInvalide));

        // Session déjà fermée : refus.
        assert!(matches!(activer(&pool, compte, en_a.session_id, dans_a).await, Err(ErreurActivation::SessionFermee)));
    }

    #[sqlx::test]
    #[ignore]
    async fn activation_refusee(pool: PgPool) {
        let compte = professionnel(&pool, "+22670170003").await;
        let autre = professionnel(&pool, "+22670170004").await;
        let ouverte = structure(&pool, compte, "Clinique ouverte", "validee").await;
        let suspendue = structure(&pool, compte, "Clinique suspendue", "suspendue").await;
        let a_l_autre = affecter(&pool, autre, ouverte, "medecin").await;
        let dans_suspendue = affecter(&pool, compte, suspendue, "medecin").await;
        let desactivee = affecter(&pool, compte, ouverte, "medecin").await;
        desactiver(&pool, desactivee, compte).await;
        let session = session::ouvrir(&pool, compte, None).await.unwrap();

        for (affectation, attendu) in [
            (a_l_autre, "inconnue"),
            (Uuid::nil(), "inconnue"),
            (dans_suspendue, "non_exercable"),
            (desactivee, "non_exercable"),
        ] {
            let resultat = activer(&pool, compte, session.session_id, affectation).await;
            let obtenu = match resultat {
                Err(ErreurActivation::AffectationInconnue) => "inconnue",
                Err(ErreurActivation::NonExercable) => "non_exercable",
                _ => "autre",
            };
            assert_eq!(obtenu, attendu);
        }
        // La session n'a pas été fermée par ces refus.
        let ouverte_encore: bool = sqlx::query_scalar("SELECT revoquee_le IS NULL FROM session WHERE id = $1")
            .bind(session.session_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(ouverte_encore);
    }

    /// Un jeton en exercice dont la session n'a pas d'affectation (session
    /// ouverte à la connexion, jamais activée) : refus propre, jamais une
    /// erreur de lecture d'un NULL.
    #[sqlx::test]
    #[ignore]
    async fn session_sans_affectation_jeton_invalide(pool: PgPool) {
        let compte = professionnel(&pool, "+22670170006").await;
        let clinique = structure(&pool, compte, "Clinique", "validee").await;
        let medecin = affecter(&pool, compte, clinique, "medecin").await;
        let sans_affectation = session::ouvrir(&pool, compte, None).await.unwrap();
        assert!(matches!(controle(&pool, &sans_affectation, medecin).await, ControleExercice::JetonInvalide));
    }

    /// D4 : droit retiré (403) ou jeton révoqué (401), à chaque requête ;
    /// licence valable jusqu'à sa date de fin comprise (date du Burkina).
    #[sqlx::test]
    #[ignore]
    async fn controle_exercer(pool: PgPool) {
        let compte = professionnel(&pool, "+22670170005").await;
        let clinique = structure(&pool, compte, "Clinique", "validee").await;
        let medecin = affecter(&pool, compte, clinique, "medecin").await;
        let initiale = session::ouvrir(&pool, compte, None).await.unwrap();
        let session = activer(&pool, compte, initiale.session_id, medecin).await.unwrap();

        licence(&pool, clinique, 0).await;
        let ControleExercice::Valide(exercice) = controle(&pool, &session, medecin).await else {
            panic!("licence valable jusqu'à aujourd'hui compris");
        };
        assert_eq!((exercice.structure_id, exercice.role.as_str(), exercice.type_structure.as_str()), (clinique, "medecin", "clinique"));

        licence(&pool, clinique, -1).await;
        assert!(matches!(controle(&pool, &session, medecin).await, ControleExercice::DroitRetire), "licence expirée");
        licence(&pool, clinique, 30).await;

        sqlx::query("UPDATE structure SET statut = 'suspendue' WHERE id = $1").bind(clinique).execute(&pool).await.unwrap();
        assert!(matches!(controle(&pool, &session, medecin).await, ControleExercice::DroitRetire), "établissement suspendu");
        sqlx::query("UPDATE structure SET statut = 'validee' WHERE id = $1").bind(clinique).execute(&pool).await.unwrap();

        // Email plus vérifié (R0) : jeton invalide.
        sqlx::query("UPDATE utilisateur SET email_verifie_le = NULL FROM compte c WHERE c.id = $1 AND utilisateur.id = c.utilisateur_id")
            .bind(compte)
            .execute(&pool)
            .await
            .unwrap();
        assert!(matches!(controle(&pool, &session, medecin).await, ControleExercice::JetonInvalide), "email");
        sqlx::query("UPDATE utilisateur SET email_verifie_le = now() FROM compte c WHERE c.id = $1 AND utilisateur.id = c.utilisateur_id")
            .bind(compte)
            .execute(&pool)
            .await
            .unwrap();

        // Version augmentée : jeton invalide.
        let ancienne_version = SessionActive { version_jeton: session.version_jeton - 1, ..session };
        assert!(matches!(controle(&pool, &ancienne_version, medecin).await, ControleExercice::JetonInvalide));

        // Affectation désactivée : droit retiré.
        desactiver(&pool, medecin, compte).await;
        assert!(matches!(controle(&pool, &ancienne_version, medecin).await, ControleExercice::JetonInvalide));
        let actuelle = SessionActive { version_jeton: ancienne_version.version_jeton + 1, ..ancienne_version };
        assert!(matches!(controle(&pool, &actuelle, medecin).await, ControleExercice::DroitRetire), "affectation désactivée");
    }
}
