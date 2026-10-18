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
