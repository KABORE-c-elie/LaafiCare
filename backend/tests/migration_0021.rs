//! Migration 0021 (historique des demandes rattaché à l'affectation de
//! l'agent, J8), vérifiée sur une base qui contient déjà des lignes
//! d'historique « à l'ancienne » (agent désigné par `agent_utilisateur_id`).
//!
//! Méthode (D7 du 2026-10-18) : `Migrator::run_to` est publique mais sans
//! documentation dans sqlx 0.9.0 ; on utilise donc des méthodes
//! documentées. `#[sqlx::test(migrations = false)]` donne une base vide
//! (doc de `sqlx::test` : « disable migrations processing entirely »),
//! `Migrator::with_migrations` (« Creates a new instance with the given
//! migrations ») applique les migrations jusqu'à la 0020, prises par
//! `Migrator::iter`, puis `run` applique le reste.
//!
//! Base temporaire par test, jamais la base de développement. Données
//! fictives. Lancement : `cargo test --test migration_0021 -- --include-ignored`.

use sqlx::migrate::Migrator;
use sqlx::types::Uuid;
use sqlx::PgPool;

static MIGRATIONS: Migrator = sqlx::migrate!();

fn contrainte(erreur: sqlx::Error) -> String {
    erreur
        .as_database_error()
        .and_then(|e| e.constraint().map(str::to_owned))
        .unwrap_or_else(|| format!("{erreur}"))
}

async fn valeur(pool: &PgPool, requete: &'static str) -> Uuid {
    sqlx::query_scalar(requete).fetch_one(pool).await.unwrap()
}

/// Une demande avec son historique d'avant la 0021 : création, puis
/// transition par un agent désigné par son identité.
async fn demande_a_l_ancienne(pool: &PgPool) -> (Uuid, Uuid) {
    let agent = valeur(pool, "INSERT INTO utilisateur (nom, prenom, telephone) VALUES ('Ancien', 'Agent', '+22670160001') RETURNING id").await;
    let personne = valeur(pool, "INSERT INTO utilisateur (nom, prenom, telephone) VALUES ('OUEDRAOGO', 'Awa', '+22670160002') RETURNING id").await;
    let patient: Uuid = sqlx::query_scalar(
        "INSERT INTO patient (utilisateur_id, nip, date_naissance, lieu_naissance) \
         VALUES ($1, '0000000000000000', '2000-01-01', 'Ouagadougou') RETURNING id",
    )
    .bind(personne)
    .fetch_one(pool)
    .await
    .unwrap();
    let contrat: Uuid = sqlx::query_scalar(
        "INSERT INTO contrat_assurance_munaseb (patient_id, numero_carte, ufr, universite, num_matricule) \
         VALUES ($1, 'TEST-0021', 'UFR', 'Université', 'M0021') RETURNING id",
    )
    .bind(patient)
    .fetch_one(pool)
    .await
    .unwrap();
    let periode: Uuid = sqlx::query_scalar(
        "INSERT INTO periode_adhesion_munaseb (contrat_id, date_paiement, date_debut, date_fin) \
         VALUES ($1, '2026-01-01', '2026-01-01', '2026-12-31') RETURNING id",
    )
    .bind(contrat)
    .fetch_one(pool)
    .await
    .unwrap();
    let partenaire = valeur(
        pool,
        "INSERT INTO partenaire_sante_munaseb (nom, type, ville, telephone) \
         VALUES ('Pharmacie de test', 'pharmacie', 'Ouagadougou', '+22670160003') RETURNING id",
    )
    .await;
    let demande: Uuid = sqlx::query_scalar(
        "INSERT INTO demande_remboursement_munaseb \
         (partenaire_id, type_acte, reference_acte, date_acte, montant_acte_fcfa, montant_demande_fcfa, periode_id) \
         VALUES ($1, 'pharmacie', 'REF-0021', '2026-06-01', 1000, 800, $2) RETURNING id",
    )
    .bind(partenaire)
    .bind(periode)
    .fetch_one(pool)
    .await
    .unwrap();
    for (precedent, agent_id) in [(None, None), (Some("en_attente"), Some(agent))] {
        sqlx::query(
            "INSERT INTO historique_statut_demande (demande_id, statut_precedent, statut_nouveau, agent_utilisateur_id) \
             VALUES ($1, $2, CASE WHEN $2 IS NULL THEN 'en_attente' ELSE 'en_cours' END, $3)",
        )
        .bind(demande)
        .bind(precedent)
        .bind(agent_id)
        .execute(pool)
        .await
        .unwrap();
    }
    (demande, agent)
}

/// Affectation d'agent MUNASEB dans une structure MUNASEB, pour une
/// nouvelle transition.
async fn affectation(pool: &PgPool, role: &str) -> (Uuid, Uuid) {
    let identite = valeur(pool, "INSERT INTO utilisateur (telephone) VALUES ('+22670160004') RETURNING id").await;
    let compte: Uuid = sqlx::query_scalar(
        "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'professionnel', 'x') RETURNING id",
    )
    .bind(identite)
    .fetch_one(pool)
    .await
    .unwrap();
    let structure: Uuid = sqlx::query_scalar(
        "INSERT INTO structure (type_structure, nom, telephone, telephone_urgence, region, email, commune, province, latitude, longitude, \
                                createur_compte_id, statut) \
         VALUES ('munaseb', 'MUNASEB', '+22670160005', '+22670160005', 'Région fictive', 'structure@example.org', 'Ouagadougou', 'Kadiogo', \
                 12.371400, -1.519700, $1, 'validee') RETURNING id",
    )
    .bind(compte)
    .fetch_one(pool)
    .await
    .unwrap();
    let affectation: Uuid = sqlx::query_scalar(
        "INSERT INTO affectation (compte_id, structure_id, role) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(compte)
    .bind(structure)
    .bind(role)
    .fetch_one(pool)
    .await
    .unwrap();
    (affectation, structure)
}

async fn transition(
    pool: &PgPool,
    demande: Uuid,
    agent_utilisateur: Option<Uuid>,
    affectation: Option<Uuid>,
    structure: Option<Uuid>,
    role: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO historique_statut_demande \
         (demande_id, statut_precedent, statut_nouveau, agent_utilisateur_id, agent_affectation_id, agent_structure_id, agent_role) \
         VALUES ($1, 'en_cours', 'valide', $2, $3, $4, $5)",
    )
    .bind(demande)
    .bind(agent_utilisateur)
    .bind(affectation)
    .bind(structure)
    .bind(role)
    .execute(pool)
    .await
    .map(|_| ())
}

#[sqlx::test(migrations = false)]
#[ignore]
async fn anciennes_lignes_gardees_nouvelles_par_affectation(pool: PgPool) {
    let jusqu_a_0020: Vec<_> = MIGRATIONS.iter().filter(|m| m.version <= 20).cloned().collect();
    Migrator::with_migrations(jusqu_a_0020).run(&pool).await.unwrap();
    let (demande, ancien_agent) = demande_a_l_ancienne(&pool).await;

    MIGRATIONS.run(&pool).await.unwrap();

    // Les anciennes lignes restent telles quelles (NOT VALID).
    let lignes: Vec<(Option<Uuid>, Option<Uuid>)> = sqlx::query_as(
        "SELECT agent_utilisateur_id, agent_affectation_id FROM historique_statut_demande \
         WHERE demande_id = $1 ORDER BY date_changement, id",
    )
    .bind(demande)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(lignes, [(None, None), (Some(ancien_agent), None)]);

    // Une nouvelle transition à l'ancienne, sans affectation : refusée.
    assert_eq!(
        contrainte(transition(&pool, demande, Some(ancien_agent), None, None, None).await.unwrap_err()),
        "transition_par_affectation"
    );
    // Affectation incomplète : refusée (MATCH FULL).
    let (affectation_id, structure_id) = affectation(&pool, "agent_assurance_munaseb").await;
    assert_eq!(
        contrainte(transition(&pool, demande, None, Some(affectation_id), None, None).await.unwrap_err()),
        "historique_agent_affectation"
    );
    // Un autre rôle : refusé.
    assert_eq!(
        contrainte(
            transition(&pool, demande, None, Some(affectation_id), Some(structure_id), Some("pharmacien"))
                .await
                .unwrap_err()
        ),
        "historique_agent_role_munaseb"
    );
    // Une création ne porte aucun agent.
    let erreur = sqlx::query(
        "INSERT INTO historique_statut_demande (demande_id, statut_nouveau, agent_affectation_id, agent_structure_id, agent_role) \
         VALUES ($1, 'en_attente', $2, $3, 'agent_assurance_munaseb')",
    )
    .bind(demande)
    .bind(affectation_id)
    .bind(structure_id)
    .execute(&pool)
    .await
    .unwrap_err();
    assert_eq!(contrainte(erreur), "agent_si_transition");
    // Transition complète : acceptée.
    transition(&pool, demande, None, Some(affectation_id), Some(structure_id), Some("agent_assurance_munaseb"))
        .await
        .unwrap();
}
