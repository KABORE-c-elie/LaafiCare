//! Prise en charge d'un patient par un établissement, bris de glace et son
//! contrôle (module médecin, étape 3 ; décisions MD1, MD2, MD3 du porteur du
//! 2026-11-17 ; migration 0037).
//!
//! **Modèle d'accès (MD1)** : le dossier ne s'ouvre que sous une prise en
//! charge **de l'établissement de l'affectation**, encore en cours :
//! - ouverte avec le **code de soin** du patient (moyen préféré, preuve
//!   qu'il est présent et d'accord), ou après **vérification de sa CNIB**
//!   par le professionnel (attestation tracée, jamais le numéro, comme Y2) ;
//! - ou par un **bris de glace** (MD2) : médecin ou infirmier seulement,
//!   motif obligatoire (catégorie et texte), contrôlé ensuite par le
//!   responsable de l'établissement, patient averti par une notification.
//!
//! **Durée (MD3)** : 24 heures, prolongeable de 24 heures pendant une
//! hospitalisation, chaque prolongation tracée. Une prise en charge ne se
//! modifie ni ne s'efface (migration 0037).
//!
//! Cadre légal : loi n° 001-2021/AN, article 13 (le traitement pour les soins
//! par un professionnel tenu au secret est permis sans consentement) et
//! article 24 (empêcher l'accès de tiers non autorisés) ; articles lus dans
//! `docs/points-ouverts.md`, le texte n'ayant pas pu être relu le 2026-11-17.
//!
//! **Ordre des verrous** : l'affectation (limites d'essais), puis le patient
//! (limite des codes faux par patient), puis le code de soin.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::types::Uuid;
use sqlx::{PgPool, Postgres, Transaction};

use crate::acces_patient::{
    self, Acteur, ECHECS_MAX, ErreurAcces, FENETRE_ECHECS_MINUTES, NatureAcces, NatureEchec,
};
use crate::affectation::Exercice;
use crate::code_soin;
use crate::notification::{self, TypeNotification};

/// Durée d'une prise en charge et d'une prolongation (MD3).
pub const DUREE_HEURES: i32 = 24;
/// Motif d'un bris de glace ou commentaire d'un contrôle : un garde-fou
/// technique, comme le motif d'un rejet MUNASEB (§12).
pub const TEXTE_MAX: usize = 1000;

/// Comment la prise en charge a été ouverte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preuve<'a> {
    CodeSoin(&'a str),
    /// Le professionnel atteste avoir vérifié la CNIB du patient.
    CnibVerifiee,
}

/// Catégorie du motif d'un bris de glace (liste fermée **proposée**, aucune
/// source officielle lue ; à valider par le porteur, docs/points-ouverts.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CategorieMotif {
    PatientInconscient,
    UrgenceVitale,
    PatientIncapableDeCommuniquer,
    Autre,
}

impl CategorieMotif {
    fn as_str(self) -> &'static str {
        match self {
            CategorieMotif::PatientInconscient => "patient_inconscient",
            CategorieMotif::UrgenceVitale => "urgence_vitale",
            CategorieMotif::PatientIncapableDeCommuniquer => "patient_incapable_de_communiquer",
            CategorieMotif::Autre => "autre",
        }
    }
}

/// Conclusion du contrôle d'un bris de glace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Conclusion {
    Justifie,
    Injustifie,
}

/// Une prise en charge, avec sa **fin effective** (prolongations comprises).
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct PriseEnCharge {
    pub id: Uuid,
    pub moyen: String,
    pub debut: DateTime<Utc>,
    pub fin: DateTime<Utc>,
}

/// Fin effective : la plus tardive de la fin et des prolongations
/// (`GREATEST` ignore une valeur nulle, doc PostgreSQL 18 §9.18.4).
macro_rules! fin_effective {
    () => {
        "GREATEST(pc.fin, (SELECT max(pp.nouvelle_fin) FROM prolongation_prise_en_charge pp \
                           WHERE pp.prise_en_charge_id = pc.id))"
    };
}

/// Qui peut ouvrir une prise en charge : l'accueil, le médecin et
/// l'infirmier d'une clinique ; le pharmacien d'une pharmacie (vue
/// pharmacie du dossier). Jamais l'auxiliaire, l'agent MUNASEB ni le
/// responsable.
fn peut_ouvrir(exercice: &Exercice) -> bool {
    matches!(
        (exercice.role.as_str(), exercice.type_structure.as_str()),
        ("agent_accueil" | "medecin" | "infirmier", "clinique") | ("pharmacien", "pharmacie")
    )
}

/// Médecin ou infirmier (sage-femme comprise) d'une clinique : bris de
/// glace (MD2) et prolongation.
fn soignant(exercice: &Exercice) -> bool {
    matches!(
        (exercice.role.as_str(), exercice.type_structure.as_str()),
        ("medecin" | "infirmier", "clinique")
    )
}

fn texte_valide(champ: &str, texte: &str) -> Result<String, ErreurAcces> {
    let texte = texte.trim();
    if texte.is_empty() || texte.chars().count() > TEXTE_MAX {
        return Err(ErreurAcces::TexteInvalide(format!(
            "Le champ « {champ} » est obligatoire, {TEXTE_MAX} caractères au plus."
        )));
    }
    Ok(texte.to_string())
}

async fn inserer(
    tx: &mut Transaction<'_, Postgres>,
    acteur: &Acteur,
    patient_id: Uuid,
    moyen: &str,
    motif: Option<(CategorieMotif, &str)>,
) -> Result<PriseEnCharge, sqlx::Error> {
    sqlx::query_as(
        "INSERT INTO prise_en_charge (patient_id, affectation_id, structure_id, role, moyen, motif_categorie, motif, fin) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, now() + make_interval(hours => $8)) \
         RETURNING id, moyen, debut, fin",
    )
    .bind(patient_id)
    .bind(acteur.affectation_id)
    .bind(acteur.structure_id)
    .bind(&acteur.role)
    .bind(moyen)
    .bind(motif.map(|(categorie, _)| categorie.as_str()))
    .bind(motif.map(|(_, texte)| texte))
    .bind(DUREE_HEURES)
    .fetch_one(&mut **tx)
    .await
}

/// Ouvre une prise en charge par le code de soin ou la CNIB vérifiée.
/// Ordre : rôle (403), limites (429), patient (404, échec tracé), code
/// (422, échec tracé avec le patient), prise en charge et journal, dans une
/// transaction validée avant la réponse.
pub async fn ouvrir(
    pool: &PgPool,
    cle: &[u8; 32],
    exercice: &Exercice,
    nip: &str,
    preuve: Preuve<'_>,
) -> Result<PriseEnCharge, ErreurAcces> {
    if !peut_ouvrir(exercice) {
        return Err(ErreurAcces::RoleNonAutorise);
    }
    let acteur = Acteur::from(exercice);
    // Un NIP inconnu sans code compte comme un scan sans résultat ; avec un
    // code, comme un code faux : chaque voie a sa propre limite.
    let nature_echec = match preuve {
        Preuve::CodeSoin(_) => NatureEchec::CodeSoin,
        Preuve::CnibVerifiee => NatureEchec::Scan,
    };
    let mut tx = pool.begin().await?;
    acces_patient::controler_echecs(&mut tx, &acteur, nature_echec).await?;
    // L'attestation CNIB est déclarative : le nombre d'ouvertures réussies
    // est limité comme les scans (relecture de sécurité du 2026-11-17 ;
    // OWASP API4:2023), et chacune reste au journal.
    acces_patient::controler_reussites(&mut tx, &acteur, NatureAcces::OuverturePriseEnCharge)
        .await?;
    let Some(patient_id) = acces_patient::patient_du_nip(&mut tx, nip).await? else {
        acces_patient::enregistrer_echec(tx, &acteur, nature_echec, None).await?;
        return Err(ErreurAcces::PatientInconnu);
    };

    let moyen = match preuve {
        Preuve::CodeSoin(code) => {
            // Limite par patient : un code de 6 chiffres ne doit pas pouvoir
            // être deviné depuis plusieurs établissements (NIST SP 800-63B
            // rév. 4 §3.2.2, limitation des essais d'un authentificateur ;
            // valeur de BF-01). Le patient est verrouillé pour compter.
            sqlx::query("SELECT id FROM patient WHERE id = $1 FOR UPDATE")
                .bind(patient_id)
                .execute(&mut *tx)
                .await?;
            let (echecs, attente): (i64, Option<i64>) = sqlx::query_as(
                "SELECT count(*), \
                        ceil(extract(epoch FROM min(date_echec) + make_interval(mins => $2) - now()))::bigint \
                 FROM echec_acces_patient \
                 WHERE patient_id = $1 AND nature = 'code_soin' AND date_echec > now() - make_interval(mins => $2)",
            )
            .bind(patient_id)
            .bind(FENETRE_ECHECS_MINUTES)
            .fetch_one(&mut *tx)
            .await?;
            if echecs >= ECHECS_MAX {
                return Err(ErreurAcces::TropDEssais {
                    reessayer_dans_secs: attente.unwrap_or(1).max(1) as u64,
                });
            }
            let accepte = code_soin::verifier(&mut tx, cle, patient_id, code)
                .await
                .map_err(|e| ErreurAcces::Interne(e.to_string()))?;
            if !accepte {
                acces_patient::enregistrer_echec(
                    tx,
                    &acteur,
                    NatureEchec::CodeSoin,
                    Some(patient_id),
                )
                .await?;
                return Err(ErreurAcces::CodeSoinInvalide);
            }
            "code_soin"
        }
        Preuve::CnibVerifiee => "cnib_verifiee",
    };

    let prise = inserer(&mut tx, &acteur, patient_id, moyen, None).await?;
    acces_patient::ecrire_journal(
        &mut tx,
        &acteur,
        patient_id,
        NatureAcces::OuverturePriseEnCharge,
        false,
        Some(prise.id),
    )
    .await?;
    tx.commit().await?;
    Ok(prise)
}

/// Bris de glace (MD2) : médecin ou infirmier d'une clinique, motif
/// obligatoire. Le patient est averti par une notification **après** la
/// validation ; un échec de la notification est journalisé et n'annule
/// jamais l'accès (même règle que les notifications MUNASEB, §12). Le texte
/// reste minimal : ni l'établissement, ni le motif (un push futur
/// s'afficherait sur l'écran verrouillé).
pub async fn briser_la_glace(
    pool: &PgPool,
    exercice: &Exercice,
    nip: &str,
    categorie: CategorieMotif,
    motif: &str,
) -> Result<PriseEnCharge, ErreurAcces> {
    if !soignant(exercice) {
        return Err(ErreurAcces::RoleNonAutorise);
    }
    let motif = texte_valide("motif", motif)?;
    let acteur = Acteur::from(exercice);
    let mut tx = pool.begin().await?;
    acces_patient::controler_echecs(&mut tx, &acteur, NatureEchec::Scan).await?;
    // Même limite que les ouvertures (relecture de sécurité du 2026-11-17).
    acces_patient::controler_reussites(&mut tx, &acteur, NatureAcces::BrisDeGlace).await?;
    let Some(patient_id) = acces_patient::patient_du_nip(&mut tx, nip).await? else {
        acces_patient::enregistrer_echec(tx, &acteur, NatureEchec::Scan, None).await?;
        return Err(ErreurAcces::PatientInconnu);
    };
    let prise = inserer(
        &mut tx,
        &acteur,
        patient_id,
        "bris_de_glace",
        Some((categorie, &motif)),
    )
    .await?;
    acces_patient::ecrire_journal(
        &mut tx,
        &acteur,
        patient_id,
        NatureAcces::BrisDeGlace,
        false,
        Some(prise.id),
    )
    .await?;
    let utilisateur_id: Uuid =
        sqlx::query_scalar("SELECT utilisateur_id FROM patient WHERE id = $1")
            .bind(patient_id)
            .fetch_one(&mut *tx)
            .await?;
    tx.commit().await?;

    if let Err(erreur) = notification::creer(
        pool,
        utilisateur_id,
        TypeNotification::AccesUrgence,
        "Accès d'urgence à votre dossier",
        "Votre dossier médical a été ouvert en urgence. Consultez le journal des accès.",
        None,
    )
    .await
    {
        tracing::error!(%erreur, prise_en_charge = %prise.id, "notification de bris de glace non créée");
    }
    Ok(prise)
}

/// Prise en charge de l'établissement de l'acteur, encore en cours :
/// (patient, moyen). Inexistante ou d'un autre établissement : 404 ;
/// terminée : 403. Utilisée par chaque lecture et écriture du dossier.
pub(crate) async fn en_cours(
    tx: &mut Transaction<'_, Postgres>,
    acteur: &Acteur,
    prise_en_charge_id: Uuid,
) -> Result<(Uuid, String), ErreurAcces> {
    let ligne: Option<(Uuid, String, bool)> = sqlx::query_as(concat!(
        "SELECT pc.patient_id, pc.moyen, ",
        fin_effective!(),
        " > now() FROM prise_en_charge pc WHERE pc.id = $1 AND pc.structure_id = $2"
    ))
    .bind(prise_en_charge_id)
    .bind(acteur.structure_id)
    .fetch_optional(&mut **tx)
    .await?;
    match ligne {
        None => Err(ErreurAcces::PriseEnChargeInconnue),
        Some((_, _, false)) => Err(ErreurAcces::PriseEnChargeExpiree),
        Some((patient_id, moyen, true)) => Ok((patient_id, moyen)),
    }
}

/// Prolonge de 24 heures une prise en charge en cours, pendant une
/// hospitalisation (MD3), par un médecin ou un infirmier du même
/// établissement. La nouvelle fin part de la fin effective actuelle.
pub async fn prolonger(
    pool: &PgPool,
    exercice: &Exercice,
    prise_en_charge_id: Uuid,
) -> Result<PriseEnCharge, ErreurAcces> {
    if !soignant(exercice) {
        return Err(ErreurAcces::RoleNonAutorise);
    }
    let acteur = Acteur::from(exercice);
    let mut tx = pool.begin().await?;
    let (patient_id, _) = en_cours(&mut tx, &acteur, prise_en_charge_id).await?;
    sqlx::query(concat!(
        "INSERT INTO prolongation_prise_en_charge (prise_en_charge_id, affectation_id, structure_id, role, motif, nouvelle_fin) \
         SELECT pc.id, $2, $3, $4, 'hospitalisation', ",
        fin_effective!(),
        " + make_interval(hours => $5) FROM prise_en_charge pc WHERE pc.id = $1"
    ))
    .bind(prise_en_charge_id)
    .bind(acteur.affectation_id)
    .bind(acteur.structure_id)
    .bind(&acteur.role)
    .bind(DUREE_HEURES)
    .execute(&mut *tx)
    .await?;
    acces_patient::ecrire_journal(
        &mut tx,
        &acteur,
        patient_id,
        NatureAcces::ProlongationPriseEnCharge,
        false,
        Some(prise_en_charge_id),
    )
    .await?;
    let prise: PriseEnCharge = sqlx::query_as(concat!(
        "SELECT pc.id, pc.moyen, pc.debut, ",
        fin_effective!(),
        " AS fin FROM prise_en_charge pc WHERE pc.id = $1"
    ))
    .bind(prise_en_charge_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(prise)
}

// ---------------------------------------------------------------------
// Contrôle des bris de glace par le responsable (MD2)
// ---------------------------------------------------------------------

/// Le responsable d'une clinique contrôle les bris de glace de son
/// établissement.
pub fn peut_controler(exercice: &Exercice) -> bool {
    exercice.role == "responsable" && exercice.type_structure == "clinique"
}

/// Un bris de glace tel que le responsable le voit : qui, quand, pourquoi,
/// pour quel patient (nom et prénom seulement, pour retrouver le dossier
/// papier), et le contrôle s'il est fait.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct BrisDeGlace {
    pub id: Uuid,
    pub debut: DateTime<Utc>,
    pub role: String,
    pub professionnel_nom: Option<String>,
    pub professionnel_prenom: Option<String>,
    pub motif_categorie: String,
    pub motif: String,
    pub patient_nom: Option<String>,
    pub patient_prenom: Option<String>,
    pub conclusion: Option<String>,
    pub commentaire: Option<String>,
    pub date_controle: Option<DateTime<Utc>>,
}

/// Bris de glace de l'établissement, les plus récents d'abord ;
/// `a_controler` : seulement ceux qui n'ont pas encore été contrôlés.
pub async fn lister_bris_de_glace(
    pool: &PgPool,
    exercice: &Exercice,
    a_controler: bool,
    avant: Option<Uuid>,
    limite: i64,
) -> Result<Vec<BrisDeGlace>, ErreurAcces> {
    if !peut_controler(exercice) {
        return Err(ErreurAcces::RoleNonAutorise);
    }
    Ok(sqlx::query_as(
        "SELECT pc.id, pc.debut, pc.role, up.nom AS professionnel_nom, up.prenom AS professionnel_prenom, \
                pc.motif_categorie, pc.motif, upa.nom AS patient_nom, upa.prenom AS patient_prenom, \
                cb.conclusion, cb.commentaire, cb.date_controle \
         FROM prise_en_charge pc \
         JOIN affectation a ON a.id = pc.affectation_id JOIN compte c ON c.id = a.compte_id \
         JOIN utilisateur up ON up.id = c.utilisateur_id \
         JOIN patient p ON p.id = pc.patient_id JOIN utilisateur upa ON upa.id = p.utilisateur_id \
         LEFT JOIN controle_bris_de_glace cb ON cb.prise_en_charge_id = pc.id \
         WHERE pc.structure_id = $1 AND pc.moyen = 'bris_de_glace' \
           AND (NOT $2 OR cb.prise_en_charge_id IS NULL) AND ($3::uuid IS NULL OR pc.id < $3) \
         ORDER BY pc.id DESC LIMIT $4",
    )
    .bind(exercice.structure_id)
    .bind(a_controler)
    .bind(avant)
    .bind(limite)
    .fetch_all(pool)
    .await?)
}

/// Contrôle un bris de glace de l'établissement, une seule fois.
pub async fn controler(
    pool: &PgPool,
    exercice: &Exercice,
    prise_en_charge_id: Uuid,
    conclusion: Conclusion,
    commentaire: Option<&str>,
) -> Result<(), ErreurAcces> {
    if !peut_controler(exercice) {
        return Err(ErreurAcces::RoleNonAutorise);
    }
    let commentaire = commentaire
        .map(|c| texte_valide("commentaire", c))
        .transpose()?;
    let existe: Option<bool> = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM controle_bris_de_glace WHERE prise_en_charge_id = pc.id) \
         FROM prise_en_charge pc WHERE pc.id = $1 AND pc.structure_id = $2 AND pc.moyen = 'bris_de_glace'",
    )
    .bind(prise_en_charge_id)
    .bind(exercice.structure_id)
    .fetch_optional(pool)
    .await?;
    match existe {
        None => return Err(ErreurAcces::PriseEnChargeInconnue),
        Some(true) => return Err(ErreurAcces::DejaControle),
        Some(false) => {}
    }
    let resultat = sqlx::query(
        "INSERT INTO controle_bris_de_glace (prise_en_charge_id, affectation_id, structure_id, role, conclusion, commentaire) \
         VALUES ($1, $2, $3, 'responsable', $4, $5)",
    )
    .bind(prise_en_charge_id)
    .bind(exercice.affectation_id)
    .bind(exercice.structure_id)
    .bind(match conclusion {
        Conclusion::Justifie => "justifie",
        Conclusion::Injustifie => "injustifie",
    })
    .bind(commentaire)
    .execute(pool)
    .await;
    match resultat {
        Ok(_) => Ok(()),
        // Deux contrôles simultanés : la clé primaire n'en garde qu'un.
        Err(e)
            if e.as_database_error()
                .is_some_and(|d| d.is_unique_violation()) =>
        {
            Err(ErreurAcces::DejaControle)
        }
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::code_soin::tests::{CLE, code_actuel, mot_de_passe};
    use crate::fixtures_medicales::{exercice, patient, structure};

    /// Une prise en charge déjà terminée, posée en SQL (dates que le code ne
    /// permet pas de créer).
    pub(crate) async fn prise_terminee(pool: &PgPool, ex: &Exercice, patient_id: Uuid) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO prise_en_charge (patient_id, affectation_id, structure_id, role, moyen, debut, fin) \
             VALUES ($1, $2, $3, $4, 'cnib_verifiee', now() - interval '2 days', now() - interval '1 day') RETURNING id",
        )
        .bind(patient_id)
        .bind(ex.affectation_id)
        .bind(ex.structure_id)
        .bind(&ex.role)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// Ouverture par le code de soin (juste, faux, limite par patient) et
    /// par la CNIB vérifiée ; rôles refusés ; chaque ouverture au journal.
    #[sqlx::test]
    #[ignore]
    async fn ouverture_par_code_et_cnib(pool: PgPool) {
        let p = patient(&pool, "+22670390001").await;
        mot_de_passe(&pool, p.compte_id).await;
        let cree = code_soin::generer(&pool, &CLE, p.compte_id, "Soin-2026!")
            .await
            .unwrap();
        let clinique = structure(&pool, "clinique", "Clinique fictive").await;
        let pharmacie = structure(&pool, "pharmacie", "Pharmacie fictive").await;
        let munaseb = structure(&pool, "munaseb", "MUNASEB").await;
        let accueil = exercice(&pool, clinique, "agent_accueil").await;

        let prise = ouvrir(
            &pool,
            &CLE,
            &accueil,
            &p.nip,
            Preuve::CodeSoin(&code_actuel(&cree)),
        )
        .await
        .unwrap();
        assert_eq!(prise.moyen, "code_soin");
        assert_eq!((prise.fin - prise.debut).num_hours(), 24);

        // Code faux : refusé et compté pour ce patient, même depuis
        // d'autres établissements ; au 5ᵉ, 429.
        for i in 0..ECHECS_MAX {
            let medecin = exercice(&pool, clinique, "medecin").await;
            let refus = ouvrir(&pool, &CLE, &medecin, &p.nip, Preuve::CodeSoin("000000")).await;
            assert!(
                matches!(refus, Err(ErreurAcces::CodeSoinInvalide)),
                "{i} : {refus:?}"
            );
        }
        let infirmier = exercice(&pool, clinique, "infirmier").await;
        assert!(matches!(
            ouvrir(&pool, &CLE, &infirmier, &p.nip, Preuve::CodeSoin("000000")).await,
            Err(ErreurAcces::TropDEssais { .. })
        ));

        // CNIB vérifiée : pour un patient sans code de soin aussi.
        let autre = patient(&pool, "+22670390002").await;
        let prise = ouvrir(&pool, &CLE, &infirmier, &autre.nip, Preuve::CnibVerifiee)
            .await
            .unwrap();
        assert_eq!(prise.moyen, "cnib_verifiee");
        let pharmacien = exercice(&pool, pharmacie, "pharmacien").await;
        ouvrir(&pool, &CLE, &pharmacien, &autre.nip, Preuve::CnibVerifiee)
            .await
            .unwrap();
        assert!(matches!(
            ouvrir(&pool, &CLE, &pharmacien, "123", Preuve::CnibVerifiee).await,
            Err(ErreurAcces::PatientInconnu)
        ));

        for (structure_id, role) in [
            (pharmacie, "auxiliaire_pharmacie"),
            (munaseb, "agent_assurance_munaseb"),
            (clinique, "responsable"),
            (pharmacie, "responsable"),
        ] {
            let ex = exercice(&pool, structure_id, role).await;
            assert!(
                matches!(
                    ouvrir(&pool, &CLE, &ex, &autre.nip, Preuve::CnibVerifiee).await,
                    Err(ErreurAcces::RoleNonAutorise)
                ),
                "{role}"
            );
        }

        let ouvertures: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM journal_acces WHERE nature = 'ouverture_prise_en_charge' AND prise_en_charge_id IS NOT NULL",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(ouvertures, 3);

        // Ouvertures réussies limitées par affectation (attestation CNIB
        // déclarative) ; même limite pour les bris de glace.
        for nature in ["ouverture_prise_en_charge", "bris_de_glace"] {
            let medecin = exercice(&pool, clinique, "medecin").await;
            sqlx::query(
                "INSERT INTO journal_acces (patient_id, affectation_id, structure_id, role, nature) \
                 SELECT $1, $2, $3, 'medecin', $4 FROM generate_series(1, $5)",
            )
            .bind(autre.patient_id)
            .bind(medecin.affectation_id)
            .bind(clinique)
            .bind(nature)
            .bind(acces_patient::SCANS_MAX as i32)
            .execute(&pool)
            .await
            .unwrap();
            let refus = if nature == "bris_de_glace" {
                briser_la_glace(&pool, &medecin, &autre.nip, CategorieMotif::Autre, "Test").await
            } else {
                ouvrir(&pool, &CLE, &medecin, &autre.nip, Preuve::CnibVerifiee).await
            };
            assert!(
                matches!(refus, Err(ErreurAcces::TropDEssais { .. })),
                "{nature} : {refus:?}"
            );
        }
    }

    /// Bris de glace : médecin ou infirmier, motif obligatoire ; jamais le
    /// pharmacien ni l'accueil (code et base) ; patient averti ; contrôle
    /// par le responsable, une seule fois ; le patient voit le motif dans
    /// son journal.
    #[sqlx::test]
    #[ignore]
    async fn bris_de_glace_et_controle(pool: PgPool) {
        let p = patient(&pool, "+22670390003").await;
        let clinique = structure(&pool, "clinique", "Clinique fictive").await;
        let autre_clinique = structure(&pool, "clinique", "Autre clinique").await;
        let pharmacie = structure(&pool, "pharmacie", "Pharmacie fictive").await;
        let medecin = exercice(&pool, clinique, "medecin").await;
        let infirmier = exercice(&pool, clinique, "infirmier").await;

        for motif in ["", "   ", &"x".repeat(TEXTE_MAX + 1)] {
            assert!(matches!(
                briser_la_glace(
                    &pool,
                    &medecin,
                    &p.nip,
                    CategorieMotif::UrgenceVitale,
                    motif
                )
                .await,
                Err(ErreurAcces::TexteInvalide(_))
            ));
        }
        for (structure_id, role) in [(pharmacie, "pharmacien"), (clinique, "agent_accueil")] {
            let ex = exercice(&pool, structure_id, role).await;
            assert!(
                matches!(
                    briser_la_glace(&pool, &ex, &p.nip, CategorieMotif::Autre, "Urgence").await,
                    Err(ErreurAcces::RoleNonAutorise)
                ),
                "{role}"
            );
        }
        // La base refuse aussi un bris de glace d'un pharmacien.
        let pharmacien = exercice(&pool, pharmacie, "pharmacien").await;
        assert!(
            sqlx::query(
                "INSERT INTO prise_en_charge (patient_id, affectation_id, structure_id, role, moyen, motif_categorie, motif, fin) \
                 VALUES ($1, $2, $3, 'pharmacien', 'bris_de_glace', 'autre', 'x', now() + interval '1 hour')",
            )
            .bind(p.patient_id)
            .bind(pharmacien.affectation_id)
            .bind(pharmacie)
            .execute(&pool)
            .await
            .is_err()
        );

        let prise = briser_la_glace(
            &pool,
            &infirmier,
            &p.nip,
            CategorieMotif::PatientInconscient,
            " Patient inconscient à l'arrivée ",
        )
        .await
        .unwrap();
        assert_eq!(prise.moyen, "bris_de_glace");
        let notification: (String, String) = sqlx::query_as(
            "SELECT n.type, n.message FROM notification n JOIN patient p ON p.utilisateur_id = n.utilisateur_id WHERE p.id = $1",
        )
        .bind(p.patient_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(notification.0, "acces_urgence");
        assert!(!notification.1.contains("Clinique") && !notification.1.contains("inconscient"));

        let vu = acces_patient::journal_du_patient(&pool, p.compte_id, None, 50)
            .await
            .unwrap();
        assert_eq!(
            (
                vu[0].nature.as_str(),
                vu[0].moyen.as_deref(),
                vu[0].motif_categorie.as_deref(),
                vu[0].motif.as_deref()
            ),
            (
                "bris_de_glace",
                Some("bris_de_glace"),
                Some("patient_inconscient"),
                Some("Patient inconscient à l'arrivée")
            )
        );

        // Contrôle par le responsable de la clinique, une seule fois.
        let responsable = exercice(&pool, clinique, "responsable").await;
        let autre_responsable = exercice(&pool, autre_clinique, "responsable").await;
        assert!(matches!(
            lister_bris_de_glace(&pool, &medecin, true, None, 50).await,
            Err(ErreurAcces::RoleNonAutorise)
        ));
        let a_controler = lister_bris_de_glace(&pool, &responsable, true, None, 50)
            .await
            .unwrap();
        assert_eq!(a_controler.len(), 1);
        assert_eq!(a_controler[0].motif_categorie, "patient_inconscient");
        assert!(
            lister_bris_de_glace(&pool, &autre_responsable, false, None, 50)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(matches!(
            controler(
                &pool,
                &autre_responsable,
                prise.id,
                Conclusion::Justifie,
                None
            )
            .await,
            Err(ErreurAcces::PriseEnChargeInconnue)
        ));
        controler(
            &pool,
            &responsable,
            prise.id,
            Conclusion::Justifie,
            Some("Admis aux urgences"),
        )
        .await
        .unwrap();
        assert!(matches!(
            controler(&pool, &responsable, prise.id, Conclusion::Injustifie, None).await,
            Err(ErreurAcces::DejaControle)
        ));
        assert!(
            lister_bris_de_glace(&pool, &responsable, true, None, 50)
                .await
                .unwrap()
                .is_empty()
        );
        // Une prise en charge ordinaire ne se contrôle pas.
        let ordinaire = ouvrir(&pool, &CLE, &medecin, &p.nip, Preuve::CnibVerifiee)
            .await
            .unwrap();
        assert!(matches!(
            controler(
                &pool,
                &responsable,
                ordinaire.id,
                Conclusion::Justifie,
                None
            )
            .await,
            Err(ErreurAcces::PriseEnChargeInconnue)
        ));
    }

    /// Une prise en charge vaut pour son établissement seulement, et
    /// jusqu'à sa fin effective ; la prolongation ajoute 24 h et se trace ;
    /// rien ne se modifie ni ne s'efface.
    #[sqlx::test]
    #[ignore]
    async fn en_cours_prolongation_et_traces(pool: PgPool) {
        let p = patient(&pool, "+22670390004").await;
        let clinique = structure(&pool, "clinique", "Clinique fictive").await;
        let autre_clinique = structure(&pool, "clinique", "Autre clinique").await;
        let medecin = exercice(&pool, clinique, "medecin").await;
        let medecin_ailleurs = exercice(&pool, autre_clinique, "medecin").await;
        let prise = ouvrir(&pool, &CLE, &medecin, &p.nip, Preuve::CnibVerifiee)
            .await
            .unwrap();

        let mut tx = pool.begin().await.unwrap();
        assert_eq!(
            en_cours(&mut tx, &Acteur::from(&medecin), prise.id)
                .await
                .unwrap()
                .0,
            p.patient_id
        );
        assert!(matches!(
            en_cours(&mut tx, &Acteur::from(&medecin_ailleurs), prise.id).await,
            Err(ErreurAcces::PriseEnChargeInconnue)
        ));
        let terminee = prise_terminee(&pool, &medecin, p.patient_id).await;
        assert!(matches!(
            en_cours(&mut tx, &Acteur::from(&medecin), terminee).await,
            Err(ErreurAcces::PriseEnChargeExpiree)
        ));
        drop(tx);

        let prolongee = prolonger(&pool, &medecin, prise.id).await.unwrap();
        assert_eq!((prolongee.fin - prise.fin).num_hours(), 24);
        let encore = prolonger(&pool, &medecin, prise.id).await.unwrap();
        assert_eq!((encore.fin - prise.fin).num_hours(), 48);
        assert!(matches!(
            prolonger(&pool, &medecin, terminee).await,
            Err(ErreurAcces::PriseEnChargeExpiree)
        ));
        assert!(matches!(
            prolonger(&pool, &medecin_ailleurs, prise.id).await,
            Err(ErreurAcces::PriseEnChargeInconnue)
        ));
        let accueil = exercice(&pool, clinique, "agent_accueil").await;
        assert!(matches!(
            prolonger(&pool, &accueil, prise.id).await,
            Err(ErreurAcces::RoleNonAutorise)
        ));

        for sql in [
            "UPDATE prise_en_charge SET fin = fin + interval '1 day'",
            "DELETE FROM prise_en_charge",
            "UPDATE prolongation_prise_en_charge SET nouvelle_fin = nouvelle_fin",
            "DELETE FROM prolongation_prise_en_charge",
        ] {
            assert!(sqlx::query(sql).execute(&pool).await.is_err(), "{sql}");
        }
    }

    /// Recherche par identité exacte (MD19) : une seule personne ou rien,
    /// jamais une liste ; échecs limités ; réservée à la clinique.
    #[sqlx::test]
    #[ignore]
    async fn recherche_par_identite(pool: PgPool) {
        let p = patient(&pool, "+22670390005").await;
        // Deux homonymes parfaits (mêmes nom, prénom, date et lieu).
        let _h1 = patient(&pool, "+22670390006").await;
        let _h2 = patient(&pool, "+22670390007").await;
        sqlx::query(
            "UPDATE utilisateur SET nom = 'ZONGO', prenom = 'Mariam' \
             WHERE id = (SELECT utilisateur_id FROM patient WHERE id = $1)",
        )
        .bind(p.patient_id)
        .execute(&pool)
        .await
        .unwrap();
        let clinique = structure(&pool, "clinique", "Clinique fictive").await;
        let pharmacie = structure(&pool, "pharmacie", "Pharmacie fictive").await;
        let accueil = exercice(&pool, clinique, "agent_accueil").await;
        let saisie = |nom: &str, prenom: &str| acces_patient::IdentiteRecherchee {
            nom: nom.into(),
            prenom: prenom.into(),
            date_naissance: chrono::NaiveDate::from_ymd_opt(1995, 4, 12).unwrap(),
            lieu_naissance: " koudougou ".into(),
        };

        let fiche =
            acces_patient::rechercher_par_identite(&pool, &accueil, &saisie(" zongo", "MARIAM"))
                .await
                .unwrap();
        assert_eq!(fiche["nip"], p.nip.as_str());
        assert_eq!(fiche["nom"], "ZONGO");
        assert!(fiche.get("telephone").is_none());

        let pharmacien = exercice(&pool, pharmacie, "pharmacien").await;
        assert!(matches!(
            acces_patient::rechercher_par_identite(&pool, &pharmacien, &saisie("ZONGO", "Mariam"))
                .await,
            Err(ErreurAcces::RoleNonAutorise)
        ));

        // Homonymes : jamais une liste, même réponse qu'aucun résultat.
        for (nom, prenom) in [
            ("OUEDRAOGO", "Awa"),
            ("ZONGO", "Marian"),
            ("ZONGÖ", "Mariam"),
        ] {
            assert!(
                matches!(
                    acces_patient::rechercher_par_identite(&pool, &accueil, &saisie(nom, prenom))
                        .await,
                    Err(ErreurAcces::PatientInconnu)
                ),
                "{nom} {prenom}"
            );
        }
        let medecin = exercice(&pool, clinique, "medecin").await;
        for _ in 0..ECHECS_MAX {
            let _ =
                acces_patient::rechercher_par_identite(&pool, &medecin, &saisie("X", "Y")).await;
        }
        assert!(matches!(
            acces_patient::rechercher_par_identite(&pool, &medecin, &saisie("ZONGO", "Mariam"))
                .await,
            Err(ErreurAcces::TropDEssais { .. })
        ));
        let recherches: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM journal_acces WHERE nature = 'recherche_identite' AND patient_id = $1",
        )
        .bind(p.patient_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(recherches, 1);
    }
}
