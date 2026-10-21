//! Envoi d'emails — interface abstraite, sur le modèle de `sms.rs` (décision
//! R8) : les modules métier ne connaissent jamais le fournisseur réel.
//! Implémentation de développement fournie ici ; le vrai fournisseur (crate
//! et service à présenter, section 14) viendra derrière le même trait.
//!
//! Méthode synchrone, pour la même raison que `sms.rs` : le seul
//! implémenteur actuel ne fait aucune entrée-sortie réseau.

#[derive(Debug, thiserror::Error)]
#[error("échec d'envoi de l'email : {0}")]
pub struct ErreurEmail(String);

pub trait EmailSender: Send + Sync {
    fn envoyer(&self, destinataire: &str, sujet: &str, corps: &str) -> Result<(), ErreurEmail>;

    /// Vrai si l'email ne part pas réellement : le serveur et les outils
    /// refusent alors de démarrer en production (`config::verifier_envois`).
    fn simule(&self) -> bool;
}

/// Implémentation de développement : écrit l'email dans le terminal au lieu
/// de l'envoyer. **Jamais dans les journaux** (`tracing`) : le corps contient
/// un jeton, et OWASP (*Email Validation and Verification Cheat Sheet*) :
/// « Never log verification, reset, or authentication tokens or full
/// verification/reset URLs ». Refusée en production (décision B2) : elle ne
/// sert qu'au développement, où le terminal est celui du développeur.
pub struct EmailSenderConsole;

impl EmailSender for EmailSenderConsole {
    fn envoyer(&self, destinataire: &str, sujet: &str, corps: &str) -> Result<(), ErreurEmail> {
        println!();
        println!("----- Email simulé (développement, non envoyé) -----");
        println!("À : {destinataire}");
        println!("Sujet : {sujet}");
        println!();
        println!("{corps}");
        println!("----------------------------------------------------");
        println!();
        Ok(())
    }

    fn simule(&self) -> bool {
        true
    }
}

/// Tests : jeton lu dans le lien de vérification, après `#jeton=`, jamais
/// en paramètre `?`.
#[cfg(test)]
pub(crate) fn jeton_du_lien(corps: &str) -> String {
    let lien = corps.lines().find(|l| l.contains("/verifier-email#jeton=")).expect("lien attendu");
    assert!(!lien.contains('?'), "{lien}");
    lien.split("#jeton=").nth(1).unwrap().trim().to_string()
}

/// Tests de la bibliothèque : garde les emails (destinataire, corps) au lieu
/// de les afficher, pour lire le lien envoyé.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct EmailsRecus(pub(crate) std::sync::Mutex<Vec<(String, String)>>);

#[cfg(test)]
impl EmailSender for EmailsRecus {
    fn envoyer(&self, destinataire: &str, _sujet: &str, corps: &str) -> Result<(), ErreurEmail> {
        self.0.lock().unwrap().push((destinataire.to_string(), corps.to_string()));
        Ok(())
    }

    fn simule(&self) -> bool {
        true
    }
}
