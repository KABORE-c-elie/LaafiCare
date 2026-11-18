mod erreur_api;
mod extracteur_jwt;
mod routes_acces_patient;
mod routes_administrateur;
mod routes_contrat_munaseb;
mod routes_espace_patient;
mod routes_invitation;
mod routes_membres;
mod routes_mot_de_passe;
mod routes_patient;
mod routes_pharmacie;
mod routes_professionnel;
mod routes_profil_professionnel;
mod routes_remboursement_munaseb;
mod routes_session;
mod routes_structure;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, MatchedPath};
use axum::http::HeaderName;
use axum::{Json, Router, extract::State, http::StatusCode, routing::get, routing::post};
use serde::Serialize;
use sqlx::PgPool;
use tower_http::request_id::{
    MakeRequestUuid, PropagateRequestIdLayer, RequestId, SetRequestIdLayer,
};
use tower_http::trace::TraceLayer;

use laaficare_backend::config::{self, CleTotp, Config, Programme};
use laaficare_backend::db;
use laaficare_backend::email::{EmailSender, EmailSenderConsole};
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
//
// Pas de `Debug`, volontairement (décision F6) : l'état contient la clé TOTP
// et le secret JWT. `cle_totp` derrière un `Arc` : une seule copie de la
// clé, partagée entre les requêtes, effacée de la mémoire à l'arrêt.
#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) db: PgPool,
    pub(crate) jwt: JwtService,
    pub(crate) sms: Arc<dyn SmsSender>,
    pub(crate) cle_totp: Arc<CleTotp>,
    /// Même principe que `sms` : `EmailSenderConsole` en développement.
    pub(crate) email: Arc<dyn EmailSender>,
    /// `URL_APPLICATION_WEB` (E5), base des liens envoyés par email.
    pub(crate) url_application_web: Arc<str>,
    /// `CONTACT_SECURITE` (D3), donné dans les alertes de sécurité.
    pub(crate) contact_securite: Arc<str>,
    /// Tâches de fond du mot de passe oublié, bornées (R11).
    pub(crate) taches_reinitialisation: Arc<laaficare_backend::reinitialisation::LimiteTaches>,
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

    let config = Config::from_env(Programme::Serveur).unwrap_or_else(|erreur| {
        tracing::error!(%erreur, "configuration invalide");
        std::process::exit(1)
    });

    // db::connecter a déjà journalisé la cause de chaque tentative et
    // l'abandon. Le message ci-dessous couvre les deux causes réelles vues en
    // test (base arrêtée, mauvais identifiants), car l'erreur de sqlx seule
    // (« pool timed out ») ne les distingue pas.
    let pool = db::connecter_avec(&config.database_url, &config.pool)
        .await
        .unwrap_or_else(|_| {
            tracing::error!(
                "Vérifier que PostgreSQL est démarré et que les identifiants dans .env sont corrects."
            );
            std::process::exit(1)
        });

    // Le serveur n'applique plus les migrations (P1, décision 5 du porteur,
    // 2026-10-22) : son rôle PostgreSQL n'en a pas le droit. Il refuse de
    // démarrer sur une base en retard, plutôt que d'échouer plus tard au
    // milieu d'une requête. Même traitement qu'une base injoignable.
    match db::migrations_manquantes(&pool).await {
        Ok(0) => tracing::info!("base à jour"),
        Ok(manquantes) => {
            tracing::error!(
                manquantes,
                "base pas à jour : lancer `cargo run --bin migrer`"
            );
            std::process::exit(1);
        }
        Err(erreur) => {
            tracing::error!(%erreur, "lecture du journal des migrations impossible");
            std::process::exit(1);
        }
    }

    let jwt = JwtService::new(&config.jwt_secret);
    let sms: Arc<dyn SmsSender> = Arc::new(sms::SmsSenderConsole);

    // B2 (décision du porteur, 2026-10-18) : en production, refus de
    // démarrer tant qu'un envoi est simulé.
    let email: Arc<dyn EmailSender> = Arc::new(EmailSenderConsole);
    if let Err(erreur) = config::verifier_envois(
        config.environnement,
        &[("SMS", sms.simule()), ("email", email.simule())],
    ) {
        tracing::error!(%erreur, "configuration invalide");
        std::process::exit(1);
    }

    let app = routeur(AppState {
        db: pool.clone(),
        jwt,
        sms,
        cle_totp: Arc::new(config.totp_cle_chiffrement),
        email,
        url_application_web: Arc::from(config.url_application_web),
        contact_securite: Arc::from(config.contact_securite),
        taches_reinitialisation: Arc::new(
            laaficare_backend::reinitialisation::LimiteTaches::default(),
        ),
    });

    // 127.0.0.1 par défaut (SERVER_HOST, `config.rs`) : cohérent avec
    // PostgreSQL exposé en local seulement. Une autre adresse est un choix
    // explicite de la configuration.
    let adresse = SocketAddr::from((config.server_host, config.server_port));
    let listener = tokio::net::TcpListener::bind(adresse)
        .await
        .unwrap_or_else(|erreur| {
            tracing::error!(%erreur, %adresse, "impossible d'écouter sur cette adresse");
            std::process::exit(1)
        });

    tracing::info!(%adresse, "serveur démarré");

    // Arrêt propre (remédiation de l'audit, reprise le 2026-11-17) : doc
    // d'axum 0.8.9, `serve::Serve::with_graceful_shutdown`, et exemple
    // officiel `examples/graceful-shutdown`. Au signal, le serveur n'accepte
    // plus de connexion et laisse finir les requêtes en cours : une vente,
    // une validation ou un paiement n'est pas coupé au milieu de sa
    // transaction.
    if let Err(erreur) = axum::serve(listener, app)
        .with_graceful_shutdown(signal_d_arret())
        .await
    {
        tracing::error!(%erreur, "le serveur s'est arrêté sur une erreur");
        std::process::exit(1);
    }

    // Connexions à PostgreSQL fermées proprement (doc de sqlx 0.9,
    // `Pool::close` : attend que les connexions empruntées soient rendues).
    pool.close().await;
    tracing::info!("serveur arrêté proprement");
}

/// Taille maximale du corps d'une requête, pour toutes les routes sauf le
/// dépôt d'une pièce justificative (remédiation de l'audit, reprise le
/// 2026-11-17 ; OWASP API Security Top 10 2023, API4 *Unrestricted Resource
/// Consumption*, « maximum allowed payload size »). Axum applique déjà
/// 2 Mo par défaut (doc d'axum 0.8.9, `DefaultBodyLimit`) ; la plus grosse
/// requête JSON de l'API (vente ou inventaire de 100 lignes) fait une dizaine
/// de Ko : 256 Kio laissent une large marge et réduisent ce qu'un appelant
/// peut faire lire au serveur. La route des pièces garde sa propre limite de
/// 5 Mio (`piece_justificative::TAILLE_MAX_OCTETS`) : la couche posée sur la
/// route est plus près du handler, elle l'emporte (même doc, « … can be
/// overridden for specific routes »).
const TAILLE_MAX_CORPS_OCTETS: usize = 256 * 1024;

/// Toutes les routes de l'API et leurs couches. Séparé de `main` pour que
/// les tests exercent exactement le routeur du serveur.
pub(crate) fn routeur(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route(
            "/api/assurance-munaseb/remboursements",
            get(routes_remboursement_munaseb::lister),
        )
        .route(
            "/api/assurance-munaseb/remboursements/simuler-acte",
            post(routes_remboursement_munaseb::simuler_acte),
        )
        .route(
            "/api/assurance-munaseb/remboursements/{id}",
            get(routes_remboursement_munaseb::detail),
        )
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
        .route(
            "/api/assurance-munaseb/parametres",
            get(routes_contrat_munaseb::parametres)
                .put(routes_contrat_munaseb::modifier_parametres),
        )
        .route(
            "/api/assurance-munaseb/parametres/historique",
            get(routes_contrat_munaseb::historique_parametres),
        )
        .route(
            "/api/assurance-munaseb/contrats",
            get(routes_contrat_munaseb::rechercher).post(routes_contrat_munaseb::adherer),
        )
        .route(
            "/api/assurance-munaseb/contrats/{id}",
            get(routes_contrat_munaseb::detail),
        )
        .route(
            "/api/assurance-munaseb/contrats/{id}/renouvellements",
            post(routes_contrat_munaseb::renouveler),
        )
        .route(
            "/api/patients/connexion",
            post(routes_patient::se_connecter),
        )
        .route("/api/patients/moi", get(routes_patient::moi))
        .route(
            "/api/patients/moi/sexe",
            axum::routing::put(routes_patient::modifier_sexe),
        )
        // Accès aux données d'un patient (module médecin, docs/api.md §2.18).
        .route(
            "/api/professionnels/patients/scan",
            post(routes_acces_patient::scanner),
        )
        .route(
            "/api/patients/journal-acces",
            get(routes_acces_patient::journal),
        )
        // Prise en charge (module médecin, docs/api.md §2.19).
        .route(
            "/api/patients/code-soin",
            post(routes_acces_patient::generer_code_soin),
        )
        .route(
            "/api/professionnels/patients/recherche",
            post(routes_acces_patient::rechercher),
        )
        .route("/api/prises-en-charge", post(routes_acces_patient::ouvrir))
        .route(
            "/api/prises-en-charge/bris-de-glace",
            post(routes_acces_patient::briser_la_glace),
        )
        .route(
            "/api/prises-en-charge/{id}/prolonger",
            post(routes_acces_patient::prolonger),
        )
        .route(
            "/api/structures/bris-de-glace",
            get(routes_acces_patient::lister_bris_de_glace),
        )
        .route(
            "/api/structures/bris-de-glace/{id}/controler",
            post(routes_acces_patient::controler),
        )
        // Profil professionnel (module médecin, docs/api.md §2.17).
        .route(
            "/api/professionnels/profil",
            get(routes_profil_professionnel::lire).put(routes_profil_professionnel::modifier),
        )
        .route(
            "/api/administrateurs/profils-professionnels",
            get(routes_profil_professionnel::lister_a_verifier),
        )
        .route(
            "/api/administrateurs/profils-professionnels/{compte_id}/verifier",
            post(routes_profil_professionnel::verifier),
        )
        .route("/api/sessions/renouveler", post(routes_session::renouveler))
        .route(
            "/api/sessions/deconnexion",
            post(routes_session::deconnexion),
        )
        .route(
            "/api/sessions/deconnexion-partout",
            post(routes_session::deconnexion_partout),
        )
        .route(
            "/api/professionnels/connexion",
            post(routes_professionnel::se_connecter),
        )
        .route(
            "/api/professionnels/connexion/second-facteur",
            post(routes_professionnel::second_facteur),
        )
        .route(
            "/api/professionnels/affectations",
            get(routes_professionnel::lister_affectations),
        )
        .route(
            "/api/professionnels/affectations/{id}/activer",
            post(routes_professionnel::activer_affectation),
        )
        .route(
            "/api/administrateurs/connexion",
            post(routes_administrateur::se_connecter),
        )
        .route(
            "/api/administrateurs/connexion/second-facteur",
            post(routes_administrateur::second_facteur),
        )
        .route(
            "/api/administrateurs/reenrolement/commencer",
            post(routes_administrateur::commencer_reenrolement),
        )
        .route(
            "/api/administrateurs/reenrolement/confirmer",
            post(routes_administrateur::confirmer_reenrolement),
        )
        .route("/api/patients/otp", post(routes_patient::demander_otp))
        .route(
            "/api/patients/inscription",
            post(routes_patient::creer_compte),
        )
        .route(
            "/api/patients/mot-de-passe/reinitialiser",
            post(routes_patient::reinitialiser_mot_de_passe),
        )
        .route(
            "/api/professionnels/email/verifier",
            post(routes_professionnel::verifier_email),
        )
        .route(
            "/api/professionnels/email/renvoyer",
            post(routes_professionnel::renvoyer_lien_email),
        )
        .route(
            "/api/administrateurs/structures",
            get(routes_administrateur::lister_structures),
        )
        .route(
            "/api/administrateurs/structures/{id}",
            get(routes_administrateur::detail_structure),
        )
        .route(
            "/api/administrateurs/structures/{id}/valider",
            post(routes_administrateur::valider_structure),
        )
        .route(
            "/api/administrateurs/structures/{id}/refuser",
            post(routes_administrateur::refuser_structure),
        )
        .route(
            "/api/professionnels/invitations",
            get(routes_invitation::lister).post(routes_invitation::creer),
        )
        .route(
            "/api/professionnels/invitations/{id}/annuler",
            post(routes_invitation::annuler),
        )
        .route(
            "/api/patients/invitations",
            get(routes_invitation::lister_recues),
        )
        .route(
            "/api/patients/invitations/{id}/accepter",
            post(routes_invitation::accepter),
        )
        .route(
            "/api/patients/invitations/{id}/refuser",
            post(routes_invitation::refuser),
        )
        .route(
            "/api/professionnels/mot-de-passe/oubli",
            post(routes_mot_de_passe::oubli_professionnel),
        )
        .route(
            "/api/administrateurs/mot-de-passe/oubli",
            post(routes_mot_de_passe::oubli_administrateur),
        )
        .route(
            "/api/mot-de-passe/reinitialiser",
            post(routes_mot_de_passe::reinitialiser),
        )
        .route("/api/structures", post(routes_structure::creer))
        .route(
            "/api/structures/mes-demandes",
            get(routes_structure::mes_demandes),
        )
        .route(
            "/api/structures/{id}/soumettre",
            post(routes_structure::soumettre),
        )
        .route(
            "/api/structures/{id}/abandonner",
            post(routes_structure::abandonner),
        )
        .route(
            "/api/structures/{id}/pieces",
            get(routes_structure::lister_pieces),
        )
        .route(
            "/api/structures/{id}/pieces/{type_piece}",
            // Limite de 5 Mio sur cette seule route (§14) ; les autres ont
            // celle du routeur (`TAILLE_MAX_CORPS_OCTETS`, 256 Kio).
            post(routes_structure::deposer_piece).layer(axum::extract::DefaultBodyLimit::max(
                laaficare_backend::piece_justificative::TAILLE_MAX_OCTETS,
            )),
        )
        .route(
            "/api/administrateurs/pieces/{id}/fichier",
            get(routes_administrateur::telecharger_piece),
        )
        .route("/api/professionnels/membres", get(routes_membres::lister))
        .route(
            "/api/professionnels/membres/{id}/desactiver",
            post(routes_membres::desactiver),
        )
        .route(
            "/api/patients/munaseb/droits",
            get(routes_espace_patient::droits),
        )
        .route(
            "/api/patients/munaseb/demandes",
            get(routes_espace_patient::lister_demandes),
        )
        .route(
            "/api/patients/munaseb/demandes/{id}",
            get(routes_espace_patient::detail_demande),
        )
        .route(
            "/api/patients/notifications",
            get(routes_espace_patient::lister_notifications),
        )
        .route(
            "/api/patients/notifications/lues",
            post(routes_espace_patient::marquer_toutes_lues),
        )
        .route(
            "/api/patients/notifications/{id}/lue",
            post(routes_espace_patient::marquer_lue),
        )
        // Pharmacie (CLAUDE.md §15, docs/api.md §2.15).
        .route(
            "/api/pharmacie/medicaments",
            get(routes_pharmacie::rechercher_medicaments),
        )
        .route(
            "/api/pharmacie/medicaments/{id}/equivalents",
            get(routes_pharmacie::equivalents),
        )
        .route("/api/pharmacie/stock", get(routes_pharmacie::etat))
        .route(
            "/api/pharmacie/stock/{medicament_id}",
            get(routes_pharmacie::lots),
        )
        .route("/api/pharmacie/entrees", post(routes_pharmacie::entrer))
        .route("/api/pharmacie/ventes", post(routes_pharmacie::vendre))
        .route("/api/pharmacie/ventes/{id}", get(routes_pharmacie::vente))
        .route(
            "/api/pharmacie/ventes/{id}/annuler",
            post(routes_pharmacie::annuler_vente),
        )
        .route(
            "/api/pharmacie/pertes",
            post(routes_pharmacie::declarer_perte),
        )
        .route(
            "/api/pharmacie/inventaires",
            post(routes_pharmacie::inventorier),
        )
        .route("/api/pharmacie/mouvements", get(routes_pharmacie::journal))
        .route(
            "/api/pharmacie/mouvements/{id}/annuler",
            post(routes_pharmacie::annuler_mouvement),
        )
        .route(
            "/api/pharmacie/prix/{medicament_id}",
            axum::routing::put(routes_pharmacie::fixer_prix),
        )
        .route(
            "/api/pharmacie/seuils/{medicament_id}",
            axum::routing::put(routes_pharmacie::fixer_seuils),
        )
        .route(
            "/api/pharmacie/reglages",
            axum::routing::put(routes_pharmacie::regler),
        )
        .route("/api/pharmacie/alertes", get(routes_pharmacie::alertes))
        .route(
            "/api/pharmacie/depenses",
            get(routes_pharmacie::lister_depenses).post(routes_pharmacie::enregistrer_depense),
        )
        .route(
            "/api/pharmacie/depenses/{id}/annuler",
            post(routes_pharmacie::annuler_depense),
        )
        .route("/api/pharmacie/rapport", get(routes_pharmacie::rapport))
        // Couches, de la plus intérieure à la plus extérieure : avec
        // `Router::layer`, la dernière ajoutée est la première traversée par
        // la requête (doc d'axum 0.8.9, module `middleware`, « Ordering »).
        // Ordre suivi par une requête : en-tête reçu retiré, identifiant
        // posé, journalisation, puis identifiant recopié dans la réponse.
        .layer(DefaultBodyLimit::max(TAILLE_MAX_CORPS_OCTETS))
        .layer(PropagateRequestIdLayer::new(ENTETE_REQUEST_ID))
        .layer(TraceLayer::new_for_http().make_span_with(span_de_requete))
        .layer(SetRequestIdLayer::new(ENTETE_REQUEST_ID, MakeRequestUuid))
        .layer(axum::middleware::map_request(retirer_request_id_recu))
        .with_state(state)
}

/// `x-request-id` : nom de l'en-tête utilisé par `tower-http` pour
/// `x_request_id()`.
const ENTETE_REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");

/// Identifiant de requête (remédiation de l'audit, reprise le 2026-11-17) :
/// un UUID tiré par le serveur (`MakeRequestUuid`, tower-http 0.6.11), rendu
/// dans l'en-tête `X-Request-ID` de la réponse et porté par toutes les
/// lignes de journal de la requête. Une personne qui signale une erreur
/// donne ce numéro : l'équipe retrouve la ligne sans qu'aucune donnée
/// personnelle n'ait été écrite.
///
/// `SetRequestIdLayer` garde l'en-tête s'il est déjà dans la requête
/// (source de tower-http 0.6.11, `request_id.rs`, `SetRequestId::call`) :
/// un client pourrait alors y mettre ce qu'il veut, un nom ou un numéro de
/// téléphone, et le faire écrire dans nos journaux. L'en-tête reçu est donc
/// toujours retiré : seul le serveur choisit l'identifiant.
async fn retirer_request_id_recu(mut requete: axum::extract::Request) -> axum::extract::Request {
    requete.headers_mut().remove(&ENTETE_REQUEST_ID);
    requete
}

/// Ce que le journal garde d'une requête : la méthode, le **modèle** de la
/// route (`/api/pharmacie/ventes/{id}`, `MatchedPath`, comme l'exemple de la
/// doc d'axum 0.8.9 pour `MatchedPath` avec `TraceLayer`) et l'identifiant.
/// **Jamais l'URI réelle** : le `make_span` par défaut de tower-http
/// l'enregistre (source 0.6.11, `trace/make_span.rs`), or elle contient des
/// identifiants et des paramètres comme `?numero_carte=` ou `?q=`. Une
/// adresse qui ne correspond à aucune route n'est pas recopiée non plus.
fn span_de_requete(requete: &axum::extract::Request) -> tracing::Span {
    let route = requete
        .extensions()
        .get::<MatchedPath>()
        .map(MatchedPath::as_str)
        .unwrap_or("route_inconnue");
    let request_id = requete
        .extensions()
        .get::<RequestId>()
        .and_then(|id| id.header_value().to_str().ok())
        .unwrap_or("");
    tracing::info_span!("requete", methode = %requete.method(), route, request_id)
}

/// Attend Ctrl+C, ou SIGTERM (le signal qu'envoient Docker et systemd pour
/// arrêter un service), comme l'exemple officiel d'axum. L'exemple fait
/// `expect` si l'écoute d'un signal ne peut pas être installée ; ici, pas de
/// panique (règle du démarrage, BNF-03) : l'erreur est journalisée et ce
/// signal-là est ignoré, l'autre reste écouté.
async fn signal_d_arret() {
    let ctrl_c = async {
        if let Err(erreur) = tokio::signal::ctrl_c().await {
            tracing::error!(%erreur, "écoute de Ctrl+C impossible");
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminaison = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(erreur) => {
                tracing::error!(%erreur, "écoute de SIGTERM impossible");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminaison = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminaison => {},
    }
    tracing::info!("signal d'arrêt reçu : fin des requêtes en cours");
}
