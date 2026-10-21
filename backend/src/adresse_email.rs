//! Contrôle de la forme d'une adresse email (décisions du porteur du
//! 2026-10-21). Une seule fonction, utilisée partout où une adresse est
//! enregistrée : email d'une structure, email professionnel, outils locaux
//! de l'équipe. La forme ne prouve rien sur la boîte (OWASP, Input
//! Validation Cheat Sheet : « Format validation does not prove mailbox
//! access ») : seule la vérification par lien le fait (`jeton_email.rs`).
//!
//! Règles, dans l'ordre :
//! 1. **ASCII seulement**, domaine compris (décision du porteur) : les
//!    domaines internationaux et les parties locales accentuées sont refusés
//!    pour l'instant (`docs/points-ouverts.md`). Conséquence pour les
//!    majuscules (façon (a), adresse gardée telle que saisie, comparée par
//!    `lower`) : PostgreSQL 18, fonctions de chaînes, `lower` « Converts the
//!    string to all lower case, according to the rules of the database's
//!    locale » ; en ASCII, toutes les règles de langue donnent le même
//!    résultat (A à Z vers a à z), la comparaison ne dépend donc plus des
//!    réglages de la base.
//! 2. **Ni espace, ni retour à la ligne, ni caractère de contrôle** : notre
//!    propre contrôle, en plus de la crate (un retour à la ligne dans une
//!    adresse servirait à ajouter des en-têtes à un email envoyé).
//! 3. **Longueurs en octets**, RFC 5321 : §4.5.3.1.1 « The maximum total
//!    length of a user name or other local-part is 64 octets » ; §4.5.3.1.3
//!    « The maximum total length of a reverse-path or forward-path is 256
//!    octets (including the punctuation and element separators) », soit 254
//!    pour l'adresse sans les `<` `>` du chemin (§4.1.2). En ASCII, un
//!    caractère est un octet.
//! 4. **Pas d'adresse IP entre crochets** à la place du domaine
//!    (`awa@[127.0.0.1]`, RFC 5321 §4.1.3) : aucune vraie boîte d'un patient
//!    ou d'un professionnel ne s'écrit ainsi (décision du porteur).
//! 5. **Crate `validator` 0.21.0**, trait `ValidateEmail` (OWASP : « Use a
//!    maintained email validation library compatible with the addresses
//!    your mail system supports ») : règle de l'adresse valide du HTML5
//!    (WHATWG), qui refuse aussi `Nom <adresse>` et les parties locales
//!    entre guillemets.
//!
//! **Piège de la crate** : `ValidateEmail` sur un `Option` vide répond
//! « valide ». Cette fonction ne prend qu'un `&str` : un appelant ne peut
//! pas lui passer un `Option`, il doit d'abord exiger la présence du champ.
//!
//! L'appelant retire les espaces autour de la saisie avant l'appel
//! (l'adresse est enregistrée sans eux, contraintes des migrations 0022).

use validator::ValidateEmail;

/// RFC 5321 §4.5.3.1.3 : 256 octets pour le chemin, moins `<` et `>`.
pub const LONGUEUR_MAX_OCTETS: usize = 254;
/// RFC 5321 §4.5.3.1.1.
pub const PARTIE_LOCALE_MAX_OCTETS: usize = 64;

/// Toujours la même erreur, quelle que soit la règle non respectée : le
/// message est celui d'une adresse mal formée (décision du porteur).
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("adresse email invalide")]
pub struct EmailInvalide;

pub fn controler(email: &str) -> Result<(), EmailInvalide> {
    if email.is_empty() || !email.is_ascii() {
        return Err(EmailInvalide);
    }
    // ASCII établi : on raisonne en octets, chacun est un caractère.
    if email.bytes().any(|o| o.is_ascii_whitespace() || o.is_ascii_control()) {
        return Err(EmailInvalide);
    }
    if email.len() > LONGUEUR_MAX_OCTETS {
        return Err(EmailInvalide);
    }
    // Même découpage que la crate (dernier « @ ») : une partie locale qui
    // contiendrait un « @ » est refusée ensuite par sa règle.
    let Some((partie_locale, domaine)) = email.rsplit_once('@') else {
        return Err(EmailInvalide);
    };
    if partie_locale.len() > PARTIE_LOCALE_MAX_OCTETS {
        return Err(EmailInvalide);
    }
    if domaine.contains(['[', ']']) {
        return Err(EmailInvalide);
    }
    if !email.validate_email() {
        return Err(EmailInvalide);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valide(email: &str) -> bool {
        controler(email).is_ok()
    }

    #[test]
    fn adresse_normale_et_majuscules() {
        assert!(valide("awa.ouedraogo@example.org"));
        assert!(valide("Awa.Ouedraogo@Example.ORG"), "les majuscules sont gardées telles quelles");
        assert!(valide("prenom+munaseb@exemple.bf"));
    }

    #[test]
    fn accent_avant_et_apres_l_arobase() {
        assert!(!valide("élodie@example.org"));
        assert!(!valide("awa@exemplé.bf"));
        // Domaine international (IDN), que la crate convertirait en
        // punycode : refusé par la règle de l'ASCII.
        assert!(!valide("awa@例え.jp"));
    }

    #[test]
    fn espaces_retours_a_la_ligne_et_controle() {
        for email in ["awa@example.org\r\nBcc: x@example.org", "awa@example.org\n", "awa ouedraogo@example.org", "awa@exa\tmple.org", "awa\u{0}@example.org", "awa\u{7f}@example.org"] {
            assert!(!valide(email), "{email:?}");
        }
    }

    #[test]
    fn nom_et_adresse_ou_adresse_ip_refuses() {
        assert!(!valide("Awa <awa@example.org>"));
        assert!(!valide("<awa@example.org>"));
        assert!(!valide("awa@[127.0.0.1]"));
        assert!(!valide("awa@[IPv6:2001:db8::1]"));
        // La crate seule accepte l'adresse IP : notre règle la refuse.
        assert!("awa@[127.0.0.1]".validate_email());
    }

    #[test]
    fn partie_locale_64_octets_au_plus() {
        let soixante_quatre = format!("{}@example.org", "a".repeat(64));
        let soixante_cinq = format!("{}@example.org", "a".repeat(65));
        assert!(valide(&soixante_quatre));
        assert!(!valide(&soixante_cinq));
    }

    #[test]
    fn adresse_254_octets_au_plus() {
        // 64 + 1 + 189 = 254 ; étiquettes de domaine de 63 caractères au
        // plus (règle du domaine de la crate).
        let domaine = |dernier: usize| format!("{}.{}.{}", "a".repeat(63), "b".repeat(63), "c".repeat(dernier));
        let a_254 = format!("{}@{}", "d".repeat(64), domaine(61));
        let a_255 = format!("{}@{}", "d".repeat(64), domaine(62));
        assert_eq!((a_254.len(), a_255.len()), (254, 255));
        assert!(valide(&a_254));
        assert!(!valide(&a_255));
    }

    #[test]
    fn chaine_vide_et_formes_cassees() {
        for email in ["", "@", "awa@", "@example.org", "awa", "awa@@example.org", "a@b@example.org", "\"awa\"@example.org"] {
            assert!(!valide(email), "{email:?}");
        }
    }
}
