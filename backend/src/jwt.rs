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

    /// Durées décidées par le porteur : 15 min pour un jeton d'accès complet
    /// (option B du 2026-10-12, renouvelé par le jeton de renouvellement de
    /// la session, `session.rs`) ; 5 min pour saisir le code TOTP et 10 min
    /// pour réenrôler un TOTP (J2).
    fn duree_secs(self) -> u64 {
        match self {
            SorteJeton::Patient
            | SorteJeton::ProfessionnelSansAffectation
            | SorteJeton::ProfessionnelEnExercice
            | SorteJeton::Administrateur => DUREE_JETON_ACCES_SECS,
            SorteJeton::SecondFacteurPatient
            | SorteJeton::SecondFacteurProfessionnel
            | SorteJeton::SecondFacteurAdministrateur => 5 * 60,
            SorteJeton::ReenrolementAdministrateur => 10 * 60,
        }
    }

    fn porte_une_affectation(self) -> bool {
        self == SorteJeton::ProfessionnelEnExercice
    }

    /// Les quatre sortes complètes appartiennent à une session (`sid`,
    /// vérifiée à chaque requête) ; les jetons intermédiaires et de
    /// réenrôlement n'en ont pas (décision W1).
    fn porte_une_session(self) -> bool {
        matches!(
            self,
            SorteJeton::Patient
                | SorteJeton::ProfessionnelSansAffectation
                | SorteJeton::ProfessionnelEnExercice
                | SorteJeton::Administrateur
        )
    }
}

/// Durée d'un jeton d'accès complet, en secondes (champ `expire_dans` des
/// réponses de connexion et de renouvellement).
pub const DUREE_JETON_ACCES_SECS: u64 = 15 * 60;

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
    /// Session (`session.id`), seulement pour les quatre sortes complètes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sid: Option<Uuid>,
    pub iat: u64,
    pub exp: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum ErreurJeton {
    /// Affectation ou session fournie pour une sorte qui n'en porte pas, ou
    /// absente pour une sorte qui en exige une (décision W2).
    #[error("contenu incohérent avec la sorte de jeton")]
    ContenuIncoherent,
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
        session: Option<Uuid>,
    ) -> Result<String, ErreurJeton> {
        if !contenu_coherent(sorte, affectation, session) {
            return Err(ErreurJeton::ContenuIncoherent);
        }
        let maintenant = jsonwebtoken::get_current_timestamp();
        let claims = Claims {
            sub: compte_id,
            aud: sorte.audience().to_string(),
            ver: version_jeton,
            aff: affectation,
            sid: session,
            iat: maintenant,
            exp: maintenant + sorte.duree_secs(),
        };
        Ok(encode(&Header::new(Algorithm::HS256), &claims, &self.encoding_key)?)
    }

    /// N'accepte que la sorte attendue. Signature, expiration et audience
    /// sont vérifiées par la crate ; la présence de `aff` et de `sid` ici.
    pub fn verifier(&self, jeton: &str, sorte: SorteJeton) -> Result<Claims, ErreurJeton> {
        let claims = decode::<Claims>(jeton, &self.decoding_key, &validation(sorte))?.claims;
        if !contenu_coherent(sorte, claims.aff, claims.sid) {
            return Err(ErreurJeton::ContenuIncoherent);
        }
        Ok(claims)
    }
}

/// `aff` présent seulement pour `ProfessionnelEnExercice`, `sid` présent
/// seulement pour les quatre sortes complètes.
fn contenu_coherent(sorte: SorteJeton, affectation: Option<Uuid>, session: Option<Uuid>) -> bool {
    sorte.porte_une_affectation() == affectation.is_some() && sorte.porte_une_session() == session.is_some()
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

    fn session_pour(sorte: SorteJeton) -> Option<Uuid> {
        sorte.porte_une_session().then(|| Uuid::from_u128(9))
    }

    fn emettre(service: &JwtService, sorte: SorteJeton, compte: Uuid) -> String {
        service.emettre(sorte, compte, 0, affectation_pour(sorte), session_pour(sorte)).unwrap()
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
            let jeton = service.emettre(sorte, compte, 3, affectation_pour(sorte), session_pour(sorte)).unwrap();
            let claims = service.verifier(&jeton, sorte).unwrap();
            assert_eq!(claims.sub, compte);
            assert_eq!(claims.ver, 3);
            assert_eq!(claims.aff, affectation_pour(sorte));
            assert_eq!(claims.sid, session_pour(sorte));
            assert_eq!(claims.aud, sorte.audience());
            assert_eq!(claims.exp - claims.iat, sorte.duree_secs(), "{sorte:?}");
        }
    }

    #[test]
    fn durees_decidees() {
        for sorte in [
            SorteJeton::Patient,
            SorteJeton::ProfessionnelSansAffectation,
            SorteJeton::ProfessionnelEnExercice,
            SorteJeton::Administrateur,
        ] {
            assert_eq!(sorte.duree_secs(), 15 * 60, "{sorte:?}");
        }
        assert_eq!(DUREE_JETON_ACCES_SECS, 15 * 60);
        assert_eq!(SorteJeton::SecondFacteurAdministrateur.duree_secs(), 5 * 60);
        assert_eq!(SorteJeton::ReenrolementAdministrateur.duree_secs(), 10 * 60);
    }

    #[test]
    fn une_sorte_n_est_jamais_acceptee_a_la_place_d_une_autre() {
        let service = service();
        for emise in SorteJeton::TOUTES {
            let jeton = emettre(&service, emise, Uuid::from_u128(1));
            for attendue in SorteJeton::TOUTES {
                if attendue != emise {
                    assert!(service.verifier(&jeton, attendue).is_err(), "{emise:?} accepté comme {attendue:?}");
                }
            }
        }
    }

    /// HS256 seulement (J1) : c'est ce qui justifie l'exception cargo audit
    /// pour `rsa` (RUSTSEC-2023-0071, `.cargo/audit.toml`) ; aucune clé RSA
    /// ne sert jamais. Refusés : un jeton signé avec la même clé mais en
    /// HS512, un en-tête qui annonce RS256 (confusion d'algorithme) ou
    /// « none » (RFC 8725 §2.1 et §3.1).
    #[test]
    fn hs256_seulement() {
        let service = service();
        let sorte = SorteJeton::Patient;
        let jeton = emettre(&service, sorte, Uuid::from_u128(1));
        assert!(service.verifier(&jeton, sorte).is_ok());

        let claims = service.verifier(&jeton, sorte).unwrap();
        let hs512 = encode(&Header::new(Algorithm::HS512), &claims, &service.encoding_key).unwrap();
        assert!(service.verifier(&hs512, sorte).is_err(), "HS512");

        // En-têtes base64url de {"alg":"RS256","typ":"JWT"} et
        // {"alg":"none","typ":"JWT"}, recollés au contenu et à la signature
        // d'un jeton HS256 valable.
        let (_, reste) = jeton.split_once('.').unwrap();
        for entete in ["eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9", "eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0"] {
            let falsifie = format!("{entete}.{reste}");
            assert!(service.verifier(&falsifie, sorte).is_err(), "{entete}");
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
                "ver": 0, "sid": Uuid::from_u128(9), "iat": maintenant - 60, "exp": maintenant - 1
            }),
        );
        assert!(service.verifier(&jeton, SorteJeton::Patient).is_err());
    }

    #[test]
    fn affectation_obligatoire_en_exercice_et_interdite_ailleurs() {
        let service = service();
        let compte = Uuid::from_u128(1);
        let sid = Some(Uuid::from_u128(9));
        assert!(matches!(
            service.emettre(SorteJeton::ProfessionnelEnExercice, compte, 0, None, sid),
            Err(ErreurJeton::ContenuIncoherent)
        ));
        assert!(matches!(
            service.emettre(SorteJeton::Patient, compte, 0, Some(Uuid::from_u128(7)), sid),
            Err(ErreurJeton::ContenuIncoherent)
        ));

        // Jetons fabriqués à la main, bien signés : la vérification refuse
        // aussi l'incohérence.
        let maintenant = jsonwebtoken::get_current_timestamp();
        let sans_aff = signer(
            &service,
            json!({
                "sub": compte, "aud": SorteJeton::ProfessionnelEnExercice.audience(),
                "ver": 0, "sid": sid, "iat": maintenant, "exp": maintenant + 60
            }),
        );
        assert!(service.verifier(&sans_aff, SorteJeton::ProfessionnelEnExercice).is_err());
        let avec_aff = signer(
            &service,
            json!({
                "sub": compte, "aud": SorteJeton::ProfessionnelSansAffectation.audience(),
                "ver": 0, "aff": Uuid::from_u128(7), "sid": sid, "iat": maintenant, "exp": maintenant + 60
            }),
        );
        assert!(service.verifier(&avec_aff, SorteJeton::ProfessionnelSansAffectation).is_err());
    }

    /// Décision W1 : les sortes complètes exigent `sid`, les autres le
    /// refusent, à l'émission comme à la vérification.
    #[test]
    fn session_obligatoire_pour_les_jetons_complets_et_interdite_ailleurs() {
        let service = service();
        let compte = Uuid::from_u128(1);
        let maintenant = jsonwebtoken::get_current_timestamp();
        for sorte in SorteJeton::TOUTES {
            let aff = affectation_pour(sorte);
            let mauvaise_session = if sorte.porte_une_session() { None } else { Some(Uuid::from_u128(9)) };
            assert!(
                matches!(service.emettre(sorte, compte, 0, aff, mauvaise_session), Err(ErreurJeton::ContenuIncoherent)),
                "{sorte:?}"
            );
            let jeton = signer(
                &service,
                json!({
                    "sub": compte, "aud": sorte.audience(), "ver": 0, "aff": aff, "sid": mauvaise_session,
                    "iat": maintenant, "exp": maintenant + 60
                }),
            );
            assert!(service.verifier(&jeton, sorte).is_err(), "{sorte:?}");
        }
    }

    #[test]
    fn refuse_un_jeton_signe_avec_un_autre_secret() {
        let a = JwtService::new(&"a".repeat(32));
        let b = JwtService::new(&"b".repeat(32));
        let jeton = emettre(&a, SorteJeton::Patient, Uuid::from_u128(1));
        assert!(b.verifier(&jeton, SorteJeton::Patient).is_err());
    }

    #[test]
    fn refuse_un_jeton_illisible() {
        assert!(service().verifier("pas-un-jeton-valide", SorteJeton::Patient).is_err());
    }
}
