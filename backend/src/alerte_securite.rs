//! Alertes de sécurité envoyées par email au titulaire d'un compte quand
//! son second facteur change (décisions D1 à D8 du porteur, 2026-10-21).
//!
//! Sources :
//! - NIST SP 800-63B rév. 4, §4.1.2.1 « Binding an Additional
//!   Authenticator » : « When an authenticator is added, the CSP SHALL
//!   notify the subscriber via a mechanism independent of the transaction
//!   binding the new authenticator, as described in Sec. 4.6 » ;
//! - §4.6 « Account Notifications » : « The notification SHALL provide
//!   clear instructions, including contact information, in case the
//!   recipient repudiates the event associated with the notification » ;
//! - OWASP ASVS 5.0, 6.3.7 : « Verify that users are notified after updates
//!   to authentication details ».
//!
//! Règles communes :
//! - l'alerte part **après** la validation en base de l'événement : elle
//!   n'annonce jamais un changement qui n'a pas eu lieu ;
//! - un échec d'envoi est journalisé **sans l'adresse** et n'annule rien :
//!   l'événement est déjà enregistré, et l'annuler laisserait la personne
//!   sans second facteur ;
//! - texte minimal, sans secret, sans code, sans lien, sans nom
//!   d'opérateur ni motif ; il donne la date et l'heure de l'événement à
//!   l'heure du Burkina Faso, et l'adresse de contact de l'équipe
//!   (`CONTACT_SECURITE`).
//!
//! Limite connue (§4.6, « CSPs SHALL support at least two notification
//! addresses per subscriber account ») : seul l'email est utilisé ; le SMS
//! reste réservé à l'OTP. Noté dans docs/points-ouverts.md.

use sqlx::PgPool;
use sqlx::types::Uuid;

use crate::auth_professionnel::nature_sqlx;
use crate::email::EmailSender;

/// Événements qui déclenchent une alerte (liste fermée).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evenement {
    /// Un administrateur a enregistré un nouveau second facteur
    /// (réenrôlement, `auth_administrateur::confirmer_reenrolement`).
    SecondFacteurEnregistre,
    /// L'équipe a désactivé le second facteur du compte
    /// (`outils_administrateur::desactiver_totp`).
    SecondFacteurDesactive,
}

/// Date et heure de l'événement, lues dans la ligne qui l'enregistre (une
/// seule horloge, celle de PostgreSQL), converties à l'heure du Burkina
/// Faso (doc PostgreSQL 18 §9.9.4, `AT TIME ZONE`, déjà utilisé pour la
/// licence, D9). Format « 21/10/2026 à 10:42 » (`to_char`, doc §9.8).
fn requete(evenement: Evenement) -> &'static str {
    match evenement {
        Evenement::SecondFacteurEnregistre => {
            "SELECT u.email, to_char(t.date_activation AT TIME ZONE 'Africa/Ouagadougou', 'DD/MM/YYYY \"à\" HH24:MI') \
             FROM compte c JOIN utilisateur u ON u.id = c.utilisateur_id \
             JOIN second_facteur_totp t ON t.compte_id = c.id AND t.statut = 'actif' \
             WHERE c.id = $1"
        }
        Evenement::SecondFacteurDesactive => {
            "SELECT u.email, to_char(d.date_desactivation AT TIME ZONE 'Africa/Ouagadougou', 'DD/MM/YYYY \"à\" HH24:MI') \
             FROM compte c JOIN utilisateur u ON u.id = c.utilisateur_id \
             JOIN LATERAL (SELECT date_desactivation FROM desactivation_totp \
                           WHERE compte_id = c.id ORDER BY date_desactivation DESC LIMIT 1) d ON true \
             WHERE c.id = $1"
        }
    }
}

/// Sujet et corps de l'email (textes D7, validés par le porteur).
fn message(evenement: Evenement, quand: &str, contact: &str) -> (&'static str, String) {
    match evenement {
        Evenement::SecondFacteurEnregistre => (
            "LaafiCare : nouveau second facteur enregistré",
            format!(
                "Bonjour,\n\n\
                 Un nouveau second facteur (application d'authentification) a été enregistré sur votre compte \
                 administrateur LaafiCare le {quand} (heure du Burkina Faso).\n\n\
                 Si c'est vous, vous n'avez rien à faire.\n\n\
                 Si ce n'est pas vous, quelqu'un a peut-être accès à votre compte : écrivez immédiatement à \
                 l'équipe LaafiCare, à {contact}."
            ),
        ),
        Evenement::SecondFacteurDesactive => (
            "LaafiCare : second facteur désactivé",
            format!(
                "Bonjour,\n\n\
                 Le second facteur (application d'authentification) de l'un de vos comptes LaafiCare a été \
                 désactivé par l'équipe LaafiCare le {quand} (heure du Burkina Faso), à la suite d'une demande \
                 vérifiée.\n\n\
                 Si vous n'êtes pas à l'origine de cette demande, écrivez immédiatement à l'équipe LaafiCare, \
                 à {contact}."
            ),
        ),
    }
}

/// Prévient le titulaire du compte, après la validation en base de
/// l'événement. Vrai si l'email est parti ; faux si le compte n'a pas
/// d'email, si l'événement est introuvable, ou si l'envoi a échoué (chaque
/// cas journalisé, jamais avec l'adresse).
pub async fn prevenir(
    pool: &PgPool,
    email_sender: &dyn EmailSender,
    contact: &str,
    compte_id: Uuid,
    evenement: Evenement,
) -> bool {
    let ligne: Result<Option<(Option<String>, String)>, sqlx::Error> =
        sqlx::query_as(requete(evenement)).bind(compte_id).fetch_optional(pool).await;
    let (adresse, quand) = match ligne {
        Ok(Some((Some(adresse), quand))) => (adresse, quand),
        Ok(Some((None, _))) => return false,
        Ok(None) => {
            tracing::error!(?evenement, "alerte de sécurité : événement introuvable");
            return false;
        }
        Err(e) => {
            tracing::error!(?evenement, nature = %nature_sqlx(&e), "alerte de sécurité : lecture impossible");
            return false;
        }
    };
    let (sujet, corps) = message(evenement, &quand, contact);
    match email_sender.envoyer(&adresse, sujet, &corps) {
        Ok(()) => true,
        Err(_) => {
            tracing::error!(?evenement, "alerte de sécurité : échec d'envoi de l'email");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn textes_avec_date_et_contact_sans_lien() {
        for evenement in [Evenement::SecondFacteurEnregistre, Evenement::SecondFacteurDesactive] {
            let (sujet, corps) = message(evenement, "21/10/2026 à 10:42", "securite@example.org");
            assert!(sujet.starts_with("LaafiCare : "));
            assert!(corps.contains("le 21/10/2026 à 10:42 (heure du Burkina Faso)"), "{corps}");
            assert!(corps.contains("securite@example.org"), "{corps}");
            assert!(!corps.contains("http") && !corps.contains("://"), "{corps}");
        }
    }
}
