//! Décisions de l'équipe LaafiCare sur les demandes de structure (étape 3.8
//! réduite, décisions V1 à V12 et A, B du porteur, 2026-10-21) : liste,
//! détail, validation (licence) et refus (motif). La suspension et la
//! réactivation viendront après le pilote.
//!
//! Les appelants sont les routes de l'équipe, derrière l'extracteur
//! `AdministrateurAuthentifie` (session, email vérifié, TOTP actif, A6).
//! Chaque décision est tracée dans `decision_structure` (migration 0012),
//! lignes jamais modifiées ni supprimées.
//!
//! Règles de chaque décision, dans cet ordre :
//! 1. structure inconnue → `StructureInconnue` ;
//! 2. **séparation des tâches** (A) : un administrateur ne décide jamais sur
//!    une structure dont il est responsable (même identité). NIST SP 800-53
//!    rév. 5, contrôle AC-5 « Separation of Duties » : « Separation of duties
//!    addresses the potential for abuse of authorized privileges and helps to
//!    reduce the risk of malevolent activity without collusion » ;
//!    application précise : décision du porteur, 2026-10-21 ;
//! 3. seulement depuis `en_attente` (V5) ;
//! 4. validation seulement : email du responsable vérifié (B), licence
//!    valable (V4), pas de seconde MUNASEB (V7), autorisations jamais
//!    validées ailleurs (V8).
//!
//! Après la validation en base, le responsable est prévenu par un email au
//! texte minimal (V9) ; un échec d'envoi n'annule rien.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::types::Uuid;
use sqlx::{PgConnection, PgPool};

use crate::auth_professionnel::nature_sqlx;
use crate::email::EmailSender;

/// Garde-fou technique sur le motif de refus, en caractères (V6, même
/// valeur que le motif de rejet d'une demande de remboursement).
pub const MOTIF_LONGUEUR_MAX: usize = 1000;

/// Statuts d'une structure (liste fermée de la migration 0012), pour le
/// filtre de la liste.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatutStructure {
    Brouillon,
    EnAttente,
    Validee,
    Refusee,
    Suspendue,
    Abandonnee,
}

impl StatutStructure {
    pub fn as_str(self) -> &'static str {
        match self {
            StatutStructure::Brouillon => "brouillon",
            StatutStructure::EnAttente => "en_attente",
            StatutStructure::Validee => "validee",
            StatutStructure::Refusee => "refusee",
            StatutStructure::Suspendue => "suspendue",
            StatutStructure::Abandonnee => "abandonnee",
        }
    }
}

// ---------------------------------------------------------------------
// Lectures (V2, V3)
// ---------------------------------------------------------------------

/// Une ligne de la liste.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ResumeStructure {
    pub id: Uuid,
    #[serde(rename = "type")]
    pub type_structure: String,
    pub nom: String,
    pub statut: String,
    pub date_creation: DateTime<Utc>,
    pub commune: String,
    pub responsable_nom: Option<String>,
    pub responsable_prenom: Option<String>,
}

macro_rules! select_resume {
    () => {
        "SELECT s.id, s.type_structure, s.nom, s.statut, s.date_creation, s.commune, \
                u.nom AS responsable_nom, u.prenom AS responsable_prenom \
         FROM structure s JOIN compte c ON c.id = s.createur_compte_id \
         JOIN utilisateur u ON u.id = c.utilisateur_id"
    };
}

/// Page de structures, dans l'ordre des identifiants (`uuidv7`, ordonnés
/// dans le temps : les plus anciennes d'abord). Pagination par curseur,
/// comme la liste des remboursements (§12) : `apres` est l'identifiant de
/// la dernière ligne de la page précédente.
pub async fn lister(
    pool: &PgPool,
    statut: Option<StatutStructure>,
    apres: Option<Uuid>,
    limite: i64,
) -> Result<Vec<ResumeStructure>, sqlx::Error> {
    sqlx::query_as::<_, ResumeStructure>(concat!(
        select_resume!(),
        " WHERE ($1::text IS NULL OR s.statut = $1) AND ($2::uuid IS NULL OR s.id > $2) ORDER BY s.id LIMIT $3"
    ))
    .bind(statut.map(StatutStructure::as_str))
    .bind(apres)
    .bind(limite)
    .fetch_all(pool)
    .await
}

/// Une décision passée, avec l'administrateur qui l'a prise.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct LigneDecision {
    pub decision: String,
    pub motif: Option<String>,
    pub licence_expire_le: Option<NaiveDate>,
    pub date_decision: DateTime<Utc>,
    pub administrateur_nom: Option<String>,
    pub administrateur_prenom: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DetailStructure {
    #[serde(flatten)]
    pub resume: ResumeStructure,
    pub telephone: String,
    pub telephone_urgence: String,
    pub email: String,
    pub region: String,
    pub province: String,
    pub parcelle: Option<String>,
    pub lot: Option<String>,
    pub section: Option<String>,
    pub latitude: f64,
    pub longitude: f64,
    pub licence_expire_le: Option<NaiveDate>,
    pub responsable_telephone: String,
    pub responsable_email: Option<String>,
    pub responsable_email_verifie: bool,
    pub decisions: Vec<LigneDecision>,
    /// MUNASEB seulement (V3, M6) : les autres demandes MUNASEB en attente,
    /// et la MUNASEB déjà validée ou suspendue s'il y en a une. Vides pour
    /// les autres types.
    pub autres_demandes_munaseb_en_attente: Vec<Uuid>,
    pub munaseb_deja_validee: Option<Uuid>,
}

/// Détail d'une structure, `None` si elle n'existe pas.
pub async fn detail(pool: &PgPool, structure_id: Uuid) -> Result<Option<DetailStructure>, sqlx::Error> {
    let Some(resume) = sqlx::query_as::<_, ResumeStructure>(concat!(select_resume!(), " WHERE s.id = $1"))
        .bind(structure_id)
        .fetch_optional(pool)
        .await?
    else {
        return Ok(None);
    };

    type Ligne = (
        String,
        String,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        f64,
        f64,
        Option<NaiveDate>,
        String,
        Option<String>,
        bool,
    );
    // Coordonnées NUMERIC(9,6) lues en double précision : pour l'affichage
    // seulement, jamais réécrites.
    let (
        telephone,
        telephone_urgence,
        email,
        region,
        province,
        parcelle,
        lot,
        section,
        latitude,
        longitude,
        licence_expire_le,
        responsable_telephone,
        responsable_email,
        responsable_email_verifie,
    ): Ligne = sqlx::query_as(
        "SELECT s.telephone, s.telephone_urgence, s.email, s.region, s.province, s.parcelle, s.lot, \
                s.section_cadastrale, s.latitude::float8, s.longitude::float8, s.licence_expire_le, \
                u.telephone, u.email, u.email_verifie_le IS NOT NULL \
         FROM structure s JOIN compte c ON c.id = s.createur_compte_id JOIN utilisateur u ON u.id = c.utilisateur_id \
         WHERE s.id = $1",
    )
    .bind(structure_id)
    .fetch_one(pool)
    .await?;

    let decisions: Vec<LigneDecision> = sqlx::query_as(
        "SELECT d.decision, d.motif, d.licence_expire_le, d.date_decision, \
                u.nom AS administrateur_nom, u.prenom AS administrateur_prenom \
         FROM decision_structure d JOIN compte c ON c.id = d.administrateur_compte_id \
         JOIN utilisateur u ON u.id = c.utilisateur_id \
         WHERE d.structure_id = $1 ORDER BY d.date_decision, d.id",
    )
    .bind(structure_id)
    .fetch_all(pool)
    .await?;

    let (autres_demandes_munaseb_en_attente, munaseb_deja_validee) = if resume.type_structure == "munaseb" {
        let autres: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM structure WHERE type_structure = 'munaseb' AND statut = 'en_attente' AND id <> $1 ORDER BY id",
        )
        .bind(structure_id)
        .fetch_all(pool)
        .await?;
        let validee: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM structure WHERE type_structure = 'munaseb' AND statut IN ('validee', 'suspendue') AND id <> $1",
        )
        .bind(structure_id)
        .fetch_optional(pool)
        .await?;
        (autres, validee)
    } else {
        (Vec::new(), None)
    };

    Ok(Some(DetailStructure {
        resume,
        telephone,
        telephone_urgence,
        email,
        region,
        province,
        parcelle,
        lot,
        section,
        latitude,
        longitude,
        licence_expire_le,
        responsable_telephone,
        responsable_email,
        responsable_email_verifie,
        decisions,
        autres_demandes_munaseb_en_attente,
        munaseb_deja_validee,
    }))
}

// ---------------------------------------------------------------------
// Décisions (V4 à V9, A, B)
// ---------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum ErreurDecision {
    #[error("Structure inconnue.")]
    StructureInconnue,
    /// A : séparation des tâches.
    #[error("Vous ne pouvez pas décider d'une demande de structure dont vous êtes responsable.")]
    PropreStructure,
    #[error("Cette demande ne peut pas recevoir de décision dans son état actuel.")]
    TransitionInvalide { statut_actuel: String },
    /// B : la validation exige un email de responsable vérifié.
    #[error("L'email professionnel du responsable n'est pas vérifié : la demande ne peut pas être validée.")]
    EmailResponsableNonVerifie,
    #[error("La date de fin de licence doit être postérieure à aujourd'hui.")]
    LicenceInvalide,
    /// V7 : une seule MUNASEB validée ou suspendue (M6).
    #[error("Une MUNASEB est déjà validée : une seconde ne peut pas l'être.")]
    MunasebDejaValidee,
    /// V8 : un numéro d'autorisation déjà validé pour une autre structure.
    #[error("Une autorisation de cette demande est déjà validée pour une autre structure.")]
    AutorisationDejaValidee,
    #[error("Le motif du refus est obligatoire.")]
    MotifVide,
    #[error("Le motif du refus dépasse {MOTIF_LONGUEUR_MAX} caractères.")]
    MotifTropLong,
    #[error("erreur interne : {0}")]
    Interne(&'static str),
}

fn interne(etape: &'static str, erreur: &sqlx::Error) -> ErreurDecision {
    tracing::error!(etape, nature = %nature_sqlx(erreur), "décision sur une structure : erreur interne");
    ErreurDecision::Interne(etape)
}

/// Ce que la décision doit savoir de la structure, lu sous verrou.
struct StructureVerrouillee {
    statut: String,
    type_structure: String,
    email_responsable: Option<String>,
    email_responsable_verifie: bool,
}

/// Verrouille la structure (`FOR UPDATE OF s` : deux décisions simultanées
/// se suivent, la seconde voit le nouveau statut), puis applique les
/// contrôles 1 à 3 communs aux deux décisions.
async fn verrouiller_et_controler(
    conn: &mut PgConnection,
    administrateur_compte_id: Uuid,
    structure_id: Uuid,
) -> Result<StructureVerrouillee, ErreurDecision> {
    let ligne: Option<(String, String, Option<String>, bool)> = sqlx::query_as(
        "SELECT s.statut, s.type_structure, u.email, u.email_verifie_le IS NOT NULL \
         FROM structure s JOIN compte c ON c.id = s.createur_compte_id JOIN utilisateur u ON u.id = c.utilisateur_id \
         WHERE s.id = $1 FOR UPDATE OF s",
    )
    .bind(structure_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(|e| interne("lecture de la structure", &e))?;
    let Some((statut, type_structure, email_responsable, email_responsable_verifie)) = ligne else {
        return Err(ErreurDecision::StructureInconnue);
    };

    // A : responsable de la structure = une affectation `responsable` active
    // d'un compte de la même identité que l'administrateur (le créateur en a
    // toujours une, posée à la création).
    let propre: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM affectation a JOIN compte pro ON pro.id = a.compte_id \
                        JOIN compte adm ON adm.utilisateur_id = pro.utilisateur_id \
                        WHERE a.structure_id = $1 AND a.role = 'responsable' AND a.statut = 'active' AND adm.id = $2)",
    )
    .bind(structure_id)
    .bind(administrateur_compte_id)
    .fetch_one(&mut *conn)
    .await
    .map_err(|e| interne("contrôle de la séparation des tâches", &e))?;
    if propre {
        return Err(ErreurDecision::PropreStructure);
    }

    if statut != "en_attente" {
        return Err(ErreurDecision::TransitionInvalide { statut_actuel: statut });
    }
    Ok(StructureVerrouillee { statut, type_structure, email_responsable, email_responsable_verifie })
}

/// Email au texte minimal (V9) : ni le résultat, ni le motif, ni le nom de
/// la structure ; le résultat se lit dans l'application. Après la
/// validation en base ; un échec est journalisé sans l'adresse.
fn prevenir_responsable(email_sender: &dyn EmailSender, adresse: &str) {
    let envoi = email_sender.envoyer(
        adresse,
        "LaafiCare : votre demande de structure a été traitée",
        "Bonjour,\n\nVotre demande de structure a été traitée. Ouvrez LaafiCare pour voir le résultat.",
    );
    if envoi.is_err() {
        tracing::error!("décision sur une structure : échec d'envoi de l'email au responsable");
    }
}

/// `POST /api/administrateurs/structures/{id}/valider` (V4 à V9, A, B).
/// `licence_expire_le` absent : licence sans fin ; présent : la licence est
/// valable jusqu'à cette date **comprise** (D9), qui doit être postérieure à
/// aujourd'hui, heure du Burkina Faso.
pub async fn valider(
    pool: &PgPool,
    email_sender: &dyn EmailSender,
    administrateur_compte_id: Uuid,
    structure_id: Uuid,
    licence_expire_le: Option<NaiveDate>,
) -> Result<(), ErreurDecision> {
    let mut tx = pool.begin().await.map_err(|e| interne("transaction", &e))?;
    let structure = verrouiller_et_controler(&mut tx, administrateur_compte_id, structure_id).await?;

    // B : l'email de V9 part toujours vers une adresse vérifiée.
    let adresse = match (&structure.email_responsable, structure.email_responsable_verifie) {
        (Some(adresse), true) => adresse.clone(),
        _ => return Err(ErreurDecision::EmailResponsableNonVerifie),
    };

    // V4 : date du Burkina (doc PostgreSQL 18 §9.9.4, D9), jamais
    // `current_date`.
    if let Some(date) = licence_expire_le {
        let future: bool = sqlx::query_scalar("SELECT $1 > (now() AT TIME ZONE 'Africa/Ouagadougou')::date")
            .bind(date)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| interne("contrôle de la licence", &e))?;
        if !future {
            return Err(ErreurDecision::LicenceInvalide);
        }
    }

    // V7 : contrôle avant l'écriture, pour un message clair ; l'index
    // `structure_une_seule_munaseb_validee` (0012) reste la garantie, et son
    // erreur est traduite plus bas (deux validations simultanées de deux
    // MUNASEB différentes).
    if structure.type_structure == "munaseb" {
        let existe: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM structure WHERE type_structure = 'munaseb' \
                            AND statut IN ('validee', 'suspendue') AND id <> $1)",
        )
        .bind(structure_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| interne("recherche d'une MUNASEB validée", &e))?;
        if existe {
            return Err(ErreurDecision::MunasebDejaValidee);
        }
    }

    sqlx::query(
        "INSERT INTO decision_structure (structure_id, administrateur_compte_id, decision, licence_expire_le) \
         VALUES ($1, $2, 'validee', $3)",
    )
    .bind(structure_id)
    .bind(administrateur_compte_id)
    .bind(licence_expire_le)
    .execute(&mut *tx)
    .await
    .map_err(|e| interne("trace de la décision", &e))?;

    sqlx::query("UPDATE structure SET statut = 'validee', licence_expire_le = $2 WHERE id = $1")
        .bind(structure_id)
        .bind(licence_expire_le)
        .execute(&mut *tx)
        .await
        .map_err(|e| match e.as_database_error().and_then(|d| d.constraint()) {
            Some("structure_une_seule_munaseb_validee") => ErreurDecision::MunasebDejaValidee,
            _ => interne("validation de la structure", &e),
        })?;

    // V8 : registre des autorisations validées, rempli dans la même
    // transaction à partir des pièces actives (0012) ; aucune pour la
    // MUNASEB, qui se soumet sans pièce (U6 b).
    sqlx::query(
        "INSERT INTO autorisation_validee (type_piece, numero_normalise, structure_id, piece_id) \
         SELECT type_piece, numero_normalise, structure_id, id FROM piece_justificative \
         WHERE structure_id = $1 AND active",
    )
    .bind(structure_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| match e.as_database_error().and_then(|d| d.constraint()) {
        Some("autorisation_validee_pkey") => ErreurDecision::AutorisationDejaValidee,
        _ => interne("registre des autorisations", &e),
    })?;

    tx.commit().await.map_err(|e| interne("validation de la transaction", &e))?;
    prevenir_responsable(email_sender, &adresse);
    Ok(())
}

/// `POST /api/administrateurs/structures/{id}/refuser` (V5, V6, V9, A).
/// Possible même si l'email du responsable n'est pas vérifié (B) ; l'email
/// de V9 ne part alors pas.
pub async fn refuser(
    pool: &PgPool,
    email_sender: &dyn EmailSender,
    administrateur_compte_id: Uuid,
    structure_id: Uuid,
    motif: &str,
) -> Result<(), ErreurDecision> {
    let motif = motif.trim();
    if motif.is_empty() {
        return Err(ErreurDecision::MotifVide);
    }
    if motif.chars().count() > MOTIF_LONGUEUR_MAX {
        return Err(ErreurDecision::MotifTropLong);
    }

    let mut tx = pool.begin().await.map_err(|e| interne("transaction", &e))?;
    let structure = verrouiller_et_controler(&mut tx, administrateur_compte_id, structure_id).await?;
    debug_assert_eq!(structure.statut, "en_attente");

    sqlx::query(
        "INSERT INTO decision_structure (structure_id, administrateur_compte_id, decision, motif) \
         VALUES ($1, $2, 'refusee', $3)",
    )
    .bind(structure_id)
    .bind(administrateur_compte_id)
    .bind(motif)
    .execute(&mut *tx)
    .await
    .map_err(|e| interne("trace de la décision", &e))?;
    sqlx::query("UPDATE structure SET statut = 'refusee' WHERE id = $1")
        .bind(structure_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| interne("refus de la structure", &e))?;
    tx.commit().await.map_err(|e| interne("validation de la transaction", &e))?;

    if let (Some(adresse), true) = (&structure.email_responsable, structure.email_responsable_verifie) {
        prevenir_responsable(email_sender, adresse);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::affectation;
    use crate::email::EmailsRecus;
    use crate::structure;

    // Base temporaire par test ; données fictives.

    /// Identité avec un compte du type demandé ; email vérifié si `verifie`.
    async fn compte(pool: &PgPool, telephone: &str, email: &str, type_compte: &str, verifie: bool) -> Uuid {
        let utilisateur_id: Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (nom, prenom, telephone, email) VALUES ('OUEDRAOGO', 'Awa', $1, $2) RETURNING id",
        )
        .bind(telephone)
        .bind(email)
        .fetch_one(pool)
        .await
        .unwrap();
        if verifie {
            sqlx::query("UPDATE utilisateur SET email_verifie_le = now() WHERE id = $1")
                .bind(utilisateur_id)
                .execute(pool)
                .await
                .unwrap();
        }
        sqlx::query_scalar("INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, $2, 'x') RETURNING id")
            .bind(utilisateur_id)
            .bind(type_compte)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /// Structure en attente, avec son responsable (créateur et affectation),
    /// comme après une création et une soumission. Renvoie (structure,
    /// compte professionnel du responsable).
    async fn demande(pool: &PgPool, telephone: &str, email: &str, type_structure: &str) -> (Uuid, Uuid) {
        let pro = compte(pool, telephone, email, "professionnel", true).await;
        let structure_id: Uuid = sqlx::query_scalar(
            "INSERT INTO structure (type_structure, nom, telephone, telephone_urgence, region, email, commune, province, \
                                    latitude, longitude, createur_compte_id, statut) \
             VALUES ($1, 'Structure fictive', '+22670000000', '+22670000000', 'Région fictive', 'structure@example.org', \
                     'Commune fictive', 'Province fictive', 12.371400, -1.519700, $2, 'en_attente') RETURNING id",
        )
        .bind(type_structure)
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

    async fn administrateur(pool: &PgPool, telephone: &str) -> Uuid {
        compte(pool, telephone, &format!("adm{telephone}@example.org").replace('+', ""), "administrateur_laaficare", true).await
    }

    async fn statut(pool: &PgPool, structure_id: Uuid) -> String {
        sqlx::query_scalar("SELECT statut FROM structure WHERE id = $1").bind(structure_id).fetch_one(pool).await.unwrap()
    }

    async fn decisions(pool: &PgPool, structure_id: Uuid) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM decision_structure WHERE structure_id = $1")
            .bind(structure_id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /// Validation sans licence (sans fin), trace, email au responsable ;
    /// son affectation `responsable` devient exerçable (V11).
    #[sqlx::test]
    #[ignore]
    async fn validation_sans_licence_et_affectation_exercable(pool: PgPool) {
        let emails = EmailsRecus::default();
        let adm = administrateur(&pool, "+22670180001").await;
        let (structure_id, pro) = demande(&pool, "+22670180002", "resp.valide@example.org", "munaseb").await;

        valider(&pool, &emails, adm, structure_id, None).await.unwrap();
        assert_eq!(statut(&pool, structure_id).await, "validee");
        assert_eq!(decisions(&pool, structure_id).await, 1);
        let licence: Option<NaiveDate> = sqlx::query_scalar("SELECT licence_expire_le FROM structure WHERE id = $1")
            .bind(structure_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(licence, None, "licence sans fin");

        let envoyes = emails.0.lock().unwrap().clone();
        assert_eq!(envoyes.len(), 1);
        assert_eq!(envoyes[0].0, "resp.valide@example.org");
        assert!(!envoyes[0].1.contains("Structure fictive"), "texte minimal");

        let liste = affectation::lister(&pool, pro).await.unwrap();
        assert!(liste[0].exercable, "affectation exerçable après la validation");
    }

    #[sqlx::test]
    #[ignore]
    async fn validation_avec_licence_et_licence_passee_refusee(pool: PgPool) {
        let emails = EmailsRecus::default();
        let adm = administrateur(&pool, "+22670180003").await;
        let (structure_id, _) = demande(&pool, "+22670180004", "resp.licence@example.org", "munaseb").await;

        let hier = (Utc::now() - chrono::Duration::days(1)).date_naive();
        assert!(matches!(valider(&pool, &emails, adm, structure_id, Some(hier)).await, Err(ErreurDecision::LicenceInvalide)));
        assert_eq!(statut(&pool, structure_id).await, "en_attente", "rien n'est écrit");

        let dans_un_an = (Utc::now() + chrono::Duration::days(365)).date_naive();
        valider(&pool, &emails, adm, structure_id, Some(dans_un_an)).await.unwrap();
        let (sur_structure, sur_decision): (Option<NaiveDate>, Option<NaiveDate>) = sqlx::query_as(
            "SELECT s.licence_expire_le, d.licence_expire_le FROM structure s JOIN decision_structure d ON d.structure_id = s.id \
             WHERE s.id = $1",
        )
        .bind(structure_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!((sur_structure, sur_decision), (Some(dans_un_an), Some(dans_un_an)));
    }

    /// Refus : motif enregistré, visible dans `mes-demandes` du responsable.
    #[sqlx::test]
    #[ignore]
    async fn refus_avec_motif_visible_par_le_responsable(pool: PgPool) {
        let emails = EmailsRecus::default();
        let adm = administrateur(&pool, "+22670180005").await;
        let (structure_id, pro) = demande(&pool, "+22670180006", "resp.refus@example.org", "munaseb").await;

        assert!(matches!(refuser(&pool, &emails, adm, structure_id, "  ").await, Err(ErreurDecision::MotifVide)));
        let trop_long = "x".repeat(MOTIF_LONGUEUR_MAX + 1);
        assert!(matches!(refuser(&pool, &emails, adm, structure_id, &trop_long).await, Err(ErreurDecision::MotifTropLong)));

        refuser(&pool, &emails, adm, structure_id, " Informations incomplètes (fictif) ").await.unwrap();
        assert_eq!(statut(&pool, structure_id).await, "refusee");
        assert_eq!(emails.0.lock().unwrap().len(), 1);

        // Le compte patient du responsable lit son motif (`mes-demandes`).
        let utilisateur_id: Uuid = sqlx::query_scalar("SELECT utilisateur_id FROM compte WHERE id = $1")
            .bind(pro)
            .fetch_one(&pool)
            .await
            .unwrap();
        let patient: Uuid = sqlx::query_scalar(
            "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'patient', 'x') RETURNING id",
        )
        .bind(utilisateur_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let suivi = structure::mes_demandes(&pool, patient).await.unwrap();
        assert_eq!(suivi.demandes[0].motif_refus.as_deref(), Some("Informations incomplètes (fictif)"));
    }

    /// V5 : seulement depuis `en_attente` ; structure inconnue.
    #[sqlx::test]
    #[ignore]
    async fn transitions_refusees(pool: PgPool) {
        let emails = EmailsRecus::default();
        let adm = administrateur(&pool, "+22670180007").await;
        let (structure_id, _) = demande(&pool, "+22670180008", "resp.transition@example.org", "munaseb").await;
        valider(&pool, &emails, adm, structure_id, None).await.unwrap();

        for resultat in [
            valider(&pool, &emails, adm, structure_id, None).await,
            refuser(&pool, &emails, adm, structure_id, "Motif fictif").await,
        ] {
            assert!(matches!(resultat, Err(ErreurDecision::TransitionInvalide { ref statut_actuel }) if statut_actuel == "validee"));
        }
        assert!(matches!(valider(&pool, &emails, adm, Uuid::nil(), None).await, Err(ErreurDecision::StructureInconnue)));
        assert_eq!(decisions(&pool, structure_id).await, 1);
    }

    /// V7 : seconde MUNASEB refusée avec un code clair ; une autre que
    /// MUNASEB reste validable.
    #[sqlx::test]
    #[ignore]
    async fn seconde_munaseb_refusee(pool: PgPool) {
        let emails = EmailsRecus::default();
        let adm = administrateur(&pool, "+22670180009").await;
        let (premiere, _) = demande(&pool, "+22670180010", "resp.premiere@example.org", "munaseb").await;
        let (seconde, _) = demande(&pool, "+22670180011", "resp.seconde@example.org", "munaseb").await;

        let detail_seconde = detail(&pool, seconde).await.unwrap().unwrap();
        assert_eq!(detail_seconde.autres_demandes_munaseb_en_attente, [premiere], "avertissement M6");

        valider(&pool, &emails, adm, premiere, None).await.unwrap();
        assert!(matches!(valider(&pool, &emails, adm, seconde, None).await, Err(ErreurDecision::MunasebDejaValidee)));
        assert_eq!(statut(&pool, seconde).await, "en_attente");
        assert_eq!(detail(&pool, seconde).await.unwrap().unwrap().munaseb_deja_validee, Some(premiere));
        // Le refus de la seconde reste possible.
        refuser(&pool, &emails, adm, seconde, "Une MUNASEB existe déjà (fictif)").await.unwrap();
    }

    /// A : un administrateur ne décide jamais sur sa propre structure.
    #[sqlx::test]
    #[ignore]
    async fn pas_de_decision_sur_sa_propre_structure(pool: PgPool) {
        let emails = EmailsRecus::default();
        let (structure_id, pro) = demande(&pool, "+22670180012", "resp.admin@example.org", "munaseb").await;
        // Compte administrateur de la même identité que le responsable.
        let utilisateur_id: Uuid = sqlx::query_scalar("SELECT utilisateur_id FROM compte WHERE id = $1")
            .bind(pro)
            .fetch_one(&pool)
            .await
            .unwrap();
        let adm: Uuid = sqlx::query_scalar(
            "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'administrateur_laaficare', 'x') \
             RETURNING id",
        )
        .bind(utilisateur_id)
        .fetch_one(&pool)
        .await
        .unwrap();

        assert!(matches!(valider(&pool, &emails, adm, structure_id, None).await, Err(ErreurDecision::PropreStructure)));
        assert!(matches!(refuser(&pool, &emails, adm, structure_id, "Motif").await, Err(ErreurDecision::PropreStructure)));
        assert_eq!((statut(&pool, structure_id).await, decisions(&pool, structure_id).await), ("en_attente".into(), 0));

        // Un autre administrateur peut décider.
        let autre = administrateur(&pool, "+22670180013").await;
        valider(&pool, &emails, autre, structure_id, None).await.unwrap();
    }

    /// B : validation refusée si l'email du responsable n'est plus vérifié ;
    /// le refus reste possible, sans email.
    #[sqlx::test]
    #[ignore]
    async fn email_du_responsable_non_verifie(pool: PgPool) {
        let emails = EmailsRecus::default();
        let adm = administrateur(&pool, "+22670180014").await;
        let (structure_id, pro) = demande(&pool, "+22670180015", "resp.nonverifie@example.org", "munaseb").await;
        sqlx::query("UPDATE utilisateur SET email_verifie_le = NULL FROM compte c WHERE c.id = $1 AND utilisateur.id = c.utilisateur_id")
            .bind(pro)
            .execute(&pool)
            .await
            .unwrap();

        assert!(matches!(
            valider(&pool, &emails, adm, structure_id, None).await,
            Err(ErreurDecision::EmailResponsableNonVerifie)
        ));
        assert_eq!(statut(&pool, structure_id).await, "en_attente");
        refuser(&pool, &emails, adm, structure_id, "Email non vérifié (fictif)").await.unwrap();
        assert_eq!(statut(&pool, structure_id).await, "refusee");
        assert!(emails.0.lock().unwrap().is_empty(), "aucun email vers une adresse non vérifiée");
    }

    /// V2 : filtre et curseur.
    #[sqlx::test]
    #[ignore]
    async fn liste_filtre_et_curseur(pool: PgPool) {
        let mut ids = Vec::new();
        for i in 0..3 {
            let (id, _) = demande(&pool, &format!("+2267018002{i}"), &format!("resp.liste{i}@example.org"), "clinique").await;
            ids.push(id);
        }
        let adm = administrateur(&pool, "+22670180030").await;
        valider(&pool, &EmailsRecus::default(), adm, ids[0], None).await.unwrap();

        let en_attente = lister(&pool, Some(StatutStructure::EnAttente), None, 50).await.unwrap();
        assert_eq!(en_attente.iter().map(|s| s.id).collect::<Vec<_>>(), ids[1..]);
        let page1 = lister(&pool, None, None, 2).await.unwrap();
        let page2 = lister(&pool, None, Some(page1[1].id), 2).await.unwrap();
        assert_eq!(page1.iter().chain(&page2).map(|s| s.id).collect::<Vec<_>>(), ids);
        assert_eq!(page1[0].responsable_nom.as_deref(), Some("OUEDRAOGO"));
    }

    /// V9 : un échec d'envoi n'annule pas la validation.
    #[sqlx::test]
    #[ignore]
    async fn echec_d_envoi_la_validation_reste(pool: PgPool) {
        struct EnEchec;
        impl EmailSender for EnEchec {
            fn envoyer(&self, _: &str, _: &str, _: &str) -> Result<(), crate::email::ErreurEmail> {
                Err(crate::email::ErreurEmail::simulee())
            }
            fn simule(&self) -> bool {
                true
            }
        }
        let adm = administrateur(&pool, "+22670180031").await;
        let (structure_id, _) = demande(&pool, "+22670180032", "resp.echec@example.org", "munaseb").await;
        valider(&pool, &EnEchec, adm, structure_id, None).await.unwrap();
        assert_eq!(statut(&pool, structure_id).await, "validee");
    }
}
