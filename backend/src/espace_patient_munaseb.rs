//! Côté patient du module MUNASEB (étape 6, décisions E1 à E9 du porteur,
//! 2026-10-21) : ses droits et ses demandes de remboursement, en lecture.
//!
//! Accès strict à ses propres données (E2) : le patient est déduit du seul
//! jeton (compte → identité → dossier patient → contrat), jamais d'un NIP
//! ou d'un identifiant passé dans l'adresse. Une demande d'un autre patient
//! est introuvable, exactement comme une demande inexistante. Source :
//! OWASP API Security Top 10 2023, API1 « Broken Object Level
//! Authorization » (contrôler, pour chaque objet demandé, qu'il appartient
//! à l'utilisateur).
//!
//! Ce que le patient ne voit jamais (E3, E5) : l'identité ou l'affectation
//! des agents, le NIP, la référence interne de l'acte chez le partenaire,
//! l'université, l'UFR, le matricule et la personne à prévenir du contrat.

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use sqlx::PgPool;
use sqlx::types::Uuid;

use crate::contrat_assurance_munaseb::{aujourdhui_au_burkina, solde_periode};
use crate::demande_remboursement_munaseb::StatutDemande;

#[derive(Debug, thiserror::Error)]
pub enum ErreurEspacePatient {
    /// Compte patient sans dossier patient : incohérence du serveur, 500
    /// (même règle que `GET /api/patients/moi`, M3).
    #[error("erreur interne : {0}")]
    Interne(String),
}

impl From<sqlx::Error> for ErreurEspacePatient {
    fn from(erreur: sqlx::Error) -> Self {
        ErreurEspacePatient::Interne(erreur.to_string())
    }
}

/// Dossier patient du compte du jeton.
async fn patient_du_compte(pool: &PgPool, compte_id: Uuid) -> Result<Uuid, ErreurEspacePatient> {
    let patient_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT p.id FROM compte c JOIN patient p ON p.utilisateur_id = c.utilisateur_id \
         WHERE c.id = $1 AND c.type_compte = 'patient'",
    )
    .bind(compte_id)
    .fetch_optional(pool)
    .await?;
    patient_id.ok_or_else(|| ErreurEspacePatient::Interne(format!("compte patient sans dossier patient : {compte_id}")))
}

// ---------------------------------------------------------------------
// Droits (E3)
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EtatDroits {
    AucunContrat,
    Couvert,
    EnCarence,
    Expire,
    Suspendu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Periode {
    pub debut: NaiveDate,
    pub fin: NaiveDate,
}

/// Toujours renvoyé en 200 : avoir peu ou pas de droits est un état à
/// afficher, pas une erreur (E3). Les champs sans objet valent `null`.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct DroitsPatient {
    pub etat: EtatDroits,
    pub numero_carte: Option<String>,
    pub periode_en_cours: Option<Periode>,
    /// Période déjà payée qui commence plus tard (renouvellement d'avance,
    /// ou carence).
    pub periode_suivante: Option<Periode>,
    pub plafond_fcfa: Option<i32>,
    pub consomme_fcfa: Option<i32>,
    /// Indicatif : calculé à la lecture, sans verrou ; la validation le
    /// recalcule sous verrou.
    pub solde_fcfa: Option<i32>,
    /// En carence (§12 : jamais « expiré » pendant la carence).
    pub debut_couverture: Option<NaiveDate>,
    pub fin_derniere_periode: Option<NaiveDate>,
}

impl DroitsPatient {
    fn vide(etat: EtatDroits, numero_carte: Option<String>) -> Self {
        DroitsPatient {
            etat,
            numero_carte,
            periode_en_cours: None,
            periode_suivante: None,
            plafond_fcfa: None,
            consomme_fcfa: None,
            solde_fcfa: None,
            debut_couverture: None,
            fin_derniere_periode: None,
        }
    }
}

/// `GET /api/patients/munaseb/droits`. Même solde que celui que voit
/// l'agent (`solde_periode`, E4), même date du jour (celle du Burkina, lue
/// dans PostgreSQL, D9).
pub async fn droits(pool: &PgPool, compte_id: Uuid) -> Result<DroitsPatient, ErreurEspacePatient> {
    let patient_id = patient_du_compte(pool, compte_id).await?;
    let contrat: Option<(Uuid, String, String)> =
        sqlx::query_as("SELECT id, numero_carte, statut FROM contrat_assurance_munaseb WHERE patient_id = $1")
            .bind(patient_id)
            .fetch_optional(pool)
            .await?;
    let Some((contrat_id, numero_carte, statut)) = contrat else {
        return Ok(DroitsPatient::vide(EtatDroits::AucunContrat, None));
    };
    // Aucun motif ni règle de suspension n'existe encore (étape 5) : rien
    // d'autre que l'état.
    if statut != "actif" {
        return Ok(DroitsPatient::vide(EtatDroits::Suspendu, Some(numero_carte)));
    }

    let jour = aujourdhui_au_burkina(pool).await?;
    let en_cours: Option<(Uuid, NaiveDate, NaiveDate)> = sqlx::query_as(
        "SELECT id, date_debut, date_fin FROM periode_adhesion_munaseb \
         WHERE contrat_id = $1 AND $2 BETWEEN date_debut AND date_fin",
    )
    .bind(contrat_id)
    .bind(jour)
    .fetch_optional(pool)
    .await?;
    // Au plus une période d'avance (décision C2) : la plus proche suffit.
    let suivante: Option<Periode> = sqlx::query_as::<_, (NaiveDate, NaiveDate)>(
        "SELECT date_debut, date_fin FROM periode_adhesion_munaseb \
         WHERE contrat_id = $1 AND date_debut > $2 ORDER BY date_debut LIMIT 1",
    )
    .bind(contrat_id)
    .bind(jour)
    .fetch_optional(pool)
    .await?
    .map(|(debut, fin)| Periode { debut, fin });

    if let Some((periode_id, debut, fin)) = en_cours {
        let solde = solde_periode(pool, periode_id).await?;
        return Ok(DroitsPatient {
            periode_en_cours: Some(Periode { debut, fin }),
            periode_suivante: suivante,
            plafond_fcfa: Some(solde.plafond_fcfa),
            consomme_fcfa: Some(solde.consomme_fcfa),
            solde_fcfa: Some(solde.solde_fcfa),
            ..DroitsPatient::vide(EtatDroits::Couvert, Some(numero_carte))
        });
    }
    if let Some(suivante) = suivante {
        return Ok(DroitsPatient {
            periode_suivante: Some(suivante),
            debut_couverture: Some(suivante.debut),
            ..DroitsPatient::vide(EtatDroits::EnCarence, Some(numero_carte))
        });
    }
    let fin_derniere_periode: Option<NaiveDate> =
        sqlx::query_scalar("SELECT max(date_fin) FROM periode_adhesion_munaseb WHERE contrat_id = $1")
            .bind(contrat_id)
            .fetch_one(pool)
            .await?;
    Ok(DroitsPatient { fin_derniere_periode, ..DroitsPatient::vide(EtatDroits::Expire, Some(numero_carte)) })
}

// ---------------------------------------------------------------------
// Demandes (E5)
// ---------------------------------------------------------------------

/// Une demande telle que le patient la voit dans sa liste.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct DemandePatient {
    pub id: Uuid,
    pub statut: String,
    pub type_acte: String,
    pub date_acte: NaiveDate,
    /// Où l'acte a été fait : le patient le sait déjà.
    pub partenaire: String,
    pub montant_acte_fcfa: i32,
    /// Part MUNASEB demandée (taux ou forfait), figée à la création.
    pub montant_demande_fcfa: i32,
    /// Fixé à la validation ; `null` avant.
    pub montant_rembourse_fcfa: Option<i32>,
    pub date_demande: DateTime<Utc>,
    pub date_dernier_changement: DateTime<Utc>,
}

// Macro plutôt que `const` + `format!` : sqlx 0.9 n'accepte que des
// chaînes SQL littérales (même raison que `select_resume!`). Le filtre
// `c.patient_id = $1` est dans chaque requête (E2).
macro_rules! select_demande_patient {
    () => {
        "SELECT d.id, d.statut, d.type_acte, d.date_acte, pa.nom AS partenaire, d.montant_acte_fcfa, \
                d.montant_demande_fcfa, d.montant_rembourse_fcfa, d.date_demande, \
                COALESCE((SELECT max(h.date_changement) FROM historique_statut_demande h WHERE h.demande_id = d.id), \
                         d.date_demande) AS date_dernier_changement \
         FROM demande_remboursement_munaseb d \
         JOIN partenaire_sante_munaseb pa ON pa.id = d.partenaire_id \
         JOIN periode_adhesion_munaseb pe ON pe.id = d.periode_id \
         JOIN contrat_assurance_munaseb c ON c.id = pe.contrat_id \
         WHERE c.patient_id = $1"
    };
}

/// `GET /api/patients/munaseb/demandes` : les plus récentes d'abord,
/// pagination par curseur `avant` (identifiant `uuidv7`, comme la liste des
/// invitations). Un patient sans contrat reçoit une liste vide.
pub async fn lister_demandes(
    pool: &PgPool,
    compte_id: Uuid,
    statut: Option<StatutDemande>,
    avant: Option<Uuid>,
    limite: i64,
) -> Result<Vec<DemandePatient>, ErreurEspacePatient> {
    let patient_id = patient_du_compte(pool, compte_id).await?;
    Ok(sqlx::query_as(concat!(
        select_demande_patient!(),
        " AND ($2::text IS NULL OR d.statut = $2) AND ($3::uuid IS NULL OR d.id < $3) ORDER BY d.id DESC LIMIT $4"
    ))
    .bind(patient_id)
    .bind(statut.map(StatutDemande::as_str))
    .bind(avant)
    .bind(limite)
    .fetch_all(pool)
    .await?)
}

/// Une étape de l'historique : statut et date seulement, jamais l'agent.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct EtapeDemande {
    pub statut: String,
    pub date: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct DetailDemandePatient {
    #[serde(flatten)]
    pub demande: DemandePatient,
    /// Seulement dans le détail : le patient doit savoir pourquoi pour
    /// contester ; jamais dans une notification (§12).
    pub motif_rejet: Option<String>,
    pub periode: Periode,
    /// Validée (ou payée) pour moins que la part demandée : le plafond de
    /// la période était atteint.
    pub plafond_atteint: bool,
    pub historique: Vec<EtapeDemande>,
}

/// `GET /api/patients/munaseb/demandes/{id}` : `None` pour une demande
/// inexistante comme pour celle d'un autre patient (E2, jamais
/// distinguées).
pub async fn detail_demande(
    pool: &PgPool,
    compte_id: Uuid,
    demande_id: Uuid,
) -> Result<Option<DetailDemandePatient>, ErreurEspacePatient> {
    let patient_id = patient_du_compte(pool, compte_id).await?;
    let Some(demande) = sqlx::query_as::<_, DemandePatient>(concat!(select_demande_patient!(), " AND d.id = $2"))
        .bind(patient_id)
        .bind(demande_id)
        .fetch_optional(pool)
        .await?
    else {
        return Ok(None);
    };
    // La demande appartient au patient (contrôlé ci-dessus) : les lectures
    // suivantes portent sur elle seule.
    let (motif_rejet, debut, fin): (Option<String>, NaiveDate, NaiveDate) = sqlx::query_as(
        "SELECT d.motif_rejet, pe.date_debut, pe.date_fin \
         FROM demande_remboursement_munaseb d JOIN periode_adhesion_munaseb pe ON pe.id = d.periode_id \
         WHERE d.id = $1",
    )
    .bind(demande_id)
    .fetch_one(pool)
    .await?;
    let historique = sqlx::query_as::<_, EtapeDemande>(
        "SELECT statut_nouveau AS statut, date_changement AS date FROM historique_statut_demande \
         WHERE demande_id = $1 ORDER BY date_changement, id",
    )
    .bind(demande_id)
    .fetch_all(pool)
    .await?;
    let plafond_atteint = matches!(demande.statut.as_str(), "valide" | "paye")
        && demande.montant_rembourse_fcfa.is_some_and(|m| m < demande.montant_demande_fcfa);
    Ok(Some(DetailDemandePatient { demande, motif_rejet, periode: Periode { debut, fin }, plafond_atteint, historique }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demande_remboursement_munaseb::{self as remboursement, Acte, AffectationAgent, TypeActeMunaseb};
    use crate::nip;

    // Base temporaire par test ; données fictives.

    /// Complète 15 chiffres par le chiffre de contrôle accepté (préfixe 9 :
    /// hors de la séquence réelle).
    fn nip_valide(payload: &str) -> String {
        (0..=9).map(|d| format!("{payload}{d}")).find(|n| nip::valider(n)).unwrap()
    }

    /// Compte patient et dossier patient. Renvoie (compte, patient, NIP).
    async fn patient(pool: &PgPool, telephone: &str, payload_nip: &str) -> (Uuid, Uuid, String) {
        let nip = nip_valide(payload_nip);
        let u: Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (nom, prenom, telephone) VALUES ('OUEDRAOGO', 'Awa', $1) RETURNING id",
        )
        .bind(telephone)
        .fetch_one(pool)
        .await
        .unwrap();
        let compte: Uuid = sqlx::query_scalar(
            "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'patient', 'x') RETURNING id",
        )
        .bind(u)
        .fetch_one(pool)
        .await
        .unwrap();
        let patient_id: Uuid = sqlx::query_scalar(
            "INSERT INTO patient (utilisateur_id, nip, date_naissance, lieu_naissance) \
             VALUES ($1, $2, '2000-01-01', 'Ouagadougou') RETURNING id",
        )
        .bind(u)
        .bind(&nip)
        .fetch_one(pool)
        .await
        .unwrap();
        (compte, patient_id, nip)
    }

    async fn contrat(pool: &PgPool, patient_id: Uuid, carte: &str) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO contrat_assurance_munaseb (patient_id, numero_carte, ufr, universite, num_matricule, personne_a_prevenir) \
             VALUES ($1, $2, 'UFR fictive', 'Université fictive', 'MAT-FICTIF', 'Personne fictive') RETURNING id",
        )
        .bind(patient_id)
        .bind(carte)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// Période posée en base (le contrat se crée encore en SQL, étape 5),
    /// décalée en jours par rapport à aujourd'hui.
    async fn periode(pool: &PgPool, contrat_id: Uuid, debut: i32, fin: i32) {
        sqlx::query(
            "INSERT INTO periode_adhesion_munaseb (contrat_id, date_paiement, date_debut, date_fin) \
             VALUES ($1, CURRENT_DATE + $2 - 40, CURRENT_DATE + $2, CURRENT_DATE + $3)",
        )
        .bind(contrat_id)
        .bind(debut)
        .bind(fin)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn partenaire(pool: &PgPool) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO partenaire_sante_munaseb (nom, type, ville, telephone) \
             VALUES ('Pharmacie fictive', 'pharmacie', 'Ouagadougou', '+22670020099') RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// Agent MUNASEB dans la structure MUNASEB validée (la base n'en admet
    /// qu'une : créée au premier appel).
    async fn agent(pool: &PgPool) -> AffectationAgent {
        let u: Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (nom, prenom, telephone) VALUES ('KABRE', 'Agent', '+22670020098') RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        let compte: Uuid = sqlx::query_scalar(
            "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'professionnel', 'x') RETURNING id",
        )
        .bind(u)
        .fetch_one(pool)
        .await
        .unwrap();
        let structure_id: Uuid = sqlx::query_scalar(
            "INSERT INTO structure (type_structure, nom, telephone, telephone_urgence, region, email, commune, province, \
                                    latitude, longitude, createur_compte_id, statut) \
             VALUES ('munaseb', 'MUNASEB', '+22670000000', '+22670000000', 'Région fictive', 'structure@example.org', \
                     'Commune fictive', 'Province fictive', 12.371400, -1.519700, $1, 'validee') RETURNING id",
        )
        .bind(compte)
        .fetch_one(pool)
        .await
        .unwrap();
        let affectation_id: Uuid = sqlx::query_scalar(
            "INSERT INTO affectation (compte_id, structure_id, role) VALUES ($1, $2, 'agent_assurance_munaseb') RETURNING id",
        )
        .bind(compte)
        .bind(structure_id)
        .fetch_one(pool)
        .await
        .unwrap();
        AffectationAgent { affectation_id, structure_id }
    }

    async fn demande(pool: &PgPool, nip: &str, partenaire_id: Uuid, reference: &str, montant: i32) -> Uuid {
        remboursement::creer_depuis_acte(
            pool,
            &Acte {
                nip: nip.to_string(),
                partenaire_id,
                type_acte: TypeActeMunaseb::Pharmacie,
                reference_acte: reference.to_string(),
                date_acte: Utc::now().date_naive(),
                montant_acte_fcfa: montant,
            },
        )
        .await
        .unwrap()
    }

    /// E3 : les cinq états des droits.
    #[sqlx::test]
    #[ignore]
    async fn cinq_etats_des_droits(pool: PgPool) {
        let (sans_contrat, _, _) = patient(&pool, "+22670020001", "900000000020001").await;
        assert_eq!(droits(&pool, sans_contrat).await.unwrap(), DroitsPatient::vide(EtatDroits::AucunContrat, None));

        // Couvert, avec un renouvellement déjà payé.
        let (couvert, p, _) = patient(&pool, "+22670020002", "900000000020002").await;
        let c = contrat(&pool, p, "CARTE-FICTIVE-2").await;
        periode(&pool, c, -30, 300).await;
        periode(&pool, c, 301, 665).await;
        let d = droits(&pool, couvert).await.unwrap();
        assert_eq!(d.etat, EtatDroits::Couvert);
        assert_eq!((d.plafond_fcfa, d.consomme_fcfa, d.solde_fcfa), (Some(100_000), Some(0), Some(100_000)));
        assert!(d.periode_en_cours.is_some() && d.periode_suivante.is_some());
        let json = serde_json::to_string(&d).unwrap();
        for interdit in ["fictive", "MAT-FICTIF", "Personne"] {
            assert!(!json.contains(interdit), "{interdit} dans {json}");
        }

        // En carence : jamais « expiré ».
        let (carence, p, _) = patient(&pool, "+22670020003", "900000000020003").await;
        let c = contrat(&pool, p, "CARTE-FICTIVE-3").await;
        periode(&pool, c, 10, 374).await;
        let d = droits(&pool, carence).await.unwrap();
        assert_eq!(d.etat, EtatDroits::EnCarence);
        assert_eq!(d.debut_couverture, d.periode_suivante.map(|s| s.debut));
        assert!(d.solde_fcfa.is_none());

        // Expiré.
        let (expire, p, _) = patient(&pool, "+22670020004", "900000000020004").await;
        let c = contrat(&pool, p, "CARTE-FICTIVE-4").await;
        periode(&pool, c, -400, -36).await;
        let d = droits(&pool, expire).await.unwrap();
        assert_eq!(d.etat, EtatDroits::Expire);
        assert!(d.fin_derniere_periode.is_some() && d.periode_en_cours.is_none());

        // Suspendu.
        let (suspendu, p, _) = patient(&pool, "+22670020005", "900000000020005").await;
        let c = contrat(&pool, p, "CARTE-FICTIVE-5").await;
        periode(&pool, c, -30, 300).await;
        sqlx::query("UPDATE contrat_assurance_munaseb SET statut = 'suspendu' WHERE id = $1")
            .bind(c)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            droits(&pool, suspendu).await.unwrap(),
            DroitsPatient::vide(EtatDroits::Suspendu, Some("CARTE-FICTIVE-5".into()))
        );
    }

    /// E4 : même solde pour l'agent et pour le patient ; E5 : détail, motif
    /// de rejet, plafond atteint, historique sans agent ; liste sans motif.
    #[sqlx::test]
    #[ignore]
    async fn demandes_et_solde_commun(pool: PgPool) {
        let (compte, p, nip) = patient(&pool, "+22670020011", "900000000020011").await;
        let c = contrat(&pool, p, "CARTE-FICTIVE-11").await;
        periode(&pool, c, -30, 300).await;
        let pa = partenaire(&pool).await;
        let ag = agent(&pool).await;

        // 120 000 FCFA à 80 % = 96 000 demandés, validés et payés.
        let d1 = demande(&pool, &nip, pa, "REF-FICTIVE-1", 120_000).await;
        remboursement::prendre_en_charge(&pool, d1, ag).await.unwrap();
        remboursement::valider(&pool, d1, ag).await.unwrap();
        remboursement::marquer_payee(&pool, d1, ag).await.unwrap();
        // 10 000 à 80 % = 8 000 demandés ; solde 4 000 : plafond atteint.
        let d2 = demande(&pool, &nip, pa, "REF-FICTIVE-2", 10_000).await;
        remboursement::prendre_en_charge(&pool, d2, ag).await.unwrap();
        assert_eq!(remboursement::valider(&pool, d2, ag).await.unwrap(), 4_000);
        let d3 = demande(&pool, &nip, pa, "REF-FICTIVE-3", 5_000).await;
        remboursement::rejeter(&pool, d3, ag, "Pièce manquante (fictif)").await.unwrap();

        let vu_par_le_patient = droits(&pool, compte).await.unwrap();
        let vu_par_l_agent = remboursement::detail_demande(&pool, d3).await.unwrap().unwrap();
        assert_eq!(vu_par_le_patient.solde_fcfa, Some(0));
        assert_eq!(vu_par_le_patient.consomme_fcfa, Some(100_000));
        assert_eq!(vu_par_le_patient.solde_fcfa, Some(vu_par_l_agent.solde_restant_periode_fcfa));

        let liste = lister_demandes(&pool, compte, None, None, 50).await.unwrap();
        assert_eq!(liste.iter().map(|d| d.id).collect::<Vec<_>>(), [d3, d2, d1], "les plus récentes d'abord");
        let json = serde_json::to_string(&liste).unwrap();
        for interdit in ["Pièce manquante", "REF-FICTIVE", "KABRE", &nip, "CARTE-FICTIVE"] {
            assert!(!json.contains(interdit), "{interdit} dans la liste");
        }

        let rejetee = detail_demande(&pool, compte, d3).await.unwrap().unwrap();
        assert_eq!(rejetee.motif_rejet.as_deref(), Some("Pièce manquante (fictif)"));
        assert!(!rejetee.plafond_atteint);
        let plafonnee = detail_demande(&pool, compte, d2).await.unwrap().unwrap();
        assert!(plafonnee.plafond_atteint);
        assert_eq!(
            plafonnee.historique.iter().map(|e| e.statut.as_str()).collect::<Vec<_>>(),
            ["en_attente", "en_cours", "valide"]
        );
        let json = serde_json::to_string(&plafonnee).unwrap();
        for interdit in ["KABRE", "Agent", "agent", "REF-FICTIVE", &nip] {
            assert!(!json.contains(interdit), "{interdit} dans le détail : {json}");
        }

        // Filtre et pagination par curseur.
        let rejetees = lister_demandes(&pool, compte, Some(StatutDemande::Rejete), None, 50).await.unwrap();
        assert_eq!(rejetees.len(), 1);
        let page1 = lister_demandes(&pool, compte, None, None, 2).await.unwrap();
        let page2 = lister_demandes(&pool, compte, None, Some(page1[1].id), 2).await.unwrap();
        assert_eq!(page2.iter().map(|d| d.id).collect::<Vec<_>>(), [d1]);
    }

    /// E2 : les demandes d'un autre patient n'existent pas pour lui ; un
    /// patient sans contrat a une liste vide.
    #[sqlx::test]
    #[ignore]
    async fn demandes_d_un_autre_patient_introuvables(pool: PgPool) {
        let (a, pa_id, nip_a) = patient(&pool, "+22670020021", "900000000020021").await;
        let c = contrat(&pool, pa_id, "CARTE-FICTIVE-21").await;
        periode(&pool, c, -30, 300).await;
        let (b, _, _) = patient(&pool, "+22670020022", "900000000020022").await;
        let pa = partenaire(&pool).await;
        let d = demande(&pool, &nip_a, pa, "REF-FICTIVE-21", 1_000).await;

        assert!(detail_demande(&pool, a, d).await.unwrap().is_some());
        assert!(detail_demande(&pool, b, d).await.unwrap().is_none());
        assert!(detail_demande(&pool, a, Uuid::nil()).await.unwrap().is_none());
        assert!(lister_demandes(&pool, b, None, None, 50).await.unwrap().is_empty());
    }
}
