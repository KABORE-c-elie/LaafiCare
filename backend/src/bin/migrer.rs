//! Commande locale : applique les migrations de la base (P1, décision 5 du
//! porteur, 2026-10-22). Le serveur ne les applique plus : son rôle
//! PostgreSQL (`laaficare_serveur`) n'est pas propriétaire des tables et n'a
//! pas le droit de les modifier. Cette commande se connecte avec le rôle
//! propriétaire, `DATABASE_URL`, la seule variable qu'elle lit.
//!
//! Lancer dans le dossier `backend` : `cargo run --bin migrer`, puis
//! démarrer le serveur. Sans danger si elle est relancée : une migration
//! déjà appliquée ne l'est jamais une seconde fois (doc sqlx 0.9,
//! `Migrator::run`).
//!
//! Les rôles doivent exister avant (`sql/roles.sql`, voir le README) : la
//! migration 0026 leur donne leurs droits et échoue s'ils manquent.

use std::process::ExitCode;

use laaficare_backend::db;

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt::init();
    match executer().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

async fn executer() -> Result<(), String> {
    match dotenvy::dotenv() {
        Ok(_) => {}
        Err(dotenvy::Error::LineParse(..)) => return Err("fichier .env mal formé (erreur de syntaxe)".into()),
        Err(erreur) if erreur.not_found() => {}
        Err(_) => return Err("fichier .env illisible".into()),
    }
    // Jamais affichée : elle contient le mot de passe du rôle propriétaire.
    let url = std::env::var("DATABASE_URL")
        .ok()
        .filter(|v| !v.is_empty())
        .ok_or("DATABASE_URL manquante : l'adresse de la base avec le rôle propriétaire (voir .env.example)")?;
    let pool = db::connecter(&url)
        .await
        .map_err(|_| "base injoignable : vérifier que PostgreSQL est démarré et que DATABASE_URL est correct")?;
    let avant = db::migrations_manquantes(&pool).await.map_err(|e| format!("lecture de la base impossible : {e}"))?;
    // L'erreur de sqlx nomme la migration et la cause (rôle manquant, par
    // exemple) ; elle ne contient pas l'adresse de la base.
    db::MIGRATIONS.run(&pool).await.map_err(|e| format!("échec des migrations : {e}"))?;
    println!("Base à jour : {avant} migration(s) appliquée(s).");
    Ok(())
}
