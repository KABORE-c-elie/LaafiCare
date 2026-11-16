//! Migration 0028 (contrat MUNASEB tenu par les agents), vérifiée sur une
//! base qui contient déjà un contrat à deux périodes posées en SQL, comme
//! avant l'étape 5.
//!
//! Même méthode que `migration_0021.rs` (D7) : `#[sqlx::test(migrations =
//! false)]` donne une base vide, `Migrator::with_migrations` applique les
//! migrations jusqu'à la 0027, puis `run` applique le reste.
//!
//! Base temporaire par test ; données fictives. Lancement :
//! `cargo test --test migration_0028 -- --include-ignored`.

use chrono::NaiveDate;
use sqlx::migrate::Migrator;
use sqlx::types::Uuid;
use sqlx::PgPool;

static MIGRATIONS: Migrator = sqlx::migrate!();

#[sqlx::test(migrations = false)]
#[ignore]
async fn periodes_existantes_gardees(pool: PgPool) {
    let jusqu_a_0027: Vec<_> = MIGRATIONS.iter().filter(|m| m.version <= 27).cloned().collect();
    Migrator::with_migrations(jusqu_a_0027).run(&pool).await.unwrap();

    // Contrat d'avant l'étape 5 : deux périodes, sans numéro propre.
    let contrat: Uuid = sqlx::query_scalar(
        "WITH u AS (INSERT INTO utilisateur (nom, prenom, telephone) VALUES ('OUEDRAOGO', 'Awa', '+22670470001') RETURNING id), \
              p AS (INSERT INTO patient (utilisateur_id, nip, date_naissance, lieu_naissance) \
                    SELECT id, '0000000000000000', '2000-01-01', 'Ouagadougou' FROM u RETURNING id) \
         INSERT INTO contrat_assurance_munaseb (patient_id, numero_carte, ufr, universite, num_matricule) \
         SELECT id, 'ANCIENNE-0001', 'UFR', 'Université', 'MAT' FROM p RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    for (debut, fin) in [("2025-01-01", "2025-12-31"), ("2026-01-01", "2026-12-31")] {
        sqlx::query(
            "INSERT INTO periode_adhesion_munaseb (contrat_id, date_paiement, date_debut, date_fin) \
             VALUES ($1, $2::date, $2::date, $3::date)",
        )
        .bind(contrat)
        .bind(debut)
        .bind(fin)
        .execute(&pool)
        .await
        .unwrap();
    }

    MIGRATIONS.run(&pool).await.unwrap();

    // Carte d'origine sur la première période seulement ; plafond de
    // l'ancienne constante sur les deux.
    let periodes: Vec<(NaiveDate, Option<String>, i32)> = sqlx::query_as(
        "SELECT date_debut, numero_carte, plafond_fcfa FROM periode_adhesion_munaseb WHERE contrat_id = $1 ORDER BY date_debut",
    )
    .bind(contrat)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        periodes,
        [
            (NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(), Some("ANCIENNE-0001".to_string()), 100_000),
            (NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(), None, 100_000),
        ]
    );

    // Valeurs initiales des paramètres, une seule version sans auteur.
    let parametres: (i32, i32, i16, i16, i32, Option<Uuid>) = sqlx::query_as(
        "SELECT cotisation_fcfa, fiche_fcfa, delai_grace_jours, carence_mois, plafond_periode_fcfa, agent_affectation_id \
         FROM parametre_munaseb",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(parametres, (5000, 100, 10, 1, 100_000, None));
    assert!(
        sqlx::query("INSERT INTO parametre_munaseb (cotisation_fcfa, fiche_fcfa, delai_grace_jours, carence_mois, plafond_periode_fcfa) \
                     VALUES (1, 1, 1, 1, 1)")
            .execute(&pool)
            .await
            .is_err(),
        "une seule version sans auteur"
    );

    // Une nouvelle période doit avoir un numéro, même posée en SQL.
    assert!(
        sqlx::query(
            "INSERT INTO periode_adhesion_munaseb (contrat_id, date_paiement, date_debut, date_fin) \
             VALUES ($1, '2027-01-01', '2027-01-01', '2027-12-31')",
        )
        .bind(contrat)
        .execute(&pool)
        .await
        .is_err()
    );
}
