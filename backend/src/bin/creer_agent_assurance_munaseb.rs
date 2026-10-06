//! Outil local de création de compte `agent_assurance_munaseb` (section 12
//! du CLAUDE.md). Volontairement **pas une route HTTP** : le mémoire MUNASEB
//! confirme que la création de comptes agents est une fonction
//! d'Administrateur ("Création et modification des comptes agents ;
//! Attribution et modification des rôles et permissions ; Activation/
//! désactivation de comptes", lignes 843-847) -- jamais une auto-inscription
//! publique. LaafiCare n'a pas encore de rôle Administrateur (hors scope de
//! cette itération, section 12) : cet outil en tient lieu temporairement,
//! exécuté à la main sur le poste du porteur, jamais exposé au réseau.
//!
//! Usage : `cargo run --bin creer_agent_assurance_munaseb -- <nom> <prenom> <telephone> <email> <mot_de_passe>`
//! Exemple : `cargo run --bin creer_agent_assurance_munaseb -- Agent Test +22670005005 agent.test@example.org motdepasseagent123`

use laaficare_backend::config::Config;
use laaficare_backend::db;
use laaficare_backend::mot_de_passe::{self, LONGUEUR_MOT_DE_PASSE_MIN};
use sqlx::types::Uuid;

fn erreur_et_sortir(message: &str) -> ! {
    eprintln!("Erreur : {message}");
    std::process::exit(1);
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    dotenvy::dotenv().ok();

    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let [nom, prenom, telephone, email, mot_de_passe_clair] = arguments.as_slice() else {
        erreur_et_sortir(
            "usage : cargo run --bin creer_agent_assurance_munaseb -- <nom> <prenom> <telephone> <email> <mot_de_passe>",
        );
    };

    if mot_de_passe::longueur(mot_de_passe_clair) < LONGUEUR_MOT_DE_PASSE_MIN {
        erreur_et_sortir(&format!(
            "mot de passe trop court : {} caractères, {LONGUEUR_MOT_DE_PASSE_MIN} minimum",
            mot_de_passe::longueur(mot_de_passe_clair)
        ));
    }

    let config = Config::from_env().unwrap_or_else(|erreur| erreur_et_sortir(&format!("configuration invalide : {erreur}")));

    let pool = db::connecter(&config.database_url)
        .await
        .unwrap_or_else(|_| erreur_et_sortir("impossible de se connecter à la base -- vérifier que PostgreSQL est démarré"));

    let deja_utilise: Option<(Uuid,)> =
        sqlx::query_as("SELECT id FROM utilisateur WHERE telephone = $1 OR lower(email) = lower($2)")
            .bind(telephone.as_str())
            .bind(email.as_str())
            .fetch_optional(&pool)
            .await
            .unwrap_or_else(|erreur| erreur_et_sortir(&format!("erreur base de données : {erreur}")));
    if deja_utilise.is_some() {
        erreur_et_sortir("ce téléphone ou cet email est déjà utilisé par un compte existant");
    }

    // Argon2id (`mot_de_passe::hacher`), même chemin de hachage que les
    // routes d'inscription -- jamais de logique de hachage dupliquée ici.
    let hash = mot_de_passe::hacher(mot_de_passe_clair)
        .unwrap_or_else(|erreur| erreur_et_sortir(&format!("échec du hachage du mot de passe : {erreur}")));

    let mut tx = pool
        .begin()
        .await
        .unwrap_or_else(|erreur| erreur_et_sortir(&format!("erreur base de données : {erreur}")));

    let utilisateur_id: Uuid = sqlx::query_scalar(
        "INSERT INTO utilisateur (nom, prenom, telephone, email, mot_de_passe_hash) \
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(nom.as_str())
    .bind(prenom.as_str())
    .bind(telephone.as_str())
    .bind(email.as_str())
    .bind(&hash)
    .fetch_one(&mut *tx)
    .await
    .unwrap_or_else(|erreur| erreur_et_sortir(&format!("erreur base de données : {erreur}")));

    sqlx::query("INSERT INTO agent_assurance_munaseb (utilisateur_id) VALUES ($1)")
        .bind(utilisateur_id)
        .execute(&mut *tx)
        .await
        .unwrap_or_else(|erreur| erreur_et_sortir(&format!("erreur base de données : {erreur}")));

    tx.commit()
        .await
        .unwrap_or_else(|erreur| erreur_et_sortir(&format!("erreur base de données : {erreur}")));

    println!("Compte agent_assurance_munaseb créé.");
    println!("  utilisateur_id : {utilisateur_id}");
    println!("  email          : {email}");
    println!();
    println!("Tester la connexion : POST /api/assurance-munaseb/connexion avec {{\"email\": \"{email}\", \"mot_de_passe\": \"***\"}}");
}
