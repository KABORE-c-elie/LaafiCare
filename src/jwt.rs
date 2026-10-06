//! Émission et vérification des JWT (BF-01, section 11 : un seul
//! `JwtService`, alimenté par les deux stratégies d'auth — Patient par OTP,
//! les autres rôles par mot de passe — pour produire un jeton de même forme.
//!
//! Doc suivie : docs.rs/jsonwebtoken/11.1.0 (README officiel du crate,
//! `encode`/`decode`, `Header::default()` = HS256, `Validation::default()`
//! qui exige la présence de `exp`). HS256 (clé partagée) plutôt que RS256 :
//! un seul backend émet et vérifie les jetons, pas d'infrastructure de
//! clés publiques/privées à distribuer.

use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode, errors::Error};
use serde::{Deserialize, Serialize};

// Durée de validité du jeton. Non spécifiée dans le CDC : valeur par défaut
// choisie ici, à ajuster si besoin -- une seule constante à changer.
const DUREE_VALIDITE_SECS: u64 = 24 * 3600;

// `role` en texte libre plutôt qu'un enum : au stade actuel (Patient,
// AgentAssurance -- section 12), un type fermé obligerait à modifier ce
// fichier à chaque nouveau rôle ajouté dans une itération ultérieure. Les
// routes protégées comparent la chaîne au rôle attendu (BF-01 : accès
// restreint par rôle).
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    /// Identité de l'utilisateur (`utilisateur.id`, UUID en texte).
    pub sub: String,
    pub role: String,
    /// Secondes depuis l'epoch Unix -- champ exigé par `Validation::default()`
    /// (doc jsonwebtoken : « requires the `exp` claim to be present »).
    pub exp: usize,
    pub iat: usize,
}

#[derive(Clone)]
pub struct JwtService {
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
}

impl JwtService {
    /// `secret` : longueur déjà vérifiée par `Config::from_env`
    /// (`ConfigError::SecretJwtTropCourt`, RFC 7518 §3.2) -- ce constructeur
    /// ne revalide pas, il fait confiance à l'appelant comme le reste du
    /// programme fait confiance à `Config`.
    pub fn new(secret: &str) -> Self {
        Self {
            encoding_key: EncodingKey::from_secret(secret.as_bytes()),
            decoding_key: DecodingKey::from_secret(secret.as_bytes()),
        }
    }

    pub fn emettre(&self, utilisateur_id: &str, role: &str) -> Result<String, Error> {
        let maintenant = jsonwebtoken::get_current_timestamp() as usize;
        let claims = Claims {
            sub: utilisateur_id.to_string(),
            role: role.to_string(),
            iat: maintenant,
            exp: maintenant + DUREE_VALIDITE_SECS as usize,
        };
        // `Header::default()` = HS256 (doc officielle du crate).
        encode(&Header::default(), &claims, &self.encoding_key)
    }

    /// `Validation::default()` vérifie `exp` (jeton expiré rejeté) --
    /// comportement du crate, pas une option qu'on active nous-mêmes.
    pub fn verifier(&self, jeton: &str) -> Result<Claims, Error> {
        let donnees = decode::<Claims>(jeton, &self.decoding_key, &Validation::default())?;
        Ok(donnees.claims)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service() -> JwtService {
        JwtService::new(&"a".repeat(32))
    }

    #[test]
    fn emet_puis_verifie() {
        let service = service();
        let jeton = service.emettre("11111111-1111-1111-1111-111111111111", "patient").unwrap();
        let claims = service.verifier(&jeton).unwrap();
        assert_eq!(claims.sub, "11111111-1111-1111-1111-111111111111");
        assert_eq!(claims.role, "patient");
    }

    #[test]
    fn refuse_un_jeton_signe_avec_un_autre_secret() {
        let a = JwtService::new(&"a".repeat(32));
        let b = JwtService::new(&"b".repeat(32));
        let jeton = a.emettre("id", "patient").unwrap();
        assert!(b.verifier(&jeton).is_err());
    }

    #[test]
    fn refuse_un_jeton_deja_expire() {
        let service = service();
        // Jeton fabriqué à la main avec un exp dans le passé. `exp` doit
        // dépasser les 60 s de tolérance d'horloge (« leeway ») de
        // `Validation::default()` (doc jsonwebtoken : « Defaults to 60 »),
        // sinon le jeton est encore considéré valide -- comportement voulu
        // du crate (absorbe un léger désynchronisme d'horloge entre
        // serveurs), pas une option qu'on active nous-mêmes.
        let maintenant = jsonwebtoken::get_current_timestamp() as usize;
        let claims = Claims {
            sub: "id".to_string(),
            role: "patient".to_string(),
            iat: maintenant - 300,
            exp: maintenant - 200,
        };
        let jeton = encode(&Header::default(), &claims, &service.encoding_key).unwrap();
        assert!(service.verifier(&jeton).is_err());
    }

    #[test]
    fn refuse_un_jeton_illisible() {
        assert!(service().verifier("pas-un-jeton-valide").is_err());
    }
}
