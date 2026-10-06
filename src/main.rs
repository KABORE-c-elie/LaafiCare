mod erreur_api;
mod extracteur_jwt;
mod routes_agent_assurance_munaseb;
mod routes_patient;
mod routes_remboursement_munaseb;

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use axum::{Json, Router, extract::State, http::StatusCode, routing::get, routing::post};
use serde::Serialize;
use sqlx::PgPool;

use laaficare_backend::config::Config;
use laaficare_backend::db;
use laaficare_backend::jwt::JwtService;
use laaficare_backend::sms::{self, SmsSender};

// La doc d'axum (`State`) recommande de regrouper l'état partagé dans une
// struct `Clone` plutôt que de passer des éléments séparés. `PgPool` et
// `JwtService` sont déjà des handles partageables : les cloner ne duplique
// ni les connexions ni la clé. `sms` est un objet trait (`dyn SmsSender`,
// derrière un `Arc` pour rester `Clone`) : l'implémentation réelle
// remplacera `SmsSenderConsole` sans toucher au reste du programme.
//
// `pub(crate)` sur la struct et ses champs : `routes_patient` (autre
// module du même binaire) doit pouvoir construire un handler `State<AppState>`
// et lire `state.db`/`state.jwt`/`state.sms`.
#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) db: PgPool,
    pub(crate) jwt: JwtService,
    pub(crate) sms: Arc<dyn SmsSender>,
}

// Struct typée plutôt qu'un `json!` en ligne : le compilateur fixe le format
// de la réponse.
#[derive(Serialize)]
struct HealthReponse {
    status: &'static str,
    database: &'static str,
}

// `/health` est un point d'infrastructure, pas une route métier : d'où la
// racine et non `/api/...`. Base injoignable = 503 : les superviseurs et
// répartiteurs de charge se fient au code HTTP, pas au corps. Le détail de
// l'erreur reste dans les logs, jamais dans la réponse (pas d'information
// interne exposée à l'appelant).
async fn health(State(state): State<AppState>) -> (StatusCode, Json<HealthReponse>) {
    match sqlx::query("select 1").execute(&state.db).await {
        Ok(_) => (
            StatusCode::OK,
            Json(HealthReponse {
                status: "ok",
                database: "up",
            }),
        ),
        Err(erreur) => {
            tracing::error!(%erreur, "health : la base ne répond pas");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(HealthReponse {
                    status: "error",
                    database: "down",
                }),
            )
        }
    }
}

// Tout échec au démarrage journalise puis sort en code 1 (jamais de panic ni
// de `unwrap`) : BNF-03, un superviseur (Docker, systemd) détecte le code non
// nul et peut relancer, alors qu'un serveur à moitié démarré resterait muet.
#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    // `dotenvy::dotenv()` renvoie une erreur si le fichier est absent (doc
    // dotenvy 0.15.7) : c'est normal hors développement, où les variables
    // viennent du vrai environnement. Un `.env` présent mais mal formé, lui,
    // est une erreur de configuration : on s'arrête plutôt que de l'ignorer.
    match dotenvy::dotenv() {
        Ok(chemin) => tracing::info!(?chemin, "fichier .env chargé"),
        // Pas de `%erreur` sur LineParse : son message reprend la ligne fautive,
        // qui peut contenir DATABASE_URL, donc le mot de passe de la base.
        Err(dotenvy::Error::LineParse(..)) => {
            tracing::error!("fichier .env mal formé (erreur de syntaxe), corriger le fichier");
            std::process::exit(1);
        }
        Err(erreur) if erreur.not_found() => {}
        Err(erreur) => {
            tracing::error!(%erreur, "fichier .env illisible");
            std::process::exit(1);
        }
    }

    let config = Config::from_env().unwrap_or_else(|erreur| {
        tracing::error!(%erreur, "configuration invalide");
        std::process::exit(1)
    });

    // db::connecter a déjà journalisé la cause de chaque tentative et
    // l'abandon. Le message ci-dessous couvre les deux causes réelles vues en
    // test (base arrêtée, mauvais identifiants), car l'erreur de sqlx seule
    // (« pool timed out ») ne les distingue pas.
    let pool = db::connecter(&config.database_url)
        .await
        .unwrap_or_else(|_| {
            tracing::error!(
                "Vérifier que PostgreSQL est démarré et que les identifiants dans .env sont corrects."
            );
            std::process::exit(1)
        });

    // `sqlx::migrate!()` intègre le SQL de `./migrations` à la compilation
    // (doc sqlx 0.9, macro.migrate.html) : aucun fichier requis au runtime,
    // le binaire compilé suffit. Un échec de migration est une erreur de
    // configuration au même titre qu'une base injoignable : même traitement
    // (log clair, sortie en code 1), jamais de panic.
    if let Err(erreur) = sqlx::migrate!().run(&pool).await {
        tracing::error!(%erreur, "échec des migrations de base de données");
        std::process::exit(1);
    }
    tracing::info!("migrations appliquées");

    let jwt = JwtService::new(&config.jwt_secret);
    let sms: Arc<dyn SmsSender> = Arc::new(sms::SmsSenderConsole);

    let app = Router::new()
        .route("/health", get(health))
        .route(
            "/api/assurance-munaseb/connexion",
            post(routes_agent_assurance_munaseb::se_connecter),
        )
        .route("/api/assurance-munaseb/remboursements", get(routes_remboursement_munaseb::lister))
        .route(
            "/api/assurance-munaseb/remboursements/simuler-acte",
            post(routes_remboursement_munaseb::simuler_acte),
        )
        .route("/api/assurance-munaseb/remboursements/{id}", get(routes_remboursement_munaseb::detail))
        .route(
            "/api/assurance-munaseb/remboursements/{id}/prendre-en-charge",
            post(routes_remboursement_munaseb::prendre_en_charge),
        )
        .route(
            "/api/assurance-munaseb/remboursements/{id}/valider",
            post(routes_remboursement_munaseb::valider),
        )
        .route(
            "/api/assurance-munaseb/remboursements/{id}/rejeter",
            post(routes_remboursement_munaseb::rejeter),
        )
        .route(
            "/api/assurance-munaseb/remboursements/{id}/payer",
            post(routes_remboursement_munaseb::payer),
        )
        .route("/api/patients/connexion", post(routes_patient::se_connecter))
        .route("/api/patients/otp", post(routes_patient::demander_otp))
        .route("/api/patients/inscription", post(routes_patient::creer_compte))
        .route(
            "/api/patients/mot-de-passe/reinitialiser",
            post(routes_patient::reinitialiser_mot_de_passe),
        )
        .with_state(AppState { db: pool, jwt, sms });

    // 127.0.0.1 : cohérent avec PostgreSQL exposé en local seulement. À
    // élargir quand l'app mobile aura besoin d'un accès depuis le réseau.
    let adresse = SocketAddr::from((Ipv4Addr::LOCALHOST, config.server_port));
    let listener = tokio::net::TcpListener::bind(adresse)
        .await
        .unwrap_or_else(|erreur| {
            tracing::error!(%erreur, %adresse, "impossible d'écouter sur cette adresse");
            std::process::exit(1)
        });

    tracing::info!(%adresse, "serveur démarré");

    // Exemple officiel d'axum::serve (docs.rs, axum 0.8.9).
    if let Err(erreur) = axum::serve(listener, app).await {
        tracing::error!(%erreur, "le serveur s'est arrêté sur une erreur");
        std::process::exit(1);
    }
}
