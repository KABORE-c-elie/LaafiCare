//! Migration 0020 (jetons d'email, motif `desactivation_totp`, traces de la
//! création et de la reprise des administrateurs), vérifiée sur une base
//! neuve recréée depuis la 0001.
//!
//! Base temporaire par test, jamais la base de développement : voir
//! « Tests et environnement », section 12 du CLAUDE.md. Données fictives.
//! Lancement : `cargo test --test migration_0020 -- --include-ignored`.

use sqlx::PgPool;
use sqlx::types::Uuid;

fn contrainte(erreur: sqlx::Error) -> String {
    erreur
        .as_database_error()
        .and_then(|e| e.constraint().map(str::to_owned))
        .unwrap_or_else(|| format!("{erreur}"))
}

async fn compte(pool: &PgPool, telephone: &str, type_compte: &str) -> Uuid {
    let utilisateur_id: Uuid =
        sqlx::query_scalar("INSERT INTO utilisateur (telephone, email) VALUES ($1, $1 || '@example.org') RETURNING id")
            .bind(telephone)
            .fetch_one(pool)
            .await
            .unwrap();
    sqlx::query_scalar("INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, $2, 'x') RETURNING id")
        .bind(utilisateur_id)
        .bind(type_compte)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn jeton(pool: &PgPool, compte_id: Uuid, type_compte: &str, objet: &str, empreinte: u8) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO jeton_email (compte_id, compte_type, objet, empreinte, email_vise, expire_le) \
         VALUES ($1, $2, $3, $4, 'a@example.org', now() + interval '24 hours')",
    )
    .bind(compte_id)
    .bind(type_compte)
    .bind(objet)
    .bind(vec![empreinte; 32])
    .execute(pool)
    .await
    .map(|_| ())
}

#[sqlx::test]
#[ignore]
async fn jetons_d_email(pool: PgPool) {
    let patient = compte(&pool, "+22670150001", "patient").await;
    let professionnel = compte(&pool, "+22670150002", "professionnel").await;

    // Jamais pour un compte patient (deux contrôles : type annoncé, et type
    // réel par la clé étrangère composite).
    assert_eq!(contrainte(jeton(&pool, patient, "patient", "verification_email", 1).await.unwrap_err()), "jeton_email_type_compte");
    assert_eq!(
        contrainte(jeton(&pool, patient, "professionnel", "verification_email", 1).await.unwrap_err()),
        "jeton_email_compte"
    );
    // Objet inconnu.
    assert_eq!(
        contrainte(jeton(&pool, professionnel, "professionnel", "autre", 1).await.unwrap_err()),
        "jeton_email_objet_check"
    );

    jeton(&pool, professionnel, "professionnel", "verification_email", 1).await.unwrap();
    // Même empreinte : refusée ; second jeton de vérification en attente :
    // refusé tant que le premier n'est pas annulé.
    assert_eq!(
        contrainte(jeton(&pool, professionnel, "professionnel", "verification_email", 1).await.unwrap_err()),
        "jeton_email_empreinte_key"
    );
    assert_eq!(
        contrainte(jeton(&pool, professionnel, "professionnel", "verification_email", 2).await.unwrap_err()),
        "jeton_email_une_verification_en_attente"
    );
    sqlx::query("UPDATE jeton_email SET annule_le = now() WHERE compte_id = $1")
        .bind(professionnel)
        .execute(&pool)
        .await
        .unwrap();
    jeton(&pool, professionnel, "professionnel", "verification_email", 2).await.unwrap();

    // Jamais utilisé et annulé à la fois.
    let erreur = sqlx::query("UPDATE jeton_email SET utilise_le = now() WHERE annule_le IS NOT NULL")
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(contrainte(erreur), "jeton_email_utilise_ou_annule");
}

#[sqlx::test]
#[ignore]
async fn motif_desactivation_totp_accepte(pool: PgPool) {
    let compte_id = compte(&pool, "+22670150003", "patient").await;
    for motif in ["deconnexion", "email_non_verifie", "desactivation_totp"] {
        sqlx::query(
            "INSERT INTO session (compte_id, expire_le, revoquee_le, motif_revocation) \
             VALUES ($1, now() + interval '1 day', now(), $2)",
        )
        .bind(compte_id)
        .bind(motif)
        .execute(&pool)
        .await
        .unwrap();
    }
    let erreur = sqlx::query(
        "INSERT INTO session (compte_id, expire_le, revoquee_le, motif_revocation) \
         VALUES ($1, now() + interval '1 day', now(), 'inconnu')",
    )
    .bind(compte_id)
    .execute(&pool)
    .await
    .unwrap_err();
    assert_eq!(contrainte(erreur), "session_motif_revocation_check");
}

#[sqlx::test]
#[ignore]
async fn traces_des_administrateurs(pool: PgPool) {
    let premier = compte(&pool, "+22670150004", "administrateur_laaficare").await;
    let second = compte(&pool, "+22670150005", "administrateur_laaficare").await;
    let troisieme = compte(&pool, "+22670150006", "administrateur_laaficare").await;
    let professionnel = compte(&pool, "+22670150007", "professionnel").await;

    sqlx::query("INSERT INTO creation_administrateur (compte_id) VALUES ($1)")
        .bind(premier)
        .execute(&pool)
        .await
        .unwrap();
    // Un seul premier administrateur.
    let erreur = sqlx::query("INSERT INTO creation_administrateur (compte_id) VALUES ($1)")
        .bind(second)
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(contrainte(erreur), "creation_administrateur_un_seul_premier");
    sqlx::query("INSERT INTO creation_administrateur (compte_id, cree_par_compte_id) VALUES ($1, $2)")
        .bind(second)
        .bind(premier)
        .execute(&pool)
        .await
        .unwrap();
    // Jamais un compte non administrateur, ni comme créé, ni comme créateur.
    for (cree, par, attendue) in [
        (professionnel, Some(premier), "creation_administrateur_compte"),
        (troisieme, Some(professionnel), "creation_administrateur_cree_par"),
        (troisieme, Some(troisieme), "creation_administrateur_pas_soi_meme"),
    ] {
        let erreur = sqlx::query("INSERT INTO creation_administrateur (compte_id, cree_par_compte_id) VALUES ($1, $2)")
            .bind(cree)
            .bind(par)
            .execute(&pool)
            .await
            .unwrap_err();
        assert_eq!(contrainte(erreur), attendue);
    }

    sqlx::query("INSERT INTO reprise_administrateur (compte_id, repris_par_compte_id) VALUES ($1, $2)")
        .bind(second)
        .bind(premier)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO reprise_administrateur (compte_id) VALUES ($1)")
        .bind(premier)
        .execute(&pool)
        .await
        .unwrap();
    let erreur = sqlx::query("INSERT INTO reprise_administrateur (compte_id, repris_par_compte_id) VALUES ($1, $1)")
        .bind(second)
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(contrainte(erreur), "reprise_administrateur_pas_soi_meme");

    // Jamais modifiées ni supprimées.
    for requete in [
        "UPDATE creation_administrateur SET date_creation = now()",
        "DELETE FROM creation_administrateur",
        "TRUNCATE creation_administrateur",
        "UPDATE reprise_administrateur SET date_reprise = now()",
        "DELETE FROM reprise_administrateur",
        "TRUNCATE reprise_administrateur",
    ] {
        let erreur = sqlx::query(requete).execute(&pool).await.unwrap_err();
        assert!(format!("{erreur}").contains("jamais modifiées ni supprimées"), "{requete} : {erreur}");
    }
}
