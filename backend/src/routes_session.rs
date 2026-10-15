//! Routes des sessions (option B, décidée le 2026-10-12 ; contrat dans
//! docs/api.md, section 1.7) : renouvellement, déconnexion d'un appareil,
//! déconnexion partout. Même principe que les autres fichiers de routes :
//! désérialiser, appeler `session.rs`, traduire le résultat.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::erreur_api::{JsonApi, reponse_erreur, reponse_erreur_interne};
use crate::extracteur_jwt::PatientAuthentifie;
use laaficare_backend::session::{self, ErreurSession, MotifRevocation, PaireJetons};

#[derive(Serialize)]
struct CorpsPaire<'a> {
    /// `complete` à la connexion et à l'inscription (décision J4) ; absent
    /// au renouvellement.
    #[serde(skip_serializing_if = "Option::is_none")]
    etape: Option<&'static str>,
    jeton_acces: &'a str,
    expire_dans: u64,
    jeton_renouvellement: &'a str,
}

/// Réponse qui remet une paire de jetons au client. Le corps est sérialisé
/// tout de suite (`into_response`), pendant que la paire existe encore :
/// le jeton de renouvellement n'est lu en clair qu'ici (`en_clair()`), puis
/// effacé de la mémoire avec la paire (`session.rs`).
pub(crate) fn reponse_paire(statut: StatusCode, paire: &PaireJetons, etape: Option<&'static str>) -> Response {
    let corps = CorpsPaire {
        etape,
        jeton_acces: &paire.jeton_acces,
        expire_dans: paire.expire_dans,
        jeton_renouvellement: paire.jeton_renouvellement.en_clair(),
    };
    (statut, Json(corps)).into_response()
}

#[derive(Deserialize)]
pub struct JetonRenouvellementRequete {
    pub jeton_renouvellement: String,
}

/// `POST /api/sessions/renouveler` -- sans en-tête `Authorization` (le
/// jeton d'accès peut être expiré). Rejeu et jeton invalide reçoivent la
/// même réponse : le client n'a pas à savoir lequel (le rejeu est déjà
/// enregistré, session révoquée, décision S3).
pub async fn renouveler(
    State(state): State<AppState>,
    JsonApi(corps): JsonApi<JetonRenouvellementRequete>,
) -> Response {
    let resultat = session::renouveler(&state.db, &corps.jeton_renouvellement)
        .await
        .and_then(|active| active.en_paire(&state.jwt));
    match resultat {
        Ok(paire) => reponse_paire(StatusCode::OK, &paire, None),
        Err(ErreurSession::JetonInvalide | ErreurSession::RejeuDetecte | ErreurSession::EmailNonVerifie) => reponse_erreur(
            StatusCode::UNAUTHORIZED,
            "jeton_invalide",
            "jeton de renouvellement invalide ou expiré",
        )
        .into_response(),
        Err(ErreurSession::Interne(detail)) => reponse_erreur_interne("renouvellement de session", &detail).into_response(),
    }
}

/// `POST /api/sessions/deconnexion` -- ferme la session de l'appareil.
/// Toujours 204, même pour un jeton inconnu : se déconnecter n'échoue
/// jamais (sauf panne du serveur).
pub async fn deconnexion(
    State(state): State<AppState>,
    JsonApi(corps): JsonApi<JetonRenouvellementRequete>,
) -> Response {
    match session::fermer(&state.db, &corps.jeton_renouvellement).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(erreur) => reponse_erreur_interne("déconnexion", &erreur.to_string()).into_response(),
    }
}

/// `POST /api/sessions/deconnexion-partout` -- jeton d'accès valide exigé
/// (s'il a expiré, le client renouvelle d'abord). Ferme toutes les sessions
/// du compte et augmente `version_jeton`, dans une transaction, le compte
/// verrouillé avant les sessions (règle S2). Seuls les comptes patients
/// ont une session dans cette étape.
pub async fn deconnexion_partout(patient: PatientAuthentifie, State(state): State<AppState>) -> Response {
    let resultat = async {
        let mut tx = state.db.begin().await?;
        session::fermer_toutes(&mut tx, patient.compte_id, MotifRevocation::DeconnexionPartout, true).await?;
        tx.commit().await
    }
    .await;
    match resultat {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(erreur) => reponse_erreur_interne("déconnexion partout", &erreur.to_string()).into_response(),
    }
}
