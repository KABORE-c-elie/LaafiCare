//! Outil local : intervention de l'équipe LaafiCare sur un compte compromis
//! (lot « avant le pilote », P2, décisions IC1 à IC8 du porteur,
//! 2026-10-22 ; J11, DS10). Lancé par un administrateur authentifié (email,
//! mot de passe, code TOTP), jamais sur ses propres comptes. Jamais une
//! route HTTP. Lancer depuis PowerShell, dans le dossier `backend` :
//! `cargo run --bin intervenir_compte`.
//!
//! Trois actions : retirer une affectation, bloquer un compte, réactiver un
//! compte bloqué (CNIB vérifiée). La logique est dans
//! `intervention_compte.rs`, testée en base ; ce programme ne fait que poser
//! les questions et afficher les réponses (B4).

use std::process::ExitCode;

use laaficare_backend::console;
use laaficare_backend::intervention_compte::{self as intervention, ErreurIntervention};
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

fn lire(question: &str) -> Result<String, String> {
    console::lire_ligne(question).map_err(|e| e.to_string())
}

fn oui(question: &str) -> Result<bool, String> {
    console::confirmer(question).map_err(|e| e.to_string())
}

async fn executer() -> Result<(), String> {
    let contexte = console::demarrer().await?;
    let operateur = console::authentifier_operateur(&contexte).await?;
    let (pool, email, contact) = (&contexte.pool, &contexte.email, contexte.config.contact_securite.as_str());

    println!("Action :");
    println!("1. Retirer une affectation (compte professionnel)");
    println!("2. Bloquer un compte");
    println!("3. Réactiver un compte bloqué");
    let action = lire("Choix : ")?;
    if !["1", "2", "3"].contains(&action.as_str()) {
        return Err("Choix inconnu.".into());
    }

    let type_compte = if action == "1" {
        TypeCompte::Professionnel
    } else {
        println!("Compte visé :");
        println!("1. Patient (recherché par son téléphone)");
        println!("2. Professionnel (recherché par son email)");
        println!("3. Administrateur (recherché par son email)");
        match lire("Choix : ")?.as_str() {
            "1" => TypeCompte::Patient,
            "2" => TypeCompte::Professionnel,
            "3" => TypeCompte::Administrateur,
            _ => return Err("Choix inconnu.".into()),
        }
    };
    let question = if type_compte == TypeCompte::Patient { "Téléphone : " } else { "Email : " };
    let identifiant = lire(question)?;
    let cible = outils_administrateur::trouver_compte(pool, type_compte, &identifiant)
        .await
        .map_err(|e| console::expliquer(&e))?;
    let statut = intervention::statut(pool, cible.compte_id).await.map_err(|e| e.to_string())?;
    println!(
        "Compte trouvé : {} {} ({:?}, statut : {statut})",
        cible.nom.as_deref().unwrap_or("(nom non renseigné)"),
        cible.prenom.as_deref().unwrap_or("(prénom non renseigné)"),
        cible.type_compte
    );
    if !oui("Est-ce bien cette personne ?")? {
        return Err("Arrêt, rien n'a été modifié.".into());
    }

    let prevenue = match action.as_str() {
        "1" => {
            let affectations = intervention::affectations_actives(pool, cible.compte_id).await.map_err(|e| e.to_string())?;
            if affectations.is_empty() {
                return Err("Ce compte n'a aucune affectation active.".into());
            }
            for (i, a) in affectations.iter().enumerate() {
                println!("{}. {} : {}", i + 1, a.structure_nom, a.role);
            }
            let numero: usize = lire("Affectation à retirer : ")?.parse().map_err(|_| "Choix inconnu.".to_string())?;
            let choisie = affectations.get(numero.wrapping_sub(1)).ok_or("Choix inconnu.")?;
            let motif = lire("Motif (par exemple « compte compromis signalé par la structure ») : ")?;
            let mut confirme = false;
            loop {
                match intervention::retirer_affectation(
                    pool,
                    email,
                    contact,
                    operateur,
                    cible.compte_id,
                    choisie.affectation_id,
                    &motif,
                    confirme,
                )
                .await
                {
                    Ok(prevenue) => break prevenue,
                    // IC3 : l'équipe peut retirer le dernier responsable,
                    // après un avertissement et une confirmation.
                    Err(ErreurIntervention::DernierResponsable) if !confirme => {
                        println!(
                            "Attention : c'est le dernier responsable actif de cette structure. Elle restera validée, \
                             mais plus personne ne pourra y inviter tant qu'un nouveau responsable n'est pas désigné."
                        );
                        if !oui("Retirer quand même cette affectation ?")? {
                            return Err("Arrêt, rien n'a été modifié.".into());
                        }
                        confirme = true;
                    }
                    Err(e) => return Err(e.to_string()),
                }
            }
        }
        "2" => {
            if type_compte == TypeCompte::Administrateur {
                let restants = intervention::administrateurs_operationnels(pool).await.map_err(|e| e.to_string())?;
                if restants <= 2 {
                    println!(
                        "Attention : il reste {restants} administrateur(s) avec un second facteur actif ; après ce \
                         blocage, la règle « toujours au moins deux » ne sera plus respectée."
                    );
                }
            }
            let motif = lire("Motif (par exemple « mot de passe volé ») : ")?;
            if !oui("Bloquer ce compte ? Toutes ses sessions seront fermées et son mot de passe ne marchera plus")? {
                return Err("Arrêt, rien n'a été modifié.".into());
            }
            let prevenue = intervention::bloquer(pool, email, contact, operateur, &cible, &motif)
                .await
                .map_err(|e| e.to_string())?;
            println!("Compte bloqué ; toutes ses sessions sont fermées et son mot de passe ne marche plus.");
            prevenue
        }
        _ => {
            let motif = lire("Motif (par exemple « identité vérifiée à l'accueil ») : ")?;
            // IC4 : attestation obligatoire pour la réactivation ; le numéro
            // de la carte n'est jamais saisi.
            let cnib = oui("Avez-vous vérifié la CNIB de la personne, en sa présence ?")?;
            let prevenue = intervention::reactiver(pool, email, contact, operateur, &cible, &motif, cnib)
                .await
                .map_err(|e| e.to_string())?;
            println!(
                "Compte réactivé. La personne choisit maintenant un nouveau mot de passe avec « Mot de passe oublié ? » \
                 (par SMS pour un patient, par email sinon)."
            );
            prevenue
        }
    };
    if prevenue {
        println!("La personne a été prévenue par email.");
    } else {
        println!("La personne n'a pas pu être prévenue par email (pas d'email vérifié, ou envoi impossible).");
    }
    Ok(())
}
