//! Migration 0022 (numéro d'urgence, région et email de la structure ;
//! emails sans espaces autour, en ASCII, de 254 octets au plus), vérifiée
//! sur des bases temporaires : les trois colonnes sont obligatoires, la
//! région n'est jamais vide, un email avec des espaces autour, accentué ou
//! trop long est refusé ; et la 0022 n'applique rien sur une base qui
//! contient déjà une structure (la base de développement repart de zéro).
//!
//! Base temporaire par test, jamais la base de développement : voir
//! « Tests et environnement », section 12 du CLAUDE.md. Données fictives.
//! Lancement : `cargo test --test migration_0022 -- --include-ignored`.

use sqlx::PgPool;
use sqlx::migrate::Migrator;
use sqlx::types::Uuid;

static MIGRATIONS: Migrator = sqlx::migrate!();

async fn compte_professionnel(pool: &PgPool, telephone: &str) -> Uuid {
    let utilisateur_id: Uuid = sqlx::query_scalar("INSERT INTO utilisateur (telephone) VALUES ($1) RETURNING id")
        .bind(telephone)
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

async fn structure_avec(
    pool: &PgPool,
    createur: Uuid,
    urgence: Option<&str>,
    region: Option<&str>,
    email: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO structure (type_structure, nom, telephone, telephone_urgence, region, email, commune, province, \
                                latitude, longitude, createur_compte_id) \
         VALUES ('munaseb', 'MUNASEB', '+22670170001', $2, $3, $4, 'Commune fictive', 'Province fictive', 12.3714, -1.5197, $1)",
    )
    .bind(createur)
    .bind(urgence)
    .bind(region)
    .bind(email)
    .execute(pool)
    .await
    .map(|_| ())
}

async fn structure(pool: &PgPool, createur: Uuid, urgence: Option<&str>, region: Option<&str>) -> Result<(), sqlx::Error> {
    structure_avec(pool, createur, urgence, region, Some("structure@example.org")).await
}

fn code(erreur: &sqlx::Error) -> Option<String> {
    erreur.as_database_error().and_then(|e| e.code()).map(|c| c.into_owned())
}

#[sqlx::test]
#[ignore]
async fn numero_d_urgence_et_region_obligatoires(pool: PgPool) {
    let createur = compte_professionnel(&pool, "+22670170002").await;

    // NOT NULL : SQLSTATE 23502 (doc PostgreSQL 18, annexe A).
    let erreur = structure(&pool, createur, None, Some("Région fictive")).await.unwrap_err();
    assert_eq!(code(&erreur).as_deref(), Some("23502"));
    let erreur = structure(&pool, createur, Some("+22670170001"), None).await.unwrap_err();
    assert_eq!(code(&erreur).as_deref(), Some("23502"));

    // Région vide ou faite d'espaces : CHECK.
    let erreur = structure(&pool, createur, Some("+22670170001"), Some("  ")).await.unwrap_err();
    assert_eq!(erreur.as_database_error().and_then(|e| e.constraint()), Some("structure_region_non_vide"));

    structure(&pool, createur, Some("+22670170001"), Some("Région fictive")).await.unwrap();
}

/// Email de la structure : obligatoire, non vide, sans espaces autour.
#[sqlx::test]
#[ignore]
async fn email_de_la_structure_obligatoire(pool: PgPool) {
    let createur = compte_professionnel(&pool, "+22670170009").await;
    let avec = |email: Option<&'static str>| structure_avec(&pool, createur, Some("+22670170001"), Some("Région fictive"), email);

    let erreur = avec(None).await.unwrap_err();
    assert_eq!(code(&erreur).as_deref(), Some("23502"));
    for email in ["", " contact@example.org", "contact@example.org "] {
        let erreur = avec(Some(email)).await.unwrap_err();
        assert_eq!(erreur.as_database_error().and_then(|e| e.constraint()), Some("structure_email_non_vide"), "{email:?}");
    }
    avec(Some("contact@example.org")).await.unwrap();
}

#[sqlx::test]
#[ignore]
async fn email_sans_espaces_autour(pool: PgPool) {
    for email in [" espace@example.org", "espace@example.org ", " espace@example.org "] {
        let erreur = sqlx::query("INSERT INTO utilisateur (telephone, email) VALUES ('+22670170003', $1)")
            .bind(email)
            .execute(&pool)
            .await
            .unwrap_err();
        assert_eq!(
            erreur.as_database_error().and_then(|e| e.constraint()),
            Some("utilisateur_email_sans_espaces_autour"),
            "{email:?}"
        );
    }
    // Sans email, ou avec un email propre : accepté.
    sqlx::query("INSERT INTO utilisateur (telephone) VALUES ('+22670170004')").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO utilisateur (telephone, email) VALUES ('+22670170005', 'propre@example.org')")
        .execute(&pool)
        .await
        .unwrap();
}

/// Email d'une identité et email d'une structure : ASCII seulement, 254
/// octets au plus (décisions du porteur du 2026-10-21).
#[sqlx::test]
#[ignore]
async fn emails_ascii_et_254_octets(pool: PgPool) {
    let trop_long = format!("{}@example.org", "a".repeat(243));
    assert_eq!(trop_long.len(), 255);
    let juste = format!("{}@example.org", "a".repeat(242));
    for (email, contrainte) in [("élodie@example.org", "utilisateur_email_ascii"), (trop_long.as_str(), "utilisateur_email_254_octets")] {
        let erreur = sqlx::query("INSERT INTO utilisateur (telephone, email) VALUES ('+22670170010', $1)")
            .bind(email)
            .execute(&pool)
            .await
            .unwrap_err();
        assert_eq!(erreur.as_database_error().and_then(|e| e.constraint()), Some(contrainte), "{email:?}");
    }
    sqlx::query("INSERT INTO utilisateur (telephone, email) VALUES ('+22670170011', $1)")
        .bind(&juste)
        .execute(&pool)
        .await
        .unwrap();

    let createur = compte_professionnel(&pool, "+22670170012").await;
    for (email, contrainte) in [("contact@exemplé.bf", "structure_email_ascii"), (trop_long.as_str(), "structure_email_254_octets")] {
        let erreur = structure_avec(&pool, createur, Some("+22670170001"), Some("Région fictive"), Some(email))
            .await
            .unwrap_err();
        assert_eq!(erreur.as_database_error().and_then(|e| e.constraint()), Some(contrainte), "{email:?}");
    }
    structure_avec(&pool, createur, Some("+22670170001"), Some("Région fictive"), Some(&juste)).await.unwrap();
}

/// Une structure déjà présente (sans numéro d'urgence, région ni email) :
/// la 0022 échoue et ne laisse rien (une migration = une transaction pour
/// SQLx) ; la version 22 n'est pas enregistrée. C'est pourquoi la base de
/// développement repart de zéro (décision du porteur, 2026-10-21).
#[sqlx::test(migrations = false)]
#[ignore]
async fn base_avec_structure_incomplete(pool: PgPool) {
    let jusqu_a_0021: Vec<_> = MIGRATIONS.iter().filter(|m| m.version <= 21).cloned().collect();
    Migrator::with_migrations(jusqu_a_0021).run(&pool).await.unwrap();
    let createur = compte_professionnel(&pool, "+22670170006").await;
    sqlx::query(
        "INSERT INTO structure (type_structure, nom, telephone, commune, province, latitude, longitude, createur_compte_id) \
         VALUES ('munaseb', 'MUNASEB', '+22670170007', 'Commune fictive', 'Province fictive', 12.3714, -1.5197, $1)",
    )
    .bind(createur)
    .execute(&pool)
    .await
    .unwrap();

    assert!(MIGRATIONS.run(&pool).await.is_err());
    let (version_22, colonne): (bool, bool) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM _sqlx_migrations WHERE version = 22), \
                EXISTS (SELECT 1 FROM information_schema.columns \
                        WHERE table_name = 'structure' AND column_name = 'telephone_urgence')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((version_22, colonne), (false, false), "rien n'est appliqué");
}
