//! Données du patient connecté (décision M1, 2026-10-11) : lectures pour
//! les routes patient protégées par l'extracteur `PatientAuthentifie`.
//! L'authentification reste dans `auth_patient.rs` ; les futures routes
//! patient (droits MUNASEB, demandes, notifications) viendront ici.

use chrono::NaiveDate;
use serde::Serialize;
use sqlx::PgPool;
use sqlx::types::Uuid;

/// Profil renvoyé par `GET /api/patients/moi`.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ProfilPatient {
    /// Identifiant du compte patient (le `sub` du jeton). Ne change jamais :
    /// l'application mobile s'en sert pour rattacher ses rappels.
    pub compte_id: Uuid,
    pub nip: String,
    pub nom: String,
    pub prenom: String,
    /// Forme normalisée (`+226…`, décision C3).
    pub telephone: String,
    pub date_naissance: NaiveDate,
    pub lieu_naissance: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ErreurProfil {
    /// Incohérence du serveur, jamais une erreur de l'application (M2, M3) :
    /// le détail va dans les journaux, la réponse est un 500.
    #[error("erreur interne : {0}")]
    Interne(String),
}

impl From<sqlx::Error> for ErreurProfil {
    fn from(erreur: sqlx::Error) -> Self {
        ErreurProfil::Interne(erreur.to_string())
    }
}

/// Profil du compte patient `compte_id`, déjà vérifié par l'extracteur
/// (compte patient actif, bonne `version_jeton`).
///
/// `LEFT JOIN patient` : un compte patient sans dossier `patient` ne peut
/// pas naître de l'inscription (une seule transaction crée les deux) ; s'il
/// existe, c'est une incohérence en base, signalée comme erreur interne
/// (M3) plutôt que masquée. Même chose pour `nom` et `prenom`, que la
/// migration 0004 laisse vides seulement pour une inscription inachevée,
/// donc sans compte (M2).
pub async fn profil(pool: &PgPool, compte_id: Uuid) -> Result<ProfilPatient, ErreurProfil> {
    type Ligne = (Option<String>, Option<String>, String, Option<String>, Option<NaiveDate>, Option<String>);
    let ligne: Option<Ligne> = sqlx::query_as(
        "SELECT u.nom, u.prenom, u.telephone, p.nip, p.date_naissance, p.lieu_naissance \
         FROM compte c JOIN utilisateur u ON u.id = c.utilisateur_id \
         LEFT JOIN patient p ON p.utilisateur_id = u.id \
         WHERE c.id = $1 AND c.type_compte = 'patient'",
    )
    .bind(compte_id)
    .fetch_optional(pool)
    .await?;

    let Some((nom, prenom, telephone, nip, date_naissance, lieu_naissance)) = ligne else {
        return Err(ErreurProfil::Interne(format!("compte patient introuvable : {compte_id}")));
    };
    let (Some(nip), Some(date_naissance), Some(lieu_naissance)) = (nip, date_naissance, lieu_naissance) else {
        return Err(ErreurProfil::Interne(format!("compte patient sans dossier patient : {compte_id}")));
    };
    let (Some(nom), Some(prenom)) = (nom, prenom) else {
        return Err(ErreurProfil::Interne(format!("compte patient sans nom ni prénom : {compte_id}")));
    };

    Ok(ProfilPatient { compte_id, nip, nom, prenom, telephone, date_naissance, lieu_naissance })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Base temporaire par test ; données fictives.

    #[sqlx::test]
    #[ignore]
    async fn profil_complet_et_incoherences(pool: PgPool) {
        let utilisateur_id: Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (nom, prenom, telephone) VALUES ('OUEDRAOGO', 'Awa', '+22670020001') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let compte_id: Uuid = sqlx::query_scalar(
            "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'patient', 'x') RETURNING id",
        )
        .bind(utilisateur_id)
        .fetch_one(&pool)
        .await
        .unwrap();

        // Compte sans dossier patient : incohérence signalée (M3).
        assert!(matches!(profil(&pool, compte_id).await, Err(ErreurProfil::Interne(_))));

        sqlx::query(
            "INSERT INTO patient (utilisateur_id, nip, date_naissance, lieu_naissance) \
             VALUES ($1, '1000000000000008', '1995-04-12', 'Koudougou')",
        )
        .bind(utilisateur_id)
        .execute(&pool)
        .await
        .unwrap();

        assert_eq!(
            profil(&pool, compte_id).await.unwrap(),
            ProfilPatient {
                compte_id,
                nip: "1000000000000008".into(),
                nom: "OUEDRAOGO".into(),
                prenom: "Awa".into(),
                telephone: "+22670020001".into(),
                date_naissance: NaiveDate::from_ymd_opt(1995, 4, 12).unwrap(),
                lieu_naissance: "Koudougou".into(),
            }
        );

        // Compte inconnu : erreur interne (l'extracteur l'a normalement écarté).
        assert!(matches!(profil(&pool, Uuid::from_u128(1)).await, Err(ErreurProfil::Interne(_))));
    }
}
