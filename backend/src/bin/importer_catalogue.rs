//! Outil local : import du catalogue national des médicaments (pharmacie,
//! étape 3 ; CLAUDE.md §15, D6). Lancé par un administrateur LaafiCare
//! authentifié (email, mot de passe, code TOTP), jamais une route HTTP.
//! Lancer dans le dossier `backend` : `cargo run --bin importer_catalogue`.
//!
//! Le fichier importé est un texte UTF-8 au format décrit dans
//! `src/catalogue.rs` (`denomination;dci;forme;dosage`), tiré **d'un
//! document officiel** déposé dans `docs/Reglementation, loi et reference/`
//! (liste nationale des médicaments essentiels 2023). L'outil demande la
//! source, l'enregistre avec l'empreinte du fichier, et importe tout ou rien.

use std::process::ExitCode;

use laaficare_backend::{catalogue, console};

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

    let chemin = console::lire_ligne("Chemin du fichier à importer : ").map_err(|e| e.to_string())?;
    let fichier = std::fs::read(chemin.trim()).map_err(|e| format!("Lecture du fichier impossible : {e}"))?;
    // Lecture d'essai : montre ce qui sera importé avant toute écriture.
    let texte = std::str::from_utf8(&fichier).map_err(|_| catalogue::ErreurCatalogue::PasUtf8.to_string())?;
    let lignes = catalogue::lire_fichier(texte).map_err(|e| e.to_string())?;
    println!("{} médicament(s) lu(s). Premier : {} ({}, {}).", lignes.len(), lignes[0].denomination, lignes[0].forme, lignes[0].dosage);

    let source = console::lire_ligne("Source officielle (titre, édition, texte de référence) : ").map_err(|e| e.to_string())?;
    if !console::confirmer("Importer ce fichier dans le catalogue ? Rien ne pourra être effacé.").map_err(|e| e.to_string())? {
        return Err("Arrêt, rien n'a été importé.".into());
    }
    let bilan = catalogue::importer(&contexte.pool, operateur.compte_id(), &source, &fichier)
        .await
        .map_err(|e| e.to_string())?;
    println!(
        "Import enregistré ({}) : {} médicament(s) ajouté(s), {} déjà présent(s).",
        bilan.import_id, bilan.medicaments_ajoutes, bilan.deja_presents
    );
    Ok(())
}
