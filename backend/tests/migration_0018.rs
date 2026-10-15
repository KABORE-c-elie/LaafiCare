//! Migration 0018 (jeton intermédiaire à usage unique), vérifiée sur une
//! base neuve recréée depuis la 0001 : colonne vide à la création d'un
//! compte, valeur positive exigée, et consommation conditionnelle telle que
//! l'écrit `auth_professionnel.rs` (une valeur ne peut que grandir).
//!
//! Base temporaire par test, jamais la base de développement : voir
//! « Tests et environnement », section 12 du CLAUDE.md. Données fictives.
//! Lancement : `cargo test --test migration_0018 -- --include-ignored`.

use sqlx::PgPool;
use sqlx::types::Uuid;

async fn compte(pool: &PgPool) -> Uuid {
    let utilisateur_id: Uuid = sqlx::query_scalar("INSERT INTO utilisateur (telephone) VALUES ('+22670080001') RETURNING id")
        .fetch_one(pool)
        .await
        .unwrap();
    sqlx::query_scalar(
        "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'professionnel', 'x') RETURNING id",
    )
    .bind(utilisateur_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Même requête que la consommation dans `auth_professionnel.rs`.
async fn consommer(pool: &PgPool, compte_id: Uuid, iat: i64) -> bool {
    sqlx::query_scalar::<_, Uuid>(
        "UPDATE compte SET second_facteur_dernier_iat = $2 \
         WHERE id = $1 AND (second_facteur_dernier_iat IS NULL OR second_facteur_dernier_iat < $2) RETURNING id",
    )
    .bind(compte_id)
    .bind(iat)
    .fetch_optional(pool)
    .await
    .unwrap()
    .is_some()
}

#[sqlx::test]
#[ignore]
async fn colonne_vide_a_la_creation_et_valeur_positive(pool: PgPool) {
    let compte_id = compte(&pool).await;
    let valeur: Option<i64> = sqlx::query_scalar("SELECT second_facteur_dernier_iat FROM compte WHERE id = $1")
        .bind(compte_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(valeur, None);

    for refusee in [0_i64, -1] {
        let erreur = sqlx::query("UPDATE compte SET second_facteur_dernier_iat = $2 WHERE id = $1")
            .bind(compte_id)
            .bind(refusee)
            .execute(&pool)
            .await
            .unwrap_err();
        assert_eq!(
            erreur.as_database_error().and_then(|e| e.constraint()),
            Some("compte_second_facteur_dernier_iat_positif")
        );
    }
}

#[sqlx::test]
#[ignore]
async fn la_consommation_ne_passe_qu_une_fois_par_iat(pool: PgPool) {
    let compte_id = compte(&pool).await;
    assert!(consommer(&pool, compte_id, 1_000).await, "premier jeton");
    assert!(!consommer(&pool, compte_id, 1_000).await, "le même jeton une seconde fois");
    assert!(!consommer(&pool, compte_id, 999).await, "un jeton plus ancien");
    assert!(consommer(&pool, compte_id, 1_001).await, "un jeton plus récent");
}
