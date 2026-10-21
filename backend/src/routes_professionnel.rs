//! Routes de la connexion professionnelle (étape 3.6.2, décisions P1 à P7
//! du 2026-10-15 ; contrat dans docs/api.md). Même principe que les autres
//! fichiers de routes : désérialiser, appeler `auth_professionnel.rs`,
//! traduire le résultat.

use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header::RETRY_AFTER};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::erreur_api::{CheminApi, JsonApi, ReponseErreur, reponse_erreur, reponse_erreur_interne};
use crate::extracteur_jwt::{IntermediaireProfessionnel, ProfessionnelConnecte, RefusAuthentification};
use crate::routes_session::reponse_paire;
use laaficare_backend::affectation::{self, AffectationListee, ErreurActivation};
use laaficare_backend::auth_professionnel::{
    self, ConnexionProfessionnelle, ErreurAuthProfessionnel, ErreurSecondFacteur, JetonIntermediaire,
    SaisieSecondFacteur,
};
use laaficare_backend::verification_email::{self, ErreurRenvoi};

#[derive(Deserialize)]
pub struct ConnexionRequete {
    pub email: String,
    pub mot_de_passe: String,
}

#[derive(Serialize)]
struct CorpsSecondFacteur<'a> {
    etape: &'static str,
    jeton_intermediaire: &'a str,
}

/// Réponse « second facteur » (Z10, J4). Le jeton intermédiaire n'est lu en
/// clair qu'ici, au moment de sérialiser (`into_response` le fait tout de
/// suite), puis effacé de la mémoire avec `JetonIntermediaire`.
pub(crate) fn reponse_second_facteur(jeton: &JetonIntermediaire) -> Response {
    let corps = CorpsSecondFacteur { etape: "second_facteur", jeton_intermediaire: jeton.en_clair() };
    (StatusCode::OK, Json(corps)).into_response()
}

/// Un seul endroit qui décide du code HTTP et du `code` de chaque erreur,
/// pour les professionnels et les administrateurs (`routes_administrateur`).
/// `contexte` : libellé des journaux seulement.
pub(crate) fn mapper_erreur(erreur: ErreurAuthProfessionnel, contexte: &'static str) -> ReponseErreur {
    match &erreur {
        // Tous les refus d'identifiants, sans distinction (P1, L1).
        ErreurAuthProfessionnel::IdentifiantsInvalides => {
            reponse_erreur(StatusCode::UNAUTHORIZED, "identifiants_invalides", erreur.to_string())
        }
        // Seulement après un mot de passe correct (P3) : 403, pas 401, le
        // demandeur est bien authentifié mais son compte n'est pas utilisable.
        ErreurAuthProfessionnel::EmailNonVerifie => {
            reponse_erreur(StatusCode::FORBIDDEN, "email_non_verifie", erreur.to_string())
        }
        // L'étape en échec va aux journaux, jamais dans la réponse.
        ErreurAuthProfessionnel::Interne(etape) => reponse_erreur_interne(contexte, etape),
    }
}

/// `POST /api/professionnels/connexion` -- email + mot de passe. 200 avec
/// `"etape": "complete"` et la paire de jetons (session sans affectation,
/// J5), ou `"etape": "second_facteur"` et un jeton intermédiaire si un TOTP
/// est actif ; 401 `identifiants_invalides` ; 403 `email_non_verifie`.
pub async fn se_connecter(State(state): State<AppState>, JsonApi(corps): JsonApi<ConnexionRequete>) -> Response {
    match auth_professionnel::se_connecter(&state.db, &state.jwt, &corps.email, &corps.mot_de_passe).await {
        Ok(ConnexionProfessionnelle::Complete(paire)) => reponse_paire(StatusCode::OK, &paire, Some("complete")),
        Ok(ConnexionProfessionnelle::SecondFacteur { jeton_intermediaire }) => {
            reponse_second_facteur(&jeton_intermediaire)
        }
        Err(erreur) => mapper_erreur(erreur, "connexion professionnelle").into_response(),
    }
}

/// Corps du second temps : **un seul** des deux champs (F4). Pas de
/// `Debug` : un code est un secret.
#[derive(Deserialize)]
pub struct SecondFacteurRequete {
    pub code: Option<String>,
    pub code_secours: Option<String>,
}

/// Les deux champs, ou aucun : `None`.
fn saisie(corps: SecondFacteurRequete) -> Option<SaisieSecondFacteur> {
    match (corps.code, corps.code_secours) {
        (Some(code), None) => Some(SaisieSecondFacteur::Code(code)),
        (None, Some(code)) => Some(SaisieSecondFacteur::CodeSecours(code)),
        _ => None,
    }
}

fn reponse_erreur_second_facteur(erreur: ErreurSecondFacteur, contexte: &'static str) -> Response {
    match erreur {
        // Une seule réponse pour un code faux, un code de secours faux ou
        // déjà utilisé, un compte verrouillé (F3). `code_invalide` existe
        // déjà pour l'OTP patient.
        ErreurSecondFacteur::CodeInvalide => {
            reponse_erreur(StatusCode::UNAUTHORIZED, "code_invalide", erreur.to_string()).into_response()
        }
        // Le jeton a été consommé par une autre requête : même réponse
        // qu'un jeton invalide (RFC 6750 §3.1), l'application revient à
        // l'écran de connexion.
        ErreurSecondFacteur::JetonDejaUtilise => RefusAuthentification::JetonInvalide.into_response(),
        ErreurSecondFacteur::Interne(etape) => reponse_erreur_interne(contexte, etape).into_response(),
    }
}

/// `POST /api/professionnels/connexion/second-facteur` (F1) -- en-tête
/// `Authorization: Bearer <jeton_intermediaire>` (F2), corps `{"code": …}`
/// ou `{"code_secours": …}`. 200 avec `"etape": "complete"` et la paire de
/// jetons ; 401 `code_invalide` ; 401 `jeton_invalide` ; 422
/// `donnees_invalides`.
pub async fn second_facteur(
    intermediaire: IntermediaireProfessionnel,
    State(state): State<AppState>,
    JsonApi(corps): JsonApi<SecondFacteurRequete>,
) -> Response {
    terminer_second_facteur(&state, intermediaire.compte_id, intermediaire.iat, corps, "second facteur professionnel")
        .await
}

/// Second temps, commun aux professionnels et aux administrateurs (A5) :
/// l'extracteur de chaque route a déjà contrôlé le jeton intermédiaire.
pub(crate) async fn terminer_second_facteur(
    state: &AppState,
    compte_id: sqlx::types::Uuid,
    iat: u64,
    corps: SecondFacteurRequete,
    contexte: &'static str,
) -> Response {
    let Some(saisie) = saisie(corps) else {
        // Même code qu'un corps mal formé (docs/api.md, section 1.4).
        return reponse_erreur(
            StatusCode::UNPROCESSABLE_ENTITY,
            "donnees_invalides",
            "indiquer soit `code`, soit `code_secours`, mais pas les deux",
        )
        .into_response();
    };
    match auth_professionnel::terminer_connexion(&state.db, &state.jwt, &state.cle_totp, compte_id, iat, &saisie).await
    {
        Ok(paire) => reponse_paire(StatusCode::OK, &paire, Some("complete")),
        Err(erreur) => reponse_erreur_second_facteur(erreur, contexte),
    }
}

// ---------------------------------------------------------------------
// Affectations (étape 3.9, D1 à D3)
// ---------------------------------------------------------------------

#[derive(Serialize)]
struct ListeAffectations {
    affectations: Vec<AffectationListee>,
}

/// `GET /api/professionnels/affectations` -- jeton sans affectation ou en
/// exercice (D2). 200 avec les affectations **actives** du compte, chacune
/// avec `exercable`.
pub async fn lister_affectations(professionnel: ProfessionnelConnecte, State(state): State<AppState>) -> Response {
    match affectation::lister(&state.db, professionnel.compte_id).await {
        Ok(affectations) => (StatusCode::OK, Json(ListeAffectations { affectations })).into_response(),
        Err(erreur) => reponse_erreur_interne("liste des affectations", &erreur.to_string()).into_response(),
    }
}

fn reponse_erreur_activation(erreur: ErreurActivation) -> Response {
    match erreur {
        // Inexistante ou d'un autre compte : jamais distingués (D1).
        ErreurActivation::AffectationInconnue => {
            reponse_erreur(StatusCode::NOT_FOUND, "affectation_inconnue", erreur.to_string()).into_response()
        }
        ErreurActivation::NonExercable => {
            reponse_erreur(StatusCode::FORBIDDEN, "affectation_non_exercable", erreur.to_string()).into_response()
        }
        // Session fermée entre le contrôle du jeton et l'activation.
        ErreurActivation::SessionFermee => RefusAuthentification::JetonInvalide.into_response(),
        ErreurActivation::Interne(detail) => reponse_erreur_interne("activation d'une affectation", &detail).into_response(),
    }
}

/// `POST /api/professionnels/affectations/{id}/activer` -- jeton sans
/// affectation ou en exercice (D2), sans corps. 200 avec une nouvelle paire
/// de jetons, d'une session de cette affectation (D3 a : l'ancienne session
/// est fermée, la nouvelle garde sa date de fin). 404
/// `affectation_inconnue`, 403 `affectation_non_exercable`.
pub async fn activer_affectation(
    professionnel: ProfessionnelConnecte,
    State(state): State<AppState>,
    CheminApi(affectation_id): CheminApi<sqlx::types::Uuid>,
) -> Response {
    let activee = affectation::activer(&state.db, professionnel.compte_id, professionnel.session_id, affectation_id)
        .await
        .map_err(reponse_erreur_activation)
        .and_then(|session| {
            session
                .en_paire(&state.jwt)
                .map_err(|e| reponse_erreur_interne("jetons de l'affectation", &e.to_string()).into_response())
        });
    match activee {
        Ok(paire) => reponse_paire(StatusCode::OK, &paire, None),
        Err(reponse) => reponse,
    }
}

// ---------------------------------------------------------------------
// Vérification de l'email (étape 3.7, E5 à E7)
// ---------------------------------------------------------------------

/// Pas de `Debug` : le jeton est un secret.
#[derive(Deserialize)]
pub struct VerifierEmailRequete {
    pub jeton: String,
}

/// `POST /api/professionnels/email/verifier` (E6) -- sans authentification,
/// corps `{"jeton": …}` envoyé par la page `/verifier-email` quand la
/// personne confirme. 204 si l'email est vérifié ; 400 `lien_invalide`
/// pour un lien inconnu, déjà utilisé, remplacé ou expiré, jamais
/// distingués.
pub async fn verifier_email(State(state): State<AppState>, JsonApi(corps): JsonApi<VerifierEmailRequete>) -> Response {
    match verification_email::verifier(&state.db, &corps.jeton).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => reponse_erreur(
            StatusCode::BAD_REQUEST,
            "lien_invalide",
            "Ce lien n'est plus valable. Demandez-en un nouveau.",
        )
        .into_response(),
        // Texte fixe : le message de PostgreSQL pourrait reprendre une
        // valeur, et le jeton ne va jamais aux journaux.
        Err(_) => reponse_erreur_interne("vérification de l'email", "erreur de la base").into_response(),
    }
}

fn reponse_erreur_renvoi(erreur: ErreurRenvoi) -> Response {
    match erreur {
        ErreurRenvoi::IdentifiantsInvalides => {
            reponse_erreur(StatusCode::UNAUTHORIZED, "identifiants_invalides", erreur.to_string()).into_response()
        }
        // Même code et même en-tête que la limite de l'OTP (O5 ; RFC 6585
        // §4) : seule la personne qui a le mot de passe l'apprend.
        ErreurRenvoi::TropTot { attendre_secondes } => {
            let mut reponse =
                reponse_erreur(StatusCode::TOO_MANY_REQUESTS, "trop_de_demandes", erreur.to_string()).into_response();
            reponse.headers_mut().insert(RETRY_AFTER, HeaderValue::from(attendre_secondes));
            reponse
        }
        ErreurRenvoi::Interne(etape) => reponse_erreur_interne("renvoi du lien de vérification", etape).into_response(),
    }
}

/// `POST /api/professionnels/email/renvoyer` (E7) -- corps `{"email",
/// "mot_de_passe"}`, depuis l'écran qui affiche `email_non_verifie`. 204
/// (lien envoyé, ou email déjà vérifié : même réponse) ; 401
/// `identifiants_invalides` ; 429 `trop_de_demandes` avec `Retry-After`.
pub async fn renvoyer_lien_email(State(state): State<AppState>, JsonApi(corps): JsonApi<ConnexionRequete>) -> Response {
    match verification_email::renvoyer(
        &state.db,
        state.email.as_ref(),
        &state.url_application_web,
        &corps.email,
        &corps.mot_de_passe,
    )
    .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(erreur) => reponse_erreur_renvoi(erreur),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erreurs_du_renvoi() {
        let reponse = reponse_erreur_renvoi(ErreurRenvoi::TropTot { attendre_secondes: 42 });
        assert_eq!(reponse.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(reponse.headers().get(RETRY_AFTER).unwrap(), "42");
        assert_eq!(reponse_erreur_renvoi(ErreurRenvoi::IdentifiantsInvalides).status(), StatusCode::UNAUTHORIZED);
        assert_eq!(ErreurRenvoi::IdentifiantsInvalides.to_string(), "Email ou mot de passe incorrect.");
    }

    #[test]
    fn chaque_erreur_a_son_code_http_et_son_code() {
        for (erreur, statut, code, message) in [
            (
                ErreurAuthProfessionnel::IdentifiantsInvalides,
                StatusCode::UNAUTHORIZED,
                "identifiants_invalides",
                "Email ou mot de passe incorrect.",
            ),
            (
                ErreurAuthProfessionnel::EmailNonVerifie,
                StatusCode::FORBIDDEN,
                "email_non_verifie",
                "Votre email n'est pas encore vérifié. Ouvrez le lien que nous vous avons envoyé, ou demandez-en un nouveau.",
            ),
            (
                ErreurAuthProfessionnel::Interne("recherche du compte"),
                StatusCode::INTERNAL_SERVER_ERROR,
                "erreur_interne",
                "erreur interne",
            ),
        ] {
            let (s, Json(corps)) = mapper_erreur(erreur, "connexion professionnelle");
            assert_eq!((s, corps.code, corps.erreur.as_str()), (statut, code, message));
        }
    }

    // Base temporaire ; données fictives. De bout en bout : la route
    // répond 200 avec un jeton `professionnel_sans_affectation`, puis 403
    // quand la vérification de l'email est effacée.
    #[sqlx::test]
    #[ignore]
    async fn connexion_de_bout_en_bout(pool: sqlx::PgPool) {
        use axum::body::to_bytes;
        use laaficare_backend::jwt::{JwtService, SorteJeton};
        use laaficare_backend::mot_de_passe;
        use laaficare_backend::sms::SmsSenderConsole;
        use sqlx::types::Uuid;
        use std::sync::Arc;

        let state = AppState {
            db: pool,
            jwt: JwtService::new(&"a".repeat(32)),
            sms: Arc::new(SmsSenderConsole),
            cle_totp: Arc::new(laaficare_backend::config::CleTotp::depuis_octets([7; 32])),
            email: Arc::new(laaficare_backend::email::EmailSenderConsole),
            url_application_web: Arc::from("http://localhost:4200"),
            contact_securite: Arc::from("securite@example.org"),
            taches_reinitialisation: Arc::new(Default::default()),
        };
        let utilisateur_id: Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (telephone, email) VALUES ('+22670060001', 'pro.route@example.org') RETURNING id",
        )
        .fetch_one(&state.db)
        .await
        .unwrap();
        sqlx::query("UPDATE utilisateur SET email_verifie_le = now() WHERE id = $1")
            .bind(utilisateur_id)
            .execute(&state.db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'professionnel', $2)")
            .bind(utilisateur_id)
            .bind(mot_de_passe::hacher("Exemple#2026").unwrap())
            .execute(&state.db)
            .await
            .unwrap();
        let corps = || ConnexionRequete { email: "pro.route@example.org".into(), mot_de_passe: "Exemple#2026".into() };

        let reponse = se_connecter(State(state.clone()), JsonApi(corps())).await;
        assert_eq!(reponse.status(), StatusCode::OK);
        let json: serde_json::Value =
            serde_json::from_slice(&to_bytes(reponse.into_body(), usize::MAX).await.unwrap()).unwrap();
        assert_eq!(json["etape"], "complete");
        assert_eq!(json["expire_dans"], 900);
        assert_eq!(json["jeton_renouvellement"].as_str().unwrap().len(), 64);
        state
            .jwt
            .verifier(json["jeton_acces"].as_str().unwrap(), SorteJeton::ProfessionnelSansAffectation)
            .unwrap();

        sqlx::query("UPDATE utilisateur SET email_verifie_le = NULL WHERE id = $1")
            .bind(utilisateur_id)
            .execute(&state.db)
            .await
            .unwrap();
        let reponse = se_connecter(State(state.clone()), JsonApi(corps())).await;
        assert_eq!(reponse.status(), StatusCode::FORBIDDEN);
        let json: serde_json::Value =
            serde_json::from_slice(&to_bytes(reponse.into_body(), usize::MAX).await.unwrap()).unwrap();
        assert_eq!(json["code"], "email_non_verifie");
    }

    /// De bout en bout : connexion avec un TOTP actif, second temps avec un
    /// vrai code (200), puis le **même** jeton intermédiaire avec un code de
    /// secours neuf : refusé par l'extracteur (401), avant tout essai de
    /// code.
    #[sqlx::test]
    #[ignore]
    async fn second_facteur_de_bout_en_bout(pool: sqlx::PgPool) {
        use axum::extract::FromRequestParts;
        use axum::http::{Request, header::AUTHORIZATION};
        use laaficare_backend::config::CleTotp;
        use laaficare_backend::jwt::{JwtService, SorteJeton};
        use laaficare_backend::sms::SmsSenderConsole;
        use laaficare_backend::{mot_de_passe, totp};
        use sqlx::types::Uuid;
        use std::sync::Arc;

        let state = AppState {
            db: pool,
            jwt: JwtService::new(&"a".repeat(32)),
            sms: Arc::new(SmsSenderConsole),
            cle_totp: Arc::new(CleTotp::depuis_octets([7; 32])),
            email: Arc::new(laaficare_backend::email::EmailSenderConsole),
            url_application_web: Arc::from("http://localhost:4200"),
            contact_securite: Arc::from("securite@example.org"),
            taches_reinitialisation: Arc::new(Default::default()),
        };
        let utilisateur_id: Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (telephone, email) VALUES ('+22670060002', 'pro.sf.route@example.org') RETURNING id",
        )
        .fetch_one(&state.db)
        .await
        .unwrap();
        sqlx::query("UPDATE utilisateur SET email_verifie_le = now() WHERE id = $1")
            .bind(utilisateur_id)
            .execute(&state.db)
            .await
            .unwrap();
        let compte_id: Uuid = sqlx::query_scalar(
            "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'professionnel', $2) RETURNING id",
        )
        .bind(utilisateur_id)
        .bind(mot_de_passe::hacher("Exemple#2026").unwrap())
        .fetch_one(&state.db)
        .await
        .unwrap();

        // TOTP réel, confirmé avec le code de l'instant présent.
        let activation =
            totp::commencer_activation(&state.db, state.cle_totp.octets(), compte_id, "Exemple#2026").await.unwrap();
        let generateur = totp_rs::Builder::new()
            .with_secret(totp_rs::Secret::try_from_base32(&activation.cle_base32).unwrap())
            .build()
            .unwrap();
        let maintenant = jsonwebtoken::get_current_timestamp();
        // Les codes de secours ne servent pas ici : le jeton déjà consommé
        // est refusé avant même la lecture du corps.
        let _codes_secours = totp::confirmer_activation(
            &state.db,
            state.cle_totp.octets(),
            compte_id,
            &generateur.generate(maintenant).to_string(),
            totp::Autorisation::Libre,
        )
        .await
        .unwrap();

        // Premier temps : jeton intermédiaire.
        let ConnexionProfessionnelle::SecondFacteur { jeton_intermediaire } = auth_professionnel::se_connecter(
            &state.db,
            &state.jwt,
            "pro.sf.route@example.org",
            "Exemple#2026",
        )
        .await
        .unwrap() else {
            panic!("second facteur attendu");
        };
        let en_tete = format!("Bearer {}", jeton_intermediaire.en_clair());
        let extraire = || async {
            let mut parts = Request::builder().header(AUTHORIZATION, en_tete.as_str()).body(()).unwrap().into_parts().0;
            IntermediaireProfessionnel::from_request_parts(&mut parts, &state).await
        };

        // Second temps : 200 et un jeton `professionnel_sans_affectation`.
        let corps = SecondFacteurRequete { code: Some(generateur.generate(maintenant + 30).to_string()), code_secours: None };
        let reponse = second_facteur(extraire().await.unwrap(), State(state.clone()), JsonApi(corps)).await;
        assert_eq!(reponse.status(), StatusCode::OK);
        let json: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(reponse.into_body(), usize::MAX).await.unwrap()).unwrap();
        assert_eq!(json["etape"], "complete");
        state
            .jwt
            .verifier(json["jeton_acces"].as_str().unwrap(), SorteJeton::ProfessionnelSansAffectation)
            .unwrap();

        // Le même jeton intermédiaire, quel que soit le code (même un code
        // de secours neuf) : refusé par l'extracteur, avant tout essai de
        // code. Le refus au niveau de `terminer_connexion` est testé dans
        // auth_professionnel.rs.
        let refus = extraire().await.map_err(IntoResponse::into_response).unwrap_err();
        assert_eq!(refus.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn un_seul_des_deux_champs() {
        let corps = |code: Option<&str>, secours: Option<&str>| SecondFacteurRequete {
            code: code.map(str::to_owned),
            code_secours: secours.map(str::to_owned),
        };
        assert!(matches!(saisie(corps(Some("123456"), None)), Some(SaisieSecondFacteur::Code(_))));
        assert!(matches!(saisie(corps(None, Some("ABCDE-FGHIJ"))), Some(SaisieSecondFacteur::CodeSecours(_))));
        assert!(saisie(corps(Some("123456"), Some("ABCDE-FGHIJ"))).is_none());
        assert!(saisie(corps(None, None)).is_none());
    }

    #[test]
    fn erreurs_du_second_facteur() {
        let contexte = "second facteur professionnel";
        let code_invalide = reponse_erreur_second_facteur(ErreurSecondFacteur::CodeInvalide, contexte);
        assert_eq!(code_invalide.status(), StatusCode::UNAUTHORIZED);
        let deja_utilise = reponse_erreur_second_facteur(ErreurSecondFacteur::JetonDejaUtilise, contexte);
        assert_eq!(deja_utilise.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            deja_utilise.headers().get(axum::http::header::WWW_AUTHENTICATE).unwrap(),
            r#"Bearer error="invalid_token""#
        );
        let interne = reponse_erreur_second_facteur(ErreurSecondFacteur::Interne("vérification du code"), contexte);
        assert_eq!(interne.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(ErreurSecondFacteur::CodeInvalide.to_string(), "Code incorrect.");
    }

    #[test]
    fn forme_de_la_reponse_second_facteur() {
        // `JetonIntermediaire` ne se construit que dans auth_professionnel :
        // la forme est vérifiée sur le corps sérialisé.
        let corps = CorpsSecondFacteur { etape: "second_facteur", jeton_intermediaire: "jeton-fictif" };
        assert_eq!(
            serde_json::to_value(&corps).unwrap(),
            serde_json::json!({ "etape": "second_facteur", "jeton_intermediaire": "jeton-fictif" })
        );
    }
}
