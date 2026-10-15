//! Garanties apportées par la base elle-même dans la migration 0016
//! (vérification de l'email d'une identité) : une identité ne naît jamais
//! vérifiée, tout changement d'email annule la vérification, et une date de
//! vérification exige un email. Les lignes sont posées directement en SQL ;
//! la vérification se pose toujours par un UPDATE, comme le fera le code.
//!
//! Base temporaire par test, jamais la base de développement : voir
//! « Tests et environnement », section 12 du CLAUDE.md. Données fictives.
//! Lancement : `cargo test --test migration_0016 -- --include-ignored`.

use sqlx::PgPool;
use sqlx::types::Uuid;

async fn creer_utilisateur(pool: &PgPool, telephone: &str, email: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO utilisateur (telephone, email) VALUES ($1, $2) RETURNING id")
        .bind(telephone)
        .bind(email)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn verifier(pool: &PgPool, utilisateur_id: Uuid) {
    sqlx::query("UPDATE utilisateur SET email_verifie_le = now() WHERE id = $1")
        .bind(utilisateur_id)
        .execute(pool)
        .await
        .unwrap();
}

async fn est_verifie(pool: &PgPool, utilisateur_id: Uuid) -> bool {
    sqlx::query_scalar("SELECT email_verifie_le IS NOT NULL FROM utilisateur WHERE id = $1")
        .bind(utilisateur_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test]
#[ignore]
async fn une_identite_ne_nait_jamais_verifiee(pool: PgPool) {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO utilisateur (telephone, email, email_verifie_le) \
         VALUES ('+22670040001', 'pro.test@example.org', now()) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!est_verifie(&pool, id).await, "la date fournie à la création est ignorée");

    // La vérification se pose ensuite par un UPDATE.
    verifier(&pool, id).await;
    assert!(est_verifie(&pool, id).await);
}

#[sqlx::test]
#[ignore]
async fn changer_l_email_annule_la_verification(pool: PgPool) {
    let id = creer_utilisateur(&pool, "+22670040002", "ancien.test@example.org").await;

    // Changement d'adresse.
    verifier(&pool, id).await;
    sqlx::query("UPDATE utilisateur SET email = 'nouveau.test@example.org' WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(!est_verifie(&pool, id).await, "nouvelle adresse");

    // Un simple changement de majuscules compte aussi.
    verifier(&pool, id).await;
    sqlx::query("UPDATE utilisateur SET email = 'Nouveau.Test@example.org' WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(!est_verifie(&pool, id).await, "majuscules");

    // Changer l'email et poser la vérification dans la même mise à jour :
    // la vérification est annulée, elle doit suivre le changement.
    sqlx::query("UPDATE utilisateur SET email = 'autre.test@example.org', email_verifie_le = now() WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(!est_verifie(&pool, id).await, "changement et vérification ensemble");
}

#[sqlx::test]
#[ignore]
async fn la_verification_reste_sans_changement_d_email(pool: PgPool) {
    let id = creer_utilisateur(&pool, "+22670040003", "stable.test@example.org").await;
    verifier(&pool, id).await;

    // Une autre colonne modifiée.
    sqlx::query("UPDATE utilisateur SET nom = 'OUEDRAOGO' WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(est_verifie(&pool, id).await, "autre colonne");

    // L'email réécrit à l'identique : le trigger se déclenche (UPDATE OF
    // email) mais IS DISTINCT FROM ne voit aucun changement.
    sqlx::query("UPDATE utilisateur SET email = email WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(est_verifie(&pool, id).await, "réécriture à l'identique");
}

#[sqlx::test]
#[ignore]
async fn une_verification_exige_un_email(pool: PgPool) {
    let id: Uuid = sqlx::query_scalar("INSERT INTO utilisateur (telephone) VALUES ('+22670040004') RETURNING id")
        .fetch_one(&pool)
        .await
        .unwrap();
    let erreur = sqlx::query("UPDATE utilisateur SET email_verifie_le = now() WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(
        erreur.as_database_error().and_then(|e| e.constraint()),
        Some("utilisateur_email_verifie_avec_email")
    );

    // Retirer l'email annule la vérification (le trigger agit avant le
    // CHECK) : l'écriture passe, sans vérification restante.
    let autre = creer_utilisateur(&pool, "+22670040005", "retire.test@example.org").await;
    verifier(&pool, autre).await;
    sqlx::query("UPDATE utilisateur SET email = NULL WHERE id = $1")
        .bind(autre)
        .execute(&pool)
        .await
        .unwrap();
    assert!(!est_verifie(&pool, autre).await);
}
