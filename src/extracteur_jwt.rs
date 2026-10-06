//! Extracteurs d'authentification des routes protégées (BF-01).
//!
//! Un extracteur par rôle (décision J2) : une route qui déclare
//! `AgentAssuranceMunasebAuthentifie` en paramètre ne peut pas oublier de
//! vérifier le rôle -- sans jeton valide du bon rôle, le handler n'est
//! jamais appelé.
//!
//! Sources suivies :
//! - axum 0.8.9, `FromRequestParts` (docs.rs) : extracteur personnalisé qui
//!   ne lit que les parties de la requête, pas le corps.
//! - Exemple officiel `examples/jwt` d'axum : lecture du jeton avec
//!   `TypedHeader<Authorization<Bearer>>` d'axum-extra, puis décodage avec
//!   `jsonwebtoken` (ici via `JwtService::verifier`, déjà écrit).
//! - RFC 6750 §3 et §3.1 pour les codes de refus : 401 avec
//!   `WWW-Authenticate: Bearer` quand aucun identifiant n'est fourni, 401
//!   avec `error="invalid_token"` pour un jeton invalide ou expiré, 403 pour
//!   des droits insuffisants. L'exemple d'axum renvoie 400 ; on suit la RFC
//!   (décision J4).

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderValue, StatusCode, header::WWW_AUTHENTICATE};
use axum::response::{IntoResponse, Response};
use axum::RequestPartsExt;
use axum_extra::TypedHeader;
use axum_extra::headers::Authorization;
use axum_extra::headers::authorization::Bearer;
use sqlx::types::Uuid;

use crate::AppState;
use crate::erreur_api::reponse_erreur;
use laaficare_backend::auth_agent_assurance_munaseb::ROLE as ROLE_AGENT_ASSURANCE_MUNASEB;

/// Motif de refus d'une requête protégée. Messages génériques : la réponse
/// n'aide pas à deviner pourquoi un jeton est refusé.
#[derive(Debug)]
pub enum RefusAuthentification {
    /// En-tête `Authorization: Bearer ...` absent ou mal formé.
    IdentifiantsAbsents,
    /// Signature invalide, jeton expiré ou contenu illisible.
    JetonInvalide,
    /// Jeton valide, mais pas le bon rôle, ou compte retiré depuis
    /// l'émission du jeton.
    Interdit,
    Interne(String),
}

impl IntoResponse for RefusAuthentification {
    fn into_response(self) -> Response {
        let (statut, www_authenticate, message, code) = match self {
            // RFC 6750 §3 : sans identifiants, pas de code d'erreur dans
            // l'en-tête.
            RefusAuthentification::IdentifiantsAbsents => {
                (StatusCode::UNAUTHORIZED, Some("Bearer"), "authentification requise", "authentification_requise")
            }
            // RFC 6750 §3.1, invalid_token : "SHOULD respond with the HTTP
            // 401 (Unauthorized) status code".
            RefusAuthentification::JetonInvalide => (
                StatusCode::UNAUTHORIZED,
                Some(r#"Bearer error="invalid_token""#),
                "jeton invalide ou expiré",
                "jeton_invalide",
            ),
            RefusAuthentification::Interdit => (StatusCode::FORBIDDEN, None, "accès refusé", "acces_refuse"),
            // Détail dans les logs seulement, même principe que les routes
            // existantes.
            RefusAuthentification::Interne(detail) => {
                tracing::error!(detail, "erreur interne pendant l'authentification");
                (StatusCode::INTERNAL_SERVER_ERROR, None, "erreur interne", "erreur_interne")
            }
        };
        let mut reponse = reponse_erreur(statut, code, message).into_response();
        if let Some(valeur) = www_authenticate {
            reponse.headers_mut().insert(WWW_AUTHENTICATE, HeaderValue::from_static(valeur));
        }
        reponse
    }
}

/// Agent MUNASEB authentifié : jeton valide, rôle `agent_assurance_munaseb`,
/// et compte agent toujours présent en base.
#[derive(Debug)]
pub struct AgentAssuranceMunasebAuthentifie {
    pub utilisateur_id: Uuid,
}

impl FromRequestParts<AppState> for AgentAssuranceMunasebAuthentifie {
    type Rejection = RefusAuthentification;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let TypedHeader(Authorization(bearer)) = parts
            .extract::<TypedHeader<Authorization<Bearer>>>()
            .await
            .map_err(|_| RefusAuthentification::IdentifiantsAbsents)?;

        // Validation::default() de jsonwebtoken : HS256 et expiration
        // vérifiés (voir jwt.rs).
        let claims = state
            .jwt
            .verifier(bearer.token())
            .map_err(|_| RefusAuthentification::JetonInvalide)?;

        if claims.role != ROLE_AGENT_ASSURANCE_MUNASEB {
            return Err(RefusAuthentification::Interdit);
        }

        let utilisateur_id =
            Uuid::parse_str(&claims.sub).map_err(|_| RefusAuthentification::JetonInvalide)?;

        // Décision J3 : un agent retiré perd l'accès tout de suite, sans
        // attendre l'expiration de son jeton (jusqu'à 24 h).
        let toujours_agent: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM agent_assurance_munaseb WHERE utilisateur_id = $1")
                .bind(utilisateur_id)
                .fetch_optional(&state.db)
                .await
                .map_err(|e| RefusAuthentification::Interne(e.to_string()))?;
        if toujours_agent.is_none() {
            return Err(RefusAuthentification::Interdit);
        }

        Ok(AgentAssuranceMunasebAuthentifie { utilisateur_id })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::http::Request;
    use axum::http::header::AUTHORIZATION;
    use jsonwebtoken::{EncodingKey, Header, encode};
    use laaficare_backend::jwt::{Claims, JwtService};
    use laaficare_backend::sms::SmsSenderConsole;
    use sqlx::PgPool;

    use super::*;

    const SECRET: &str = "secret-de-test-long-de-32-caracteres!";

    fn etat(db: PgPool) -> AppState {
        AppState { db, jwt: JwtService::new(SECRET), sms: Arc::new(SmsSenderConsole) }
    }

    fn parts(authorization: Option<&str>) -> Parts {
        let mut requete = Request::builder();
        if let Some(valeur) = authorization {
            requete = requete.header(AUTHORIZATION, valeur);
        }
        requete.body(()).unwrap().into_parts().0
    }

    async fn extraire(state: &AppState, authorization: Option<&str>) -> Result<AgentAssuranceMunasebAuthentifie, Response> {
        AgentAssuranceMunasebAuthentifie::from_request_parts(&mut parts(authorization), state)
            .await
            .map_err(IntoResponse::into_response)
    }

    fn www_authenticate(reponse: &Response) -> Option<&str> {
        reponse.headers().get(WWW_AUTHENTICATE).map(|v| v.to_str().unwrap())
    }

    // Aucune requête SQL dans ces cas : le refus arrive avant la
    // vérification en base. `connect_lazy` n'ouvre aucune connexion tant
    // qu'on ne s'en sert pas, le test tourne donc sans PostgreSQL.
    #[tokio::test]
    async fn refus_avant_toute_requete_en_base() {
        let state = etat(PgPool::connect_lazy("postgres://localhost/aucune_base").unwrap());
        let id = "11111111-1111-1111-1111-111111111111";

        let absent = extraire(&state, None).await.unwrap_err();
        assert_eq!(absent.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(www_authenticate(&absent), Some("Bearer"));

        let mal_forme = extraire(&state, Some("Basic dXNlcjpwYXNz")).await.unwrap_err();
        assert_eq!(mal_forme.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(www_authenticate(&mal_forme), Some("Bearer"));

        let autre_cle = JwtService::new(&"b".repeat(32)).emettre(id, ROLE_AGENT_ASSURANCE_MUNASEB).unwrap();
        let refus = extraire(&state, Some(&format!("Bearer {autre_cle}"))).await.unwrap_err();
        assert_eq!(refus.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(www_authenticate(&refus), Some(r#"Bearer error="invalid_token""#));

        // Expiré au-delà des 60 s de tolérance de Validation::default().
        let maintenant = jsonwebtoken::get_current_timestamp() as usize;
        let expire = encode(
            &Header::default(),
            &Claims { sub: id.into(), role: ROLE_AGENT_ASSURANCE_MUNASEB.into(), iat: maintenant - 300, exp: maintenant - 200 },
            &EncodingKey::from_secret(SECRET.as_bytes()),
        )
        .unwrap();
        let refus = extraire(&state, Some(&format!("Bearer {expire}"))).await.unwrap_err();
        assert_eq!(refus.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(www_authenticate(&refus), Some(r#"Bearer error="invalid_token""#));

        let patient = state.jwt.emettre(id, "patient").unwrap();
        let refus = extraire(&state, Some(&format!("Bearer {patient}"))).await.unwrap_err();
        assert_eq!(refus.status(), StatusCode::FORBIDDEN);
        assert_eq!(www_authenticate(&refus), None);
    }

    // Base temporaire par test, jamais la base de développement : voir
    // « Tests et environnement », section 12 du CLAUDE.md.
    #[sqlx::test]
    #[ignore]
    async fn agent_valide_puis_retire_en_base(pool: PgPool) {
        let state = etat(pool);

        let utilisateur_id: Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (nom, prenom, telephone, email) \
             VALUES ('Test', 'Extracteur', '+22670011011', 'test.extracteur@example.org') RETURNING id",
        )
        .fetch_one(&state.db)
        .await
        .unwrap();
        sqlx::query("INSERT INTO agent_assurance_munaseb (utilisateur_id) VALUES ($1)")
            .bind(utilisateur_id)
            .execute(&state.db)
            .await
            .unwrap();

        let jeton = state.jwt.emettre(&utilisateur_id.to_string(), ROLE_AGENT_ASSURANCE_MUNASEB).unwrap();
        let en_tete = format!("Bearer {jeton}");

        let agent = extraire(&state, Some(&en_tete)).await.unwrap();
        assert_eq!(agent.utilisateur_id, utilisateur_id);

        // Agent retiré : son jeton, encore valide, est refusé.
        sqlx::query("DELETE FROM agent_assurance_munaseb WHERE utilisateur_id = $1")
            .bind(utilisateur_id)
            .execute(&state.db)
            .await
            .unwrap();
        let refus = extraire(&state, Some(&en_tete)).await.unwrap_err();
        assert_eq!(refus.status(), StatusCode::FORBIDDEN);
    }
}
