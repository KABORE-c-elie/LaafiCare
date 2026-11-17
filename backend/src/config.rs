//! Lecture de la configuration du serveur depuis les variables d'environnement.

use std::env;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;

use zeroize::Zeroizing;

use crate::adresse_email;
use crate::db::OptionsPool;

// Pas de `#[derive(Debug)]` volontaire : `database_url` contient le mot de
// passe de la base. Sans `Debug`, un `{:?}` oublié dans un log ne peut pas le
// faire fuiter (le compilateur refuse).
pub struct Config {
    pub database_url: String,
    /// Adresse d'écoute (variable `SERVER_HOST`, facultative) : 127.0.0.1
    /// par défaut, le serveur n'est joignable que depuis la machine même.
    /// `0.0.0.0` l'ouvre au réseau (conteneur derrière un proxy en
    /// production) : un choix explicite, jamais un défaut.
    pub server_host: IpAddr,
    pub server_port: u16,
    /// Dimensionnement du pool de connexions (variables `DATABASE_*`,
    /// facultatives ; défauts dans `db::OptionsPool`).
    pub pool: OptionsPool,
    pub jwt_secret: String,
    /// Clé AES-256 du chiffrement des secrets TOTP (décision Z4), distincte
    /// du secret JWT. Sa perte rend tous les TOTP inutilisables : en
    /// production, elle est sauvegardée à part (section 14).
    pub totp_cle_chiffrement: CleTotp,
    /// Développement ou production (décision du porteur, 2026-10-18).
    pub environnement: Environnement,
    /// Adresse de l'application web, sans `/` final : les liens envoyés par
    /// email y mènent (E5, page `/verifier-email`).
    pub url_application_web: String,
    /// Adresse email de l'équipe LaafiCare, donnée dans les alertes de
    /// sécurité (`alerte_securite.rs`) : NIST SP 800-63B rév. 4 §4.6, « The
    /// notification SHALL provide clear instructions, including contact
    /// information » (décision du porteur, 2026-10-21).
    pub contact_securite: String,
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

    #[error("SERVER_HOST invalide : « {0} » n'est pas une adresse IP (ex. 127.0.0.1 ou 0.0.0.0)")]
    HoteInvalide(String),

    // Une valeur présente mais fausse arrête le démarrage : retomber en
    // silence sur la valeur par défaut cacherait la faute de frappe.
    #[error("{0} invalide : un nombre entier positif attendu")]
    NombreInvalide(&'static str),

    #[error("DATABASE_MIN_CONNECTIONS ne peut pas dépasser DATABASE_MAX_CONNECTIONS")]
    PoolIncoherent,

    // RFC 7518 §3.2 : « A key of the same size as the hash output (for
    // instance, 256 bits for "HS256") or larger MUST be used ». 256 bits =
    // 32 octets, vérifié sur la longueur de la chaîne UTF-8 (pas sur des
    // octets décodés depuis du base64 : le secret est utilisé tel quel,
    // comme dans l'exemple officiel du crate `jsonwebtoken`).
    #[error("JWT_SECRET trop court : {0} caractères, 32 minimum (RFC 7518 §3.2, HS256)")]
    SecretJwtTropCourt(usize),

    // La valeur n'est jamais reprise dans le message : c'est une clé.
    #[error(
        "TOTP_CLE_CHIFFREMENT invalide : 64 caractères hexadécimaux attendus (32 octets, AES-256)"
    )]
    CleTotpInvalide,

    #[error("ENVIRONNEMENT invalide : « developpement » ou « production » attendu")]
    EnvironnementInvalide,

    #[error(
        "URL_APPLICATION_WEB invalide : adresse en http:// ou https://, sans espace, sans « ? » ni « # »"
    )]
    UrlApplicationWebInvalide,

    // Les liens envoyés par email portent un jeton : en production, la page
    // qui le reçoit et le renvoie au serveur doit être servie en https
    // (décision du porteur, 2026-10-21).
    #[error(
        "URL_APPLICATION_WEB doit commencer par https:// en production (ENVIRONNEMENT=production)"
    )]
    UrlApplicationWebSansHttps,

    #[error("CONTACT_SECURITE invalide : une adresse email attendue (ASCII, sans espace)")]
    ContactSecuriteInvalide,

    // Une adresse fictive ne doit jamais partir dans une alerte réelle
    // (décision du porteur, 2026-10-21).
    #[error(
        "CONTACT_SECURITE est une adresse d'exemple (RFC 2606) : refusée en production (ENVIRONNEMENT=production)"
    )]
    ContactSecuriteFictif,

    // Le SMS porte le code d'inscription, l'email le jeton de vérification :
    // simulés, ils ne partent jamais et s'affichent dans le terminal.
    #[error("envoi de {0} simulé : refusé en production (ENVIRONNEMENT=production)")]
    EnvoiSimuleEnProduction(&'static str),
}

/// Le programme qui lit la configuration : chacun se connecte à la base avec
/// son propre rôle PostgreSQL (P1, décision 6 du porteur, 2026-10-22).
/// `DATABASE_URL`, celle du rôle propriétaire, ne sert qu'aux migrations
/// (`migrer`) et aux tests (`#[sqlx::test]` la lit et crée des bases) : le
/// serveur et les outils ne la lisent jamais.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Programme {
    /// Rôle `laaficare_serveur`, variable `DATABASE_URL_SERVEUR`.
    Serveur,
    /// Rôle `laaficare_outils`, variable `DATABASE_URL_OUTILS`.
    Outils,
}

impl Programme {
    pub fn variable_base(self) -> &'static str {
        match self {
            Programme::Serveur => "DATABASE_URL_SERVEUR",
            Programme::Outils => "DATABASE_URL_OUTILS",
        }
    }
}

impl Config {
    /// Aucune valeur par défaut : une variable manquante est une erreur
    /// explicite plutôt qu'un comportement caché.
    pub fn from_env(programme: Programme) -> Result<Config, ConfigError> {
        let database_url = lire(programme.variable_base())?;
        let server_port = parse_port(&lire("SERVER_PORT")?)?;
        let jwt_secret = valider_secret_jwt(lire("JWT_SECRET")?)?;
        let totp_cle_chiffrement = decoder_cle_totp(&lire("TOTP_CLE_CHIFFREMENT")?)?;
        let environnement = parse_environnement(env::var("ENVIRONNEMENT").ok().as_deref())?;
        let url_application_web =
            valider_url_application_web(&lire("URL_APPLICATION_WEB")?, environnement)?;
        let contact_securite = valider_contact_securite(&lire("CONTACT_SECURITE")?, environnement)?;
        // Exceptions assumées à « aucune valeur par défaut » (remédiation de
        // l'audit, 2026-11-17) : ces réglages ne touchent ni la sécurité des
        // données ni les envois, et leur défaut est le choix le plus fermé
        // (écoute locale, petit pool).
        let server_host = parse_hote(env::var("SERVER_HOST").ok().as_deref())?;
        let pool = lire_options_pool(|nom| env::var(nom).ok())?;

        Ok(Config {
            database_url,
            server_host,
            server_port,
            pool,
            jwt_secret,
            totp_cle_chiffrement,
            environnement,
            url_application_web,
            contact_securite,
        })
    }
}

// Même contrôle que toute adresse enregistrée (`adresse_email.rs`). En
// production, les noms réservés aux exemples et aux essais par la RFC 2606
// sont refusés : §2, domaines de premier niveau « .test », « .example »,
// « .invalid » et « .localhost » ; §3, « example.com », « example.net » et
// « example.org » (et leurs sous-domaines). Comparaison sans tenir compte
// des majuscules (RFC 5321 §2.4 : « Mailbox domains follow normal DNS rules
// and are hence not case sensitive »).
fn valider_contact_securite(
    valeur: &str,
    environnement: Environnement,
) -> Result<String, ConfigError> {
    adresse_email::controler(valeur).map_err(|_| ConfigError::ContactSecuriteInvalide)?;
    if environnement == Environnement::Production {
        let domaine = valeur
            .rsplit_once('@')
            .map(|(_, d)| d.to_ascii_lowercase())
            .unwrap_or_default();
        let reserve = |nom: &str| domaine == nom || domaine.ends_with(&format!(".{nom}"));
        let fictif = [
            "example.com",
            "example.net",
            "example.org",
            "test",
            "example",
            "invalid",
            "localhost",
        ]
        .into_iter()
        .any(reserve);
        if fictif {
            return Err(ConfigError::ContactSecuriteFictif);
        }
    }
    Ok(valeur.to_string())
}

// Le lien de vérification est construit en ajoutant
// `/verifier-email#jeton=…` : une adresse qui contiendrait déjà « ? » ou
// « # » donnerait un lien faux, où le jeton pourrait partir en paramètre
// (envoyé au serveur, jamais voulu : R6). Le `/` final est retiré pour ne
// pas doubler celui du chemin.
//
// En production, https seulement (décision du porteur, 2026-10-21) : le
// jeton du lien est un secret d'authentification, et NIST SP 800-63B rév. 4
// exige un canal authentifié et chiffré pour ces secrets. En
// développement, http reste permis (http://localhost:4200, rien ne sort de
// la machine).
fn valider_url_application_web(
    valeur: &str,
    environnement: Environnement,
) -> Result<String, ConfigError> {
    let sans_schema = valeur
        .strip_prefix("https://")
        .or_else(|| valeur.strip_prefix("http://"));
    match sans_schema {
        Some(reste)
            if !reste.trim_end_matches('/').is_empty()
                && !valeur
                    .chars()
                    .any(|c| c.is_whitespace() || c.is_control() || c == '?' || c == '#') =>
        {
            if environnement == Environnement::Production && !valeur.starts_with("https://") {
                return Err(ConfigError::UrlApplicationWebSansHttps);
            }
            Ok(valeur.trim_end_matches('/').to_string())
        }
        _ => Err(ConfigError::UrlApplicationWebInvalide),
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
pub fn verifier_envois(
    environnement: Environnement,
    envois: &[(&'static str, bool)],
) -> Result<(), ConfigError> {
    if environnement == Environnement::Production
        && let Some((nature, _)) = envois.iter().find(|(_, simule)| *simule)
    {
        return Err(ConfigError::EnvoiSimuleEnProduction(nature));
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

/// `SERVER_HOST` absent ou vide : 127.0.0.1. Analyse par la bibliothèque
/// standard (`IpAddr::from_str`) : une adresse IPv4 ou IPv6, jamais un nom
/// d'hôte (« localhost » se résoudrait d'abord en IPv6 sous Windows, voir
/// `db.rs`).
fn parse_hote(valeur: Option<&str>) -> Result<IpAddr, ConfigError> {
    match valeur.map(str::trim).filter(|v| !v.is_empty()) {
        None => Ok(IpAddr::V4(Ipv4Addr::LOCALHOST)),
        Some(v) => v
            .parse()
            .map_err(|_| ConfigError::HoteInvalide(v.to_string())),
    }
}

/// Options du pool, chacune facultative : absente ou vide, la valeur de
/// `OptionsPool::default()`. Le lecteur de variables est passé en
/// paramètre pour être testé sans toucher l'environnement du processus.
fn lire_options_pool(
    lire: impl Fn(&'static str) -> Option<String>,
) -> Result<OptionsPool, ConfigError> {
    let defaut = OptionsPool::default();
    let nombre = |nom: &'static str, defaut: u64| -> Result<u64, ConfigError> {
        match lire(nom)
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            None => Ok(defaut),
            Some(v) => v
                .parse::<u64>()
                .ok()
                .filter(|n| *n >= 1)
                .ok_or(ConfigError::NombreInvalide(nom)),
        }
    };
    let entier = |nom: &'static str, defaut: u32| -> Result<u32, ConfigError> {
        u32::try_from(nombre(nom, u64::from(defaut))?).map_err(|_| ConfigError::NombreInvalide(nom))
    };
    // Le minimum peut valoir 0 (défaut de sqlx) : lu à part.
    let min_connexions = match lire("DATABASE_MIN_CONNECTIONS")
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        None => defaut.min_connexions,
        Some(v) => v
            .parse::<u32>()
            .map_err(|_| ConfigError::NombreInvalide("DATABASE_MIN_CONNECTIONS"))?,
    };
    let options = OptionsPool {
        max_connexions: entier("DATABASE_MAX_CONNECTIONS", defaut.max_connexions)?,
        min_connexions,
        acquire_timeout: Duration::from_secs(nombre(
            "DATABASE_ACQUIRE_TIMEOUT_SECS",
            defaut.acquire_timeout.as_secs(),
        )?),
        idle_timeout: Duration::from_secs(nombre(
            "DATABASE_IDLE_TIMEOUT_SECS",
            defaut.idle_timeout.as_secs(),
        )?),
        max_lifetime: Duration::from_secs(nombre(
            "DATABASE_MAX_LIFETIME_SECS",
            defaut.max_lifetime.as_secs(),
        )?),
    };
    // La doc de sqlx (`min_connections`) borne déjà le minimum au maximum,
    // mais recommande que l'application le vérifie elle-même (« it should be
    // checking this condition itself and returning a nicer error »).
    if options.min_connexions > options.max_connexions {
        return Err(ConfigError::PoolIncoherent);
    }
    Ok(options)
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
    fn adresse_d_ecoute() {
        assert_eq!(
            parse_hote(None).unwrap(),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            "défaut : local seulement"
        );
        assert_eq!(
            parse_hote(Some("  ")).unwrap(),
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        );
        assert_eq!(
            parse_hote(Some("0.0.0.0")).unwrap(),
            IpAddr::V4(Ipv4Addr::UNSPECIFIED)
        );
        assert_eq!(parse_hote(Some("::1")).unwrap().to_string(), "::1");
        for faux in ["localhost", "127.0.0", "0.0.0.0:8080"] {
            assert!(
                matches!(parse_hote(Some(faux)), Err(ConfigError::HoteInvalide(_))),
                "{faux}"
            );
        }
    }

    #[test]
    fn options_du_pool() {
        let aucune = |_: &'static str| None;
        assert_eq!(lire_options_pool(aucune).unwrap(), OptionsPool::default());
        let avec = |valeurs: &'static [(&'static str, &'static str)]| {
            move |nom: &'static str| {
                valeurs
                    .iter()
                    .find(|(n, _)| *n == nom)
                    .map(|(_, v)| v.to_string())
            }
        };
        let lu = lire_options_pool(avec(&[
            ("DATABASE_MAX_CONNECTIONS", "20"),
            ("DATABASE_MIN_CONNECTIONS", "2"),
            ("DATABASE_ACQUIRE_TIMEOUT_SECS", "8"),
        ]))
        .unwrap();
        assert_eq!(
            (
                lu.max_connexions,
                lu.min_connexions,
                lu.acquire_timeout.as_secs()
            ),
            (20, 2, 8)
        );
        assert_eq!(lu.idle_timeout, OptionsPool::default().idle_timeout);
        for faux in [
            &[("DATABASE_MAX_CONNECTIONS", "0")][..],
            &[("DATABASE_MAX_CONNECTIONS", "dix")],
            &[("DATABASE_ACQUIRE_TIMEOUT_SECS", "-5")],
            &[("DATABASE_MAX_CONNECTIONS", "99999999999")],
        ] {
            assert!(
                matches!(
                    lire_options_pool(avec(faux)),
                    Err(ConfigError::NombreInvalide(_))
                ),
                "{faux:?}"
            );
        }
        assert!(matches!(
            lire_options_pool(avec(&[
                ("DATABASE_MAX_CONNECTIONS", "3"),
                ("DATABASE_MIN_CONNECTIONS", "4")
            ])),
            Err(ConfigError::PoolIncoherent)
        ));
    }

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
        assert!(
            !affichage.contains(&hexadecimal) && !affichage.contains("60"),
            "{affichage}"
        );
        assert_eq!(format!("{:?}", CleTotp::depuis_octets([7; 32])), "***");
    }

    #[test]
    fn cle_totp_invalide_refusee() {
        for valeur in [
            "ab".repeat(31),
            "ab".repeat(33),
            format!("+{}", "a".repeat(63)),
            "zz".repeat(32),
        ] {
            assert!(
                matches!(decoder_cle_totp(&valeur), Err(ConfigError::CleTotpInvalide)),
                "{valeur}"
            );
        }
    }

    #[test]
    fn environnement_absent_ou_inconnu_refuse() {
        assert!(matches!(
            parse_environnement(None),
            Err(ConfigError::Manquante("ENVIRONNEMENT"))
        ));
        assert!(matches!(
            parse_environnement(Some("")),
            Err(ConfigError::Manquante("ENVIRONNEMENT"))
        ));
        for valeur in ["Production", "prod", "dev", "developpement "] {
            assert!(
                matches!(
                    parse_environnement(Some(valeur)),
                    Err(ConfigError::EnvironnementInvalide)
                ),
                "{valeur}"
            );
        }
    }

    #[test]
    fn environnement_reconnu() {
        assert_eq!(
            parse_environnement(Some("developpement")).unwrap(),
            Environnement::Developpement
        );
        assert_eq!(
            parse_environnement(Some("production")).unwrap(),
            Environnement::Production
        );
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
            verifier_envois(
                Environnement::Production,
                &[("SMS", false), ("email", true)]
            ),
            Err(ConfigError::EnvoiSimuleEnProduction("email"))
        ));
        assert!(
            verifier_envois(
                Environnement::Production,
                &[("SMS", false), ("email", false)]
            )
            .is_ok()
        );
    }

    #[test]
    fn url_application_web() {
        let dev = Environnement::Developpement;
        let prod = Environnement::Production;
        assert_eq!(
            valider_url_application_web("http://localhost:4200", dev).unwrap(),
            "http://localhost:4200"
        );
        assert_eq!(
            valider_url_application_web("https://exemple.bf/app/", dev).unwrap(),
            "https://exemple.bf/app"
        );
        for valeur in [
            "localhost:4200",
            "ftp://exemple.bf",
            "https://",
            "https:///",
            "https://a.bf/?x=1",
            "https://a.bf/#x",
            "https://a b.bf",
        ] {
            for environnement in [dev, prod] {
                assert!(
                    matches!(
                        valider_url_application_web(valeur, environnement),
                        Err(ConfigError::UrlApplicationWebInvalide)
                    ),
                    "{valeur}"
                );
            }
        }
        // En production, https seulement.
        assert_eq!(
            valider_url_application_web("https://exemple.bf", prod).unwrap(),
            "https://exemple.bf"
        );
        assert!(matches!(
            valider_url_application_web("http://exemple.bf", prod),
            Err(ConfigError::UrlApplicationWebSansHttps)
        ));
    }

    #[test]
    fn contact_securite() {
        let dev = Environnement::Developpement;
        let prod = Environnement::Production;
        // En développement, une adresse d'exemple est acceptée.
        assert_eq!(
            valider_contact_securite("securite@example.org", dev).unwrap(),
            "securite@example.org"
        );
        assert_eq!(
            valider_contact_securite("securite@laaficare.bf", prod).unwrap(),
            "securite@laaficare.bf"
        );
        for valeur in [
            "",
            "pas une adresse",
            "sécurité@laaficare.bf",
            " securite@laaficare.bf",
        ] {
            for environnement in [dev, prod] {
                assert!(
                    matches!(
                        valider_contact_securite(valeur, environnement),
                        Err(ConfigError::ContactSecuriteInvalide)
                    ),
                    "{valeur:?}"
                );
            }
        }
        for valeur in [
            "securite@example.org",
            "securite@EXAMPLE.COM",
            "securite@mail.example.net",
            "securite@laaficare.test",
            "securite@laaficare.example",
            "securite@x.invalid",
            "securite@localhost",
        ] {
            assert!(
                matches!(
                    valider_contact_securite(valeur, prod),
                    Err(ConfigError::ContactSecuriteFictif)
                ),
                "{valeur}"
            );
        }
        // Un domaine qui contient « example » sans être réservé reste accepté.
        assert!(valider_contact_securite("securite@counterexample.org", prod).is_ok());
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
