//! Outil local : création du premier administrateur LaafiCare (étape 3.6.5,
//! B5 et B6). Jamais une route HTTP. Lancer depuis PowerShell, dans le
//! dossier `backend` : `cargo run --bin creer_premier_administrateur`.
//!
//! Sans administrateur : création (la personne doit avoir un compte patient
//! actif). Avec un seul administrateur qui n'a jamais eu de second facteur :
//! reprise, par lui-même, avec son mot de passe (outil interrompu, ou
//! fenêtre de 10 minutes expirée). Sinon : refus, c'est le rôle des outils
//! `creer_administrateur` et `desactiver_totp`.

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
    let Contexte { pool, email, .. } = console::demarrer().await?;
    let nombre = outils_administrateur::nombre_administrateurs(&pool).await.map_err(|e| console::expliquer(&e))?;

    let en_cours = if nombre == 0 {
        println!("Création du premier administrateur LaafiCare.");
        let telephone = console::lire_ligne("Téléphone du compte patient de la personne : ").map_err(|e| e.to_string())?;
        let adresse = console::lire_ligne("Email administrateur : ").map_err(|e| e.to_string())?;
        let cnib = console::lire_cnib().map_err(|e| e.to_string())?;
        let Some(mot_de_passe) = console::lire_nouveau_mot_de_passe().map_err(|e| e.to_string())? else {
            return Err("Les deux saisies du mot de passe diffèrent. Rien n'a été créé.".into());
        };
        let saisie = NouvelAdministrateur { telephone: &telephone, email: &adresse, cnib_chiffres: &cnib, mot_de_passe: &mot_de_passe };
        outils_administrateur::creer_premier_administrateur(&pool, &email, &saisie)
            .await
            .map_err(|e| console::expliquer(&e))?
    } else {
        println!("Un administrateur existe déjà. Reprise du premier administrateur (il n'a jamais eu de second facteur).");
        let adresse = console::lire_ligne("Email administrateur : ").map_err(|e| e.to_string())?;
        let mot_de_passe = console::lire_secret("Mot de passe administrateur : ").map_err(|e| e.to_string())?;
        outils_administrateur::reprendre_premier_administrateur(&pool, &email, &adresse, &mot_de_passe)
            .await
            .map_err(|e| console::expliquer(&e))?
    };
    console::terminer(&pool, &en_cours).await
}
