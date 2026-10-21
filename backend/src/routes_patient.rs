//! Routes HTTP exposant `AuthPatientService` (section 11). Chaque handler
//! ne fait que : désérialiser, appeler le service métier, traduire le
//! résultat en réponse HTTP -- aucune logique métier ici.

use axum::http::{HeaderValue, StatusCode, header::RETRY_AFTER};
use axum::response::{IntoResponse, Response};
use axum::{Json, extract::State};
use chrono::NaiveDate;
use serde::Deserialize;

use crate::AppState;
use crate::erreur_api::{ErreurApi, JsonApi, ReponseErreur, reponse_erreur, reponse_erreur_interne};
use crate::extracteur_jwt::PatientAuthentifie;
use crate::routes_session::reponse_paire;
use laaficare_backend::auth_patient::{self, ErreurAuthPatient};
use laaficare_backend::patient::{self, ErreurProfil, ProfilPatient};

// Un seul endroit qui décide du code HTTP et du `code` par variante --
// section 13, codes HTTP validés avec le porteur avant d'écrire ce
// fichier ; `code` ajouté pour le format d'erreur unique (section 12).
fn mapper_erreur(erreur: ErreurAuthPatient) -> ReponseErreur {
    let (statut, code) = match &erreur {
        ErreurAuthPatient::TelephoneInvalide => (StatusCode::UNPROCESSABLE_ENTITY, "telephone_invalide"),
        ErreurAuthPatient::CodeInvalide => (StatusCode::UNAUTHORIZED, "code_invalide"),
        // O5, O6 : RFC 6585 §4. L'en-tête `Retry-After` est ajouté par
        // `reponse_erreur_patient`.
        ErreurAuthPatient::TropDeDemandes { .. } => (StatusCode::TOO_MANY_REQUESTS, "trop_de_demandes"),
        ErreurAuthPatient::TelephoneDejaUtilise => (StatusCode::CONFLICT, "telephone_deja_utilise"),
        ErreurAuthPatient::TelephoneInconnu => (StatusCode::NOT_FOUND, "telephone_inconnu"),
        // Toutes les règles non respectées, pour une liste à cocher dans
        // l'application (section 14).
        ErreurAuthPatient::MotDePasseNonConforme(regles) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(ErreurApi {
                    regles_non_respectees: Some(regles.clone()),
                    ..ErreurApi::simple("mot_de_passe_non_conforme", erreur.to_string())
                }),
            );
        }
        // Une seule réponse pour tout échec de connexion (P1, L1).
        ErreurAuthPatient::IdentifiantsInvalides => (StatusCode::UNAUTHORIZED, "identifiants_invalides"),
        // Détail dans les logs seulement, jamais dans la réponse -- même
        // principe que `/health` pour la base injoignable.
        ErreurAuthPatient::Interne(detail) => return reponse_erreur_interne("AuthPatientService", detail),
    };
    reponse_erreur(statut, code, erreur.to_string())
}

/// Réponse d'erreur complète des routes patient : celle de `mapper_erreur`,
/// plus l'en-tête `Retry-After` (en secondes) quand une demande de code est
/// refusée pour trop de demandes. RFC 6585 §4 : une réponse 429 « MAY
/// include a Retry-After header indicating how long to wait before making a
/// new request ». Construction de la réponse puis ajout de l'en-tête : même
/// méthode que `extracteur_jwt.rs` pour `WWW-Authenticate` (`headers_mut`,
/// doc axum 0.8.9).
fn reponse_erreur_patient(erreur: ErreurAuthPatient) -> Response {
    let reessayer_dans_secs = match &erreur {
        ErreurAuthPatient::TropDeDemandes { reessayer_dans_secs } => *reessayer_dans_secs,
        _ => None,
    };
    let mut reponse = mapper_erreur(erreur).into_response();
    if let Some(secs) = reessayer_dans_secs {
        reponse.headers_mut().insert(RETRY_AFTER, HeaderValue::from(secs));
    }
    reponse
}

#[derive(Deserialize)]
pub struct DemandeOtpRequete {
    pub telephone: String,
}

/// `POST /api/patients/otp` -- déclenche un OTP (création de compte ou mot
/// de passe oublié, `demander_otp` n'a pas besoin de savoir lequel). Ne
/// renvoie jamais le code lui-même : en dev, il apparaît dans les logs du
/// serveur (`SmsSenderConsole`).
pub async fn demander_otp(
    State(state): State<AppState>,
    JsonApi(corps): JsonApi<DemandeOtpRequete>,
) -> Result<StatusCode, Response> {
    let (code, telephone) = auth_patient::demander_otp(&state.db, &corps.telephone)
        .await
        .map_err(reponse_erreur_patient)?;

    let message = format!("Votre code LaafiCare : {code}. Valide 5 minutes.");
    // Envoi vers le numéro NORMALISÉ, jamais vers la saisie brute (décision
    // T4). L'échec d'envoi n'annule pas la demande : le code est déjà
    // enregistré en base. `SmsSenderConsole` ne peut pas échouer ; un vrai
    // fournisseur pourra, à traiter alors.
    if let Err(erreur) = state.sms.envoyer(&telephone, &message) {
        tracing::error!(%erreur, "échec d'envoi du SMS OTP");
    }

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct InscriptionRequete {
    pub telephone: String,
    pub code_otp: String,
    pub nom: String,
    pub prenom: String,
    pub date_naissance: NaiveDate,
    pub lieu_naissance: String,
    pub mot_de_passe: String,
}

/// `POST /api/patients/inscription` -- flux de création de compte
/// (section 11) : un seul appel, atomique côté service. Réponse 201 avec
/// la paire de jetons (option B, `docs/api.md` section 1.7) et
/// `"etape": "complete"`, même forme que la connexion.
pub async fn creer_compte(
    State(state): State<AppState>,
    JsonApi(corps): JsonApi<InscriptionRequete>,
) -> Result<Response, Response> {
    let paire = auth_patient::creer_compte(
        &state.db,
        &state.jwt,
        &corps.telephone,
        &corps.code_otp,
        &corps.nom,
        &corps.prenom,
        corps.date_naissance,
        &corps.lieu_naissance,
        &corps.mot_de_passe,
    )
    .await
    .map_err(reponse_erreur_patient)?;

    Ok(reponse_paire(StatusCode::CREATED, &paire, Some("complete")))
}

#[derive(Deserialize)]
pub struct ReinitialisationRequete {
    pub telephone: String,
    pub code_otp: String,
    pub nouveau_mot_de_passe: String,
}

/// `POST /api/patients/mot-de-passe/reinitialiser` -- flux mot de passe
/// oublié (section 11).
pub async fn reinitialiser_mot_de_passe(
    State(state): State<AppState>,
    JsonApi(corps): JsonApi<ReinitialisationRequete>,
) -> Result<StatusCode, Response> {
    auth_patient::reinitialiser_mot_de_passe(
        &state.db,
        &corps.telephone,
        &corps.code_otp,
        &corps.nouveau_mot_de_passe,
    )
    .await
    .map_err(reponse_erreur_patient)?;

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct ConnexionRequete {
    pub telephone: String,
    pub mot_de_passe: String,
}

/// `POST /api/patients/connexion` -- téléphone + mot de passe, verrouillage
/// BF-01. Tout échec : 401 `identifiants_invalides` (décisions P1, L1).
/// Succès : 200 avec `"etape": "complete"` et la paire de jetons (J4,
/// option B). L'étape `second_facteur` viendra avec l'activation du TOTP
/// patient (J10).
pub async fn se_connecter(
    State(state): State<AppState>,
    JsonApi(corps): JsonApi<ConnexionRequete>,
) -> Result<Response, Response> {
    let paire = auth_patient::se_connecter(&state.db, &state.jwt, &corps.telephone, &corps.mot_de_passe)
        .await
        .map_err(reponse_erreur_patient)?;
    Ok(reponse_paire(StatusCode::OK, &paire, Some("complete")))
}

/// `GET /api/patients/moi` -- profil du patient connecté (jeton
/// `laaficare:patient`, compte actif, version de jeton à jour : voir
/// `PatientAuthentifie`). Toute incohérence en base est une erreur interne
/// (décisions M2, M3), le détail restant dans les journaux.
pub async fn moi(
    patient: PatientAuthentifie,
    State(state): State<AppState>,
) -> Result<Json<ProfilPatient>, ReponseErreur> {
    patient::profil(&state.db, patient.compte_id)
        .await
        .map(Json)
        .map_err(|ErreurProfil::Interne(detail)| reponse_erreur_interne("profil patient", &detail))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chaque_erreur_a_son_code_http_et_son_code() {
        for (erreur, statut, code) in [
            (ErreurAuthPatient::TelephoneInvalide, StatusCode::UNPROCESSABLE_ENTITY, "telephone_invalide"),
            (ErreurAuthPatient::CodeInvalide, StatusCode::UNAUTHORIZED, "code_invalide"),
            (ErreurAuthPatient::TelephoneDejaUtilise, StatusCode::CONFLICT, "telephone_deja_utilise"),
            (ErreurAuthPatient::TelephoneInconnu, StatusCode::NOT_FOUND, "telephone_inconnu"),
            (ErreurAuthPatient::IdentifiantsInvalides, StatusCode::UNAUTHORIZED, "identifiants_invalides"),
            (ErreurAuthPatient::Interne("détail secret".into()), StatusCode::INTERNAL_SERVER_ERROR, "erreur_interne"),
        ] {
            let (s, Json(corps)) = mapper_erreur(erreur);
            assert_eq!((s, corps.code), (statut, code));
            assert!(!corps.erreur.contains("secret"), "le détail interne ne sort jamais");
            assert!(corps.regles_non_respectees.is_none());
        }
    }

    #[test]
    fn trop_de_demandes_429_avec_ou_sans_retry_after() {
        let reponse = reponse_erreur_patient(ErreurAuthPatient::TropDeDemandes { reessayer_dans_secs: Some(42) });
        assert_eq!(reponse.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(reponse.headers().get(RETRY_AFTER).unwrap(), "42");

        // Dernier essai du code épuisé : pas de délai, il faut un nouveau code.
        let reponse = reponse_erreur_patient(ErreurAuthPatient::TropDeDemandes { reessayer_dans_secs: None });
        assert_eq!(reponse.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(reponse.headers().get(RETRY_AFTER).is_none());

        let (_, Json(corps)) = mapper_erreur(ErreurAuthPatient::TropDeDemandes { reessayer_dans_secs: Some(42) });
        assert_eq!(corps.code, "trop_de_demandes");
    }

    #[test]
    fn mot_de_passe_non_conforme_liste_les_regles() {
        use laaficare_backend::mot_de_passe::RegleMotDePasse;
        let regles = vec![RegleMotDePasse::Chiffre, RegleMotDePasse::CaractereSpecial];
        let (s, Json(corps)) = mapper_erreur(ErreurAuthPatient::MotDePasseNonConforme(regles.clone()));
        assert_eq!((s, corps.code), (StatusCode::UNPROCESSABLE_ENTITY, "mot_de_passe_non_conforme"));
        assert_eq!(corps.regles_non_respectees, Some(regles));
        let json = serde_json::to_string(&corps).unwrap();
        assert!(json.contains(r#""regles_non_respectees":["chiffre","caractere_special"]"#), "{json}");
    }

    // Base temporaire ; données fictives. De l'inscription au profil : le
    // jeton reçu ouvre `GET /api/patients/moi`, et `compte_id` est son `sub`.
    #[sqlx::test]
    #[ignore]
    async fn profil_du_patient_inscrit(pool: sqlx::PgPool) {
        use axum::extract::FromRequestParts;
        use axum::http::{Request, header::AUTHORIZATION};
        use laaficare_backend::jwt::{JwtService, SorteJeton};
        use laaficare_backend::sms::SmsSenderConsole;
        use std::sync::Arc;

        let state = AppState {
            db: pool,
            jwt: JwtService::new(&"a".repeat(32)),
            sms: Arc::new(SmsSenderConsole),
            cle_totp: Arc::new(laaficare_backend::config::CleTotp::depuis_octets([7; 32])),
            email: Arc::new(laaficare_backend::email::EmailSenderConsole),
            url_application_web: Arc::from("http://localhost:4200"),
            contact_securite: Arc::from("securite@example.org"),
        };
        let (code, _) = auth_patient::demander_otp(&state.db, "70 02 20 01").await.unwrap();
        let naissance = NaiveDate::from_ymd_opt(1995, 4, 12).unwrap();
        let jeton = auth_patient::creer_compte(
            &state.db, &state.jwt, "70 02 20 01", &code, "OUEDRAOGO", "Awa", naissance, "Koudougou", "Exemple#2026",
        )
        .await
        .unwrap()
        .jeton_acces;

        let mut parts = Request::builder()
            .header(AUTHORIZATION, format!("Bearer {jeton}"))
            .body(())
            .unwrap()
            .into_parts()
            .0;
        let patient = PatientAuthentifie::from_request_parts(&mut parts, &state).await.unwrap();
        let Json(profil) = moi(patient, State(state.clone())).await.unwrap();

        let sub = state.jwt.verifier(&jeton, SorteJeton::Patient).unwrap().sub;
        assert_eq!(profil.compte_id, sub);
        assert_eq!((profil.nom.as_str(), profil.prenom.as_str()), ("OUEDRAOGO", "Awa"));
        assert_eq!(profil.telephone, "+22670022001");
        assert_eq!((profil.date_naissance, profil.lieu_naissance.as_str()), (naissance, "Koudougou"));
        assert_eq!(profil.nip.len(), 16);
    }
}
