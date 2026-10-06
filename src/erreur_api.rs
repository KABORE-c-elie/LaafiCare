//! Format d'erreur unique de toute l'API (section 12 du CLAUDE.md) :
//! `{"erreur": "<message lisible>", "code": "<code_machine>"}`, plus des
//! champs complémentaires quand le cas l'exige. Les applications testent
//! `code`, jamais le message.
//!
//! `JsonApi<T>` remplace `axum::Json<T>` dans les handlers qui lisent un
//! corps JSON, pour que les rejets d'axum (JSON mal formé, champ manquant,
//! type d'acte inconnu...) sortent eux aussi dans ce format. Modèle suivi :
//! doc axum 0.8.9, module `extract`, section "Customizing extractor
//! responses" ("Create your own extractor that in its `FromRequest`
//! implementation calls one of axum's built in extractors but returns a
//! different response for rejections"), et l'exemple officiel
//! `examples/customize-extractor-error/src/custom_extractor.rs`. Aucune
//! fonctionnalité supplémentaire d'axum n'est nécessaire.

use axum::Json;
use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::extract::{FromRequest, FromRequestParts, Path, Query, Request};
use axum::http::StatusCode;
use axum::http::request::Parts;
use chrono::NaiveDate;
use laaficare_backend::mot_de_passe::RegleMotDePasse;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ErreurApi {
    pub erreur: String,
    pub code: &'static str,
    /// Seulement pour `en_carence` : date de début de couverture.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debut_couverture: Option<NaiveDate>,
    /// Seulement pour `transition_invalide` : statut actuel de la demande
    /// (décision V2).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub statut_actuel: Option<String>,
    /// Seulement pour `mot_de_passe_non_conforme` : toutes les règles non
    /// respectées, pour que l'application affiche une liste à cocher.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub regles_non_respectees: Option<Vec<RegleMotDePasse>>,
}

impl ErreurApi {
    /// Erreur sans champ complémentaire.
    pub fn simple(code: &'static str, erreur: impl Into<String>) -> Self {
        ErreurApi {
            erreur: erreur.into(),
            code,
            debut_couverture: None,
            statut_actuel: None,
            regles_non_respectees: None,
        }
    }
}

/// Réponse d'erreur renvoyée par tous les handlers.
pub type ReponseErreur = (StatusCode, Json<ErreurApi>);

pub fn reponse_erreur(statut: StatusCode, code: &'static str, erreur: impl Into<String>) -> ReponseErreur {
    (statut, Json(ErreurApi::simple(code, erreur)))
}

/// Erreur interne : le détail va dans les logs, jamais dans la réponse.
pub fn reponse_erreur_interne(contexte: &'static str, detail: &str) -> ReponseErreur {
    tracing::error!(detail, contexte, "erreur interne");
    reponse_erreur(StatusCode::INTERNAL_SERVER_ERROR, "erreur_interne", "erreur interne")
}

/// Corps JSON lu comme `axum::Json<T>`, avec les rejets au format de l'API.
pub struct JsonApi<T>(pub T);

impl<S, T> FromRequest<S> for JsonApi<T>
where
    Json<T>: FromRequest<S, Rejection = JsonRejection>,
    S: Send + Sync,
{
    type Rejection = ReponseErreur;

    async fn from_request(requete: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(requete, state).await {
            Ok(Json(valeur)) => Ok(JsonApi(valeur)),
            Err(rejet) => Err(convertir_rejet_json(rejet)),
        }
    }
}

fn convertir_rejet_json(rejet: JsonRejection) -> ReponseErreur {
    let code = match &rejet {
        JsonRejection::JsonSyntaxError(_) => "json_invalide",
        JsonRejection::JsonDataError(_) => "donnees_invalides",
        JsonRejection::MissingJsonContentType(_) => "content_type_invalide",
        // BytesRejection, et toute variante qu'axum ajouterait plus tard
        // (l'énumération est marquée non exhaustive).
        _ => "corps_illisible",
    };
    // `body_text()` va dans la réponse, JAMAIS dans les logs (condition
    // posée par le porteur) : il peut reprendre une valeur saisie par
    // erreur, y compris une donnée de santé. Le code HTTP reste celui
    // choisi par axum (`status()`).
    (rejet.status(), Json(ErreurApi::simple(code, rejet.body_text())))
}

/// Paramètre d'URL lu comme `axum::extract::Path<T>`, avec les rejets au
/// format de l'API (même modèle que `JsonApi`). `PathRejection` a les mêmes
/// méthodes `status()` et `body_text()` (doc axum 0.8.9).
pub struct CheminApi<T>(pub T);

impl<S, T> FromRequestParts<S> for CheminApi<T>
where
    Path<T>: FromRequestParts<S, Rejection = PathRejection>,
    S: Send + Sync,
{
    type Rejection = ReponseErreur;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match Path::<T>::from_request_parts(parts, state).await {
            Ok(Path(valeur)) => Ok(CheminApi(valeur)),
            // body_text() dans la réponse, jamais dans les logs (même
            // condition que pour JsonApi). Une seule famille de code :
            // l'énumération est non exhaustive, un nouveau cas tombe ici.
            Err(rejet) => Err((rejet.status(), Json(ErreurApi::simple("chemin_invalide", rejet.body_text())))),
        }
    }
}

/// Paramètres de requête lus comme `axum::extract::Query<T>`, avec les
/// rejets au format de l'API. `QueryRejection` a aussi `status()` et
/// `body_text()` (doc axum 0.8.9).
pub struct RequeteApi<T>(pub T);

impl<S, T> FromRequestParts<S> for RequeteApi<T>
where
    Query<T>: FromRequestParts<S, Rejection = QueryRejection>,
    S: Send + Sync,
{
    type Rejection = ReponseErreur;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match Query::<T>::from_request_parts(parts, state).await {
            Ok(Query(valeur)) => Ok(RequeteApi(valeur)),
            Err(rejet) => Err((rejet.status(), Json(ErreurApi::simple("parametre_invalide", rejet.body_text())))),
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::header::CONTENT_TYPE;
    use laaficare_backend::demande_remboursement_munaseb::TypeActeMunaseb;
    use serde::Deserialize;

    use super::*;

    #[derive(Debug, Deserialize)]
    struct Corps {
        #[allow(dead_code)]
        type_acte: TypeActeMunaseb,
    }

    async fn lire(content_type: Option<&str>, corps: &'static str) -> Result<JsonApi<Corps>, ReponseErreur> {
        let mut requete = axum::http::Request::builder().method("POST").uri("/");
        if let Some(valeur) = content_type {
            requete = requete.header(CONTENT_TYPE, valeur);
        }
        JsonApi::<Corps>::from_request(requete.body(Body::from(corps)).unwrap(), &()).await
    }

    fn code(erreur: &ReponseErreur) -> &'static str {
        erreur.1.0.code
    }

    #[tokio::test]
    async fn corps_valide() {
        assert!(lire(Some("application/json"), r#"{"type_acte":"pharmacie"}"#).await.is_ok());
    }

    #[tokio::test]
    async fn json_mal_forme() {
        let erreur = lire(Some("application/json"), r#"{"type_acte":"#).await.err().unwrap();
        assert_eq!(code(&erreur), "json_invalide");
        assert!(erreur.0.is_client_error());
    }

    #[tokio::test]
    async fn type_d_acte_inconnu() {
        let erreur = lire(Some("application/json"), r#"{"type_acte":"dentaire"}"#).await.err().unwrap();
        assert_eq!(erreur.0, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(code(&erreur), "donnees_invalides");
        assert!(erreur.1.0.erreur.contains("dentaire"), "le message aide à corriger la requête");
    }

    #[tokio::test]
    async fn champ_manquant() {
        let erreur = lire(Some("application/json"), "{}").await.err().unwrap();
        assert_eq!(code(&erreur), "donnees_invalides");
    }

    #[tokio::test]
    async fn sans_content_type_json() {
        let erreur = lire(None, r#"{"type_acte":"pharmacie"}"#).await.err().unwrap();
        assert_eq!(code(&erreur), "content_type_invalide");
        assert!(erreur.0.is_client_error());
    }

    #[derive(Debug, Deserialize)]
    struct Filtre {
        #[allow(dead_code)]
        statut: Option<laaficare_backend::demande_remboursement_munaseb::StatutDemande>,
    }

    async fn lire_requete(uri: &str) -> Result<RequeteApi<Filtre>, ReponseErreur> {
        let mut parts = axum::http::Request::builder().uri(uri).body(()).unwrap().into_parts().0;
        RequeteApi::<Filtre>::from_request_parts(&mut parts, &()).await
    }

    #[tokio::test]
    async fn parametres_de_requete() {
        assert!(lire_requete("/?statut=en_attente").await.is_ok());
        assert!(lire_requete("/").await.is_ok(), "filtre facultatif");
        let erreur = lire_requete("/?statut=archive").await.err().unwrap();
        assert_eq!(code(&erreur), "parametre_invalide");
        assert!(erreur.0.is_client_error());
    }

    #[test]
    fn uuid_lu_depuis_le_json_grace_a_la_fonctionnalite_serde() {
        #[derive(Deserialize)]
        struct AvecId {
            id: sqlx::types::Uuid,
        }
        let lu: AvecId = serde_json::from_str(r#"{"id":"00000000-0000-7000-8000-000000000001"}"#).unwrap();
        assert_eq!(lu.id.to_string(), "00000000-0000-7000-8000-000000000001");
        assert!(serde_json::from_str::<AvecId>(r#"{"id":"pas-un-uuid"}"#).is_err());
    }
}
