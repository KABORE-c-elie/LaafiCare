//! Migration 0019 (fenêtre de réenrôlement d'un administrateur), vérifiée
//! sur une base neuve recréée depuis la 0001 : colonne vide à la création
//! d'un compte, autorisation refusée sur un compte patient ou
//! professionnel, acceptée sur un compte administrateur.
//!
//! Base temporaire par test, jamais la base de développement : voir
//! « Tests et environnement », section 12 du CLAUDE.md. Données fictives.
//! Lancement : `cargo test --test migration_0019 -- --include-ignored`.

use sqlx::PgPool;
use sqlx::types::Uuid;

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

/// Ouvre une fenêtre comme le fera l'outil local (étape 3.6.5).
async fn autoriser(pool: &PgPool, compte_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE compte SET reenrolement_autorise_jusqu_au = now() + interval '10 minutes' WHERE id = $1")
        .bind(compte_id)
        .execute(pool)
        .await
        .map(|_| ())
}

#[sqlx::test]
#[ignore]
async fn colonne_vide_a_la_creation(pool: PgPool) {
    let compte_id = compte(&pool, "+22670100001", "administrateur_laaficare").await;
    let valeur: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT reenrolement_autorise_jusqu_au FROM compte WHERE id = $1")
            .bind(compte_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(valeur, None);
}

#[sqlx::test]
#[ignore]
async fn autorisation_reservee_aux_administrateurs(pool: PgPool) {
    for (telephone, type_compte) in [("+22670100002", "patient"), ("+22670100003", "professionnel")] {
        let compte_id = compte(&pool, telephone, type_compte).await;
        let erreur = autoriser(&pool, compte_id).await.unwrap_err();
        assert_eq!(
            erreur.as_database_error().and_then(|e| e.constraint()),
            Some("compte_reenrolement_administrateur_seulement"),
            "{type_compte}"
        );
    }

    let administrateur = compte(&pool, "+22670100004", "administrateur_laaficare").await;
    autoriser(&pool, administrateur).await.unwrap();
    let ouverte: bool = sqlx::query_scalar("SELECT reenrolement_autorise_jusqu_au > now() FROM compte WHERE id = $1")
        .bind(administrateur)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(ouverte);
}
