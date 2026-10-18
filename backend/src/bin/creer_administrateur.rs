//! Outil local : administrateur supplémentaire (étape 3.6.5, décision du
//! porteur du 2026-10-18), ou reprise d'un administrateur qui n'a jamais eu
//! de second facteur. Lancé par un administrateur authentifié (email, mot de
//! passe, code TOTP, Z9) ; chaque création ou reprise est tracée avec lui.
//! Jamais une route HTTP. Lancer depuis PowerShell, dans le dossier
//! `backend` : `cargo run --bin creer_administrateur`.
//!
//! Le nouvel administrateur est présent : il choisit et tape lui-même son
//! mot de passe, et colle le code reçu à son email.

use std::process::ExitCode;

use laaficare_backend::console::{self, Contexte};
use laaficare_backend::outils_administrateur::{self, NouvelAdministrateur};

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
    let contexte = console::demarrer().await?;
    let operateur = console::authentifier_operateur(&contexte).await?;
    let Contexte { pool, email, .. } = &contexte;

    println!("1. Créer un administrateur");
    println!("2. Reprendre un administrateur qui n'a jamais eu de second facteur");
    let choix = console::lire_ligne("Choix : ").map_err(|e| e.to_string())?;
    let en_cours = match choix.as_str() {
        "1" => {
            println!("Le nouvel administrateur doit avoir un compte patient actif, et être présent pour taper son mot de passe.");
            let telephone = console::lire_ligne("Téléphone de son compte patient : ").map_err(|e| e.to_string())?;
            let adresse = console::lire_ligne("Son email administrateur : ").map_err(|e| e.to_string())?;
            let cnib = console::lire_cnib().map_err(|e| e.to_string())?;
            println!("Au nouvel administrateur de taper son mot de passe.");
            let Some(mot_de_passe) = console::lire_nouveau_mot_de_passe().map_err(|e| e.to_string())? else {
                return Err("Les deux saisies du mot de passe diffèrent. Rien n'a été créé.".into());
            };
            let saisie =
                NouvelAdministrateur { telephone: &telephone, email: &adresse, cnib_chiffres: &cnib, mot_de_passe: &mot_de_passe };
            outils_administrateur::creer_administrateur(pool, email, operateur, &saisie)
                .await
                .map_err(|e| console::expliquer(&e))?
        }
        "2" => {
            let adresse = console::lire_ligne("Email de l'administrateur à reprendre : ").map_err(|e| e.to_string())?;
            outils_administrateur::reprendre_administrateur(pool, email, operateur, &adresse)
                .await
                .map_err(|e| console::expliquer(&e))?
        }
        _ => return Err("Choix inconnu.".into()),
    };
    console::terminer(pool, &en_cours).await
}
