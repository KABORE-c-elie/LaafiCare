//! ContratAssurance MUNASEB (section 12 du CLAUDE.md) : création,
//! renouvellement et vérification des droits (plafond restant). Champs
//! basés sur les sources MUNASEB (voir migration 0006) : aucune classe
//! `Contrat`/`ContratAssurance` n'existe dans
//! `docs/diagrammes/diagramme_classes_munaseb oki11.drawio`, son équivalent
//! réel est `carteAdhesion`.
//!
//! Les dates vivent dans `periode_adhesion_munaseb` (migration 0009) : une
//! période par adhésion ou réabonnement, jamais écrasée (le réabonnement
//! du mémoire a ses propres dateDebut/dateFin, dictionnaire l. 1555-1562).
//!
//! Seule exception aux sources : la colonne `statut` (actif/suspendu,
//! migration 0008), décision propre à LaafiCare -- la carte du mémoire n'a
//! qu'une date d'expiration.
//!
//! Le taux de prise en charge n'est pas ici : il dépend du type d'acte et
//! vit en base (`tarif_acte_munaseb`, migration 0007), modifiable par les
//! agents, jamais dans une constante Rust.

use chrono::{Days, Months, NaiveDate, Utc};
use sqlx::PgPool;
use sqlx::types::Uuid;

// Plafond MUNASEB, identique pour tout mutualiste (mémoire, l. 64, 569,
// 826) -- pas une colonne par contrat.
pub const PLAFOND_ANNUEL_FCFA: i32 = 100_000;

// Règles de la section 12 : carence d'un mois (première adhésion ou
// renouvellement en retard), durée de 12 mois à partir du début (mémoire,
// dictionnaire l. 1546 : "carte d'1 an").
const CARENCE: Months = Months::new(1);
const DUREE: Months = Months::new(12);

#[derive(Debug, thiserror::Error)]
pub enum ErreurContratAssuranceMunaseb {
    #[error("ce numéro de carte est déjà utilisé")]
    NumeroCarteDejaUtilise,

    #[error("ce patient a déjà un contrat")]
    ContratDejaExistant,

    #[error("aucun contrat pour ce patient")]
    AucunContrat,

    /// Échoue par défaut (décision actée) : un appelant ne peut pas traiter
    /// par oubli un contrat suspendu comme actif.
    #[error("contrat suspendu")]
    ContratSuspendu,

    /// Aucune période en cours, mais une période payée commence plus tard.
    /// Distinct de `ContratExpire` : quelqu'un qui vient de payer ne doit
    /// pas lire "expiré" (section 12).
    #[error("en carence, couverture à partir du {debut_couverture}")]
    EnCarence { debut_couverture: NaiveDate },

    #[error("contrat expiré")]
    ContratExpire,

    /// Une période payée et pas encore commencée existe déjà : au plus une
    /// période d'avance (décision C2). Couvre aussi un renouvellement tenté
    /// pendant une carence.
    #[error("une période future est déjà payée")]
    PeriodeFutureDejaPayee,

    #[error("erreur interne : {0}")]
    Interne(String),
}

impl From<sqlx::Error> for ErreurContratAssuranceMunaseb {
    fn from(erreur: sqlx::Error) -> Self {
        ErreurContratAssuranceMunaseb::Interne(erreur.to_string())
    }
}

/// Droits d'un patient sous contrat MUNASEB, tels que visibles par le
/// patient.
#[derive(Debug, PartialEq)]
pub struct DroitsContrat {
    pub plafond_annuel_fcfa: i32,
    pub plafond_restant_fcfa: i32,
}

/// Date du jour. UTC suffit : le Burkina Faso est à UTC+0 toute l'année,
/// la date UTC est donc la date locale.
fn aujourdhui() -> NaiveDate {
    Utc::now().date_naive()
}

/// Dates (début, fin) de la prochaine période, d'après la dernière période
/// du contrat (`None` pour une première adhésion) et la date de paiement.
/// Fonction pure : toutes les règles de début de période de la section 12
/// sont ici, testées sans base.
///
/// Ajout de mois : `NaiveDate::checked_add_months` (doc chrono 0.4.45)
/// ramène au dernier jour du mois quand le jour d'origine n'existe pas
/// (31 janvier + 1 mois = 28 ou 29 février) et ne renvoie `None` qu'hors de
/// la plage de dates valides.
pub fn calculer_periode(
    derniere: Option<(NaiveDate, NaiveDate)>,
    date_paiement: NaiveDate,
) -> Result<(NaiveDate, NaiveDate), ErreurContratAssuranceMunaseb> {
    let hors_plage = || ErreurContratAssuranceMunaseb::Interne("date hors de la plage valide".into());

    let debut = match derniere {
        // Première adhésion : carence.
        None => date_paiement.checked_add_months(CARENCE).ok_or_else(hors_plage)?,
        Some((debut_derniere, _)) if debut_derniere > date_paiement => {
            return Err(ErreurContratAssuranceMunaseb::PeriodeFutureDejaPayee);
        }
        // Dans les temps (payé au plus tard le dernier jour) : lendemain de
        // la fin, sans carence -- ni chevauchement, ni jour perdu.
        Some((_, fin_derniere)) if date_paiement <= fin_derniere => {
            fin_derniere.checked_add_days(Days::new(1)).ok_or_else(hors_plage)?
        }
        // En retard : carence, sans effet rétroactif.
        Some(_) => date_paiement.checked_add_months(CARENCE).ok_or_else(hors_plage)?,
    };

    let fin = debut
        .checked_add_months(DUREE)
        .and_then(|d| d.checked_sub_days(Days::new(1)))
        .ok_or_else(hors_plage)?;

    Ok((debut, fin))
}

/// Crée le contrat MUNASEB d'un patient (équivalent de `carteAdhesion`) et
/// sa première période, dans une seule transaction. La date de paiement
/// est la date du jour, jamais fournie par l'appelant (section 12, V1).
///
/// Un seul contrat par patient (contrainte UNIQUE sur `patient_id`) --
/// vérifié explicitement pour renvoyer une erreur métier nommée plutôt
/// qu'un code SQL brut, même principe que `auth_patient::creer_compte`.
#[allow(clippy::too_many_arguments)]
pub async fn creer(
    pool: &PgPool,
    patient_id: Uuid,
    numero_carte: &str,
    ufr: &str,
    universite: &str,
    num_matricule: &str,
    personne_a_prevenir: Option<&str>,
) -> Result<Uuid, ErreurContratAssuranceMunaseb> {
    let contrat_existant: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM contrat_assurance_munaseb WHERE patient_id = $1")
            .bind(patient_id)
            .fetch_optional(pool)
            .await?;
    if contrat_existant.is_some() {
        return Err(ErreurContratAssuranceMunaseb::ContratDejaExistant);
    }

    let numero_deja_utilise: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM contrat_assurance_munaseb WHERE numero_carte = $1")
            .bind(numero_carte)
            .fetch_optional(pool)
            .await?;
    if numero_deja_utilise.is_some() {
        return Err(ErreurContratAssuranceMunaseb::NumeroCarteDejaUtilise);
    }

    let date_paiement = aujourdhui();
    let (debut, fin) = calculer_periode(None, date_paiement)?;

    let mut tx = pool.begin().await?;

    let contrat_id: Uuid = sqlx::query_scalar(
        "INSERT INTO contrat_assurance_munaseb \
         (patient_id, numero_carte, ufr, universite, num_matricule, personne_a_prevenir) \
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
    )
    .bind(patient_id)
    .bind(numero_carte)
    .bind(ufr)
    .bind(universite)
    .bind(num_matricule)
    .bind(personne_a_prevenir)
    .fetch_one(&mut *tx)
    .await?;

    inserer_periode(&mut tx, contrat_id, date_paiement, debut, fin).await?;

    tx.commit().await?;
    Ok(contrat_id)
}

/// Renouvelle un contrat : ajoute une période, sans jamais toucher aux
/// précédentes. Date de paiement = date du jour (section 12, V1). Renvoie
/// l'id de la nouvelle période.
pub async fn renouveler(
    pool: &PgPool,
    contrat_id: Uuid,
) -> Result<Uuid, ErreurContratAssuranceMunaseb> {
    let mut tx = pool.begin().await?;

    // Verrou sur la ligne du contrat : deux renouvellements simultanés
    // liraient la même dernière période et calculeraient le même début.
    // La contrainte d'exclusion de la migration 0009 refuserait le second,
    // mais sous forme d'erreur SQL brute ; le verrou les fait passer l'un
    // après l'autre.
    let existe: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM contrat_assurance_munaseb WHERE id = $1 FOR UPDATE")
            .bind(contrat_id)
            .fetch_optional(&mut *tx)
            .await?;
    if existe.is_none() {
        return Err(ErreurContratAssuranceMunaseb::AucunContrat);
    }

    // Les périodes d'un contrat ne se chevauchent pas : la dernière est
    // celle qui finit le plus tard.
    let derniere: Option<(NaiveDate, NaiveDate)> = sqlx::query_as(
        "SELECT date_debut, date_fin FROM periode_adhesion_munaseb \
         WHERE contrat_id = $1 ORDER BY date_fin DESC LIMIT 1",
    )
    .bind(contrat_id)
    .fetch_optional(&mut *tx)
    .await?;

    let date_paiement = aujourdhui();
    let (debut, fin) = calculer_periode(derniere, date_paiement)?;
    let periode_id = inserer_periode(&mut tx, contrat_id, date_paiement, debut, fin).await?;

    tx.commit().await?;
    Ok(periode_id)
}

async fn inserer_periode(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    contrat_id: Uuid,
    date_paiement: NaiveDate,
    debut: NaiveDate,
    fin: NaiveDate,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO periode_adhesion_munaseb (contrat_id, date_paiement, date_debut, date_fin) \
         VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(contrat_id)
    .bind(date_paiement)
    .bind(debut)
    .bind(fin)
    .fetch_one(&mut **tx)
    .await
}

/// Vérifie les droits d'un patient : contrat actif et période en cours,
/// avec le plafond restant de cette période.
pub async fn verifier_droits(
    pool: &PgPool,
    patient_id: Uuid,
) -> Result<DroitsContrat, ErreurContratAssuranceMunaseb> {
    let contrat: Option<(Uuid, String)> =
        sqlx::query_as("SELECT id, statut FROM contrat_assurance_munaseb WHERE patient_id = $1")
            .bind(patient_id)
            .fetch_optional(pool)
            .await?;
    let (contrat_id, statut) = contrat.ok_or(ErreurContratAssuranceMunaseb::AucunContrat)?;

    if statut != "actif" {
        return Err(ErreurContratAssuranceMunaseb::ContratSuspendu);
    }

    let jour = aujourdhui();

    let periode_en_cours: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM periode_adhesion_munaseb \
         WHERE contrat_id = $1 AND $2 BETWEEN date_debut AND date_fin",
    )
    .bind(contrat_id)
    .bind(jour)
    .fetch_optional(pool)
    .await?;

    let Some(periode_id) = periode_en_cours else {
        // Pas de période en cours : soit une période payée commence plus
        // tard (carence), soit tout est expiré.
        let prochain_debut: Option<NaiveDate> = sqlx::query_scalar(
            "SELECT min(date_debut) FROM periode_adhesion_munaseb WHERE contrat_id = $1 AND date_debut > $2",
        )
        .bind(contrat_id)
        .bind(jour)
        .fetch_one(pool)
        .await?;
        return Err(match prochain_debut {
            Some(debut_couverture) => ErreurContratAssuranceMunaseb::EnCarence { debut_couverture },
            None => ErreurContratAssuranceMunaseb::ContratExpire,
        });
    };

    // Décisions actées (section 12) : plafond par période ; seuls les
    // montants Validé et Payé le consomment. Chaque demande est rattachée à
    // sa période dès sa création, aucun filtre de dates n'est nécessaire.
    let consomme: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(montant_rembourse_fcfa), 0)::BIGINT FROM demande_remboursement_munaseb \
         WHERE periode_id = $1 AND statut IN ('valide', 'paye')",
    )
    .bind(periode_id)
    .fetch_one(pool)
    .await?;

    // Le plafonnement au passage à Validé empêche normalement tout
    // dépassement ; le `max(0)` couvre une baisse future du plafond, qui
    // laisserait une consommation passée supérieure au nouveau montant.
    let restant = (i64::from(PLAFOND_ANNUEL_FCFA) - consomme).max(0);

    Ok(DroitsContrat {
        plafond_annuel_fcfa: PLAFOND_ANNUEL_FCFA,
        // restant <= PLAFOND_ANNUEL_FCFA : tient dans un i32.
        plafond_restant_fcfa: restant as i32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(annee: i32, mois: u32, jour: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(annee, mois, jour).unwrap()
    }

    #[test]
    fn premiere_adhesion_avec_carence_d_un_mois() {
        assert_eq!(calculer_periode(None, d(2026, 3, 15)).unwrap(), (d(2026, 4, 15), d(2027, 4, 14)));
    }

    #[test]
    fn paiement_un_31_janvier_ramene_au_dernier_jour_de_fevrier() {
        assert_eq!(calculer_periode(None, d(2026, 1, 31)).unwrap(), (d(2026, 2, 28), d(2027, 2, 27)));
        // Année bissextile : 29 février.
        assert_eq!(calculer_periode(None, d(2028, 1, 31)).unwrap(), (d(2028, 2, 29), d(2029, 2, 27)));
    }

    #[test]
    fn periode_commencant_un_29_fevrier() {
        // 29/02/2028 + 12 mois = 28/02/2029 (ramené), - 1 jour = 27/02/2029.
        let (debut, fin) = calculer_periode(None, d(2028, 1, 29)).unwrap();
        assert_eq!((debut, fin), (d(2028, 2, 29), d(2029, 2, 27)));
        // Le renouvellement dans les temps repart le lendemain : aucun jour
        // perdu.
        assert_eq!(calculer_periode(Some((debut, fin)), d(2029, 2, 1)).unwrap().0, d(2029, 2, 28));
    }

    #[test]
    fn renouvellement_dans_les_temps_sans_trou_ni_carence() {
        let en_cours = Some((d(2026, 4, 15), d(2027, 4, 14)));
        let suivante = (d(2027, 4, 15), d(2028, 4, 14));
        // Payé le dernier jour de la période.
        assert_eq!(calculer_periode(en_cours, d(2027, 4, 14)).unwrap(), suivante);
        // Payé longtemps avant la fin : même période suivante.
        assert_eq!(calculer_periode(en_cours, d(2026, 6, 1)).unwrap(), suivante);
    }

    #[test]
    fn renouvellement_en_retard_avec_carence() {
        let expiree = Some((d(2026, 4, 15), d(2027, 4, 14)));
        // Payé le lendemain de la fin : déjà en retard.
        assert_eq!(calculer_periode(expiree, d(2027, 4, 15)).unwrap(), (d(2027, 5, 15), d(2028, 5, 14)));
    }

    #[test]
    fn refus_d_une_deuxieme_periode_d_avance() {
        // La dernière période n'a pas encore commencé à la date du paiement
        // (période suivante déjà payée, ou carence en cours).
        let future = Some((d(2027, 4, 15), d(2028, 4, 14)));
        assert!(matches!(
            calculer_periode(future, d(2027, 1, 10)),
            Err(ErreurContratAssuranceMunaseb::PeriodeFutureDejaPayee)
        ));
    }

    // #[sqlx::test] (doc sqlx 0.9, attr.test) : "a new test database is
    // created" pour ce test, migrations appliquées automatiquement, base
    // supprimée si le test réussit (gardée pour analyse s'il échoue). La
    // base de développement n'est jamais touchée, et aucun nettoyage n'est
    // nécessaire -- indispensable depuis que l'historique des statuts ne
    // peut plus être supprimé (migration 0010). `#[ignore]` : exige un
    // PostgreSQL démarré, comme les autres tests en base.
    #[sqlx::test]
    #[ignore]
    async fn creer_renouveler_et_verifier_les_droits_en_base(pool: PgPool) {
        let utilisateur_id: Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (nom, prenom, telephone) VALUES ('Test', 'Contrat', '+22670006006') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let patient_id: Uuid = sqlx::query_scalar(
            "INSERT INTO patient (utilisateur_id, nip, date_naissance, lieu_naissance) \
             VALUES ($1, '1234567890123452', '2000-01-01', 'Ouagadougou') RETURNING id",
        )
        .bind(utilisateur_id)
        .fetch_one(&pool)
        .await
        .unwrap();

        let jour = aujourdhui();

        assert!(matches!(
            verifier_droits(&pool, patient_id).await,
            Err(ErreurContratAssuranceMunaseb::AucunContrat)
        ));

        // --- création : première période avec carence, payée aujourd'hui ---
        let contrat_id = creer(&pool, patient_id, "MUNASEB-TEST-0001", "UFR Sciences", "Université Joseph Ki-Zerbo", "MAT-TEST-0001", None)
            .await
            .unwrap();
        let (paiement, debut, fin): (NaiveDate, NaiveDate, NaiveDate) = sqlx::query_as(
            "SELECT date_paiement, date_debut, date_fin FROM periode_adhesion_munaseb WHERE contrat_id = $1",
        )
        .bind(contrat_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(paiement, jour);
        assert_eq!((debut, fin), calculer_periode(None, jour).unwrap());

        assert!(matches!(
            creer(&pool, patient_id, "MUNASEB-TEST-0002", "UFR", "Univ", "MAT", None).await,
            Err(ErreurContratAssuranceMunaseb::ContratDejaExistant)
        ));

        // --- en carence : ni droits, ni "expiré" ---
        match verifier_droits(&pool, patient_id).await {
            Err(ErreurContratAssuranceMunaseb::EnCarence { debut_couverture }) => assert_eq!(debut_couverture, debut),
            autre => panic!("EnCarence attendu, obtenu {autre:?}"),
        }

        // --- renouvellement pendant la carence : refusé ---
        assert!(matches!(
            renouveler(&pool, contrat_id).await,
            Err(ErreurContratAssuranceMunaseb::PeriodeFutureDejaPayee)
        ));

        // --- suspendu ---
        sqlx::query("UPDATE contrat_assurance_munaseb SET statut = 'suspendu' WHERE id = $1")
            .bind(contrat_id)
            .execute(&pool)
            .await
            .unwrap();
        assert!(matches!(
            verifier_droits(&pool, patient_id).await,
            Err(ErreurContratAssuranceMunaseb::ContratSuspendu)
        ));
    }
}
