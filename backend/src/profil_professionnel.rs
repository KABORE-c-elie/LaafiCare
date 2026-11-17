//! Profil professionnel : numéro d'ordre et spécialité (module médecin,
//! étape 1 ; décisions MD9 et MD10 du porteur, 2026-11-17 ; migration 0035).
//!
//! Ils appartiennent à la **personne**, pas à l'affectation (CLAUDE.md §6,
//! Q10) : un médecin garde son numéro d'une structure à l'autre. Le numéro
//! est en texte libre (aucune source n'en donne le format) et l'équipe
//! LaafiCare le **contrôle** : elle le marque vérifié. Tout changement par
//! la personne annule la vérification (trigger de la migration 0035).
//!
//! La vérification n'est **exigée nulle part** pour l'instant : faut-il un
//! numéro vérifié pour ouvrir un dossier ? Question dans
//! docs/points-ouverts.md.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use sqlx::types::Uuid;

/// Longueur maximale du numéro d'ordre, en caractères (garantie aussi par
/// la base). Un garde-fou, aucune source ne donne le format.
pub const NUMERO_ORDRE_MAX: usize = 50;

/// Spécialités (décision MD10 : liste courte proposée au porteur, à
/// valider). La sage-femme est une spécialité d'infirmier (CLAUDE.md §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Specialite {
    MedecineGenerale,
    GynecologieObstetrique,
    Pediatrie,
    Chirurgie,
    SageFemme,
}

const SPECIALITES: [Specialite; 5] = [
    Specialite::MedecineGenerale,
    Specialite::GynecologieObstetrique,
    Specialite::Pediatrie,
    Specialite::Chirurgie,
    Specialite::SageFemme,
];

impl Specialite {
    /// Valeur en base (CHECK de la migration 0035).
    pub fn as_str(self) -> &'static str {
        match self {
            Specialite::MedecineGenerale => "medecine_generale",
            Specialite::GynecologieObstetrique => "gynecologie_obstetrique",
            Specialite::Pediatrie => "pediatrie",
            Specialite::Chirurgie => "chirurgie",
            Specialite::SageFemme => "sage_femme",
        }
    }

    fn depuis_base(valeur: &str) -> Option<Self> {
        SPECIALITES.into_iter().find(|s| s.as_str() == valeur)
    }
}

fn specialite_lue(valeur: Option<String>) -> Result<Option<Specialite>, ErreurProfil> {
    valeur
        .map(|v| {
            Specialite::depuis_base(&v)
                .ok_or_else(|| ErreurProfil::Interne(format!("spécialité inconnue en base : {v}")))
        })
        .transpose()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProfilProfessionnel {
    pub numero_ordre: Option<String>,
    pub specialite: Option<Specialite>,
    pub verifie: bool,
    pub verifie_le: Option<DateTime<Utc>>,
}

#[derive(Debug, thiserror::Error)]
pub enum ErreurProfil {
    #[error(
        "Le numéro d'ordre doit faire entre 1 et {NUMERO_ORDRE_MAX} caractères, sans retour à la ligne."
    )]
    NumeroOrdreInvalide,
    /// Profil absent, sans numéro, déjà vérifié, ou modifié depuis que
    /// l'équipe l'a lu : jamais distingués.
    #[error("Ce profil ne peut pas être vérifié dans cet état.")]
    VerificationImpossible,
    #[error("erreur interne : {0}")]
    Interne(String),
}

impl From<sqlx::Error> for ErreurProfil {
    fn from(erreur: sqlx::Error) -> Self {
        ErreurProfil::Interne(erreur.to_string())
    }
}

fn numero_valide(saisie: Option<&str>) -> Result<Option<String>, ErreurProfil> {
    let Some(numero) = saisie.map(str::trim) else {
        return Ok(None);
    };
    let longueur = numero.chars().count();
    if longueur == 0 || longueur > NUMERO_ORDRE_MAX || numero.chars().any(char::is_control) {
        return Err(ErreurProfil::NumeroOrdreInvalide);
    }
    Ok(Some(numero.to_string()))
}

/// Profil du compte professionnel ; vide s'il n'a jamais été rempli.
pub async fn lire(pool: &PgPool, compte_id: Uuid) -> Result<ProfilProfessionnel, ErreurProfil> {
    // Numéro, spécialité, date de vérification.
    type Ligne = (Option<String>, Option<String>, Option<DateTime<Utc>>);
    let ligne: Option<Ligne> =
        sqlx::query_as(
            "SELECT numero_ordre, specialite, verifie_le FROM profil_professionnel WHERE compte_id = $1",
        )
        .bind(compte_id)
        .fetch_optional(pool)
        .await?;
    Ok(match ligne {
        Some((numero_ordre, specialite, verifie_le)) => ProfilProfessionnel {
            numero_ordre,
            specialite: specialite_lue(specialite)?,
            verifie: verifie_le.is_some(),
            verifie_le,
        },
        None => ProfilProfessionnel {
            numero_ordre: None,
            specialite: None,
            verifie: false,
            verifie_le: None,
        },
    })
}

/// Remplace le numéro et la spécialité (un champ absent est vidé). Un
/// changement annule la vérification (trigger) ; le même contenu la garde.
pub async fn modifier(
    pool: &PgPool,
    compte_id: Uuid,
    numero_ordre: Option<&str>,
    specialite: Option<Specialite>,
) -> Result<ProfilProfessionnel, ErreurProfil> {
    let numero = numero_valide(numero_ordre)?;
    sqlx::query(
        "INSERT INTO profil_professionnel (compte_id, numero_ordre, specialite) VALUES ($1, $2, $3) \
         ON CONFLICT (compte_id) DO UPDATE SET numero_ordre = EXCLUDED.numero_ordre, specialite = EXCLUDED.specialite",
    )
    .bind(compte_id)
    .bind(&numero)
    .bind(specialite.map(Specialite::as_str))
    .execute(pool)
    .await?;
    lire(pool, compte_id).await
}

/// Un profil à contrôler, tel que l'équipe le voit.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ProfilAVerifier {
    pub compte_id: Uuid,
    pub nom: Option<String>,
    pub prenom: Option<String>,
    pub email: Option<String>,
    pub numero_ordre: String,
    /// Valeur de la liste `Specialite`, telle qu'en base.
    pub specialite: Option<String>,
    pub modifie_le: DateTime<Utc>,
}

/// Profils avec un numéro, pas encore vérifiés, les plus anciens d'abord
/// (curseur sur l'identifiant du compte, `uuidv7`).
pub async fn lister_a_verifier(
    pool: &PgPool,
    apres: Option<Uuid>,
    limite: i64,
) -> Result<Vec<ProfilAVerifier>, sqlx::Error> {
    sqlx::query_as(
        "SELECT p.compte_id, u.nom, u.prenom, u.email, p.numero_ordre, p.specialite, p.modifie_le \
         FROM profil_professionnel p JOIN compte c ON c.id = p.compte_id \
         JOIN utilisateur u ON u.id = c.utilisateur_id \
         WHERE p.numero_ordre IS NOT NULL AND p.verifie_le IS NULL \
           AND ($1::uuid IS NULL OR p.compte_id > $1) \
         ORDER BY p.compte_id LIMIT $2",
    )
    .bind(apres)
    .bind(limite)
    .fetch_all(pool)
    .await
}

/// L'équipe marque vérifié **le numéro et la spécialité qu'elle a lus** :
/// s'ils ont changé entre-temps, rien n'est vérifié (pas de vérification
/// d'une valeur que l'équipe n'a pas vue).
pub async fn verifier(
    pool: &PgPool,
    administrateur_compte_id: Uuid,
    compte_id: Uuid,
    numero_ordre_lu: &str,
    specialite_lue: Option<Specialite>,
) -> Result<(), ErreurProfil> {
    let verifie = sqlx::query(
        "UPDATE profil_professionnel SET verifie_le = now(), verifie_par_compte_id = $2 \
         WHERE compte_id = $1 AND verifie_le IS NULL AND numero_ordre = $3 \
           AND specialite IS NOT DISTINCT FROM $4",
    )
    .bind(compte_id)
    .bind(administrateur_compte_id)
    .bind(numero_ordre_lu)
    .bind(specialite_lue.map(Specialite::as_str))
    .execute(pool)
    .await?
    .rows_affected()
        == 1;
    if verifie {
        Ok(())
    } else {
        Err(ErreurProfil::VerificationImpossible)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn compte_pro(pool: &PgPool, telephone: &str) -> Uuid {
        let utilisateur: Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (nom, prenom, telephone, email) VALUES ('SAWADOGO', 'Ali', $1, $2) RETURNING id",
        )
        .bind(telephone)
        .bind(format!("{}@example.org", &telephone[1..]))
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query_scalar(
            "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, $2, 'x') RETURNING id",
        )
        .bind(utilisateur)
        .bind("professionnel")
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn administrateur(pool: &PgPool, telephone: &str) -> Uuid {
        let utilisateur: Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (nom, prenom, telephone) VALUES ('KONE', 'Adama', $1) RETURNING id",
        )
        .bind(telephone)
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query_scalar(
            "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'administrateur_laaficare', 'x') RETURNING id",
        )
        .bind(utilisateur)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[test]
    fn numero_controle() {
        assert_eq!(numero_valide(None).unwrap(), None);
        assert_eq!(
            numero_valide(Some("  ONMBF-1234 ")).unwrap().as_deref(),
            Some("ONMBF-1234")
        );
        for faux in ["", "   ", "A\nB"] {
            assert!(matches!(
                numero_valide(Some(faux)),
                Err(ErreurProfil::NumeroOrdreInvalide)
            ));
        }
        let long = "1".repeat(NUMERO_ORDRE_MAX + 1);
        assert!(numero_valide(Some(&long)).is_err());
    }

    #[sqlx::test]
    #[ignore]
    async fn profil_verifie_puis_modifie(pool: PgPool) {
        let pro = compte_pro(&pool, "+22670350001").await;
        let adm = administrateur(&pool, "+22670350002").await;
        assert_eq!(lire(&pool, pro).await.unwrap().numero_ordre, None);

        let p = modifier(&pool, pro, Some("ONM-0001"), Some(Specialite::Pediatrie))
            .await
            .unwrap();
        assert!(!p.verifie);
        assert_eq!(
            lister_a_verifier(&pool, None, 50).await.unwrap()[0].compte_id,
            pro
        );

        // Une valeur que l'équipe n'a pas lue n'est pas vérifiée.
        assert!(matches!(
            verifier(&pool, adm, pro, "ONM-9999", Some(Specialite::Pediatrie)).await,
            Err(ErreurProfil::VerificationImpossible)
        ));
        verifier(&pool, adm, pro, "ONM-0001", Some(Specialite::Pediatrie))
            .await
            .unwrap();
        assert!(lire(&pool, pro).await.unwrap().verifie);
        assert!(lister_a_verifier(&pool, None, 50).await.unwrap().is_empty());

        // Le même contenu garde la vérification ; un changement l'annule.
        assert!(
            modifier(&pool, pro, Some("ONM-0001"), Some(Specialite::Pediatrie))
                .await
                .unwrap()
                .verifie
        );
        assert!(
            !modifier(&pool, pro, Some("ONM-0001"), Some(Specialite::Chirurgie))
                .await
                .unwrap()
                .verifie
        );

        // La base refuse qu'un compte non administrateur vérifie, et qu'un
        // profil soit effacé par le serveur (droits, testés dans
        // tests/droits_des_roles.rs).
        let refus = sqlx::query(
            "UPDATE profil_professionnel SET verifie_le = now(), verifie_par_compte_id = $1 WHERE compte_id = $1",
        )
        .bind(pro)
        .execute(&pool)
        .await;
        assert!(refus.is_err());
    }
}
