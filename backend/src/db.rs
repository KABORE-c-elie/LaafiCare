//! Connexion à PostgreSQL au démarrage, avec nouvelles tentatives.

use std::time::Duration;

use sqlx::PgPool;
use sqlx::migrate::Migrator;
use sqlx::postgres::PgPoolOptions;

// BNF-03 (reprise après incident) : PostgreSQL peut démarrer après le
// backend (redémarrage de la machine, Docker). Option A validée : 6 tentatives,
// délai doublé à chaque échec (1, 2, 4, 8, 16 s) = 31 s d'attente cumulée.
// Pas de jitter : un seul client au démarrage, donc rien à désynchroniser.
const MAX_TENTATIVES: u32 = 6;
const DELAI_INITIAL: Duration = Duration::from_secs(1);

// Explicites car la doc de sqlx 0.9 (PoolOptions) n'indique pas leurs valeurs
// par défaut. 5 connexions suffisent pour un développeur seul en local (le
// maximum de PostgreSQL est de 100) ; à réviser avant un déploiement
// multi-utilisateurs.
//
// DATABASE_URL pointe sur 127.0.0.1, jamais sur `localhost` (cause mesurée le
// 2026-09-25) : sous Windows, `localhost` se résout d'abord en `::1` (IPv6),
// or le docker-compose ne publie PostgreSQL que sur 127.0.0.1:5433. La
// tentative IPv6 est refusée au bout de ~2 s (2 087 ms mesurés) avant le
// repli sur IPv4 -- d'où le « slow acquire » de sqlx à chaque nouvelle
// connexion, et des tests `#[sqlx::test]` très lents.
// Les valeurs de dimensionnement et de timeouts sont définies dans `OptionsPool`.

// `PoolOptions::connect` ouvre une connexion immédiatement et renvoie l'erreur
// telle quelle si elle échoue : la doc de sqlx 0.9 ne décrit aucun mécanisme
// de retry. La boucle est donc écrite ici. Toutes les erreurs sont réessayées
// (décision validée) : quand PostgreSQL démarre, il répond par une erreur de
// base de données et pas par une erreur réseau, donc filtrer par type
// manquerait ce cas. Un mot de passe faux échoue plus lentement, mais jamais
// en silence : chaque tentative est journalisée avec sa cause.
/// Les migrations de `./migrations`, intégrées au programme à la compilation
/// (doc sqlx 0.9, `migrate!` : « static MIGRATOR: Migrator =
/// sqlx::migrate!(); »). Seule la commande `migrer` les applique (P1,
/// décision 5 du porteur, 2026-10-22) ; le serveur et les outils vérifient
/// seulement qu'elles le sont.
pub static MIGRATIONS: Migrator = sqlx::migrate!();

/// Vérifie que toutes les migrations du programme ont été appliquées avec
/// succès. Le serveur et les outils n'ont pas le droit d'écrire le journal
/// des migrations (migration 0026) : sans ce contrôle, ils démarreraient sur
/// une base en retard et échoueraient plus tard, au milieu d'une requête.
/// Renvoie le nombre de migrations manquantes.
pub async fn migrations_manquantes(pool: &PgPool) -> Result<usize, sqlx::Error> {
    let journal_present: bool = sqlx::query_scalar("SELECT to_regclass('public._sqlx_migrations') IS NOT NULL")
        .fetch_one(pool)
        .await?;
    if !journal_present {
        return Ok(MIGRATIONS.iter().count());
    }
    let appliquees: Vec<i64> = sqlx::query_scalar("SELECT version FROM _sqlx_migrations WHERE success")
        .fetch_all(pool)
        .await?;
    Ok(MIGRATIONS.iter().filter(|m| !appliquees.contains(&m.version)).count())
}

/// Options de dimensionnement et de timeouts du pool de connexions PostgreSQL (décision P1 / DBA).
#[derive(Debug, Clone)]
pub struct OptionsPool {
    pub max_connexions: u32,
    pub min_connexions: u32,
    pub acquire_timeout: Duration,
    pub idle_timeout: Duration,
    pub max_lifetime: Duration,
}

impl Default for OptionsPool {
    fn default() -> Self {
        Self {
            max_connexions: 10,
            min_connexions: 2,
            acquire_timeout: Duration::from_secs(5),
            idle_timeout: Duration::from_secs(600), // 10 minutes
            max_lifetime: Duration::from_secs(1800), // 30 minutes
        }
    }
}

pub async fn connecter(database_url: &str) -> Result<PgPool, sqlx::Error> {
    connecter_avec_options(database_url, OptionsPool::default()).await
}

pub async fn connecter_avec_options(database_url: &str, options: OptionsPool) -> Result<PgPool, sqlx::Error> {
    let mut tentative: u32 = 1;
    let mut delai = DELAI_INITIAL;

    loop {
        let resultat = PgPoolOptions::new()
            .max_connections(options.max_connexions)
            .min_connections(options.min_connexions)
            .acquire_timeout(options.acquire_timeout)
            .idle_timeout(options.idle_timeout)
            .max_lifetime(options.max_lifetime)
            .connect(database_url)
            .await;

        match resultat {
            Ok(pool) => {
                tracing::info!(
                    tentative,
                    max_conn = options.max_connexions,
                    min_conn = options.min_connexions,
                    "connexion à PostgreSQL établie"
                );
                return Ok(pool);
            }
            Err(erreur) if tentative >= MAX_TENTATIVES => {
                tracing::error!(
                    tentatives = MAX_TENTATIVES,
                    %erreur,
                    "connexion à PostgreSQL impossible, abandon"
                );
                return Err(erreur);
            }
            Err(erreur) => {
                tracing::warn!(
                    tentative,
                    sur = MAX_TENTATIVES,
                    prochaine_dans_secs = delai.as_secs(),
                    %erreur,
                    "connexion à PostgreSQL échouée, nouvelle tentative"
                );
                tokio::time::sleep(delai).await;
                tentative += 1;
                delai *= 2;
            }
        }
    }
}
