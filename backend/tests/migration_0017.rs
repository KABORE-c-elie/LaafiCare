//! Migration 0017 (motif de fermeture `email_non_verifie`), vérifiée sur une
//! base **neuve** : `#[sqlx::test]` crée une base temporaire et y applique
//! toutes les migrations depuis la 0001. Si la contrainte de la 0015 avait
//! reçu un autre nom automatique que `session_motif_revocation_check`, la
//! 0017 échouerait ici, et pas seulement dans une base de développement.
//!
//! Base temporaire par test, jamais la base de développement : voir
//! « Tests et environnement », section 12 du CLAUDE.md. Données fictives.
//! Lancement : `cargo test --test migration_0017 -- --include-ignored`.

use sqlx::PgPool;
use sqlx::types::Uuid;

const MOTIFS: [&str; 7] = [
    "deconnexion",
    "deconnexion_partout",
    "reinitialisation_mot_de_passe",
    "rejeu_detecte",
    "changement_affectation",
    "desactivation_compte",
    "email_non_verifie",
];

async fn session(pool: &PgPool, telephone: &str) -> Uuid {
    let utilisateur_id: Uuid = sqlx::query_scalar("INSERT INTO utilisateur (telephone) VALUES ($1) RETURNING id")
        .bind(telephone)
        .fetch_one(pool)
        .await
        .unwrap();
    let compte_id: Uuid = sqlx::query_scalar(
        "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'patient', 'x') RETURNING id",
    )
    .bind(utilisateur_id)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query_scalar("INSERT INTO session (compte_id, expire_le) VALUES ($1, now() + interval '1 day') RETURNING id")
        .bind(compte_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test]
#[ignore]
async fn la_contrainte_porte_son_nom_et_le_nouveau_motif(pool: PgPool) {
    let definition: String = sqlx::query_scalar(
        "SELECT pg_get_constraintdef(oid) FROM pg_constraint \
         WHERE conrelid = 'session'::regclass AND conname = 'session_motif_revocation_check'",
    )
    .fetch_one(&pool)
    .await
    .expect("la contrainte session_motif_revocation_check existe sur une base neuve");
    for motif in MOTIFS {
        assert!(definition.contains(&format!("'{motif}'")), "{motif} absent de : {definition}");
    }
    // Une seule contrainte sur ce motif : l'ancienne a bien été retirée.
    let nombre: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_constraint WHERE conrelid = 'session'::regclass \
         AND contype = 'c' AND pg_get_constraintdef(oid) LIKE '%motif_revocation = ANY%'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(nombre, 1);
}

#[sqlx::test]
#[ignore]
async fn chaque_motif_accepte_et_motif_inconnu_refuse(pool: PgPool) {
    for (i, motif) in MOTIFS.iter().enumerate() {
        let id = session(&pool, &format!("+2267007000{i}")).await;
        sqlx::query("UPDATE session SET revoquee_le = now(), motif_revocation = $2 WHERE id = $1")
            .bind(id)
            .bind(motif)
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("{motif} refusé : {e}"));
    }

    let id = session(&pool, "+22670070099").await;
    let erreur = sqlx::query("UPDATE session SET revoquee_le = now(), motif_revocation = 'motif_inconnu' WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(
        erreur.as_database_error().and_then(|e| e.constraint()),
        Some("session_motif_revocation_check")
    );
}
