//! Routes de la connexion administrateur et du réenrôlement de son TOTP
//! (étape 3.6.4, décisions A1 à A7 du 2026-10-15 ; contrat dans
//! docs/api.md). Même principe que les autres fichiers de routes :
//! désérialiser, appeler `auth_administrateur.rs`, traduire le résultat.
//! Les erreurs de connexion et du second temps reprennent celles des
//! professionnels (mêmes codes, mêmes messages).

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::erreur_api::{JsonApi, reponse_erreur, reponse_erreur_interne};
use crate::extracteur_jwt::{IntermediaireAdministrateur, ReenrolementAdministrateur};
use crate::routes_professionnel::{
    ConnexionRequete, SecondFacteurRequete, mapper_erreur, reponse_second_facteur, terminer_second_facteur,
};
use laaficare_backend::auth_administrateur::{self, ConnexionAdministrateur, ErreurReenrolement};
use laaficare_backend::auth_professionnel::JetonIntermediaire;

#[derive(Serialize)]
struct CorpsReenrolement<'a> {
    etape: &'static str,
    jeton_reenrolement: &'a str,
}

/// Réponse « réenrôlement » : le jeton n'est lu en clair qu'ici, au moment
/// de sérialiser, puis effacé de la mémoire avec `JetonIntermediaire`.
fn reponse_reenrolement(jeton: &JetonIntermediaire) -> Response {
    let corps = CorpsReenrolement { etape: "reenrolement", jeton_reenrolement: jeton.en_clair() };
    (StatusCode::OK, Json(corps)).into_response()
}

/// `POST /api/administrateurs/connexion` -- email + mot de passe. 200 avec
/// `"etape": "second_facteur"` (TOTP actif) ou `"etape": "reenrolement"`
/// (pas de TOTP, fenêtre ouverte par l'équipe) ; jamais de session directe
/// (Y4). 401 `identifiants_invalides`, y compris pour un administrateur sans
/// TOTP ni fenêtre ; 403 `email_non_verifie`.
pub async fn se_connecter(State(state): State<AppState>, JsonApi(corps): JsonApi<ConnexionRequete>) -> Response {
    match auth_administrateur::se_connecter(&state.db, &state.jwt, &corps.email, &corps.mot_de_passe).await {
        Ok(ConnexionAdministrateur::SecondFacteur { jeton_intermediaire }) => {
            reponse_second_facteur(&jeton_intermediaire)
        }
        Ok(ConnexionAdministrateur::Reenrolement { jeton_reenrolement }) => reponse_reenrolement(&jeton_reenrolement),
        Err(erreur) => mapper_erreur(erreur, "connexion administrateur").into_response(),
    }
}

/// `POST /api/administrateurs/connexion/second-facteur` (A5) -- comme pour
/// les professionnels ; la session ouverte est une session administrateur
/// (24 h au plus, jeton `laaficare:administrateur`).
pub async fn second_facteur(
    intermediaire: IntermediaireAdministrateur,
    State(state): State<AppState>,
    JsonApi(corps): JsonApi<SecondFacteurRequete>,
) -> Response {
    terminer_second_facteur(&state, intermediaire.compte_id, intermediaire.iat, corps, "second facteur administrateur")
        .await
}

/// Pas de `Debug` : un mot de passe est un secret.
#[derive(Deserialize)]
pub struct CommencerRequete {
    pub mot_de_passe: String,
}

/// Pas de `Debug` : un code est un secret.
#[derive(Deserialize)]
pub struct ConfirmerRequete {
    pub code: String,
}

#[derive(Serialize)]
struct CorpsActivation<'a> {
    url_otpauth: &'a str,
    cle_base32: &'a str,
}

#[derive(Serialize)]
struct CorpsCodesSecours<'a> {
    codes_secours: &'a [String],
}

fn reponse_erreur_reenrolement(erreur: ErreurReenrolement) -> Response {
    match erreur {
        // A7 : l'email n'est pas resaisi ici, d'où un message propre ; même
        // code que la connexion.
        ErreurReenrolement::MotDePasseIncorrect => {
            reponse_erreur(StatusCode::UNAUTHORIZED, "identifiants_invalides", erreur.to_string()).into_response()
        }
        ErreurReenrolement::CodeInvalide => {
            reponse_erreur(StatusCode::UNAUTHORIZED, "code_invalide", erreur.to_string()).into_response()
        }
        ErreurReenrolement::Interne(etape) => reponse_erreur_interne("réenrôlement administrateur", etape).into_response(),
    }
}

/// `POST /api/administrateurs/reenrolement/commencer` -- en-tête
/// `Authorization: Bearer <jeton_reenrolement>`, corps `{"mot_de_passe"}`
/// (A2 a). 200 `{"url_otpauth", "cle_base32"}` : l'application dessine le
/// QR code (Z2). Peut être relancé tant que la fenêtre est ouverte (X3).
pub async fn commencer_reenrolement(
    reenrolement: ReenrolementAdministrateur,
    State(state): State<AppState>,
    JsonApi(corps): JsonApi<CommencerRequete>,
) -> Response {
    match auth_administrateur::commencer_reenrolement(&state.db, &state.cle_totp, reenrolement.compte_id, &corps.mot_de_passe)
        .await
    {
        Ok(activation) => {
            let corps = CorpsActivation { url_otpauth: &activation.url_otpauth, cle_base32: &activation.cle_base32 };
            (StatusCode::OK, Json(corps)).into_response()
        }
        Err(erreur) => reponse_erreur_reenrolement(erreur),
    }
}

/// `POST /api/administrateurs/reenrolement/confirmer` -- en-tête
/// `Authorization: Bearer <jeton_reenrolement>`, corps `{"code"}`. 200
/// `{"codes_secours": [10 codes]}`, la seule fois où ils sont visibles (Z6).
/// Aucune session (A3) : l'application revient à l'écran de connexion.
/// Ensuite, le jeton de réenrôlement est refusé (TOTP actif, A4).
pub async fn confirmer_reenrolement(
    reenrolement: ReenrolementAdministrateur,
    State(state): State<AppState>,
    JsonApi(corps): JsonApi<ConfirmerRequete>,
) -> Response {
    match auth_administrateur::confirmer_reenrolement(&state.db, &state.cle_totp, reenrolement.compte_id, &corps.code).await
    {
        Ok(codes) => (StatusCode::OK, Json(CorpsCodesSecours { codes_secours: &codes })).into_response(),
        Err(erreur) => reponse_erreur_reenrolement(erreur),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn erreurs_du_reenrolement() {
        for (erreur, statut, code, message) in [
            (ErreurReenrolement::MotDePasseIncorrect, StatusCode::UNAUTHORIZED, "identifiants_invalides", "Mot de passe incorrect."),
            (ErreurReenrolement::CodeInvalide, StatusCode::UNAUTHORIZED, "code_invalide", "Code incorrect."),
            (
                ErreurReenrolement::Interne("début du réenrôlement"),
                StatusCode::INTERNAL_SERVER_ERROR,
                "erreur_interne",
                "erreur interne",
            ),
        ] {
            let reponse = reponse_erreur_reenrolement(erreur);
            assert_eq!(reponse.status(), statut, "{code}");
            let json: serde_json::Value =
                serde_json::from_slice(&axum::body::to_bytes(reponse.into_body(), usize::MAX).await.unwrap()).unwrap();
            assert_eq!((json["code"].as_str().unwrap(), json["erreur"].as_str().unwrap()), (code, message));
        }
    }

    /// De bout en bout, par les routes : connexion sans TOTP pendant la
    /// fenêtre (réenrôlement), début et confirmation (10 codes de secours),
    /// jeton de réenrôlement refusé ensuite, puis connexion et second temps
    /// avec le nouveau TOTP (session administrateur).
    #[sqlx::test]
    #[ignore]
    async fn reenrolement_de_bout_en_bout(pool: sqlx::PgPool) {
        use axum::body::to_bytes;
        use axum::extract::FromRequestParts;
        use axum::http::{Request, header::AUTHORIZATION};
        use laaficare_backend::config::CleTotp;
        use laaficare_backend::jwt::{JwtService, SorteJeton};
        use laaficare_backend::mot_de_passe;
        use laaficare_backend::sms::SmsSenderConsole;
        use sqlx::types::Uuid;
        use std::sync::Arc;

        const EMAIL: &str = "adm.route@example.org";
        const MOT_DE_PASSE: &str = "Exemple#2026";
        let state = AppState {
            db: pool,
            jwt: JwtService::new(&"a".repeat(32)),
            sms: Arc::new(SmsSenderConsole),
            cle_totp: Arc::new(CleTotp::depuis_octets([7; 32])),
        };
        let utilisateur_id: Uuid =
            sqlx::query_scalar("INSERT INTO utilisateur (telephone, email) VALUES ('+22670120001', $1) RETURNING id")
                .bind(EMAIL)
                .fetch_one(&state.db)
                .await
                .unwrap();
        sqlx::query("UPDATE utilisateur SET email_verifie_le = now() WHERE id = $1")
            .bind(utilisateur_id)
            .execute(&state.db)
            .await
            .unwrap();
        // Fenêtre comme l'ouvrira l'outil local (étape 3.6.5).
        sqlx::query(
            "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash, reenrolement_autorise_jusqu_au) \
             VALUES ($1, 'administrateur_laaficare', $2, now() + make_interval(mins => $3))",
        )
        .bind(utilisateur_id)
        .bind(mot_de_passe::hacher(MOT_DE_PASSE).unwrap())
        .bind(auth_administrateur::FENETRE_REENROLEMENT_MINUTES)
        .execute(&state.db)
        .await
        .unwrap();

        let json_de = |reponse: Response| async move {
            serde_json::from_slice::<serde_json::Value>(&to_bytes(reponse.into_body(), usize::MAX).await.unwrap()).unwrap()
        };
        let connexion = || ConnexionRequete { email: EMAIL.into(), mot_de_passe: MOT_DE_PASSE.into() };
        let parts_avec = |jeton: &str| {
            Request::builder().header(AUTHORIZATION, format!("Bearer {jeton}")).body(()).unwrap().into_parts().0
        };

        // Connexion : réenrôlement.
        let reponse = se_connecter(State(state.clone()), JsonApi(connexion())).await;
        assert_eq!(reponse.status(), StatusCode::OK);
        let json = json_de(reponse).await;
        assert_eq!(json["etape"], "reenrolement");
        let jeton_reenrolement = json["jeton_reenrolement"].as_str().unwrap().to_owned();
        let extraire = || async {
            ReenrolementAdministrateur::from_request_parts(&mut parts_avec(&jeton_reenrolement), &state).await
        };

        // Commencer : mot de passe faux (401), puis correct (200).
        let reponse = commencer_reenrolement(
            extraire().await.unwrap(),
            State(state.clone()),
            JsonApi(CommencerRequete { mot_de_passe: "Mauvais#2026".into() }),
        )
        .await;
        assert_eq!(reponse.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(json_de(reponse).await["code"], "identifiants_invalides");
        let reponse = commencer_reenrolement(
            extraire().await.unwrap(),
            State(state.clone()),
            JsonApi(CommencerRequete { mot_de_passe: MOT_DE_PASSE.into() }),
        )
        .await;
        assert_eq!(reponse.status(), StatusCode::OK);
        let json = json_de(reponse).await;
        assert!(json["url_otpauth"].as_str().unwrap().starts_with("otpauth://totp/"));
        let generateur = totp_rs::Builder::new()
            .with_secret(totp_rs::Secret::try_from_base32(json["cle_base32"].as_str().unwrap()).unwrap())
            .build()
            .unwrap();
        let maintenant = jsonwebtoken::get_current_timestamp();

        // Confirmer : 10 codes de secours, puis le jeton est refusé.
        let reponse = confirmer_reenrolement(
            extraire().await.unwrap(),
            State(state.clone()),
            JsonApi(ConfirmerRequete { code: generateur.generate(maintenant).to_string() }),
        )
        .await;
        assert_eq!(reponse.status(), StatusCode::OK);
        assert_eq!(json_de(reponse).await["codes_secours"].as_array().unwrap().len(), 10);
        let refus = extraire().await.map_err(IntoResponse::into_response).unwrap_err();
        assert_eq!(refus.status(), StatusCode::UNAUTHORIZED);

        // Reconnexion : second facteur, puis session administrateur.
        let json = json_de(se_connecter(State(state.clone()), JsonApi(connexion())).await).await;
        assert_eq!(json["etape"], "second_facteur");
        let jeton_intermediaire = json["jeton_intermediaire"].as_str().unwrap().to_owned();
        let intermediaire =
            IntermediaireAdministrateur::from_request_parts(&mut parts_avec(&jeton_intermediaire), &state).await.unwrap();
        let corps = SecondFacteurRequete { code: Some(generateur.generate(maintenant + 30).to_string()), code_secours: None };
        let reponse = second_facteur(intermediaire, State(state.clone()), JsonApi(corps)).await;
        assert_eq!(reponse.status(), StatusCode::OK);
        let json = json_de(reponse).await;
        assert_eq!(json["etape"], "complete");
        state.jwt.verifier(json["jeton_acces"].as_str().unwrap(), SorteJeton::Administrateur).unwrap();
    }
}
