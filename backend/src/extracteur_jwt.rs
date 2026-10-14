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
use laaficare_backend::jwt::SorteJeton;

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

/// Patient authentifié : jeton `laaficare:patient` valide, compte patient
/// toujours actif avec la même `version_jeton` qu'à l'émission du jeton, et
/// session du `sid` ouverte.
#[derive(Debug)]
pub struct PatientAuthentifie {
    /// Identifiant du compte patient (le `sub` du jeton, décision J3).
    pub compte_id: Uuid,
}

impl FromRequestParts<AppState> for PatientAuthentifie {
    type Rejection = RefusAuthentification;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let TypedHeader(Authorization(bearer)) = parts
            .extract::<TypedHeader<Authorization<Bearer>>>()
            .await
            .map_err(|_| RefusAuthentification::IdentifiantsAbsents)?;

        // Signature HS256, expiration et audience `laaficare:patient`
        // vérifiées par `jwt.rs` : tout autre sorte de jeton est invalide ici.
        let claims = state
            .jwt
            .verifier(bearer.token(), SorteJeton::Patient)
            .map_err(|_| RefusAuthentification::JetonInvalide)?;

        // Révocation (décision C2) : un jeton émis avant une réinitialisation
        // du mot de passe porte une version périmée ; un compte désactivé
        // perd l'accès tout de suite. Les deux sont des jetons révoqués, donc
        // invalides (RFC 6750 §3.1 : « expired, revoked, malformed, or
        // invalid for other reasons »), pas un manque de droits : 401.
        // Le verrouillage n'est pas regardé : il ne porte que sur les essais
        // de connexion (BF-01), pas sur un jeton déjà émis.
        //
        // Session (option B, 2026-10-12) : la session du `sid` doit exister,
        // appartenir à ce compte, et n'être ni révoquée ni expirée. Une
        // déconnexion ferme donc l'accès aussitôt, sans attendre la fin des
        // 15 min du jeton. `jwt.rs` garantit déjà la présence du `sid`.
        let session_id = claims.sid.ok_or(RefusAuthentification::JetonInvalide)?;
        let compte: Option<Uuid> = sqlx::query_scalar(
            "SELECT c.id FROM compte c JOIN session s ON s.compte_id = c.id \
             WHERE c.id = $1 AND c.type_compte = 'patient' AND c.statut = 'actif' AND c.version_jeton = $2 \
               AND s.id = $3 AND s.revoquee_le IS NULL AND s.expire_le > now()",
        )
        .bind(claims.sub)
        .bind(claims.ver)
        .bind(session_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| RefusAuthentification::Interne(e.to_string()))?;
        let compte_id = compte.ok_or(RefusAuthentification::JetonInvalide)?;

        Ok(PatientAuthentifie { compte_id })
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

        // Signature HS256, expiration et audience vérifiées par `jwt.rs` :
        // un jeton d'une autre sorte (patient, intermédiaire…) est un jeton
        // invalide pour cette route (RFC 6750 §3.1, invalid_token), pas un
        // manque de droits.
        let claims = state
            .jwt
            .verifier(bearer.token(), SorteJeton::ProfessionnelEnExercice)
            .map_err(|_| RefusAuthentification::JetonInvalide)?;

        // TEMPORAIRE jusqu'à l'étape 3.9 : la table `agent_assurance_munaseb`
        // a été supprimée par la migration 0011, cette requête échoue donc
        // (500) et les routes MUNASEB restent hors service. L'étape 3.9 la
        // remplace par la vérification de l'affectation portée par le jeton
        // (compte actif, version_jeton, affectation active, structure
        // validée, licence en cours).
        let utilisateur_id: Option<Uuid> = sqlx::query_scalar(
            "SELECT c.utilisateur_id FROM compte c \
             JOIN agent_assurance_munaseb a ON a.utilisateur_id = c.utilisateur_id WHERE c.id = $1",
        )
        .bind(claims.sub)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| RefusAuthentification::Interne(e.to_string()))?;
        let utilisateur_id = utilisateur_id.ok_or(RefusAuthentification::Interdit)?;

        Ok(AgentAssuranceMunasebAuthentifie { utilisateur_id })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::http::Request;
    use axum::http::header::AUTHORIZATION;
    use jsonwebtoken::{EncodingKey, Header, encode};
    use laaficare_backend::jwt::JwtService;
    use laaficare_backend::session;
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

    async fn extraire_patient(state: &AppState, authorization: Option<&str>) -> Result<PatientAuthentifie, Response> {
        PatientAuthentifie::from_request_parts(&mut parts(authorization), state)
            .await
            .map_err(IntoResponse::into_response)
    }

    /// `sid` attendu par `jwt.rs` : présent pour les quatre sortes
    /// complètes, absent ailleurs.
    fn sid_pour(sorte: SorteJeton) -> Option<Uuid> {
        matches!(
            sorte,
            SorteJeton::Patient
                | SorteJeton::ProfessionnelSansAffectation
                | SorteJeton::ProfessionnelEnExercice
                | SorteJeton::Administrateur
        )
        .then_some(Uuid::from_u128(9))
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
        let id = Uuid::from_u128(1);
        let affectation = Some(Uuid::from_u128(7));

        let absent = extraire(&state, None).await.unwrap_err();
        assert_eq!(absent.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(www_authenticate(&absent), Some("Bearer"));

        let mal_forme = extraire(&state, Some("Basic dXNlcjpwYXNz")).await.unwrap_err();
        assert_eq!(mal_forme.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(www_authenticate(&mal_forme), Some("Bearer"));

        let autre_cle = JwtService::new(&"b".repeat(32))
            .emettre(SorteJeton::ProfessionnelEnExercice, id, 0, affectation, sid_pour(SorteJeton::ProfessionnelEnExercice))
            .unwrap();
        let refus = extraire(&state, Some(&format!("Bearer {autre_cle}"))).await.unwrap_err();
        assert_eq!(refus.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(www_authenticate(&refus), Some(r#"Bearer error="invalid_token""#));

        // Expiré d'une seconde : aucune tolérance (leeway = 0, jwt.rs).
        let maintenant = jsonwebtoken::get_current_timestamp();
        let expire = encode(
            &Header::default(),
            &serde_json::json!({
                "sub": id, "aud": SorteJeton::ProfessionnelEnExercice.audience(), "ver": 0,
                "aff": affectation, "sid": sid_pour(SorteJeton::ProfessionnelEnExercice),
                "iat": maintenant - 60, "exp": maintenant - 1
            }),
            &EncodingKey::from_secret(SECRET.as_bytes()),
        )
        .unwrap();
        let refus = extraire(&state, Some(&format!("Bearer {expire}"))).await.unwrap_err();
        assert_eq!(refus.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(www_authenticate(&refus), Some(r#"Bearer error="invalid_token""#));

        // Toute autre sorte de jeton, même bien signée : mauvaise audience,
        // donc jeton invalide pour cette route (401), jamais 403.
        for sorte in SorteJeton::TOUTES {
            if sorte == SorteJeton::ProfessionnelEnExercice {
                continue;
            }
            let jeton = state.jwt.emettre(sorte, id, 0, None, sid_pour(sorte)).unwrap();
            let refus = extraire(&state, Some(&format!("Bearer {jeton}"))).await.unwrap_err();
            assert_eq!(refus.status(), StatusCode::UNAUTHORIZED, "{sorte:?}");
            assert_eq!(www_authenticate(&refus), Some(r#"Bearer error="invalid_token""#));
        }

        // Extracteur patient : sans en-tête, ou avec toute autre sorte de
        // jeton, refus avant la base.
        let absent = extraire_patient(&state, None).await.unwrap_err();
        assert_eq!(absent.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(www_authenticate(&absent), Some("Bearer"));
        for sorte in SorteJeton::TOUTES {
            if sorte == SorteJeton::Patient {
                continue;
            }
            let aff = (sorte == SorteJeton::ProfessionnelEnExercice).then_some(Uuid::from_u128(7));
            let jeton = state.jwt.emettre(sorte, id, 0, aff, sid_pour(sorte)).unwrap();
            let refus = extraire_patient(&state, Some(&format!("Bearer {jeton}"))).await.unwrap_err();
            assert_eq!(refus.status(), StatusCode::UNAUTHORIZED, "{sorte:?}");
            assert_eq!(www_authenticate(&refus), Some(r#"Bearer error="invalid_token""#));
        }
    }

    #[sqlx::test]
    #[ignore]
    async fn patient_version_statut_et_verrouillage_en_base(pool: PgPool) {
        let state = etat(pool);
        let utilisateur_id: Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (nom, prenom, telephone) VALUES ('Test', 'Patient', '+22670021001') RETURNING id",
        )
        .fetch_one(&state.db)
        .await
        .unwrap();
        let compte_id: Uuid = sqlx::query_scalar(
            "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'patient', 'x') RETURNING id",
        )
        .bind(utilisateur_id)
        .fetch_one(&state.db)
        .await
        .unwrap();
        let session_id = session::ouvrir(&state.db, compte_id, None).await.unwrap().session_id;
        let jeton = |version: i32| state.jwt.emettre(SorteJeton::Patient, compte_id, version, None, Some(session_id));
        let en_tete = format!("Bearer {}", jeton(0).unwrap());
        let modifier = |sql: &'static str| {
            let db = state.db.clone();
            async move { sqlx::query(sql).bind(compte_id).execute(&db).await.unwrap() }
        };

        // Jeton valide : accepté.
        assert_eq!(extraire_patient(&state, Some(&en_tete)).await.unwrap().compte_id, compte_id);

        // Compte verrouillé : le jeton déjà émis reste accepté (BF-01 ne
        // porte que sur les essais de connexion).
        modifier("UPDATE compte SET verrouille_jusqua = now() + interval '15 minutes' WHERE id = $1").await;
        assert!(extraire_patient(&state, Some(&en_tete)).await.is_ok());

        // Version augmentée (réinitialisation du mot de passe) : refusé.
        modifier("UPDATE compte SET version_jeton = version_jeton + 1 WHERE id = $1").await;
        let refus = extraire_patient(&state, Some(&en_tete)).await.unwrap_err();
        assert_eq!(refus.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(www_authenticate(&refus), Some(r#"Bearer error="invalid_token""#));

        // Un jeton émis avec la nouvelle version passe ; compte désactivé : refusé.
        let nouveau = format!("Bearer {}", jeton(1).unwrap());
        assert!(extraire_patient(&state, Some(&nouveau)).await.is_ok());
        modifier("UPDATE compte SET statut = 'desactive' WHERE id = $1").await;
        let refus = extraire_patient(&state, Some(&nouveau)).await.unwrap_err();
        assert_eq!(refus.status(), StatusCode::UNAUTHORIZED);
    }

    /// Un jeton d'accès encore valide (moins de 15 min) dont la session a
    /// été fermée est refusé aussitôt ; de même pour une session inconnue
    /// ou expirée, ou une session d'un autre compte.
    #[sqlx::test]
    #[ignore]
    async fn patient_session_fermee_refusee(pool: PgPool) {
        let state = etat(pool);
        let mut comptes = Vec::new();
        for telephone in ["+22670021002", "+22670021003"] {
            let utilisateur_id: Uuid = sqlx::query_scalar("INSERT INTO utilisateur (telephone) VALUES ($1) RETURNING id")
                .bind(telephone)
                .fetch_one(&state.db)
                .await
                .unwrap();
            let compte_id: Uuid = sqlx::query_scalar(
                "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'patient', 'x') RETURNING id",
            )
            .bind(utilisateur_id)
            .fetch_one(&state.db)
            .await
            .unwrap();
            comptes.push(compte_id);
        }
        let (compte_id, autre_compte) = (comptes[0], comptes[1]);
        let en_tete = |session: Uuid| {
            format!(
                "Bearer {}",
                state.jwt.emettre(SorteJeton::Patient, compte_id, 0, None, Some(session)).unwrap()
            )
        };
        let refuse = |reponse: Result<PatientAuthentifie, Response>| {
            reponse.is_err_and(|r| r.status() == StatusCode::UNAUTHORIZED)
        };

        // Session ouverte : accepté ; fermée par déconnexion : refusé.
        let ouverte = session::ouvrir(&state.db, compte_id, None).await.unwrap();
        let acces = en_tete(ouverte.session_id);
        assert_eq!(extraire_patient(&state, Some(&acces)).await.unwrap().compte_id, compte_id);
        session::fermer(&state.db, ouverte.jeton_renouvellement.en_clair()).await.unwrap();
        assert!(refuse(extraire_patient(&state, Some(&acces)).await), "session fermée");

        // Session inconnue.
        assert!(refuse(extraire_patient(&state, Some(&en_tete(Uuid::from_u128(5)))).await), "session inconnue");

        // Session expirée.
        let expiree = session::ouvrir(&state.db, compte_id, None).await.unwrap();
        sqlx::query("UPDATE session SET creee_le = now() - interval '2 hours', expire_le = now() - interval '1 hour' WHERE id = $1")
            .bind(expiree.session_id)
            .execute(&state.db)
            .await
            .unwrap();
        assert!(refuse(extraire_patient(&state, Some(&en_tete(expiree.session_id))).await), "session expirée");

        // Session d'un autre compte.
        let autre = session::ouvrir(&state.db, autre_compte, None).await.unwrap();
        assert!(refuse(extraire_patient(&state, Some(&en_tete(autre.session_id))).await), "session d'un autre compte");
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

        // Échoue dès l'insertion ci-dessus (table supprimée) : test réécrit
        // à l'étape 3.9, avec les affectations.
        let jeton = state
            .jwt
            .emettre(SorteJeton::ProfessionnelEnExercice, utilisateur_id, 0, Some(Uuid::from_u128(7)), Some(Uuid::from_u128(9)))
            .unwrap();
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
