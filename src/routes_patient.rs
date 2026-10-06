//! Routes HTTP exposant `AuthPatientService` (section 11). Chaque handler
//! ne fait que : désérialiser, appeler le service métier, traduire le
//! résultat en réponse HTTP -- aucune logique métier ici.

use axum::{Json, extract::State, http::StatusCode};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::erreur_api::{ErreurApi, JsonApi, ReponseErreur, reponse_erreur, reponse_erreur_interne};
use laaficare_backend::auth_patient::{self, ErreurAuthPatient};

// Un seul endroit qui décide du code HTTP et du `code` par variante --
// section 13, codes HTTP validés avec le porteur avant d'écrire ce
// fichier ; `code` ajouté pour le format d'erreur unique (section 12).
fn mapper_erreur(erreur: ErreurAuthPatient) -> ReponseErreur {
    let (statut, code) = match &erreur {
        ErreurAuthPatient::TelephoneInvalide => (StatusCode::UNPROCESSABLE_ENTITY, "telephone_invalide"),
        ErreurAuthPatient::CodeInvalide => (StatusCode::UNAUTHORIZED, "code_invalide"),
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
) -> Result<StatusCode, ReponseErreur> {
    let (code, telephone) = auth_patient::demander_otp(&state.db, &corps.telephone)
        .await
        .map_err(mapper_erreur)?;

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

#[derive(Serialize)]
pub struct InscriptionReponse {
    pub jeton: String,
}

/// `POST /api/patients/inscription` -- flux de création de compte
/// (section 11) : un seul appel, atomique côté service.
pub async fn creer_compte(
    State(state): State<AppState>,
    JsonApi(corps): JsonApi<InscriptionRequete>,
) -> Result<(StatusCode, Json<InscriptionReponse>), ReponseErreur> {
    let jeton = auth_patient::creer_compte(
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
    .map_err(mapper_erreur)?;

    Ok((StatusCode::CREATED, Json(InscriptionReponse { jeton })))
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
) -> Result<StatusCode, ReponseErreur> {
    auth_patient::reinitialiser_mot_de_passe(
        &state.db,
        &corps.telephone,
        &corps.code_otp,
        &corps.nouveau_mot_de_passe,
    )
    .await
    .map_err(mapper_erreur)?;

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct ConnexionRequete {
    pub telephone: String,
    pub mot_de_passe: String,
}

#[derive(Serialize)]
pub struct ConnexionReponse {
    pub jeton: String,
}

/// `POST /api/patients/connexion` -- téléphone + mot de passe, verrouillage
/// BF-01. Tout échec : 401 `identifiants_invalides` (décisions P1, L1).
pub async fn se_connecter(
    State(state): State<AppState>,
    JsonApi(corps): JsonApi<ConnexionRequete>,
) -> Result<Json<ConnexionReponse>, ReponseErreur> {
    let jeton = auth_patient::se_connecter(&state.db, &state.jwt, &corps.telephone, &corps.mot_de_passe)
        .await
        .map_err(mapper_erreur)?;
    Ok(Json(ConnexionReponse { jeton }))
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
    fn mot_de_passe_non_conforme_liste_les_regles() {
        use laaficare_backend::mot_de_passe::RegleMotDePasse;
        let regles = vec![RegleMotDePasse::Chiffre, RegleMotDePasse::CaractereSpecial];
        let (s, Json(corps)) = mapper_erreur(ErreurAuthPatient::MotDePasseNonConforme(regles.clone()));
        assert_eq!((s, corps.code), (StatusCode::UNPROCESSABLE_ENTITY, "mot_de_passe_non_conforme"));
        assert_eq!(corps.regles_non_respectees, Some(regles));
        let json = serde_json::to_string(&corps).unwrap();
        assert!(json.contains(r#""regles_non_respectees":["chiffre","caractere_special"]"#), "{json}");
    }
}
