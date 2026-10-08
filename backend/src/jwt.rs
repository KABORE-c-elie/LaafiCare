//! Émission et vérification des JWT (BF-01). Un seul `JwtService` pour
//! toutes les sortes de jetons (décision J1, 2026-10-08).
//!
//! Doc suivie : docs.rs/jsonwebtoken/11.1.0 (`encode`/`decode`, `Validation`)
//! et le code de `validation.rs` de la crate pour les cas que la doc ne
//! précise pas. HS256 (clé partagée) plutôt que RS256 : un seul backend émet
//! et vérifie les jetons, pas d'infrastructure de clés à distribuer.
//!
//! Huit sortes de jetons, une audience (`aud`) chacune. RFC 8725 §3.12 : les
//! règles de validation de jetons de sortes différentes « MUST be written
//! such that they are mutually exclusive » ; l'audience est l'un des moyens
//! cités. Chaque vérification n'accepte qu'une seule audience : un jeton
//! intermédiaire ne peut jamais servir de jeton complet, ni un jeton patient
//! de jeton professionnel.

use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use sqlx::types::Uuid;

/// Sorte de jeton (décision J1). Le rôle professionnel n'y figure pas : il
/// est celui de l'affectation active, relu en base à chaque requête.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SorteJeton {
    Patient,
    /// Après la connexion professionnelle : ne sert qu'à lister ses
    /// affectations et à en activer une (J5).
    ProfessionnelSansAffectation,
    /// Seule sorte qui porte une affectation (`aff`).
    ProfessionnelEnExercice,
    Administrateur,
    /// Jetons intermédiaires (Z10) : mot de passe correct, code TOTP encore
    /// à saisir. Un par sorte de compte (J1), pour qu'un jeton intermédiaire
    /// patient ne soit pas accepté par le second facteur administrateur.
    SecondFacteurPatient,
    SecondFacteurProfessionnel,
    SecondFacteurAdministrateur,
    /// Administrateur dont le TOTP a été désactivé : ne permet que
    /// l'activation d'un nouveau TOTP (note Z9 de la section 14).
    ReenrolementAdministrateur,
}

impl SorteJeton {
    pub const TOUTES: [SorteJeton; 8] = [
        SorteJeton::Patient,
        SorteJeton::ProfessionnelSansAffectation,
        SorteJeton::ProfessionnelEnExercice,
        SorteJeton::Administrateur,
        SorteJeton::SecondFacteurPatient,
        SorteJeton::SecondFacteurProfessionnel,
        SorteJeton::SecondFacteurAdministrateur,
        SorteJeton::ReenrolementAdministrateur,
    ];

    pub fn audience(self) -> &'static str {
        match self {
            SorteJeton::Patient => "laaficare:patient",
            SorteJeton::ProfessionnelSansAffectation => "laaficare:professionnel_sans_affectation",
            SorteJeton::ProfessionnelEnExercice => "laaficare:professionnel_en_exercice",
            SorteJeton::Administrateur => "laaficare:administrateur",
            SorteJeton::SecondFacteurPatient => "laaficare:second_facteur_patient",
            SorteJeton::SecondFacteurProfessionnel => "laaficare:second_facteur_professionnel",
            SorteJeton::SecondFacteurAdministrateur => "laaficare:second_facteur_administrateur",
            SorteJeton::ReenrolementAdministrateur => "laaficare:reenrolement_administrateur",
        }
    }

    /// Durées décidées par le porteur (J2) : 24 h pour un jeton complet,
    /// 5 min pour saisir le code TOTP, 10 min pour réenrôler un TOTP.
    fn duree_secs(self) -> u64 {
        match self {
            SorteJeton::Patient
            | SorteJeton::ProfessionnelSansAffectation
            | SorteJeton::ProfessionnelEnExercice
            | SorteJeton::Administrateur => 24 * 3600,
            SorteJeton::SecondFacteurPatient
            | SorteJeton::SecondFacteurProfessionnel
            | SorteJeton::SecondFacteurAdministrateur => 5 * 60,
            SorteJeton::ReenrolementAdministrateur => 10 * 60,
        }
    }

    fn porte_une_affectation(self) -> bool {
        self == SorteJeton::ProfessionnelEnExercice
    }
}

/// Contenu du jeton. Les applications ne le lisent pas : elles le gardent et
/// le renvoient tel quel (docs/api.md).
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    /// Identifiant du compte (J3), pas de l'identité : la `version_jeton` et
    /// le verrouillage sont portés par le compte.
    pub sub: Uuid,
    pub aud: String,
    /// `compte.version_jeton` à l'émission. Comparée en base par les
    /// extracteurs : un jeton émis avant une réinitialisation du mot de
    /// passe ou une bascule est refusé (décision C2, section 14).
    pub ver: i32,
    /// Affectation active, seulement pour `ProfessionnelEnExercice`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aff: Option<Uuid>,
    pub iat: u64,
    pub exp: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum ErreurJeton {
    /// Affectation fournie pour une sorte qui n'en porte pas, ou absente
    /// pour `ProfessionnelEnExercice`.
    #[error("affectation incohérente avec la sorte de jeton")]
    AffectationIncoherente,
    #[error(transparent)]
    Jwt(#[from] jsonwebtoken::errors::Error),
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

    pub fn emettre(
        &self,
        sorte: SorteJeton,
        compte_id: Uuid,
        version_jeton: i32,
        affectation: Option<Uuid>,
    ) -> Result<String, ErreurJeton> {
        if sorte.porte_une_affectation() != affectation.is_some() {
            return Err(ErreurJeton::AffectationIncoherente);
        }
        let maintenant = jsonwebtoken::get_current_timestamp();
        let claims = Claims {
            sub: compte_id,
            aud: sorte.audience().to_string(),
            ver: version_jeton,
            aff: affectation,
            iat: maintenant,
            exp: maintenant + sorte.duree_secs(),
        };
        Ok(encode(&Header::new(Algorithm::HS256), &claims, &self.encoding_key)?)
    }

    /// N'accepte que la sorte attendue. Signature, expiration et audience
    /// sont vérifiées par la crate ; la présence de `aff` ici.
    pub fn verifier(&self, jeton: &str, sorte: SorteJeton) -> Result<Claims, ErreurJeton> {
        let claims = decode::<Claims>(jeton, &self.decoding_key, &validation(sorte))?.claims;
        if sorte.porte_une_affectation() != claims.aff.is_some() {
            return Err(ErreurJeton::AffectationIncoherente);
        }
        Ok(claims)
    }
}

fn validation(sorte: SorteJeton) -> Validation {
    // Un seul algorithme accepté : `Validation::new(alg)` n'autorise que
    // celui-là (« Create a default validation setup allowing the given
    // alg »).
    let mut validation = Validation::new(Algorithm::HS256);
    // Doc `Validation::aud` : « Validation only happens if `aud` claim is
    // present in the token. Adding `aud` to `required_spec_claims` will
    // make it required. » Sans cela, un jeton sans audience passerait
    // partout. `sub` exigé de même : c'est le compte.
    validation.set_required_spec_claims(&["exp", "aud", "sub"]);
    validation.set_audience(&[sorte.audience()]);
    // 60 s par défaut (doc : « to account for clock skew »), utile quand
    // l'émetteur et le vérificateur sont deux machines. Ici, le même
    // serveur fait les deux ; avec 60 s, un jeton intermédiaire de 5 min
    // vivrait 6 min.
    validation.leeway = 0;
    validation
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn service() -> JwtService {
        JwtService::new(&"a".repeat(32))
    }

    fn affectation_pour(sorte: SorteJeton) -> Option<Uuid> {
        sorte.porte_une_affectation().then(|| Uuid::from_u128(7))
    }

    /// Jeton signé avec la bonne clé mais au contenu choisi par le test.
    fn signer(service: &JwtService, contenu: serde_json::Value) -> String {
        encode(&Header::new(Algorithm::HS256), &contenu, &service.encoding_key).unwrap()
    }

    #[test]
    fn chaque_sorte_emise_puis_verifiee() {
        let service = service();
        let compte = Uuid::from_u128(1);
        for sorte in SorteJeton::TOUTES {
            let jeton = service.emettre(sorte, compte, 3, affectation_pour(sorte)).unwrap();
            let claims = service.verifier(&jeton, sorte).unwrap();
            assert_eq!(claims.sub, compte);
            assert_eq!(claims.ver, 3);
            assert_eq!(claims.aff, affectation_pour(sorte));
            assert_eq!(claims.aud, sorte.audience());
            assert_eq!(claims.exp - claims.iat, sorte.duree_secs(), "{sorte:?}");
        }
    }

    #[test]
    fn durees_decidees() {
        assert_eq!(SorteJeton::Patient.duree_secs(), 24 * 3600);
        assert_eq!(SorteJeton::SecondFacteurAdministrateur.duree_secs(), 5 * 60);
        assert_eq!(SorteJeton::ReenrolementAdministrateur.duree_secs(), 10 * 60);
    }

    #[test]
    fn une_sorte_n_est_jamais_acceptee_a_la_place_d_une_autre() {
        let service = service();
        for emise in SorteJeton::TOUTES {
            let jeton = service.emettre(emise, Uuid::from_u128(1), 0, affectation_pour(emise)).unwrap();
            for attendue in SorteJeton::TOUTES {
                if attendue != emise {
                    assert!(service.verifier(&jeton, attendue).is_err(), "{emise:?} accepté comme {attendue:?}");
                }
            }
        }
    }

    #[test]
    fn audiences_toutes_differentes() {
        for (i, a) in SorteJeton::TOUTES.iter().enumerate() {
            for b in &SorteJeton::TOUTES[i + 1..] {
                assert_ne!(a.audience(), b.audience());
            }
        }
    }

    #[test]
    fn refuse_un_jeton_sans_audience() {
        let service = service();
        let maintenant = jsonwebtoken::get_current_timestamp();
        let jeton = signer(
            &service,
            json!({ "sub": Uuid::from_u128(1), "ver": 0, "iat": maintenant, "exp": maintenant + 60 }),
        );
        for sorte in SorteJeton::TOUTES {
            assert!(service.verifier(&jeton, sorte).is_err(), "{sorte:?}");
        }
    }

    #[test]
    fn refuse_un_jeton_sans_sub() {
        let service = service();
        let maintenant = jsonwebtoken::get_current_timestamp();
        let jeton = signer(
            &service,
            json!({ "aud": SorteJeton::Patient.audience(), "ver": 0, "iat": maintenant, "exp": maintenant + 60 }),
        );
        assert!(service.verifier(&jeton, SorteJeton::Patient).is_err());
    }

    #[test]
    fn refuse_un_jeton_expire_d_une_seconde() {
        // leeway = 0 : aucune tolérance après l'expiration.
        let service = service();
        let maintenant = jsonwebtoken::get_current_timestamp();
        let jeton = signer(
            &service,
            json!({
                "sub": Uuid::from_u128(1), "aud": SorteJeton::Patient.audience(),
                "ver": 0, "iat": maintenant - 60, "exp": maintenant - 1
            }),
        );
        assert!(service.verifier(&jeton, SorteJeton::Patient).is_err());
    }

    #[test]
    fn affectation_obligatoire_en_exercice_et_interdite_ailleurs() {
        let service = service();
        let compte = Uuid::from_u128(1);
        assert!(matches!(
            service.emettre(SorteJeton::ProfessionnelEnExercice, compte, 0, None),
            Err(ErreurJeton::AffectationIncoherente)
        ));
        assert!(matches!(
            service.emettre(SorteJeton::Patient, compte, 0, Some(Uuid::from_u128(7))),
            Err(ErreurJeton::AffectationIncoherente)
        ));

        // Jetons fabriqués à la main, bien signés : la vérification refuse
        // aussi l'incohérence.
        let maintenant = jsonwebtoken::get_current_timestamp();
        let sans_aff = signer(
            &service,
            json!({
                "sub": compte, "aud": SorteJeton::ProfessionnelEnExercice.audience(),
                "ver": 0, "iat": maintenant, "exp": maintenant + 60
            }),
        );
        assert!(service.verifier(&sans_aff, SorteJeton::ProfessionnelEnExercice).is_err());
        let avec_aff = signer(
            &service,
            json!({
                "sub": compte, "aud": SorteJeton::ProfessionnelSansAffectation.audience(),
                "ver": 0, "aff": Uuid::from_u128(7), "iat": maintenant, "exp": maintenant + 60
            }),
        );
        assert!(service.verifier(&avec_aff, SorteJeton::ProfessionnelSansAffectation).is_err());
    }

    #[test]
    fn refuse_un_jeton_signe_avec_un_autre_secret() {
        let a = JwtService::new(&"a".repeat(32));
        let b = JwtService::new(&"b".repeat(32));
        let jeton = a.emettre(SorteJeton::Patient, Uuid::from_u128(1), 0, None).unwrap();
        assert!(b.verifier(&jeton, SorteJeton::Patient).is_err());
    }

    #[test]
    fn refuse_un_jeton_illisible() {
        assert!(service().verifier("pas-un-jeton-valide", SorteJeton::Patient).is_err());
    }
}
