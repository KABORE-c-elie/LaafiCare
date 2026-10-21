//! Extracteurs d'authentification des routes protégées (BF-01).
//!
//! Un extracteur par rôle (décision J2) : une route qui déclare
//! `AgentMunaseb` en paramètre ne peut pas oublier de vérifier le rôle --
//! sans jeton valide du bon rôle, le handler n'est jamais appelé.
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
use laaficare_backend::affectation::{self, ControleExercice, Exercice};
use laaficare_backend::demande_remboursement_munaseb::{AffectationAgent, ROLE_AGENT};
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

/// Jeton intermédiaire du second facteur professionnel (étape 3.6.3) :
/// mot de passe correct, code TOTP ou code de secours encore à saisir.
#[derive(Debug)]
pub struct IntermediaireProfessionnel {
    pub compte_id: Uuid,
    /// Date d'émission du jeton : `auth_professionnel::terminer_connexion`
    /// le consomme (usage unique, F5 b).
    pub iat: u64,
}

impl FromRequestParts<AppState> for IntermediaireProfessionnel {
    type Rejection = RefusAuthentification;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let (compte_id, iat) =
            intermediaire(parts, state, SorteJeton::SecondFacteurProfessionnel, "professionnel").await?;
        Ok(IntermediaireProfessionnel { compte_id, iat })
    }
}

/// Jeton intermédiaire du second facteur administrateur (étape 3.6.4, A5) :
/// mêmes contrôles que pour un professionnel, sur un compte administrateur.
#[derive(Debug)]
pub struct IntermediaireAdministrateur {
    pub compte_id: Uuid,
    /// Consommé par `auth_professionnel::terminer_connexion` (F5 b).
    pub iat: u64,
}

impl FromRequestParts<AppState> for IntermediaireAdministrateur {
    type Rejection = RefusAuthentification;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let (compte_id, iat) =
            intermediaire(parts, state, SorteJeton::SecondFacteurAdministrateur, "administrateur_laaficare").await?;
        Ok(IntermediaireAdministrateur { compte_id, iat })
    }
}

/// En-tête `Authorization: Bearer …` ; absent ou mal formé : 401 sans code
/// d'erreur (RFC 6750 §3).
async fn bearer_de(parts: &mut Parts) -> Result<Bearer, RefusAuthentification> {
    let TypedHeader(Authorization(bearer)) = parts
        .extract::<TypedHeader<Authorization<Bearer>>>()
        .await
        .map_err(|_| RefusAuthentification::IdentifiantsAbsents)?;
    Ok(bearer)
}

/// Contrôles communs aux deux jetons intermédiaires (A5). `sorte` : la
/// seule sorte acceptée (J1), 5 min ; `type_compte` : valeur de
/// `compte.type_compte`. Renvoie le compte et l'`iat` du jeton.
async fn intermediaire(
    parts: &mut Parts,
    state: &AppState,
    sorte: SorteJeton,
    type_compte: &'static str,
) -> Result<(Uuid, u64), RefusAuthentification> {
    let bearer = bearer_de(parts).await?;
    let claims = state.jwt.verifier(bearer.token(), sorte).map_err(|_| RefusAuthentification::JetonInvalide)?;
    let iat = i64::try_from(claims.iat).map_err(|_| RefusAuthentification::JetonInvalide)?;

    // En une requête, avant tout essai de code :
    // - compte du bon type, actif, même `version_jeton` (une
    //   réinitialisation du mot de passe invalide le jeton) ;
    // - email toujours vérifié (R0 : il a pu changer dans les 5 min) ;
    // - jeton jamais consommé (F5 b) : `iat` plus grand que le dernier
    //   jeton consommé. Les deux valeurs viennent de l'horloge du
    //   serveur Rust (migration 0018). La consommation elle-même est
    //   faite plus tard, en une requête conditionnelle qui couvre deux
    //   requêtes simultanées.
    // Un refus est un jeton invalide (RFC 6750 §3.1) : 401, et
    // l'application revient à l'écran de connexion.
    let compte: Option<Uuid> = sqlx::query_scalar(
        "SELECT c.id FROM compte c JOIN utilisateur u ON u.id = c.utilisateur_id \
         WHERE c.id = $1 AND c.type_compte = $4 AND c.statut = 'actif' AND c.version_jeton = $2 \
           AND u.email_verifie_le IS NOT NULL \
           AND (c.second_facteur_dernier_iat IS NULL OR c.second_facteur_dernier_iat < $3)",
    )
    .bind(claims.sub)
    .bind(claims.ver)
    .bind(iat)
    .bind(type_compte)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| RefusAuthentification::Interne(e.to_string()))?;
    let compte_id = compte.ok_or(RefusAuthentification::JetonInvalide)?;
    Ok((compte_id, claims.iat))
}

/// Jeton de réenrôlement d'un administrateur (étape 3.6.4, A4, migration
/// 0019) : ne sert qu'aux routes `reenrolement/*`.
#[derive(Debug)]
pub struct ReenrolementAdministrateur {
    pub compte_id: Uuid,
}

impl FromRequestParts<AppState> for ReenrolementAdministrateur {
    type Rejection = RefusAuthentification;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let bearer = bearer_de(parts).await?;
        let claims = state
            .jwt
            .verifier(bearer.token(), SorteJeton::ReenrolementAdministrateur)
            .map_err(|_| RefusAuthentification::JetonInvalide)?;

        // À chaque requête : compte administrateur actif, même version,
        // email vérifié, **aucun TOTP actif** (le jeton s'use donc de
        // lui-même dès la confirmation, A4), et **fenêtre de réenrôlement
        // en cours**, comparée à `now()` de PostgreSQL qui l'a calculée
        // (une seule horloge, migration 0019). Une demande de TOTP en
        // attente n'est pas un TOTP actif : `commencer` peut être relancé.
        let compte: Option<Uuid> = sqlx::query_scalar(
            "SELECT c.id FROM compte c JOIN utilisateur u ON u.id = c.utilisateur_id \
             WHERE c.id = $1 AND c.type_compte = 'administrateur_laaficare' AND c.statut = 'actif' \
               AND c.version_jeton = $2 AND u.email_verifie_le IS NOT NULL \
               AND c.reenrolement_autorise_jusqu_au > now() \
               AND NOT EXISTS (SELECT 1 FROM second_facteur_totp t WHERE t.compte_id = c.id AND t.statut = 'actif')",
        )
        .bind(claims.sub)
        .bind(claims.ver)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| RefusAuthentification::Interne(e.to_string()))?;
        let compte_id = compte.ok_or(RefusAuthentification::JetonInvalide)?;
        Ok(ReenrolementAdministrateur { compte_id })
    }
}

/// Administrateur LaafiCare authentifié (étape 3.6.4, A6), pour les routes
/// réservées à l'équipe (décisions sur les structures, étape 3.8).
#[derive(Debug)]
pub struct AdministrateurAuthentifie {
    pub compte_id: Uuid,
}

impl FromRequestParts<AppState> for AdministrateurAuthentifie {
    type Rejection = RefusAuthentification;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let bearer = bearer_de(parts).await?;
        let claims = state
            .jwt
            .verifier(bearer.token(), SorteJeton::Administrateur)
            .map_err(|_| RefusAuthentification::JetonInvalide)?;
        let session_id = claims.sid.ok_or(RefusAuthentification::JetonInvalide)?;

        // À chaque requête, comme pour le patient (session du `sid` ouverte,
        // compte actif, même version), plus :
        // - email toujours vérifié (R0) ;
        // - **TOTP actif** (Y4) : un administrateur dont le TOTP vient
        //   d'être désactivé est refusé aussitôt, même avec un jeton
        //   d'accès encore valable.
        let compte: Option<Uuid> = sqlx::query_scalar(
            "SELECT c.id FROM compte c JOIN utilisateur u ON u.id = c.utilisateur_id \
                                     JOIN session s ON s.compte_id = c.id \
             WHERE c.id = $1 AND c.type_compte = 'administrateur_laaficare' AND c.statut = 'actif' \
               AND c.version_jeton = $2 AND u.email_verifie_le IS NOT NULL \
               AND s.id = $3 AND s.revoquee_le IS NULL AND s.expire_le > now() \
               AND EXISTS (SELECT 1 FROM second_facteur_totp t WHERE t.compte_id = c.id AND t.statut = 'actif')",
        )
        .bind(claims.sub)
        .bind(claims.ver)
        .bind(session_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| RefusAuthentification::Interne(e.to_string()))?;
        let compte_id = compte.ok_or(RefusAuthentification::JetonInvalide)?;
        Ok(AdministrateurAuthentifie { compte_id })
    }
}

/// Professionnel connecté, avec ou sans affectation (étape 3.9, D2) : pour
/// lister ses affectations et en activer une. **Exception assumée à J1**,
/// limitée à ces deux routes : deux sortes de jeton sont acceptées,
/// `professionnel_sans_affectation` (juste après la connexion) et
/// `professionnel_en_exercice` (changer de structure sans mot de passe,
/// J5). Chacune est vérifiée avec sa propre audience, l'une après l'autre.
#[derive(Debug)]
pub struct ProfessionnelConnecte {
    pub compte_id: Uuid,
    pub session_id: Uuid,
}

impl FromRequestParts<AppState> for ProfessionnelConnecte {
    type Rejection = RefusAuthentification;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let bearer = bearer_de(parts).await?;
        let claims = state
            .jwt
            .verifier(bearer.token(), SorteJeton::ProfessionnelSansAffectation)
            .or_else(|_| state.jwt.verifier(bearer.token(), SorteJeton::ProfessionnelEnExercice))
            .map_err(|_| RefusAuthentification::JetonInvalide)?;
        let session_id = claims.sid.ok_or(RefusAuthentification::JetonInvalide)?;

        // À chaque requête : session du `sid` ouverte et de ce compte (son
        // affectation n'est pas regardée : on peut quitter une affectation
        // désactivée pour en choisir une autre), compte professionnel actif,
        // même version, email toujours vérifié (R0).
        let compte: Option<Uuid> = sqlx::query_scalar(
            "SELECT c.id FROM compte c JOIN utilisateur u ON u.id = c.utilisateur_id \
                                     JOIN session s ON s.compte_id = c.id \
             WHERE c.id = $1 AND c.type_compte = 'professionnel' AND c.statut = 'actif' AND c.version_jeton = $2 \
               AND u.email_verifie_le IS NOT NULL \
               AND s.id = $3 AND s.revoquee_le IS NULL AND s.expire_le > now()",
        )
        .bind(claims.sub)
        .bind(claims.ver)
        .bind(session_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| RefusAuthentification::Interne(e.to_string()))?;
        let compte_id = compte.ok_or(RefusAuthentification::JetonInvalide)?;
        Ok(ProfessionnelConnecte { compte_id, session_id })
    }
}

/// Professionnel en exercice (étape 3.9, D4) : jeton
/// `laaficare:professionnel_en_exercice` seulement, dont l'affectation (`aff`)
/// est vérifiée à chaque requête par `affectation::controler_exercice` :
/// session ouverte et de cette affectation, compte actif, même version,
/// email vérifié (401 sinon) ; affectation active, établissement validé,
/// licence en cours à la date du Burkina (403 sinon : droit retiré, J1).
#[derive(Debug)]
pub struct ProfessionnelEnExercice(pub Exercice);

impl FromRequestParts<AppState> for ProfessionnelEnExercice {
    type Rejection = RefusAuthentification;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let bearer = bearer_de(parts).await?;
        let claims = state
            .jwt
            .verifier(bearer.token(), SorteJeton::ProfessionnelEnExercice)
            .map_err(|_| RefusAuthentification::JetonInvalide)?;
        // `jwt.rs` garantit déjà `aff` et `sid` pour cette sorte de jeton.
        let (Some(affectation_id), Some(session_id)) = (claims.aff, claims.sid) else {
            return Err(RefusAuthentification::JetonInvalide);
        };
        match affectation::controler_exercice(&state.db, claims.sub, claims.ver, session_id, affectation_id).await {
            Ok(ControleExercice::Valide(exercice)) => Ok(ProfessionnelEnExercice(exercice)),
            Ok(ControleExercice::JetonInvalide) => Err(RefusAuthentification::JetonInvalide),
            Ok(ControleExercice::DroitRetire) => Err(RefusAuthentification::Interdit),
            Err(e) => Err(RefusAuthentification::Interne(e.to_string())),
        }
    }
}

/// Agent MUNASEB en exercice (étape 3.9, D5), pour les routes des
/// remboursements : un professionnel en exercice (tous les contrôles de
/// `ProfessionnelEnExercice`) dont l'affectation a le rôle
/// `agent_assurance_munaseb` dans une structure de type `munaseb`. Sinon :
/// 403 `acces_refuse`.
#[derive(Debug)]
pub struct AgentMunaseb(pub AffectationAgent);

impl FromRequestParts<AppState> for AgentMunaseb {
    type Rejection = RefusAuthentification;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let ProfessionnelEnExercice(exercice) = ProfessionnelEnExercice::from_request_parts(parts, state).await?;

        // Contrôle de la structure (décision du porteur, 2026-10-18, option
        // a). Une demande de remboursement n'est rattachée à aucune
        // structure (migrations 0006, 0007, 0009 : MUNASEB est câblée en dur,
        // section 12) : on ne peut pas comparer « la structure de la
        // demande » à celle de l'agent. Ce contrôle suffit pourtant à ce
        // qu'un agent ne touche que les demandes de sa mutuelle :
        // - l'agent exerce dans une structure de type `munaseb`, validée et
        //   sous licence (`ProfessionnelEnExercice`, ci-dessus) ;
        // - la base n'admet qu'**une seule** structure MUNASEB `validee` ou
        //   `suspendue` (index `structure_une_seule_munaseb_validee`,
        //   migration 0012).
        // Tout agent qui peut agir appartient donc à la seule MUNASEB qui
        // puisse exister. Un agent d'une autre demande MUNASEB (brouillon,
        // en attente) est refusé, puisque sa structure n'est pas validée. Le
        // rattachement des contrats et des demandes à une structure viendra
        // avec le module « Assurance » générique, avant d'admettre un second
        // assureur (docs/points-ouverts.md).
        if exercice.role != ROLE_AGENT || exercice.type_structure != "munaseb" {
            return Err(RefusAuthentification::Interdit);
        }
        Ok(AgentMunaseb(AffectationAgent {
            affectation_id: exercice.affectation_id,
            structure_id: exercice.structure_id,
        }))
    }
}

/// N'importe quel compte connecté, pour `POST /api/sessions/deconnexion-partout`
/// (étape 3.9, D8) : jeton d'accès patient, professionnel (avec ou sans
/// affectation) ou administrateur, chacun vérifié avec sa propre audience.
/// Seules la session et le compte sont vérifiés (session ouverte et de ce
/// compte, compte actif, même version) : ni l'affectation, ni l'email, ni le
/// TOTP. On doit pouvoir se déconnecter partout même si son affectation
/// vient d'être désactivée.
#[derive(Debug)]
pub struct CompteConnecte {
    pub compte_id: Uuid,
}

impl FromRequestParts<AppState> for CompteConnecte {
    type Rejection = RefusAuthentification;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        const SORTES: [SorteJeton; 4] = [
            SorteJeton::Patient,
            SorteJeton::ProfessionnelSansAffectation,
            SorteJeton::ProfessionnelEnExercice,
            SorteJeton::Administrateur,
        ];
        let bearer = bearer_de(parts).await?;
        let claims = SORTES
            .iter()
            .find_map(|sorte| state.jwt.verifier(bearer.token(), *sorte).ok())
            .ok_or(RefusAuthentification::JetonInvalide)?;
        let session_id = claims.sid.ok_or(RefusAuthentification::JetonInvalide)?;
        let compte: Option<Uuid> = sqlx::query_scalar(
            "SELECT c.id FROM compte c JOIN session s ON s.compte_id = c.id \
             WHERE c.id = $1 AND c.statut = 'actif' AND c.version_jeton = $2 \
               AND s.id = $3 AND s.revoquee_le IS NULL AND s.expire_le > now()",
        )
        .bind(claims.sub)
        .bind(claims.ver)
        .bind(session_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| RefusAuthentification::Interne(e.to_string()))?;
        let compte_id = compte.ok_or(RefusAuthentification::JetonInvalide)?;
        Ok(CompteConnecte { compte_id })
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
        AppState {
            db,
            jwt: JwtService::new(SECRET),
            sms: Arc::new(SmsSenderConsole),
            cle_totp: Arc::new(laaficare_backend::config::CleTotp::depuis_octets([7; 32])),
            email: Arc::new(laaficare_backend::email::EmailSenderConsole),
            url_application_web: Arc::from("http://localhost:4200"),
            contact_securite: Arc::from("securite@example.org"),
        }
    }

    fn parts(authorization: Option<&str>) -> Parts {
        let mut requete = Request::builder();
        if let Some(valeur) = authorization {
            requete = requete.header(AUTHORIZATION, valeur);
        }
        requete.body(()).unwrap().into_parts().0
    }

    async fn extraire(state: &AppState, authorization: Option<&str>) -> Result<AgentMunaseb, Response> {
        AgentMunaseb::from_request_parts(&mut parts(authorization), state)
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

    /// Extracteur du jeton intermédiaire professionnel : accepté seulement
    /// pour un compte professionnel actif, même version, email vérifié, et
    /// jamais consommé ; tout autre cas 401.
    #[sqlx::test]
    #[ignore]
    async fn intermediaire_professionnel_en_base(pool: PgPool) {
        let state = etat(pool);
        let utilisateur_id: Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (telephone, email) VALUES ('+22670021010', 'inter.pro@example.org') RETURNING id",
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
            "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'professionnel', 'x') RETURNING id",
        )
        .bind(utilisateur_id)
        .fetch_one(&state.db)
        .await
        .unwrap();
        let jeton = state.jwt.emettre(SorteJeton::SecondFacteurProfessionnel, compte_id, 0, None, None).unwrap();
        let iat = state.jwt.verifier(&jeton, SorteJeton::SecondFacteurProfessionnel).unwrap().iat as i64;
        let en_tete = format!("Bearer {jeton}");
        let extraire_inter = || async {
            IntermediaireProfessionnel::from_request_parts(&mut parts(Some(&en_tete)), &state)
                .await
                .map_err(IntoResponse::into_response)
        };
        let executer = |sql: &'static str, valeur: i64| {
            let db = state.db.clone();
            async move { sqlx::query(sql).bind(compte_id).bind(valeur).execute(&db).await.unwrap() }
        };
        let refuse = |resultat: Result<IntermediaireProfessionnel, Response>| {
            resultat.is_err_and(|r| r.status() == StatusCode::UNAUTHORIZED)
        };

        // Accepté.
        let accepte = extraire_inter().await.unwrap();
        assert_eq!((accepte.compte_id, accepte.iat as i64), (compte_id, iat));

        // Usage unique : un jeton dont l'`iat` a déjà été consommé est refusé
        // avant tout essai de code ; un `iat` plus récent que le dernier
        // consommé est accepté.
        executer("UPDATE compte SET second_facteur_dernier_iat = $2 WHERE id = $1", iat).await;
        assert!(refuse(extraire_inter().await), "jeton déjà consommé");
        executer("UPDATE compte SET second_facteur_dernier_iat = $2 WHERE id = $1", iat - 1).await;
        assert!(extraire_inter().await.is_ok(), "jeton plus récent que le dernier consommé");

        // Version augmentée, email plus vérifié, compte désactivé : refusés.
        executer("UPDATE compte SET version_jeton = version_jeton + $2 WHERE id = $1", 1).await;
        assert!(refuse(extraire_inter().await), "version");
        executer("UPDATE compte SET version_jeton = version_jeton - $2 WHERE id = $1", 1).await;
        sqlx::query("UPDATE utilisateur SET email_verifie_le = NULL WHERE id = $1")
            .bind(utilisateur_id)
            .execute(&state.db)
            .await
            .unwrap();
        assert!(refuse(extraire_inter().await), "email plus vérifié");
        sqlx::query("UPDATE utilisateur SET email_verifie_le = now() WHERE id = $1")
            .bind(utilisateur_id)
            .execute(&state.db)
            .await
            .unwrap();
        sqlx::query("UPDATE compte SET statut = 'desactive' WHERE id = $1")
            .bind(compte_id)
            .execute(&state.db)
            .await
            .unwrap();
        assert!(refuse(extraire_inter().await), "compte désactivé");

        // Une autre sorte de jeton (ici patient) : refusée sans regarder la base.
        let patient = state.jwt.emettre(SorteJeton::Patient, compte_id, 0, None, Some(Uuid::from_u128(9))).unwrap();
        let refus = IntermediaireProfessionnel::from_request_parts(&mut parts(Some(&format!("Bearer {patient}"))), &state)
            .await
            .map_err(IntoResponse::into_response);
        assert!(refuse(refus));
    }

    /// Extracteurs administrateur (étape 3.6.4) : jeton intermédiaire,
    /// jeton de réenrôlement (fenêtre en cours, aucun TOTP actif) et jeton
    /// d'accès (session ouverte, TOTP actif). Tout refus : 401.
    #[sqlx::test]
    #[ignore]
    async fn extracteurs_administrateur_en_base(pool: PgPool) {
        use laaficare_backend::{mot_de_passe, totp};

        const MOT_DE_PASSE: &str = "Exemple#2026";
        let state = etat(pool);
        let creer = |telephone: &'static str, email: &'static str, type_compte: &'static str| {
            let db = state.db.clone();
            async move {
                let utilisateur_id: Uuid =
                    sqlx::query_scalar("INSERT INTO utilisateur (telephone, email) VALUES ($1, $2) RETURNING id")
                        .bind(telephone)
                        .bind(email)
                        .fetch_one(&db)
                        .await
                        .unwrap();
                sqlx::query("UPDATE utilisateur SET email_verifie_le = now() WHERE id = $1")
                    .bind(utilisateur_id)
                    .execute(&db)
                    .await
                    .unwrap();
                let compte_id: Uuid = sqlx::query_scalar(
                    "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, $2, $3) RETURNING id",
                )
                .bind(utilisateur_id)
                .bind(type_compte)
                .bind(mot_de_passe::hacher(MOT_DE_PASSE).unwrap())
                .fetch_one(&db)
                .await
                .unwrap();
                compte_id
            }
        };
        let administrateur = creer("+22670021020", "adm.extr@example.org", "administrateur_laaficare").await;
        let professionnel = creer("+22670021021", "pro.extr@example.org", "professionnel").await;
        let fenetre = |minutes: i32| {
            let db = state.db.clone();
            async move {
                sqlx::query(
                    "UPDATE compte SET reenrolement_autorise_jusqu_au = now() + make_interval(mins => $2) WHERE id = $1",
                )
                .bind(administrateur)
                .bind(minutes)
                .execute(&db)
                .await
                .unwrap();
            }
        };
        let refuse = |resultat: Result<Uuid, Response>| resultat.is_err_and(|r| r.status() == StatusCode::UNAUTHORIZED);
        let en_tete = |sorte: SorteJeton, compte_id: Uuid, sid: Option<Uuid>| {
            format!("Bearer {}", state.jwt.emettre(sorte, compte_id, 0, None, sid).unwrap())
        };
        let st = &state;
        let inter = |valeur: String| async move {
            IntermediaireAdministrateur::from_request_parts(&mut parts(Some(&valeur)), st)
                .await
                .map(|e| e.compte_id)
                .map_err(IntoResponse::into_response)
        };
        let reenrolement = |valeur: String| async move {
            ReenrolementAdministrateur::from_request_parts(&mut parts(Some(&valeur)), st)
                .await
                .map(|e| e.compte_id)
                .map_err(IntoResponse::into_response)
        };
        let acces = |valeur: String| async move {
            AdministrateurAuthentifie::from_request_parts(&mut parts(Some(&valeur)), st)
                .await
                .map(|e| e.compte_id)
                .map_err(IntoResponse::into_response)
        };

        // Jeton intermédiaire : accepté pour l'administrateur ; refusé pour
        // un professionnel (mauvais type de compte) et pour un jeton
        // intermédiaire professionnel (mauvaise sorte).
        assert_eq!(inter(en_tete(SorteJeton::SecondFacteurAdministrateur, administrateur, None)).await.unwrap(), administrateur);
        assert!(refuse(inter(en_tete(SorteJeton::SecondFacteurAdministrateur, professionnel, None)).await));
        assert!(refuse(inter(en_tete(SorteJeton::SecondFacteurProfessionnel, administrateur, None)).await));

        // Réenrôlement : refusé sans fenêtre, accepté pendant, refusé après.
        let jeton_reenrolement = en_tete(SorteJeton::ReenrolementAdministrateur, administrateur, None);
        assert!(refuse(reenrolement(jeton_reenrolement.clone()).await), "sans fenêtre");
        fenetre(10).await;
        assert_eq!(reenrolement(jeton_reenrolement.clone()).await.unwrap(), administrateur);
        fenetre(-1).await;
        assert!(refuse(reenrolement(jeton_reenrolement.clone()).await), "fenêtre expirée");
        fenetre(10).await;

        // Jeton d'accès sans TOTP actif : refusé (Y4).
        let session_id = session::ouvrir(&state.db, administrateur, None).await.unwrap().session_id;
        let jeton_acces = en_tete(SorteJeton::Administrateur, administrateur, Some(session_id));
        assert!(refuse(acces(jeton_acces.clone()).await), "aucun TOTP actif");

        // TOTP activé : le jeton de réenrôlement tombe, le jeton d'accès passe.
        let cle = laaficare_backend::config::CleTotp::depuis_octets([7; 32]);
        let activation = totp::commencer_activation(&state.db, cle.octets(), administrateur, MOT_DE_PASSE).await.unwrap();
        let code = totp_rs::Builder::new()
            .with_secret(totp_rs::Secret::try_from_base32(&activation.cle_base32).unwrap())
            .build()
            .unwrap()
            .generate(jsonwebtoken::get_current_timestamp())
            .to_string();
        totp::confirmer_activation(&state.db, cle.octets(), administrateur, &code, totp::Autorisation::ReenrolementAdministrateur)
            .await
            .unwrap();
        assert!(refuse(reenrolement(jeton_reenrolement).await), "TOTP actif");
        assert_eq!(acces(jeton_acces.clone()).await.unwrap(), administrateur);

        // Session fermée : refusé aussitôt.
        sqlx::query("UPDATE session SET revoquee_le = now(), motif_revocation = 'deconnexion' WHERE id = $1")
            .bind(session_id)
            .execute(&state.db)
            .await
            .unwrap();
        assert!(refuse(acces(jeton_acces).await), "session fermée");
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

    // --- Étape 3.9 : professionnels en exercice et agents MUNASEB ---------

    /// Compte professionnel à l'email vérifié.
    async fn professionnel(db: &PgPool, telephone: &str) -> Uuid {
        let utilisateur_id: Uuid =
            sqlx::query_scalar("INSERT INTO utilisateur (telephone, email) VALUES ($1, $1 || '@example.org') RETURNING id")
                .bind(telephone)
                .fetch_one(db)
                .await
                .unwrap();
        sqlx::query("UPDATE utilisateur SET email_verifie_le = now() WHERE id = $1")
            .bind(utilisateur_id)
            .execute(db)
            .await
            .unwrap();
        sqlx::query_scalar(
            "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'professionnel', 'x') RETURNING id",
        )
        .bind(utilisateur_id)
        .fetch_one(db)
        .await
        .unwrap()
    }

    async fn structure(db: &PgPool, createur: Uuid, type_structure: &str, statut: &str) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO structure (type_structure, nom, telephone, telephone_urgence, region, email, commune, province, latitude, longitude, \
                                    createur_compte_id, statut) \
             VALUES ($2, 'Structure', '+22670000000', '+22670000000', 'Région fictive', 'structure@example.org', 'Ouagadougou', 'Kadiogo', \
                     12.371400, -1.519700, $1, $3) RETURNING id",
        )
        .bind(createur)
        .bind(type_structure)
        .bind(statut)
        .fetch_one(db)
        .await
        .unwrap()
    }

    /// Affectation, session qui la porte, et en-tête du jeton en exercice.
    /// La session est ouverte directement avec l'affectation : l'activation
    /// refuserait une structure non validée, or ce test vérifie justement
    /// que l'extracteur refuse un jeton qui en porterait une.
    async fn en_exercice(state: &AppState, compte: Uuid, structure_id: Uuid, role: &str) -> (Uuid, String) {
        let affectation_id: Uuid =
            sqlx::query_scalar("INSERT INTO affectation (compte_id, structure_id, role) VALUES ($1, $2, $3) RETURNING id")
                .bind(compte)
                .bind(structure_id)
                .bind(role)
                .fetch_one(&state.db)
                .await
                .unwrap();
        let session = session::ouvrir(&state.db, compte, Some(affectation_id)).await.unwrap();
        let jeton = state
            .jwt
            .emettre(SorteJeton::ProfessionnelEnExercice, compte, 0, Some(affectation_id), Some(session.session_id))
            .unwrap();
        (affectation_id, format!("Bearer {jeton}"))
    }

    fn statut(resultat: Result<AgentMunaseb, Response>) -> StatusCode {
        match resultat {
            Ok(_) => StatusCode::OK,
            Err(reponse) => reponse.status(),
        }
    }

    /// D5, avec deux structures MUNASEB (décision du porteur, 2026-10-18) :
    /// la base n'en admet qu'une validée ; l'agent d'une seconde, en attente
    /// de validation, est refusé (403). Un autre rôle, ou un agent dans une
    /// autre sorte de structure, aussi.
    #[sqlx::test]
    #[ignore]
    async fn agent_munaseb_seulement_dans_la_munaseb_validee(pool: PgPool) {
        let state = etat(pool);
        let chef = professionnel(&state.db, "+22670180001").await;
        let validee = structure(&state.db, chef, "munaseb", "validee").await;
        let autre_chef = professionnel(&state.db, "+22670180002").await;
        let en_attente = structure(&state.db, autre_chef, "munaseb", "en_attente").await;
        let clinique = structure(&state.db, chef, "clinique", "validee").await;

        let agent_a = professionnel(&state.db, "+22670180003").await;
        let (affectation_a, en_tete_a) = en_exercice(&state, agent_a, validee, ROLE_AGENT).await;
        let agent_b = professionnel(&state.db, "+22670180004").await;
        let (_, en_tete_b) = en_exercice(&state, agent_b, en_attente, ROLE_AGENT).await;
        let (_, responsable) = en_exercice(&state, chef, validee, "responsable").await;
        let (_, dans_clinique) = en_exercice(&state, agent_a, clinique, ROLE_AGENT).await;

        let AgentMunaseb(agent) = extraire(&state, Some(&en_tete_a)).await.unwrap();
        assert_eq!(agent, AffectationAgent { affectation_id: affectation_a, structure_id: validee });
        assert_eq!(statut(extraire(&state, Some(&en_tete_b)).await), StatusCode::FORBIDDEN, "MUNASEB en attente");
        assert_eq!(statut(extraire(&state, Some(&responsable)).await), StatusCode::FORBIDDEN, "autre rôle");
        assert_eq!(statut(extraire(&state, Some(&dans_clinique)).await), StatusCode::FORBIDDEN, "autre structure");

        // Affectation désactivée : son jeton encore valable est refusé (403).
        sqlx::query(
            "UPDATE affectation SET statut = 'desactivee', desactivee_le = now(), desactivee_par_compte_id = $2 WHERE id = $1",
        )
        .bind(affectation_a)
        .bind(chef)
        .execute(&state.db)
        .await
        .unwrap();
        assert_eq!(statut(extraire(&state, Some(&en_tete_a)).await), StatusCode::FORBIDDEN, "affectation désactivée");
    }

    /// D8 : `CompteConnecte` accepte les quatre jetons d'accès, et refuse une
    /// session fermée ou un jeton intermédiaire.
    #[sqlx::test]
    #[ignore]
    async fn compte_connecte_toutes_les_sortes(pool: PgPool) {
        let state = etat(pool);
        let compte = professionnel(&state.db, "+22670180005").await;
        let clinique = structure(&state.db, compte, "clinique", "validee").await;
        let (_, exercice) = en_exercice(&state, compte, clinique, "medecin").await;
        let sans = session::ouvrir(&state.db, compte, None).await.unwrap();
        let jeton_sans = state
            .jwt
            .emettre(SorteJeton::ProfessionnelSansAffectation, compte, 0, None, Some(sans.session_id))
            .unwrap();
        let extraire_compte = |en_tete: String| {
            let state = state.clone();
            async move { CompteConnecte::from_request_parts(&mut parts(Some(&en_tete)), &state).await.map(|c| c.compte_id) }
        };
        assert_eq!(extraire_compte(exercice).await.unwrap(), compte);
        assert_eq!(extraire_compte(format!("Bearer {jeton_sans}")).await.unwrap(), compte);

        let intermediaire = state.jwt.emettre(SorteJeton::SecondFacteurProfessionnel, compte, 0, None, None).unwrap();
        assert!(extraire_compte(format!("Bearer {intermediaire}")).await.is_err());
        session::fermer(&state.db, sans.jeton_renouvellement.en_clair()).await.unwrap();
        assert!(extraire_compte(format!("Bearer {jeton_sans}")).await.is_err(), "session fermée");
    }

    /// D2 : `ProfessionnelConnecte` accepte les jetons sans affectation et en
    /// exercice, jamais un jeton patient.
    #[sqlx::test]
    #[ignore]
    async fn professionnel_connecte_deux_sortes(pool: PgPool) {
        let state = etat(pool);
        let compte = professionnel(&state.db, "+22670180006").await;
        let clinique = structure(&state.db, compte, "clinique", "validee").await;
        let (_, exercice) = en_exercice(&state, compte, clinique, "medecin").await;
        let sans = session::ouvrir(&state.db, compte, None).await.unwrap();
        let jeton_sans = state
            .jwt
            .emettre(SorteJeton::ProfessionnelSansAffectation, compte, 0, None, Some(sans.session_id))
            .unwrap();
        let patient = state.jwt.emettre(SorteJeton::Patient, compte, 0, None, Some(sans.session_id)).unwrap();
        let extraire_pro = |en_tete: String| {
            let state = state.clone();
            async move { ProfessionnelConnecte::from_request_parts(&mut parts(Some(&en_tete)), &state).await.map(|c| c.session_id) }
        };
        assert_eq!(extraire_pro(format!("Bearer {jeton_sans}")).await.unwrap(), sans.session_id);
        assert!(extraire_pro(exercice).await.is_ok());
        assert!(extraire_pro(format!("Bearer {patient}")).await.is_err());
    }
}
