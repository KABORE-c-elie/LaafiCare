//! Migration 0034 (index des clés étrangères et des listes paginées),
//! vérifiée sur une base neuve recréée depuis la 0001 :
//! - chaque index attendu existe ;
//! - aucune clé étrangère dont la table parente peut perdre des lignes
//!   (`session`, `second_facteur_totp`, `jeton_renouvellement`) n'est sans
//!   index : sans lui, PostgreSQL parcourt toute la table fille à chaque
//!   suppression (doc PostgreSQL 18, §5.5.5).
//!
//! Base temporaire par test, jamais la base de développement : voir
//! « Tests et environnement », section 12 du CLAUDE.md.
//! Lancement : `cargo test --test migration_0034 -- --include-ignored`.

use sqlx::PgPool;

#[sqlx::test]
#[ignore]
async fn index_presents(pool: PgPool) {
    for (table, index) in [
        ("code_secours", "code_secours_second_facteur_idx"),
        (
            "jeton_renouvellement",
            "jeton_renouvellement_remplace_par_idx",
        ),
        ("structure", "structure_createur_idx"),
        ("piece_justificative", "piece_justificative_structure_idx"),
        ("jeton_email", "jeton_email_compte_objet_idx"),
        ("session", "session_affectation_idx"),
        ("mouvement_stock", "mouvement_stock_structure_id_idx"),
        ("depense_pharmacie", "depense_pharmacie_structure_id_idx"),
    ] {
        let present: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_indexes WHERE schemaname = 'public' AND tablename = $1 AND indexname = $2)",
        )
        .bind(table)
        .bind(index)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(present, "{table}.{index}");
    }
}

/// Les tables dont des lignes s'effacent par le code (purge des sessions,
/// désactivation d'un TOTP) : toute clé étrangère qui les désigne a un index
/// complet (sans condition, ou dont la condition est « colonne non nulle »)
/// commençant par sa première colonne.
#[sqlx::test]
#[ignore]
async fn cles_vers_des_tables_effacees_indexees(pool: PgPool) {
    let sans_index: Vec<String> = sqlx::query_scalar(
        "SELECT c.conrelid::regclass::text || '.' || c.conname \
         FROM pg_constraint c \
         WHERE c.contype = 'f' AND c.connamespace = 'public'::regnamespace \
           AND c.confrelid::regclass::text IN ('session', 'second_facteur_totp', 'jeton_renouvellement') \
           AND NOT EXISTS ( \
               SELECT 1 FROM pg_index i \
               WHERE i.indrelid = c.conrelid AND i.indkey[0] = c.conkey[1] \
                 AND (i.indpred IS NULL OR pg_get_expr(i.indpred, i.indrelid) LIKE '%IS NOT NULL%'))",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(
        sans_index.is_empty(),
        "clés étrangères sans index : {sans_index:?}"
    );
}
