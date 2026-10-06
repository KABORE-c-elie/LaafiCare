//! Envoi de SMS — interface abstraite (section 11/12 du CLAUDE.md) : le
//! module métier (`auth_patient`) ne connaît jamais le fournisseur réel.
//! Implémentation dev fournie ici (log console). L'implémentation réelle
//! (API Orange BF / Telecel, section 4) sera branchée plus tard derrière la
//! même interface, sans toucher à `auth_patient.rs`.
//!
//! Méthode volontairement **synchrone** : le seul appelant actuel
//! (`SmsSenderConsole`) ne fait aucun I/O. Rendre le trait `async` pour
//! anticiper un futur fournisseur HTTP casserait la compatibilité avec les
//! objets `dyn SmsSender` sans dépendance supplémentaire (`async-trait`) --
//! pas ajoutée maintenant (section 13 : tout nouvel ajout au Cargo.toml se
//! présente avant d'être écrit). À revoir au moment d'implémenter un
//! fournisseur réel, pas avant.

#[derive(Debug, thiserror::Error)]
#[error("échec d'envoi SMS : {0}")]
pub struct ErreurSms(String);

pub trait SmsSender: Send + Sync {
    fn envoyer(&self, telephone: &str, message: &str) -> Result<(), ErreurSms>;
}

/// Implémentation de développement : journalise au lieu d'envoyer. Ne peut
/// pas échouer (pas d'I/O), `Result` gardé pour respecter l'interface.
pub struct SmsSenderConsole;

impl SmsSender for SmsSenderConsole {
    fn envoyer(&self, telephone: &str, message: &str) -> Result<(), ErreurSms> {
        tracing::info!(telephone, message, "SMS (dev, non envoyé réellement)");
        Ok(())
    }
}
