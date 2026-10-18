//! Outil local : désactivation du second facteur (TOTP) d'un compte dont le
//! titulaire a perdu son téléphone et ses codes de secours (Z9, étape
//! 3.6.5, B7). Lancé par un administrateur authentifié (email, mot de
//! passe, code TOTP), jamais sur ses propres comptes ; la CNIB de la
//! personne doit avoir été vérifiée. Jamais une route HTTP. Lancer depuis
//! PowerShell, dans le dossier `backend` : `cargo run --bin desactiver_totp`.
//!
//! Pour un compte administrateur, une fenêtre de 10 minutes est ouverte :
//! il enregistre aussitôt un nouveau TOTP par les routes de la 3.6.4.

use std::process::ExitCode;

use laaficare_backend::console;
use laaficare_backend::outils_administrateur::{self, TypeCompte};

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

    println!("Compte dont le second facteur est à désactiver :");
    println!("1. Patient (recherché par son téléphone)");
    println!("2. Professionnel (recherché par son email)");
    println!("3. Administrateur (recherché par son email)");
    let type_compte = match console::lire_ligne("Choix : ").map_err(|e| e.to_string())?.as_str() {
        "1" => TypeCompte::Patient,
        "2" => TypeCompte::Professionnel,
        "3" => TypeCompte::Administrateur,
        _ => return Err("Choix inconnu.".into()),
    };
    let question = if type_compte == TypeCompte::Patient { "Téléphone : " } else { "Email : " };
    let identifiant = console::lire_ligne(question).map_err(|e| e.to_string())?;
    let cible = outils_administrateur::trouver_compte(&contexte.pool, type_compte, &identifiant)
        .await
        .map_err(|e| console::expliquer(&e))?;

    println!(
        "Compte trouvé : {} {}",
        cible.nom.as_deref().unwrap_or("(nom non renseigné)"),
        cible.prenom.as_deref().unwrap_or("(prénom non renseigné)")
    );
    if !console::confirmer("Est-ce bien cette personne ?").map_err(|e| e.to_string())? {
        return Err("Arrêt, rien n'a été modifié.".into());
    }
    let motif = console::lire_ligne("Motif (par exemple « téléphone perdu ») : ").map_err(|e| e.to_string())?;
    // Y2 : on atteste le contrôle de la carte ; son numéro n'est pas saisi.
    let cnib_verifiee = console::confirmer("Avez-vous vérifié la CNIB de la personne, en sa présence ?").map_err(|e| e.to_string())?;

    let resultat = outils_administrateur::desactiver_totp(&contexte.pool, &contexte.email, operateur, &cible, &motif, cnib_verifiee)
        .await
        .map_err(|e| console::expliquer(&e))?;

    println!("Second facteur désactivé ; toutes les sessions du compte sont fermées.");
    if resultat.personne_prevenue {
        println!("La personne a été prévenue par email.");
    } else {
        println!("La personne n'a pas pu être prévenue par email (pas d'email, ou envoi impossible).");
    }
    if resultat.fenetre_ouverte {
        println!(
            "Fenêtre ouverte pour {} minutes : l'administrateur se connecte et enregistre un nouveau second facteur \
             (POST /api/administrateurs/connexion, puis /api/administrateurs/reenrolement/commencer et /confirmer).",
            laaficare_backend::auth_administrateur::FENETRE_REENROLEMENT_MINUTES
        );
    }
    Ok(())
}
