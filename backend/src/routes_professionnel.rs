//! Routes de la connexion professionnelle (étape 3.6.2, décisions P1 à P7
//! du 2026-10-15 ; contrat dans docs/api.md). Même principe que les autres
//! fichiers de routes : désérialiser, appeler `auth_professionnel.rs`,
//! traduire le résultat.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::erreur_api::{JsonApi, ReponseErreur, reponse_erreur, reponse_erreur_interne};
use crate::routes_session::reponse_paire;
use laaficare_backend::auth_professionnel::{
    self, ConnexionProfessionnelle, ErreurAuthProfessionnel, JetonIntermediaire,
};

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
fn reponse_second_facteur(jeton: &JetonIntermediaire) -> Response {
    let corps = CorpsSecondFacteur { etape: "second_facteur", jeton_intermediaire: jeton.en_clair() };
    (StatusCode::OK, Json(corps)).into_response()
}

/// Un seul endroit qui décide du code HTTP et du `code` de chaque erreur.
fn mapper_erreur(erreur: ErreurAuthProfessionnel) -> ReponseErreur {
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
        ErreurAuthProfessionnel::Interne(etape) => reponse_erreur_interne("connexion professionnelle", etape),
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
        Err(erreur) => mapper_erreur(erreur).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            let (s, Json(corps)) = mapper_erreur(erreur);
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

        let state = AppState { db: pool, jwt: JwtService::new(&"a".repeat(32)), sms: Arc::new(SmsSenderConsole) };
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
