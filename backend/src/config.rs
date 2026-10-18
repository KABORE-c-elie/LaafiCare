//! Lecture de la configuration du serveur depuis les variables d'environnement.

use std::env;
use std::fmt;

use zeroize::Zeroizing;

// Pas de `#[derive(Debug)]` volontaire : `database_url` contient le mot de
// passe de la base. Sans `Debug`, un `{:?}` oublié dans un log ne peut pas le
// faire fuiter (le compilateur refuse).
pub struct Config {
    pub database_url: String,
    pub server_port: u16,
    pub jwt_secret: String,
    /// Clé AES-256 du chiffrement des secrets TOTP (décision Z4), distincte
    /// du secret JWT. Sa perte rend tous les TOTP inutilisables : en
    /// production, elle est sauvegardée à part (section 14).
    pub totp_cle_chiffrement: CleTotp,
    /// Développement ou production (décision du porteur, 2026-10-18).
    pub environnement: Environnement,
}

/// Où tourne le programme (variable `ENVIRONNEMENT`, obligatoire). Aucune
/// valeur par défaut : un oubli en production ne doit jamais laisser passer
/// l'envoi simulé des SMS ou des emails (refus fermé).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environnement {
    Developpement,
    Production,
}

/// Clé AES-256 des secrets TOTP (décision F6, 2026-10-15). `Debug` affiche
/// `***` ; ni `Display` ni `Serialize` : la seule lecture est `octets()`, au
/// moment de chiffrer ou de déchiffrer. Effacée de la mémoire à sa
/// libération (`Zeroizing`, crate `zeroize`).
pub struct CleTotp(Zeroizing<[u8; 32]>);

impl CleTotp {
    pub fn depuis_octets(octets: [u8; 32]) -> Self {
        CleTotp(Zeroizing::new(octets))
    }

    pub fn octets(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for CleTotp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

// `thiserror` (déjà dans le Cargo.toml) : chaque variante porte le message
// affiché au démarrage, pour qu'une variable oubliée soit identifiée sans
// avoir à lire le code.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("variable d'environnement absente, vide ou illisible : {0}")]
    Manquante(&'static str),

    #[error("SERVER_PORT invalide : « {0} » n'est pas un numéro de port (0 à 65535)")]
    PortInvalide(String),

    // RFC 7518 §3.2 : « A key of the same size as the hash output (for
    // instance, 256 bits for "HS256") or larger MUST be used ». 256 bits =
    // 32 octets, vérifié sur la longueur de la chaîne UTF-8 (pas sur des
    // octets décodés depuis du base64 : le secret est utilisé tel quel,
    // comme dans l'exemple officiel du crate `jsonwebtoken`).
    #[error("JWT_SECRET trop court : {0} caractères, 32 minimum (RFC 7518 §3.2, HS256)")]
    SecretJwtTropCourt(usize),

    // La valeur n'est jamais reprise dans le message : c'est une clé.
    #[error("TOTP_CLE_CHIFFREMENT invalide : 64 caractères hexadécimaux attendus (32 octets, AES-256)")]
    CleTotpInvalide,

    #[error("ENVIRONNEMENT invalide : « developpement » ou « production » attendu")]
    EnvironnementInvalide,

    // Le SMS porte le code d'inscription, l'email le jeton de vérification :
    // simulés, ils ne partent jamais et s'affichent dans le terminal.
    #[error("envoi de {0} simulé : refusé en production (ENVIRONNEMENT=production)")]
    EnvoiSimuleEnProduction(&'static str),
}

impl Config {
    /// Aucune valeur par défaut : une variable manquante est une erreur
    /// explicite plutôt qu'un comportement caché.
    pub fn from_env() -> Result<Config, ConfigError> {
        let database_url = lire("DATABASE_URL")?;
        let server_port = parse_port(&lire("SERVER_PORT")?)?;
        let jwt_secret = valider_secret_jwt(lire("JWT_SECRET")?)?;
        let totp_cle_chiffrement = decoder_cle_totp(&lire("TOTP_CLE_CHIFFREMENT")?)?;
        let environnement = parse_environnement(env::var("ENVIRONNEMENT").ok().as_deref())?;

        Ok(Config {
            database_url,
            server_port,
            jwt_secret,
            totp_cle_chiffrement,
            environnement,
        })
    }
}

// Absente, vide ou inconnue : refus (décision du porteur, 2026-10-18).
// Comparaison exacte, sans tolérer de majuscules : une seule écriture
// valable, celle du `.env.example`.
fn parse_environnement(valeur: Option<&str>) -> Result<Environnement, ConfigError> {
    match valeur {
        None | Some("") => Err(ConfigError::Manquante("ENVIRONNEMENT")),
        Some("developpement") => Ok(Environnement::Developpement),
        Some("production") => Ok(Environnement::Production),
        Some(_) => Err(ConfigError::EnvironnementInvalide),
    }
}

/// À appeler au démarrage du serveur et de chaque outil, avec ce qu'ils
/// envoient (`SmsSender::simule`, `EmailSender::simule`). En production,
/// tout envoi simulé est refusé : sinon les codes et les jetons ne
/// partiraient jamais, et s'afficheraient dans le terminal du serveur.
pub fn verifier_envois(environnement: Environnement, envois: &[(&'static str, bool)]) -> Result<(), ConfigError> {
    if environnement == Environnement::Production {
        if let Some((nature, _)) = envois.iter().find(|(_, simule)| *simule) {
            return Err(ConfigError::EnvoiSimuleEnProduction(nature));
        }
    }
    Ok(())
}

// `env::var` renvoie `Err` si la variable est absente ou n'est pas de l'UTF-8
// valide (doc : std::env::var). Une valeur vide (`DATABASE_URL=`) est traitée
// comme absente : sinon elle passerait ici et ne ferait échouer sqlx que plus
// tard, après les 31 s de tentatives de connexion.
fn lire(nom: &'static str) -> Result<String, ConfigError> {
    env::var(nom)
        .ok()
        .filter(|valeur| !valeur.is_empty())
        .ok_or(ConfigError::Manquante(nom))
}

// `u16` : un port TCP va de 0 à 65535, donc le parsing refuse de lui-même
// les valeurs hors plage et non numériques.
fn parse_port(valeur: &str) -> Result<u16, ConfigError> {
    valeur
        .parse::<u16>()
        .map_err(|_| ConfigError::PortInvalide(valeur.to_string()))
}

// Voir RFC 7518 §3.2 en commentaire sur ConfigError::SecretJwtTropCourt.
fn valider_secret_jwt(valeur: String) -> Result<String, ConfigError> {
    if valeur.len() < 32 {
        return Err(ConfigError::SecretJwtTropCourt(valeur.len()));
    }
    Ok(valeur)
}

// Décision X4 : 64 caractères hexadécimaux, décodés avec la bibliothèque
// standard, sans nouvelle dépendance. Les caractères sont vérifiés un par un
// avant `from_str_radix`, qui accepterait un « + » initial (doc
// `u8::from_str_radix`).
fn decoder_cle_totp(valeur: &str) -> Result<CleTotp, ConfigError> {
    let octets = valeur.as_bytes();
    if octets.len() != 64 || !octets.iter().all(u8::is_ascii_hexdigit) {
        return Err(ConfigError::CleTotpInvalide);
    }
    // Décodée directement dans le `Zeroizing` : pas de copie intermédiaire.
    let mut cle = CleTotp(Zeroizing::new([0u8; 32]));
    for (octet, paire) in cle.0.iter_mut().zip(octets.chunks_exact(2)) {
        // Toujours de l'ASCII hexadécimal ici : ni l'UTF-8 ni la conversion
        // ne peuvent échouer.
        let paire = std::str::from_utf8(paire).map_err(|_| ConfigError::CleTotpInvalide)?;
        *octet = u8::from_str_radix(paire, 16).map_err(|_| ConfigError::CleTotpInvalide)?;
    }
    Ok(cle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cle_totp_hexadecimale_decodee() {
        let cle = decoder_cle_totp(&"0aFf".repeat(16)).unwrap();
        assert_eq!(cle.octets()[0], 0x0a);
        assert_eq!(cle.octets()[1], 0xff);
        assert_eq!(cle.octets()[31], 0xff);
    }

    /// Décision F6 : la clé n'apparaît jamais dans `Debug`, ni en octets ni
    /// en hexadécimal.
    #[test]
    fn cle_totp_masquee_dans_debug() {
        let hexadecimal = "3c".repeat(32);
        let cle = decoder_cle_totp(&hexadecimal).unwrap();
        let affichage = format!("{cle:?}");
        assert_eq!(affichage, "***");
        assert!(!affichage.contains(&hexadecimal) && !affichage.contains("60"), "{affichage}");
        assert_eq!(format!("{:?}", CleTotp::depuis_octets([7; 32])), "***");
    }

    #[test]
    fn cle_totp_invalide_refusee() {
        for valeur in ["ab".repeat(31), "ab".repeat(33), format!("+{}", "a".repeat(63)), "zz".repeat(32)] {
            assert!(matches!(decoder_cle_totp(&valeur), Err(ConfigError::CleTotpInvalide)), "{valeur}");
        }
    }

    #[test]
    fn environnement_absent_ou_inconnu_refuse() {
        assert!(matches!(parse_environnement(None), Err(ConfigError::Manquante("ENVIRONNEMENT"))));
        assert!(matches!(parse_environnement(Some("")), Err(ConfigError::Manquante("ENVIRONNEMENT"))));
        for valeur in ["Production", "prod", "dev", "developpement "] {
            assert!(matches!(parse_environnement(Some(valeur)), Err(ConfigError::EnvironnementInvalide)), "{valeur}");
        }
    }

    #[test]
    fn environnement_reconnu() {
        assert_eq!(parse_environnement(Some("developpement")).unwrap(), Environnement::Developpement);
        assert_eq!(parse_environnement(Some("production")).unwrap(), Environnement::Production);
    }

    #[test]
    fn envois_simules_refuses_en_production_seulement() {
        let simules = [("SMS", true), ("email", true)];
        assert!(verifier_envois(Environnement::Developpement, &simules).is_ok());
        assert!(matches!(
            verifier_envois(Environnement::Production, &simules),
            Err(ConfigError::EnvoiSimuleEnProduction("SMS"))
        ));
        assert!(matches!(
            verifier_envois(Environnement::Production, &[("SMS", false), ("email", true)]),
            Err(ConfigError::EnvoiSimuleEnProduction("email"))
        ));
        assert!(verifier_envois(Environnement::Production, &[("SMS", false), ("email", false)]).is_ok());
    }

    #[test]
    fn port_valide() {
        assert_eq!(parse_port("8080").unwrap(), 8080);
    }

    #[test]
    fn port_non_numerique_refuse() {
        assert!(matches!(
            parse_port("abc"),
            Err(ConfigError::PortInvalide(_))
        ));
    }

    #[test]
    fn port_hors_plage_refuse() {
        assert!(matches!(
            parse_port("70000"),
            Err(ConfigError::PortInvalide(_))
        ));
    }

    #[test]
    fn secret_jwt_de_32_caracteres_accepte() {
        let secret = "a".repeat(32);
        assert_eq!(valider_secret_jwt(secret.clone()).unwrap(), secret);
    }

    #[test]
    fn secret_jwt_trop_court_refuse() {
        assert!(matches!(
            valider_secret_jwt("trop-court".to_string()),
            Err(ConfigError::SecretJwtTropCourt(_))
        ));
    }
}
