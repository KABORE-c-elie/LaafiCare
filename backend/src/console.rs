//! Saisie et démarrage des outils locaux (`src/bin/`, étape 3.6.5).
//!
//! Saisie masquée des secrets (décision J9) : crate `rpassword` 7.5.4,
//! fonction `prompt_password` (doc docs.rs : « Prompts on the TTY, then
//! reads a password from it », saisie cachée par défaut, Unix et Windows).
//! Sa documentation ne parle pas d'effacer la chaîne renvoyée : elle est
//! placée aussitôt dans `Zeroizing` (crate `zeroize`), sans copie.
//! La saisie se fait sur le terminal : lancer les outils depuis PowerShell
//! (le terminal de Git Bash, mintty, n'est pas une console Windows).

use std::io::{self, BufRead, Write};

use sqlx::PgPool;
use zeroize::Zeroizing;

use crate::auth_administrateur::FENETRE_REENROLEMENT_MINUTES;
use crate::config::{self, Config, Environnement, Programme};
use crate::sms::{SmsSender, SmsSenderConsole};
use crate::db;
use crate::email::{EmailSender, EmailSenderConsole};
use crate::mot_de_passe::RegleMotDePasse;
use crate::outils_administrateur::{self, AdministrateurEnCours, ErreurOutil, Operateur};

/// Essais pour coller le jeton reçu par email, avant d'arrêter l'outil.
const ESSAIS_JETON: u32 = 3;

/// Ligne ordinaire (téléphone, email, motif), sans les espaces autour.
pub fn lire_ligne(question: &str) -> io::Result<String> {
    print!("{question}");
    io::stdout().flush()?;
    let mut ligne = String::new();
    io::stdin().lock().read_line(&mut ligne)?;
    Ok(ligne.trim().to_owned())
}

/// Secret (mot de passe, code TOTP, jeton reçu par email) : rien ne
/// s'affiche pendant la saisie, et la chaîne est effacée de la mémoire à sa
/// libération.
pub fn lire_secret(question: &str) -> io::Result<Zeroizing<String>> {
    rpassword::prompt_password(question).map(Zeroizing::new)
}

/// Mot de passe choisi, saisi deux fois. `None` si les deux saisies
/// diffèrent.
pub fn lire_nouveau_mot_de_passe() -> io::Result<Option<Zeroizing<String>>> {
    let premier = lire_secret("Mot de passe administrateur (8 à 128 caractères, une lettre, un chiffre, un caractère spécial) : ")?;
    let second = lire_secret("Le même mot de passe, une seconde fois : ")?;
    Ok((*premier == *second).then_some(premier))
}

/// Numéro de CNIB : « B » est déjà affiché, seuls les 8 chiffres sont
/// demandés (décision du porteur, 2026-10-18). Entrée seule : pas de CNIB
/// (facultative).
pub fn lire_cnib() -> io::Result<String> {
    lire_ligne("Numéro de CNIB, facultatif (Entrée pour passer) : B")
}

/// Vrai seulement pour « oui » : toute autre réponse vaut non.
pub fn confirmer(question: &str) -> io::Result<bool> {
    Ok(lire_ligne(&format!("{question} (oui/non) : "))?.to_lowercase() == "oui")
}

/// Message d'une erreur des outils, avec la liste des règles non
/// respectées pour un mot de passe.
pub fn expliquer(erreur: &ErreurOutil) -> String {
    let ErreurOutil::MotDePasseNonConforme(non_conforme) = erreur else {
        return erreur.to_string();
    };
    let regles: Vec<&str> = non_conforme
        .regles_non_respectees
        .iter()
        .map(|regle| match regle {
            RegleMotDePasse::LongueurMin => "au moins 8 caractères",
            RegleMotDePasse::LongueurMax => "au plus 128 caractères",
            RegleMotDePasse::CaractereDeControle => "aucun caractère de contrôle (tabulation, retour à la ligne)",
            RegleMotDePasse::Lettre => "au moins une lettre",
            RegleMotDePasse::Chiffre => "au moins un chiffre",
            RegleMotDePasse::CaractereSpecial => "au moins un caractère spécial, par exemple ! @ #",
        })
        .collect();
    format!("{erreur} Il faut : {}.", regles.join(" ; "))
}

/// Fin commune de la création et de la reprise : l'email est vérifié avec
/// le jeton collé (s'il vient d'être envoyé), puis la fenêtre de
/// réenrôlement est ouverte et la suite est expliquée. Aucun secret TOTP
/// n'est affiché (Y4 révisé) : l'administrateur l'enregistre lui-même par
/// les routes de la 3.6.4.
pub async fn terminer(pool: &PgPool, en_cours: &AdministrateurEnCours) -> Result<(), String> {
    if en_cours.verification_envoyee {
        println!("Un code de vérification a été envoyé à {}.", en_cours.email);
        let mut essai = 1;
        loop {
            let jeton = lire_secret("Code reçu par email (la saisie ne s'affiche pas) : ").map_err(|e| e.to_string())?;
            match outils_administrateur::verifier_email(pool, en_cours.compte_id, &jeton).await {
                Ok(()) => break,
                Err(ErreurOutil::JetonInvalide) if essai < ESSAIS_JETON => {
                    println!("{}", ErreurOutil::JetonInvalide);
                    essai += 1;
                }
                Err(erreur) => {
                    return Err(format!("{} Relancez l'outil pour reprendre (un nouveau code sera envoyé).", expliquer(&erreur)));
                }
            }
        }
        println!("Email vérifié.");
    }
    outils_administrateur::ouvrir_fenetre(pool, en_cours.compte_id).await.map_err(|e| expliquer(&e))?;
    println!();
    println!("Fenêtre ouverte pour {FENETRE_REENROLEMENT_MINUTES} minutes. Dans ce délai, l'administrateur :");
    println!("  1. se connecte : POST /api/administrateurs/connexion (email et mot de passe administrateur) ;");
    println!("  2. avec le jeton de réenrôlement reçu : POST /api/administrateurs/reenrolement/commencer ;");
    println!("  3. ajoute le compte dans son application d'authentification, puis confirme avec un premier");
    println!("     code : POST /api/administrateurs/reenrolement/confirmer, et note ses codes de secours.");
    println!("Si la fenêtre expire, relancer l'outil pour reprendre.");
    Ok(())
}

/// L'administrateur qui lance l'outil s'authentifie d'abord (Z9) : email,
/// mot de passe et code TOTP, saisis masqués sauf l'email.
pub async fn authentifier_operateur(contexte: &Contexte) -> Result<Operateur, String> {
    println!("Authentification de l'administrateur qui lance l'outil.");
    let adresse = lire_ligne("Votre email administrateur : ").map_err(|e| e.to_string())?;
    let mot_de_passe = lire_secret("Votre mot de passe : ").map_err(|e| e.to_string())?;
    let code = lire_secret("Code de votre application d'authentification : ").map_err(|e| e.to_string())?;
    outils_administrateur::authentifier_operateur(
        &contexte.pool,
        &contexte.config.totp_cle_chiffrement,
        &adresse,
        &mot_de_passe,
        &code,
    )
    .await
    .map_err(|e| expliquer(&e))
}

/// Même contrôle que le serveur (`main.rs`), SMS compris (décision du
/// porteur, 2026-10-18) : les outils n'envoient pas de SMS aujourd'hui, mais
/// la règle reste identique partout, et un outil qui en enverrait plus tard
/// serait déjà couvert. Sortie de `demarrer` pour être testée sans `.env` ni
/// base.
fn verifier_envois_des_outils(environnement: Environnement, email: &dyn EmailSender) -> Result<(), config::ConfigError> {
    config::verifier_envois(environnement, &[("SMS", SmsSenderConsole.simule()), ("email", email.simule())])
}

/// Ce dont chaque outil a besoin pour travailler.
pub struct Contexte {
    pub config: Config,
    pub pool: PgPool,
    pub email: EmailSenderConsole,
}

/// Démarrage commun des outils : même `.env` que le serveur (B10), refus en
/// production tant que l'email est simulé (B2), connexion à la base avec le
/// rôle des outils (`DATABASE_URL_OUTILS`, P1), et contrôle que toutes les
/// migrations sont appliquées (les outils n'en appliquent pas : c'est le
/// rôle de la commande `migrer`). Le message d'erreur ne contient jamais de
/// valeur de la configuration.
pub async fn demarrer() -> Result<Contexte, String> {
    match dotenvy::dotenv() {
        Ok(_) => {}
        Err(dotenvy::Error::LineParse(..)) => return Err("fichier .env mal formé (erreur de syntaxe)".into()),
        Err(erreur) if erreur.not_found() => {}
        Err(_) => return Err("fichier .env illisible".into()),
    }
    let config = Config::from_env(Programme::Outils).map_err(|e| format!("configuration invalide : {e}"))?;
    let email = EmailSenderConsole;
    verifier_envois_des_outils(config.environnement, &email).map_err(|e| format!("configuration invalide : {e}"))?;
    let pool = db::connecter(&config.database_url).await.map_err(|_| {
        "base injoignable : vérifier que PostgreSQL est démarré et que DATABASE_URL_OUTILS est correct".to_string()
    })?;
    let manquantes =
        db::migrations_manquantes(&pool).await.map_err(|_| "lecture de la base impossible".to_string())?;
    if manquantes > 0 {
        return Err("base pas à jour : lancer `cargo run --bin migrer`".into());
    }
    Ok(Contexte { config, pool, email })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigError;
    use crate::email::ErreurEmail;

    /// Un envoi d'email réel, pour vérifier que le SMS simulé suffit à
    /// refuser.
    struct EmailReel;

    impl EmailSender for EmailReel {
        fn envoyer(&self, _: &str, _: &str, _: &str) -> Result<(), ErreurEmail> {
            Ok(())
        }
        fn simule(&self) -> bool {
            false
        }
    }

    /// Les trois outils passent par `demarrer`, donc par ce contrôle : en
    /// production, l'email ou le SMS simulé est refusé ; en développement,
    /// tout passe.
    #[test]
    fn outils_refuses_en_production_avec_un_envoi_simule() {
        assert!(verifier_envois_des_outils(Environnement::Developpement, &EmailSenderConsole).is_ok());
        assert!(matches!(
            verifier_envois_des_outils(Environnement::Production, &EmailSenderConsole),
            Err(ConfigError::EnvoiSimuleEnProduction(_))
        ));
        assert!(matches!(
            verifier_envois_des_outils(Environnement::Production, &EmailReel),
            Err(ConfigError::EnvoiSimuleEnProduction("SMS"))
        ));
    }
}
