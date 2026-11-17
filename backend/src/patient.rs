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
    /// Facultatif (décision du porteur du 2026-11-17, migration 0035) :
    /// `null` tant que la personne ne l'a pas donné. Champ ajouté : la
    /// réponse existante ne change pas.
    pub sexe: Option<Sexe>,
}

/// Sexe du patient (migration 0035). Deux valeurs ; aucun texte officiel
/// lu ne fixe la liste (docs/points-ouverts.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Sexe {
    Feminin,
    Masculin,
}

impl Sexe {
    pub fn as_str(self) -> &'static str {
        match self {
            Sexe::Feminin => "feminin",
            Sexe::Masculin => "masculin",
        }
    }

    /// Valeur lue en base ; une valeur hors liste est impossible (CHECK).
    pub fn depuis_base(valeur: Option<&str>) -> Option<Self> {
        match valeur {
            Some("feminin") => Some(Sexe::Feminin),
            Some("masculin") => Some(Sexe::Masculin),
            _ => None,
        }
    }
}

/// Le patient donne, change ou retire son sexe (`None`).
pub async fn modifier_sexe(
    pool: &PgPool,
    compte_id: Uuid,
    sexe: Option<Sexe>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE patient p SET sexe = $2 FROM compte c \
         WHERE c.id = $1 AND c.type_compte = 'patient' AND p.utilisateur_id = c.utilisateur_id",
    )
    .bind(compte_id)
    .bind(sexe.map(Sexe::as_str))
    .execute(pool)
    .await?;
    Ok(())
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
    type Ligne = (
        Option<String>,
        Option<String>,
        String,
        Option<String>,
        Option<NaiveDate>,
        Option<String>,
        Option<String>,
    );
    let ligne: Option<Ligne> = sqlx::query_as(
        "SELECT u.nom, u.prenom, u.telephone, p.nip, p.date_naissance, p.lieu_naissance, p.sexe \
         FROM compte c JOIN utilisateur u ON u.id = c.utilisateur_id \
         LEFT JOIN patient p ON p.utilisateur_id = u.id \
         WHERE c.id = $1 AND c.type_compte = 'patient'",
    )
    .bind(compte_id)
    .fetch_optional(pool)
    .await?;

    let Some((nom, prenom, telephone, nip, date_naissance, lieu_naissance, sexe)) = ligne else {
        return Err(ErreurProfil::Interne(format!(
            "compte patient introuvable : {compte_id}"
        )));
    };
    let (Some(nip), Some(date_naissance), Some(lieu_naissance)) =
        (nip, date_naissance, lieu_naissance)
    else {
        return Err(ErreurProfil::Interne(format!(
            "compte patient sans dossier patient : {compte_id}"
        )));
    };
    let (Some(nom), Some(prenom)) = (nom, prenom) else {
        return Err(ErreurProfil::Interne(format!(
            "compte patient sans nom ni prénom : {compte_id}"
        )));
    };

    Ok(ProfilPatient {
        compte_id,
        nip,
        nom,
        prenom,
        telephone,
        date_naissance,
        lieu_naissance,
        sexe: Sexe::depuis_base(sexe.as_deref()),
    })
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
        assert!(matches!(
            profil(&pool, compte_id).await,
            Err(ErreurProfil::Interne(_))
        ));

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
                sexe: None,
            }
        );

        // Sexe facultatif : donné, puis retiré.
        modifier_sexe(&pool, compte_id, Some(Sexe::Feminin))
            .await
            .unwrap();
        assert_eq!(
            profil(&pool, compte_id).await.unwrap().sexe,
            Some(Sexe::Feminin)
        );
        modifier_sexe(&pool, compte_id, None).await.unwrap();
        assert_eq!(profil(&pool, compte_id).await.unwrap().sexe, None);

        // Compte inconnu : erreur interne (l'extracteur l'a normalement écarté).
        assert!(matches!(
            profil(&pool, Uuid::from_u128(1)).await,
            Err(ErreurProfil::Interne(_))
        ));
    }
}
