//! Sessions et jetons de renouvellement (option B, décidée le 2026-10-12 ;
//! migration 0015). Sources : RFC 9700 §2.2.2 et §4.14.2 (rotation du jeton
//! de renouvellement à chaque usage, détection du rejeu, expiration après
//! inactivité) ; NIST SP 800-63B rév. 4 §2.1.3 et §2.2.3 (durées).
//!
//! Ce module gère les sessions et les jetons de renouvellement ; il n'émet
//! pas le jeton d'accès. Celui-ci (avec le `sid` de la session) est émis par
//! l'appelant, à partir de `SessionActive` : le passage des jetons d'accès à
//! 15 min ne doit arriver qu'avec les routes de session (décision S5).
//!
//! **Ordre des verrous (règles permanentes, décision S2)** :
//! - `renouveler` verrouille **le jeton avant la session** (le jeton
//!   présenté, puis son successeur, puis la session) ;
//! - toute opération qui touche à la fois le compte et ses sessions
//!   (`fermer_toutes`, réinitialisation du mot de passe, déconnexion
//!   partout) verrouille **le compte avant les sessions**.
//!
//! `renouveler` ne verrouille jamais le compte (il le lit seulement), et
//! `fermer_toutes` ne verrouille jamais un jeton : aucun cycle d'attente,
//! donc aucun interblocage.

use sha2::{Digest, Sha256};
use sqlx::types::Uuid;
use sqlx::{PgConnection, PgPool};

/// Tolérance pour une réponse perdue (décision du 2026-10-12, sur le modèle
/// de la « Rotation Overlap Period » d'Auth0 ; la RFC 9700 n'en prévoit
/// pas) : un jeton déjà utilisé peut resservir une seule fois pendant ce
/// délai, si son successeur n'a jamais servi.
pub const TOLERANCE_REUTILISATION_SECS: f64 = 30.0;

/// Type du compte, qui fixe les durées (décisions du porteur, CLAUDE.md
/// section 14).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeCompte {
    Patient,
    Professionnel,
    Administrateur,
}

impl TypeCompte {
    fn depuis(valeur: &str) -> Result<Self, ErreurSession> {
        match valeur {
            "patient" => Ok(TypeCompte::Patient),
            "professionnel" => Ok(TypeCompte::Professionnel),
            "administrateur_laaficare" => Ok(TypeCompte::Administrateur),
            autre => Err(ErreurSession::Interne(format!("type de compte inconnu : {autre}"))),
        }
    }

    /// Durée absolue d'une session : au-delà, le mot de passe est
    /// redemandé. Patient : 30 jours (NIST AAL1, « no more than 30 days ») ;
    /// administrateur : 24 h (NIST AAL2) ; professionnel : 12 h, une
    /// journée de travail (choix du porteur).
    pub fn duree_absolue_secs(self) -> i64 {
        match self {
            TypeCompte::Patient => 30 * 24 * 3600,
            TypeCompte::Professionnel => 12 * 3600,
            TypeCompte::Administrateur => 24 * 3600,
        }
    }

    /// Inactivité : un jeton de renouvellement non utilisé pendant cette
    /// durée expire (RFC 9700 §4.14.2, « SHOULD expire if the client has
    /// been inactive for some time »). Patient : 30 jours ; professionnel et
    /// administrateur : 1 h (NIST AAL2, « no more than 1 hour »).
    pub fn inactivite_secs(self) -> i64 {
        match self {
            TypeCompte::Patient => 30 * 24 * 3600,
            TypeCompte::Professionnel | TypeCompte::Administrateur => 3600,
        }
    }
}

/// Motifs de révocation d'une session (CHECK de la migration 0015).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotifRevocation {
    Deconnexion,
    DeconnexionPartout,
    ReinitialisationMotDePasse,
    RejeuDetecte,
    ChangementAffectation,
    DesactivationCompte,
}

impl MotifRevocation {
    pub fn as_str(self) -> &'static str {
        match self {
            MotifRevocation::Deconnexion => "deconnexion",
            MotifRevocation::DeconnexionPartout => "deconnexion_partout",
            MotifRevocation::ReinitialisationMotDePasse => "reinitialisation_mot_de_passe",
            MotifRevocation::RejeuDetecte => "rejeu_detecte",
            MotifRevocation::ChangementAffectation => "changement_affectation",
            MotifRevocation::DesactivationCompte => "desactivation_compte",
        }
    }
}

/// Session ouverte ou renouvelée : ce qu'il faut pour émettre le jeton
/// d'accès (`sid` = `session_id`), et le nouveau jeton de renouvellement à
/// remettre au client. Ce jeton n'existe en clair qu'ici : la base n'en
/// garde que l'empreinte.
#[derive(Debug)]
pub struct SessionActive {
    pub session_id: Uuid,
    pub compte_id: Uuid,
    pub affectation_id: Option<Uuid>,
    pub type_compte: TypeCompte,
    pub version_jeton: i32,
    pub jeton_renouvellement: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ErreurSession {
    /// Jeton inconnu, mal formé ou expiré, session révoquée ou expirée,
    /// compte désactivé : 401 `jeton_invalide`, sans distinction.
    #[error("jeton de renouvellement invalide")]
    JetonInvalide,
    /// Rejeu détecté (RFC 9700 §4.14.2) : la session a été révoquée, et
    /// cette révocation est déjà enregistrée (décision S3). Même réponse
    /// au client que `JetonInvalide`.
    #[error("rejeu d'un jeton de renouvellement détecté, session révoquée")]
    RejeuDetecte,
    #[error("erreur interne : {0}")]
    Interne(String),
}

impl From<sqlx::Error> for ErreurSession {
    fn from(erreur: sqlx::Error) -> Self {
        ErreurSession::Interne(erreur.to_string())
    }
}

// ---------------------------------------------------------------------
// Jeton de renouvellement : 32 octets aléatoires, en hexadécimal (S1)
// ---------------------------------------------------------------------

/// 32 octets (256 bits) tirés par `rand::random`, qui utilise `ThreadRng`,
/// générateur cryptographique (doc rand 0.10.3, déjà suivie dans `otp.rs`).
/// Hexadécimal : 64 caractères, sans nouvelle dépendance (décision S1).
fn nouveau_jeton() -> ([u8; 32], String) {
    let octets: [u8; 32] = rand::random();
    let texte = octets.iter().map(|octet| format!("{octet:02x}")).collect();
    (octets, texte)
}

/// Un texte qui n'est pas exactement 64 caractères hexadécimaux ne peut pas
/// être un jeton émis par le serveur.
fn decoder(texte: &str) -> Option<[u8; 32]> {
    if texte.len() != 64 || !texte.is_ascii() {
        return None;
    }
    let mut octets = [0u8; 32];
    for (i, octet) in octets.iter_mut().enumerate() {
        *octet = u8::from_str_radix(&texte[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(octets)
}

/// Seule l'empreinte SHA-256 est enregistrée. Au-delà de 112 bits
/// d'entropie, NIST SP 800-63B rév. 4 §3.1.2 admet un hachage simple ; la
/// recherche se fait par l'empreinte (index unique), donc sans comparaison
/// révélatrice par le temps.
fn empreinte(octets: &[u8; 32]) -> Vec<u8> {
    Sha256::digest(octets).to_vec()
}

/// Crée un jeton de renouvellement pour une session. Son expiration est
/// l'inactivité, **bornée par la fin de la session** avec `LEAST` : la base
/// ne peut pas le garantir (un CHECK ne lit pas une autre table), d'où
/// cette borne dans la requête elle-même (testée).
async fn inserer_jeton(
    conn: &mut PgConnection,
    session_id: Uuid,
    type_compte: TypeCompte,
) -> Result<(Uuid, String), sqlx::Error> {
    let (octets, texte) = nouveau_jeton();
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO jeton_renouvellement (session_id, empreinte, expire_le) \
         SELECT s.id, $2, LEAST(now() + make_interval(secs => $3), s.expire_le) \
         FROM session s WHERE s.id = $1 RETURNING id",
    )
    .bind(session_id)
    .bind(empreinte(&octets))
    .bind(type_compte.inactivite_secs() as f64)
    .fetch_one(conn)
    .await?;
    Ok((id, texte))
}

/// Purge (décision Q-b) : sessions terminées (expirées ou révoquées) depuis
/// plus de 30 jours, sauf celles révoquées pour `rejeu_detecte`, gardées
/// 6 mois. Leurs jetons partent en cascade.
///
/// Sans risque d'interblocage avec `renouveler` : une session purgeable n'a
/// plus aucun jeton valide (chaque jeton expire au plus tard avec sa
/// session, et au plus 30 jours après son émission), et `renouveler` ne
/// verrouille que des jetons encore valides.
async fn purger(conn: &mut PgConnection) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM session WHERE CASE \
             WHEN motif_revocation = 'rejeu_detecte' THEN revoquee_le < now() - interval '6 months' \
             ELSE LEAST(expire_le, COALESCE(revoquee_le, expire_le)) < now() - interval '30 days' \
         END",
    )
    .execute(conn)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------
// Ouverture, renouvellement, fermeture
// ---------------------------------------------------------------------

/// Ouvre une session après une authentification réussie (inscription,
/// connexion). Purge d'abord les vieilles sessions (Q-b).
pub async fn ouvrir(
    pool: &PgPool,
    compte_id: Uuid,
    affectation_id: Option<Uuid>,
) -> Result<SessionActive, ErreurSession> {
    let mut tx = pool.begin().await?;
    purger(&mut tx).await?;

    let (type_compte, version_jeton): (String, i32) =
        sqlx::query_as("SELECT type_compte, version_jeton FROM compte WHERE id = $1")
            .bind(compte_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| ErreurSession::Interne(format!("compte introuvable : {compte_id}")))?;
    let type_compte = TypeCompte::depuis(&type_compte)?;

    let session_id: Uuid = sqlx::query_scalar(
        "INSERT INTO session (compte_id, affectation_id, expire_le) \
         VALUES ($1, $2, now() + make_interval(secs => $3)) RETURNING id",
    )
    .bind(compte_id)
    .bind(affectation_id)
    .bind(type_compte.duree_absolue_secs() as f64)
    .fetch_one(&mut *tx)
    .await?;

    let (_, jeton_renouvellement) = inserer_jeton(&mut tx, session_id, type_compte).await?;
    tx.commit().await?;

    Ok(SessionActive { session_id, compte_id, affectation_id, type_compte, version_jeton, jeton_renouvellement })
}

/// Échange un jeton de renouvellement contre un nouveau (rotation, RFC 9700
/// §4.14.2), avec la tolérance de 30 s pour une réponse perdue.
///
/// | Cas | Résultat |
/// |---|---|
/// | jeton inconnu ou expiré ; session révoquée ou expirée ; compte désactivé | `JetonInvalide` |
/// | jeton annulé par la tolérance qui revient | rejeu |
/// | jeton jamais utilisé | rotation |
/// | jeton déjà utilisé, jamais réutilisé, il y a 30 s au plus, successeur jamais utilisé | tolérance |
/// | jeton déjà utilisé, tout autre cas | rejeu |
///
/// Un rejeu révoque la session (`rejeu_detecte`) et **cette révocation est
/// validée en base avant de renvoyer l'erreur** (décision S3) : elle ne
/// doit jamais être annulée avec la transaction.
pub async fn renouveler(pool: &PgPool, jeton: &str) -> Result<SessionActive, ErreurSession> {
    let Some(octets) = decoder(jeton) else {
        return Err(ErreurSession::JetonInvalide);
    };
    let mut tx = pool.begin().await?;

    // 1. Le jeton présenté, verrouillé, seulement s'il est encore valide :
    //    un jeton expiré n'est jamais verrouillé (voir `purger`).
    type LigneJeton = (Uuid, Uuid, bool, Option<Uuid>, bool, bool, bool);
    let jeton_present: Option<LigneJeton> = sqlx::query_as(
        "SELECT id, session_id, utilise_le IS NOT NULL, remplace_par_id, reutilise_le IS NOT NULL, \
                revoque_le IS NOT NULL, \
                COALESCE(utilise_le >= now() - make_interval(secs => $2), false) \
         FROM jeton_renouvellement WHERE empreinte = $1 AND expire_le > now() FOR UPDATE",
    )
    .bind(empreinte(&octets))
    .bind(TOLERANCE_REUTILISATION_SECS)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((jeton_id, session_id, utilise, successeur_id, reutilise, annule, dans_tolerance)) = jeton_present
    else {
        return Err(ErreurSession::JetonInvalide);
    };

    // 2. Son successeur, verrouillé avant la session (ordre « jeton avant
    //    session ») : a-t-il déjà servi ?
    let successeur_a_servi = match successeur_id {
        Some(successeur) => sqlx::query_scalar::<_, bool>(
            "SELECT utilise_le IS NOT NULL FROM jeton_renouvellement WHERE id = $1 FOR UPDATE",
        )
        .bind(successeur)
        .fetch_one(&mut *tx)
        .await?,
        None => false,
    };

    // 3. La session, puis le compte en simple lecture (jamais verrouillé ici).
    let (compte_id, affectation_id, session_active, type_compte, statut, version_jeton): (
        Uuid,
        Option<Uuid>,
        bool,
        String,
        String,
        i32,
    ) = sqlx::query_as(
        "SELECT s.compte_id, s.affectation_id, s.revoquee_le IS NULL AND s.expire_le > now(), \
                c.type_compte, c.statut, c.version_jeton \
         FROM session s JOIN compte c ON c.id = s.compte_id WHERE s.id = $1 FOR UPDATE OF s",
    )
    .bind(session_id)
    .fetch_one(&mut *tx)
    .await?;
    if !session_active || statut != "actif" {
        return Err(ErreurSession::JetonInvalide);
    }
    let type_compte = TypeCompte::depuis(&type_compte)?;

    let tolerance = utilise && !reutilise && dans_tolerance && !successeur_a_servi;
    if annule || (utilise && !tolerance) {
        sqlx::query(
            "UPDATE session SET revoquee_le = now(), motif_revocation = $2 WHERE id = $1 AND revoquee_le IS NULL",
        )
        .bind(session_id)
        .bind(MotifRevocation::RejeuDetecte.as_str())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        return Err(ErreurSession::RejeuDetecte);
    }

    let (nouveau_id, jeton_renouvellement) = inserer_jeton(&mut tx, session_id, type_compte).await?;
    if tolerance {
        // Le successeur jamais reçu est annulé ; le jeton réutilisé pointe
        // vers le nouveau, pour que la chaîne reste traçable.
        sqlx::query("UPDATE jeton_renouvellement SET revoque_le = now() WHERE id = $1")
            .bind(successeur_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE jeton_renouvellement SET reutilise_le = now(), remplace_par_id = $2 WHERE id = $1")
            .bind(jeton_id)
            .bind(nouveau_id)
            .execute(&mut *tx)
            .await?;
    } else {
        sqlx::query("UPDATE jeton_renouvellement SET utilise_le = now(), remplace_par_id = $2 WHERE id = $1")
            .bind(jeton_id)
            .bind(nouveau_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;

    Ok(SessionActive { session_id, compte_id, affectation_id, type_compte, version_jeton, jeton_renouvellement })
}

/// Déconnexion de l'appareil : révoque la session du jeton. Ne renvoie
/// jamais d'erreur pour un jeton inconnu ou mal formé : se déconnecter
/// n'échoue jamais.
pub async fn fermer(pool: &PgPool, jeton: &str) -> Result<(), ErreurSession> {
    let Some(octets) = decoder(jeton) else {
        return Ok(());
    };
    sqlx::query(
        "UPDATE session SET revoquee_le = now(), motif_revocation = $2 \
         WHERE revoquee_le IS NULL AND id = (SELECT session_id FROM jeton_renouvellement WHERE empreinte = $1)",
    )
    .bind(empreinte(&octets))
    .bind(MotifRevocation::Deconnexion.as_str())
    .execute(pool)
    .await?;
    Ok(())
}

/// Révoque toutes les sessions d'un compte, dans la transaction de
/// l'appelant. Verrouille **le compte avant les sessions** (décision S2).
/// `augmenter_version` : la déconnexion partout augmente `version_jeton` ;
/// la réinitialisation du mot de passe le fait déjà elle-même.
pub async fn fermer_toutes(
    conn: &mut PgConnection,
    compte_id: Uuid,
    motif: MotifRevocation,
    augmenter_version: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT id FROM compte WHERE id = $1 FOR UPDATE")
        .bind(compte_id)
        .fetch_one(&mut *conn)
        .await?;
    sqlx::query(
        "UPDATE session SET revoquee_le = now(), motif_revocation = $2 WHERE compte_id = $1 AND revoquee_le IS NULL",
    )
    .bind(compte_id)
    .bind(motif.as_str())
    .execute(&mut *conn)
    .await?;
    if augmenter_version {
        sqlx::query("UPDATE compte SET version_jeton = version_jeton + 1 WHERE id = $1")
            .bind(compte_id)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jeton_de_64_caracteres_hexadecimaux() {
        let (octets, texte) = nouveau_jeton();
        assert_eq!(texte.len(), 64);
        assert_eq!(decoder(&texte), Some(octets));
        assert_eq!(decoder(&texte.to_uppercase()), Some(octets));
        assert_eq!(decoder("abc"), None);
        assert_eq!(decoder(&"g".repeat(64)), None);
        assert_eq!(decoder(&"é".repeat(32)), None);
    }

    #[test]
    fn durees_decidees() {
        assert_eq!(TypeCompte::Patient.duree_absolue_secs(), 30 * 24 * 3600);
        assert_eq!(TypeCompte::Patient.inactivite_secs(), 30 * 24 * 3600);
        assert_eq!(TypeCompte::Professionnel.duree_absolue_secs(), 12 * 3600);
        assert_eq!(TypeCompte::Professionnel.inactivite_secs(), 3600);
        assert_eq!(TypeCompte::Administrateur.duree_absolue_secs(), 24 * 3600);
        assert_eq!(TypeCompte::Administrateur.inactivite_secs(), 3600);
    }

    // Base temporaire par test ; données fictives.

    async fn compte(pool: &PgPool, telephone: &str, type_compte: &str) -> Uuid {
        let utilisateur_id: Uuid = sqlx::query_scalar("INSERT INTO utilisateur (telephone) VALUES ($1) RETURNING id")
            .bind(telephone)
            .fetch_one(pool)
            .await
            .unwrap();
        sqlx::query_scalar(
            "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, $2, 'x') RETURNING id",
        )
        .bind(utilisateur_id)
        .bind(type_compte)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn id_du_jeton(pool: &PgPool, jeton: &str) -> Uuid {
        sqlx::query_scalar("SELECT id FROM jeton_renouvellement WHERE empreinte = sha256(decode($1, 'hex'))")
            .bind(jeton)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    async fn motif(pool: &PgPool, session_id: Uuid) -> Option<String> {
        sqlx::query_scalar("SELECT motif_revocation FROM session WHERE id = $1")
            .bind(session_id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    fn est_rejeu(resultat: Result<SessionActive, ErreurSession>) -> bool {
        matches!(resultat, Err(ErreurSession::RejeuDetecte))
    }

    fn est_invalide(resultat: Result<SessionActive, ErreurSession>) -> bool {
        matches!(resultat, Err(ErreurSession::JetonInvalide))
    }

    #[sqlx::test]
    #[ignore]
    async fn ouvrir_selon_le_type_de_compte(pool: PgPool) {
        for (telephone, type_compte, absolue, inactivite) in [
            ("+22670030001", "patient", "30 days", "30 days"),
            ("+22670030002", "professionnel", "12 hours", "1 hour"),
            ("+22670030003", "administrateur_laaficare", "24 hours", "1 hour"),
        ] {
            let compte_id = compte(&pool, telephone, type_compte).await;
            let session = ouvrir(&pool, compte_id, None).await.unwrap();
            assert_eq!(session.jeton_renouvellement.len(), 64);
            let (duree_ok, inactivite_ok, en_clair): (bool, bool, i64) = sqlx::query_as(
                "SELECT s.expire_le - s.creee_le = $2::interval, \
                        j.expire_le - j.cree_le = $3::interval, \
                        (SELECT count(*) FROM jeton_renouvellement WHERE encode(empreinte, 'hex') = $4) \
                 FROM session s JOIN jeton_renouvellement j ON j.session_id = s.id WHERE s.id = $1",
            )
            .bind(session.session_id)
            .bind(absolue)
            .bind(inactivite)
            .bind(&session.jeton_renouvellement)
            .fetch_one(&pool)
            .await
            .unwrap();
            assert!(duree_ok && inactivite_ok, "{type_compte}");
            assert_eq!(en_clair, 0, "le jeton n'est jamais enregistré en clair");
            id_du_jeton(&pool, &session.jeton_renouvellement).await;
        }
    }

    #[sqlx::test]
    #[ignore]
    async fn rotation_normale(pool: PgPool) {
        let compte_id = compte(&pool, "+22670030010", "patient").await;
        let a = ouvrir(&pool, compte_id, None).await.unwrap().jeton_renouvellement;
        let b = renouveler(&pool, &a).await.unwrap().jeton_renouvellement;
        assert_ne!(a, b);
        let id_b = id_du_jeton(&pool, &b).await;
        let (utilise, remplace_par): (bool, Option<Uuid>) = sqlx::query_as(
            "SELECT utilise_le IS NOT NULL, remplace_par_id FROM jeton_renouvellement WHERE id = $1",
        )
        .bind(id_du_jeton(&pool, &a).await)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(utilise);
        assert_eq!(remplace_par, Some(id_b));
        renouveler(&pool, &b).await.unwrap();
    }

    #[sqlx::test]
    #[ignore]
    async fn expiration_bornee_par_la_session(pool: PgPool) {
        let compte_id = compte(&pool, "+22670030011", "patient").await;
        let session = ouvrir(&pool, compte_id, None).await.unwrap();
        // La session finit dans 1 jour, avant les 30 jours d'inactivité.
        sqlx::query("UPDATE session SET expire_le = now() + interval '1 day' WHERE id = $1")
            .bind(session.session_id)
            .execute(&pool)
            .await
            .unwrap();
        let b = renouveler(&pool, &session.jeton_renouvellement).await.unwrap().jeton_renouvellement;
        let borne: bool = sqlx::query_scalar(
            "SELECT j.expire_le = s.expire_le FROM jeton_renouvellement j JOIN session s ON s.id = j.session_id \
             WHERE j.id = $1",
        )
        .bind(id_du_jeton(&pool, &b).await)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(borne, "le jeton expire au plus tard avec sa session (LEAST)");
    }

    #[sqlx::test]
    #[ignore]
    async fn tolerance_une_fois_successeur_jamais_utilise(pool: PgPool) {
        let compte_id = compte(&pool, "+22670030012", "patient").await;
        let a = ouvrir(&pool, compte_id, None).await.unwrap().jeton_renouvellement;
        let b = renouveler(&pool, &a).await.unwrap().jeton_renouvellement;
        // La réponse « b » est perdue : le client renvoie « a ».
        let c = renouveler(&pool, &a).await.unwrap().jeton_renouvellement;
        let (id_b, id_c) = (id_du_jeton(&pool, &b).await, id_du_jeton(&pool, &c).await);

        let (remplace_par, reutilise): (Option<Uuid>, bool) = sqlx::query_as(
            "SELECT remplace_par_id, reutilise_le IS NOT NULL FROM jeton_renouvellement WHERE id = $1",
        )
        .bind(id_du_jeton(&pool, &a).await)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(remplace_par, Some(id_c), "la chaîne pointe vers le dernier jeton émis");
        assert!(reutilise);
        let b_annule: bool = sqlx::query_scalar("SELECT revoque_le IS NOT NULL FROM jeton_renouvellement WHERE id = $1")
            .bind(id_b)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(b_annule);
        renouveler(&pool, &c).await.unwrap();
    }

    // Couvre aussi la décision S3 : après l'erreur, la révocation est
    // toujours en base.
    #[sqlx::test]
    #[ignore]
    async fn successeur_revoque_qui_revient_est_un_rejeu(pool: PgPool) {
        let compte_id = compte(&pool, "+22670030013", "patient").await;
        let session = ouvrir(&pool, compte_id, None).await.unwrap();
        let a = session.jeton_renouvellement;
        let b = renouveler(&pool, &a).await.unwrap().jeton_renouvellement;
        let c = renouveler(&pool, &a).await.unwrap().jeton_renouvellement;

        assert!(est_rejeu(renouveler(&pool, &b).await));
        assert_eq!(motif(&pool, session.session_id).await.as_deref(), Some("rejeu_detecte"));
        assert!(est_invalide(renouveler(&pool, &c).await), "la session révoquée n'accepte plus rien");
    }

    #[sqlx::test]
    #[ignore]
    async fn rejeu_si_successeur_deja_utilise(pool: PgPool) {
        let compte_id = compte(&pool, "+22670030014", "patient").await;
        let session = ouvrir(&pool, compte_id, None).await.unwrap();
        let a = session.jeton_renouvellement;
        let b = renouveler(&pool, &a).await.unwrap().jeton_renouvellement;
        renouveler(&pool, &b).await.unwrap();
        assert!(est_rejeu(renouveler(&pool, &a).await));
        assert_eq!(motif(&pool, session.session_id).await.as_deref(), Some("rejeu_detecte"));
    }

    #[sqlx::test]
    #[ignore]
    async fn rejeu_apres_30_secondes(pool: PgPool) {
        let compte_id = compte(&pool, "+22670030015", "patient").await;
        let session = ouvrir(&pool, compte_id, None).await.unwrap();
        let a = session.jeton_renouvellement;
        renouveler(&pool, &a).await.unwrap();
        sqlx::query("UPDATE jeton_renouvellement SET utilise_le = utilise_le - interval '31 seconds' WHERE id = $1")
            .bind(id_du_jeton(&pool, &a).await)
            .execute(&pool)
            .await
            .unwrap();
        assert!(est_rejeu(renouveler(&pool, &a).await));
        assert_eq!(motif(&pool, session.session_id).await.as_deref(), Some("rejeu_detecte"));
    }

    #[sqlx::test]
    #[ignore]
    async fn rejeu_si_deuxieme_reutilisation(pool: PgPool) {
        let compte_id = compte(&pool, "+22670030016", "patient").await;
        let session = ouvrir(&pool, compte_id, None).await.unwrap();
        let a = session.jeton_renouvellement;
        renouveler(&pool, &a).await.unwrap();
        renouveler(&pool, &a).await.unwrap();
        assert!(est_rejeu(renouveler(&pool, &a).await));
        assert_eq!(motif(&pool, session.session_id).await.as_deref(), Some("rejeu_detecte"));
    }

    #[sqlx::test]
    #[ignore]
    async fn jeton_ou_session_expire_refuse(pool: PgPool) {
        let compte_id = compte(&pool, "+22670030017", "patient").await;

        let premiere = ouvrir(&pool, compte_id, None).await.unwrap();
        sqlx::query(
            "UPDATE jeton_renouvellement SET cree_le = now() - interval '2 hours', expire_le = now() - interval '1 hour' \
             WHERE session_id = $1",
        )
        .bind(premiere.session_id)
        .execute(&pool)
        .await
        .unwrap();
        assert!(est_invalide(renouveler(&pool, &premiere.jeton_renouvellement).await), "inactivité dépassée");

        let seconde = ouvrir(&pool, compte_id, None).await.unwrap();
        sqlx::query(
            "UPDATE session SET creee_le = now() - interval '2 hours', expire_le = now() - interval '1 hour' WHERE id = $1",
        )
        .bind(seconde.session_id)
        .execute(&pool)
        .await
        .unwrap();
        assert!(est_invalide(renouveler(&pool, &seconde.jeton_renouvellement).await), "session expirée");

        // Jeton mal formé ou inconnu.
        assert!(est_invalide(renouveler(&pool, "pas-un-jeton").await));
        assert!(est_invalide(renouveler(&pool, &"0".repeat(64)).await));
    }

    #[sqlx::test]
    #[ignore]
    async fn renouvellements_simultanes(pool: PgPool) {
        let compte_id = compte(&pool, "+22670030018", "patient").await;
        let session = ouvrir(&pool, compte_id, None).await.unwrap();
        let a = session.jeton_renouvellement;

        let mut taches = tokio::task::JoinSet::new();
        for _ in 0..2 {
            let (pool, a) = (pool.clone(), a.clone());
            taches.spawn(async move { renouveler(&pool, &a).await.map(|s| s.jeton_renouvellement) });
        }
        while let Some(resultat) = taches.join_next().await {
            // Rotation pour l'un, tolérance pour l'autre : aucun
            // interblocage, aucune erreur.
            resultat.unwrap().unwrap();
        }
        let valides: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM jeton_renouvellement \
             WHERE session_id = $1 AND utilise_le IS NULL AND revoque_le IS NULL",
        )
        .bind(session.session_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(valides, 1, "un seul jeton reste utilisable");
    }

    #[sqlx::test]
    #[ignore]
    async fn fermer_et_fermer_toutes(pool: PgPool) {
        let compte_id = compte(&pool, "+22670030019", "patient").await;

        let une = ouvrir(&pool, compte_id, None).await.unwrap();
        fermer(&pool, &une.jeton_renouvellement).await.unwrap();
        assert_eq!(motif(&pool, une.session_id).await.as_deref(), Some("deconnexion"));
        assert!(est_invalide(renouveler(&pool, &une.jeton_renouvellement).await));
        fermer(&pool, "pas-un-jeton").await.unwrap();
        fermer(&pool, &"0".repeat(64)).await.unwrap();

        let deux = ouvrir(&pool, compte_id, None).await.unwrap();
        let trois = ouvrir(&pool, compte_id, None).await.unwrap();
        let mut tx = pool.begin().await.unwrap();
        fermer_toutes(&mut tx, compte_id, MotifRevocation::DeconnexionPartout, true).await.unwrap();
        tx.commit().await.unwrap();
        for session in [&deux, &trois] {
            assert_eq!(motif(&pool, session.session_id).await.as_deref(), Some("deconnexion_partout"));
        }
        assert_eq!(motif(&pool, une.session_id).await.as_deref(), Some("deconnexion"), "motif d'origine gardé");
        let version: i32 = sqlx::query_scalar("SELECT version_jeton FROM compte WHERE id = $1")
            .bind(compte_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(version, 1);
    }

    #[sqlx::test]
    #[ignore]
    async fn reinitialisation_et_deconnexion_partout_simultanees(pool: PgPool) {
        let compte_id = compte(&pool, "+22670030020", "patient").await;
        for _ in 0..10 {
            ouvrir(&pool, compte_id, None).await.unwrap();

            // Réinitialisation : compte d'abord (comme `auth_patient`),
            // puis les sessions ; pause pour forcer le chevauchement.
            let p1 = pool.clone();
            let reinitialisation = tokio::spawn(async move {
                let mut tx = p1.begin().await?;
                sqlx::query("UPDATE compte SET version_jeton = version_jeton + 1 WHERE id = $1")
                    .bind(compte_id)
                    .execute(&mut *tx)
                    .await?;
                sqlx::query("SELECT pg_sleep(0.05)").execute(&mut *tx).await?;
                fermer_toutes(&mut tx, compte_id, MotifRevocation::ReinitialisationMotDePasse, false).await?;
                tx.commit().await
            });
            let p2 = pool.clone();
            let partout = tokio::spawn(async move {
                let mut tx = p2.begin().await?;
                fermer_toutes(&mut tx, compte_id, MotifRevocation::DeconnexionPartout, true).await?;
                tx.commit().await
            });
            // Aucun interblocage (PostgreSQL renverrait l'erreur 40P01).
            reinitialisation.await.unwrap().unwrap();
            partout.await.unwrap().unwrap();
        }
        let ouvertes: i64 = sqlx::query_scalar("SELECT count(*) FROM session WHERE compte_id = $1 AND revoquee_le IS NULL")
            .bind(compte_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(ouvertes, 0);
    }

    #[sqlx::test]
    #[ignore]
    async fn purge_30_jours_et_6_mois_pour_rejeu(pool: PgPool) {
        let compte_id = compte(&pool, "+22670030021", "patient").await;
        let mut sessions = Vec::new();
        // (fin, motif) ; la date de création est mise 8 mois avant.
        for (revoquee, expire, motif_revocation) in [
            (Some("31 days"), "1 day", Some("deconnexion")),    // effacée
            (None, "31 days", None),                            // expirée, effacée
            (Some("31 days"), "1 day", Some("rejeu_detecte")),  // gardée (moins de 6 mois)
            (Some("7 months"), "1 day", Some("rejeu_detecte")), // effacée
            (Some("10 days"), "1 day", Some("deconnexion")),    // gardée
        ] {
            let session = ouvrir(&pool, compte_id, None).await.unwrap();
            sqlx::query(
                "UPDATE session SET creee_le = now() - interval '8 months', \
                     expire_le = now() - $2::interval, \
                     revoquee_le = now() - $3::interval, motif_revocation = $4 \
                 WHERE id = $1",
            )
            .bind(session.session_id)
            .bind(expire)
            .bind(revoquee)
            .bind(motif_revocation)
            .execute(&pool)
            .await
            .unwrap();
            sessions.push(session.session_id);
        }
        ouvrir(&pool, compte_id, None).await.unwrap(); // déclenche la purge

        let mut restantes = Vec::new();
        for session_id in &sessions {
            let existe: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM session WHERE id = $1)")
                .bind(session_id)
                .fetch_one(&pool)
                .await
                .unwrap();
            restantes.push(existe);
        }
        assert_eq!(restantes, vec![false, false, true, false, true]);
    }

    #[sqlx::test]
    #[ignore]
    async fn purge_session_avec_jetons_enchaines(pool: PgPool) {
        let compte_id = compte(&pool, "+22670030022", "patient").await;
        let session = ouvrir(&pool, compte_id, None).await.unwrap();
        let a = session.jeton_renouvellement;
        let b = renouveler(&pool, &a).await.unwrap().jeton_renouvellement;
        let c = renouveler(&pool, &a).await.unwrap().jeton_renouvellement; // tolérance : A → C, B annulé
        renouveler(&pool, &c).await.unwrap(); // C → D
        let _ = b;
        let jetons: i64 = sqlx::query_scalar("SELECT count(*) FROM jeton_renouvellement WHERE session_id = $1")
            .bind(session.session_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(jetons, 4);

        sqlx::query(
            "UPDATE session SET creee_le = now() - interval '3 months', expire_le = now() - interval '2 months', \
                 revoquee_le = now() - interval '2 months', motif_revocation = 'deconnexion' WHERE id = $1",
        )
        .bind(session.session_id)
        .execute(&pool)
        .await
        .unwrap();
        // Purge : la suppression en cascade des jetons liés entre eux par
        // remplace_par_id ne doit pas être bloquée (NO ACTION).
        ouvrir(&pool, compte_id, None).await.unwrap();

        let restants: i64 = sqlx::query_scalar("SELECT count(*) FROM jeton_renouvellement WHERE session_id = $1")
            .bind(session.session_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(restants, 0);
    }
}
