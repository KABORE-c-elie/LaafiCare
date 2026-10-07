//! Lecture de la configuration du serveur depuis les variables d'environnement.

use std::env;

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
    pub totp_cle_chiffrement: [u8; 32],
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
}

impl Config {
    /// Aucune valeur par défaut : une variable manquante est une erreur
    /// explicite plutôt qu'un comportement caché.
    pub fn from_env() -> Result<Config, ConfigError> {
        let database_url = lire("DATABASE_URL")?;
        let server_port = parse_port(&lire("SERVER_PORT")?)?;
        let jwt_secret = valider_secret_jwt(lire("JWT_SECRET")?)?;
        let totp_cle_chiffrement = decoder_cle_totp(&lire("TOTP_CLE_CHIFFREMENT")?)?;

        Ok(Config {
            database_url,
            server_port,
            jwt_secret,
            totp_cle_chiffrement,
        })
    }
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
fn decoder_cle_totp(valeur: &str) -> Result<[u8; 32], ConfigError> {
    let octets = valeur.as_bytes();
    if octets.len() != 64 || !octets.iter().all(u8::is_ascii_hexdigit) {
        return Err(ConfigError::CleTotpInvalide);
    }
    let mut cle = [0u8; 32];
    for (octet, paire) in cle.iter_mut().zip(octets.chunks_exact(2)) {
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
        assert_eq!(cle[0], 0x0a);
        assert_eq!(cle[1], 0xff);
        assert_eq!(cle[31], 0xff);
    }

    #[test]
    fn cle_totp_invalide_refusee() {
        for valeur in ["ab".repeat(31), "ab".repeat(33), format!("+{}", "a".repeat(63)), "zz".repeat(32)] {
            assert!(matches!(decoder_cle_totp(&valeur), Err(ConfigError::CleTotpInvalide)), "{valeur}");
        }
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
