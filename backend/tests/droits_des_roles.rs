//! Droits des rôles PostgreSQL du serveur et des outils (migration 0026,
//! P1, décision 7 du porteur, 2026-10-22).
//!
//! Les rôles `laaficare_serveur` et `laaficare_outils` doivent exister sur
//! le serveur PostgreSQL (`sql/roles.sql`, voir le README) : ils sont
//! communs à toutes les bases, donc aussi aux bases temporaires des tests.
//!
//! Deux parties :
//! - ce que le rôle du serveur ne peut PAS faire, et ce qu'il peut encore
//!   faire (commande `SET ROLE`, doc PostgreSQL 18 : la session prend les
//!   droits du rôle désigné) ;
//! - un parcours réel du serveur, exécuté avec ce rôle : inscription d'un
//!   patient, connexion, renouvellement, création et abandon d'une
//!   structure, demande de remboursement de bout en bout, espace patient,
//!   retrait d'un accès. Une table oubliée dans les droits ferait échouer
//!   ce test, alors que le reste de la suite tourne avec le rôle
//!   propriétaire.
//!
//! Base temporaire par test (`#[sqlx::test]`) ; données fictives.

use chrono::NaiveDate;
use sqlx::postgres::PgPoolOptions;
use sqlx::types::Uuid;
use sqlx::{Executor, PgPool};

use laaficare_backend::demande_remboursement_munaseb::{self as remboursement, Acte, AffectationAgent, TypeActeMunaseb};
use laaficare_backend::email::EmailSenderConsole;
use laaficare_backend::jwt::JwtService;
use laaficare_backend::structure::{self, NouvelleStructure, TypeStructure};
use laaficare_backend::{auth_patient, espace_patient_munaseb, membres, notification, session};

/// Code SQLSTATE « insufficient_privilege » (doc PostgreSQL 18, annexe A).
const REFUS: &str = "42501";

async fn refuse(conn: &mut sqlx::PgConnection, sql: &'static str) {
    let erreur = conn.execute(sql).await.expect_err(sql);
    let code = erreur.as_database_error().and_then(|e| e.code()).map(|c| c.into_owned());
    assert_eq!(code.as_deref(), Some(REFUS), "{sql} : {erreur}");
}

async fn permis(conn: &mut sqlx::PgConnection, sql: &'static str) {
    conn.execute(sql).await.unwrap_or_else(|e| panic!("{sql} : {e}"));
}

/// Le rôle du serveur ne peut ni modifier les traces, ni désactiver un
/// trigger, ni ouvrir une fenêtre de réenrôlement, ni créer de table, ni
/// écrire le journal des migrations ; il peut lire et écrire le reste,
/// prendre un numéro de NIP et vider une fenêtre par la fonction prévue. Le
/// rôle des outils ouvre la fenêtre, et n'a pas plus de droits sur les
/// traces.
#[sqlx::test]
#[ignore]
async fn ce_que_chaque_role_peut_faire(pool: PgPool) {
    let mut conn = pool.acquire().await.unwrap();
    permis(&mut conn, "SET ROLE laaficare_serveur").await;

    refuse(&mut conn, "UPDATE historique_statut_demande SET statut_nouveau = statut_nouveau WHERE false").await;
    refuse(&mut conn, "DELETE FROM historique_statut_demande WHERE false").await;
    refuse(&mut conn, "DELETE FROM decision_structure WHERE false").await;
    refuse(&mut conn, "UPDATE desactivation_totp SET motif = motif WHERE false").await;
    refuse(&mut conn, "DELETE FROM creation_administrateur WHERE false").await;
    refuse(&mut conn, "DELETE FROM reprise_administrateur WHERE false").await;
    refuse(&mut conn, "ALTER TABLE historique_statut_demande DISABLE TRIGGER ALL").await;
    refuse(&mut conn, "UPDATE compte SET reenrolement_autorise_jusqu_au = now() WHERE false").await;
    refuse(&mut conn, "CREATE TABLE intrus (i INTEGER)").await;
    refuse(&mut conn, "TRUNCATE notification").await;
    refuse(&mut conn, "DELETE FROM _sqlx_migrations WHERE false").await;
    // Trace des interventions de l'équipe (migration 0027) : le serveur la
    // lit seulement.
    refuse(
        &mut conn,
        "INSERT INTO intervention_compte (compte_id, action, administrateur_compte_id, motif) \
         SELECT id, 'blocage_compte', id, 'x' FROM compte WHERE false",
    )
    .await;
    permis(&mut conn, "SELECT count(*) FROM intervention_compte").await;

    permis(&mut conn, "SELECT count(*) FROM _sqlx_migrations").await;
    permis(&mut conn, "UPDATE compte SET statut = statut, version_jeton = version_jeton WHERE false").await;
    permis(&mut conn, "SELECT nextval('nip_seq')").await;
    permis(&mut conn, "SELECT vider_fenetre_reenrolement('00000000-0000-0000-0000-000000000000')").await;

    permis(&mut conn, "RESET ROLE").await;
    permis(&mut conn, "SET ROLE laaficare_outils").await;
    permis(&mut conn, "UPDATE compte SET reenrolement_autorise_jusqu_au = now() WHERE false").await;
    permis(
        &mut conn,
        "INSERT INTO intervention_compte (compte_id, action, administrateur_compte_id, motif) \
         SELECT id, 'blocage_compte', id, 'x' FROM compte WHERE false",
    )
    .await;
    refuse(&mut conn, "DELETE FROM intervention_compte WHERE false").await;
    refuse(&mut conn, "DELETE FROM historique_statut_demande WHERE false").await;
    refuse(&mut conn, "ALTER TABLE historique_statut_demande DISABLE TRIGGER ALL").await;
}

/// Pool de la même base temporaire, dont chaque connexion prend le rôle du
/// serveur (doc sqlx 0.9, `PoolOptions::after_connect`).
async fn pool_du_serveur(pool: &PgPool) -> PgPool {
    PgPoolOptions::new()
        .max_connections(3)
        .after_connect(|conn, _meta| {
            Box::pin(async move {
                conn.execute("SET ROLE laaficare_serveur").await?;
                Ok(())
            })
        })
        .connect_with((*pool.connect_options()).clone())
        .await
        .unwrap()
}

/// Un parcours réel du serveur, avec son rôle seulement.
#[sqlx::test]
#[ignore]
async fn parcours_du_serveur_avec_son_role(proprietaire: PgPool) {
    let serveur = pool_du_serveur(&proprietaire).await;
    let jwt = JwtService::new(&"a".repeat(32));
    let email = EmailSenderConsole;

    // Patient : code, inscription (NIP pris dans la séquence), connexion,
    // renouvellement.
    let (code, telephone) = auth_patient::demander_otp(&serveur, "70 04 00 01").await.unwrap();
    auth_patient::creer_compte(
        &serveur,
        &jwt,
        &telephone,
        &code,
        "OUEDRAOGO",
        "Awa",
        NaiveDate::from_ymd_opt(2000, 1, 1).unwrap(),
        "Ouagadougou",
        "Patient#2026",
    )
    .await
    .unwrap();
    let paire = auth_patient::se_connecter(&serveur, &jwt, &telephone, "Patient#2026").await.unwrap();
    session::renouveler(&serveur, paire.jeton_renouvellement.en_clair()).await.unwrap();
    let (compte_patient, nip): (Uuid, String) = sqlx::query_as(
        "SELECT c.id, p.nip FROM compte c JOIN patient p ON p.utilisateur_id = c.utilisateur_id \
         WHERE c.type_compte = 'patient'",
    )
    .fetch_one(&serveur)
    .await
    .unwrap();

    // Structure : création (compte professionnel, affectation, jeton
    // d'email), puis abandon.
    let creee = structure::creer(
        &serveur,
        &email,
        "http://localhost:4200",
        compte_patient,
        &NouvelleStructure {
            type_structure: TypeStructure::Munaseb,
            nom: "MUNASEB (test)",
            telephone: "70000001",
            telephone_urgence: "70000002",
            region: "Région fictive",
            province: "Province fictive",
            commune: "Commune fictive",
            parcelle: None,
            lot: None,
            section: None,
            latitude: 12.3714277,
            longitude: -1.5196603,
            email: "structure@example.org",
            email_professionnel: Some("awa.pro@example.org"),
            mot_de_passe_professionnel: "Professionnel#2026",
        },
    )
    .await
    .unwrap();
    structure::abandonner(&serveur, compte_patient, creee.structure_id).await.unwrap();

    // Préparation par le propriétaire : MUNASEB validée, responsable et
    // agent, partenaire, contrat et période du patient.
    let mut ids = Vec::new();
    for (telephone, type_compte) in [("+22670040002", "professionnel"), ("+22670040003", "professionnel")] {
        let u: Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (nom, prenom, telephone, email) VALUES ('ZONGO', 'Paul', $1, $2) RETURNING id",
        )
        .bind(telephone)
        .bind(format!("p{}@example.org", &telephone[4..]))
        .fetch_one(&proprietaire)
        .await
        .unwrap();
        sqlx::query("UPDATE utilisateur SET email_verifie_le = now() WHERE id = $1")
            .bind(u)
            .execute(&proprietaire)
            .await
            .unwrap();
        let c: Uuid = sqlx::query_scalar(
            "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, $2, 'x') RETURNING id",
        )
        .bind(u)
        .bind(type_compte)
        .fetch_one(&proprietaire)
        .await
        .unwrap();
        ids.push(c);
    }
    let (responsable, agent_compte) = (ids[0], ids[1]);
    let munaseb: Uuid = sqlx::query_scalar(
        "INSERT INTO structure (type_structure, nom, telephone, telephone_urgence, region, email, commune, province, \
                                latitude, longitude, createur_compte_id, statut) \
         VALUES ('munaseb', 'MUNASEB', '+22670000000', '+22670000000', 'Région fictive', 'structure@example.org', \
                 'Commune fictive', 'Province fictive', 12.371400, -1.519700, $1, 'validee') RETURNING id",
    )
    .bind(responsable)
    .fetch_one(&proprietaire)
    .await
    .unwrap();
    let mut affectations = Vec::new();
    for (compte, role) in [(responsable, "responsable"), (agent_compte, "agent_assurance_munaseb")] {
        let a: Uuid =
            sqlx::query_scalar("INSERT INTO affectation (compte_id, structure_id, role) VALUES ($1, $2, $3) RETURNING id")
                .bind(compte)
                .bind(munaseb)
                .bind(role)
                .fetch_one(&proprietaire)
                .await
                .unwrap();
        affectations.push(a);
    }
    let agent = AffectationAgent { affectation_id: affectations[1], structure_id: munaseb };
    let partenaire: Uuid = sqlx::query_scalar(
        "INSERT INTO partenaire_sante_munaseb (nom, type, ville, telephone) \
         VALUES ('Pharmacie fictive', 'pharmacie', 'Ouagadougou', '+22670040099') RETURNING id",
    )
    .fetch_one(&proprietaire)
    .await
    .unwrap();
    let contrat: Uuid = sqlx::query_scalar(
        "INSERT INTO contrat_assurance_munaseb (patient_id, numero_carte, ufr, universite, num_matricule) \
         SELECT id, '0001/2026', 'UFR fictive', 'Université fictive', 'MAT-FICTIF' FROM patient WHERE nip = $1 RETURNING id",
    )
    .bind(&nip)
    .fetch_one(&proprietaire)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO periode_adhesion_munaseb (contrat_id, date_paiement, date_debut, date_fin) \
         VALUES ($1, CURRENT_DATE - 30, CURRENT_DATE - 30, CURRENT_DATE + 300)",
    )
    .bind(contrat)
    .execute(&proprietaire)
    .await
    .unwrap();

    // Demande de bout en bout, avec le rôle du serveur.
    let demande = remboursement::creer_depuis_acte(
        &serveur,
        &Acte {
            nip: nip.clone(),
            partenaire_id: partenaire,
            type_acte: TypeActeMunaseb::Pharmacie,
            reference_acte: "REF-ROLE-1".into(),
            date_acte: chrono::Utc::now().date_naive(),
            montant_acte_fcfa: 10_000,
        },
    )
    .await
    .unwrap();
    remboursement::prendre_en_charge(&serveur, demande, agent).await.unwrap();
    assert_eq!(remboursement::valider(&serveur, demande, agent).await.unwrap(), 8_000);
    remboursement::marquer_payee(&serveur, demande, agent).await.unwrap();
    assert!(remboursement::detail_demande(&serveur, demande).await.unwrap().is_some());

    // Espace patient et notifications.
    let droits = espace_patient_munaseb::droits(&serveur, compte_patient).await.unwrap();
    assert_eq!(droits.solde_fcfa, Some(92_000));
    assert_eq!(espace_patient_munaseb::lister_demandes(&serveur, compte_patient, None, None, 50).await.unwrap().len(), 1);
    let (notifications, non_lues) = notification::lister_pour_patient(&serveur, compte_patient, None, 50).await.unwrap();
    assert!(non_lues > 0);
    assert!(notification::marquer_lue(&serveur, compte_patient, notifications[0].id).await.unwrap());
    notification::marquer_toutes_lues(&serveur, compte_patient).await.unwrap();

    // Retrait de l'accès de l'agent (affectation close, sessions fermées).
    session::ouvrir(&serveur, agent_compte, Some(agent.affectation_id)).await.unwrap();
    membres::desactiver(&serveur, &email, "securite@example.org", responsable, munaseb, agent.affectation_id)
        .await
        .unwrap();
}
