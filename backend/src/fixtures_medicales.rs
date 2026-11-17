//! Données de test du module médecin (tests seulement) : patients,
//! établissements validés et affectations, toutes **fictives** (CLAUDE.md
//! §13 : jamais de vrais noms ni de vrais numéros).

use sqlx::PgPool;
use sqlx::types::Uuid;

use crate::affectation::Exercice;
use crate::nip;

/// Un patient fictif : identifiant interne, compte patient, NIP.
pub(crate) struct PatientTest {
    pub patient_id: Uuid,
    pub compte_id: Uuid,
    pub nip: String,
}

pub(crate) async fn patient(pool: &PgPool, telephone: &str) -> PatientTest {
    let utilisateur_id: Uuid = sqlx::query_scalar(
        "INSERT INTO utilisateur (nom, prenom, telephone) VALUES ('OUEDRAOGO', 'Awa', $1) RETURNING id",
    )
    .bind(telephone)
    .fetch_one(pool)
    .await
    .unwrap();
    let compte_id: Uuid = sqlx::query_scalar(
        "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'patient', 'x') RETURNING id",
    )
    .bind(utilisateur_id)
    .fetch_one(pool)
    .await
    .unwrap();
    let nip = nip::generer(pool).await.unwrap();
    let patient_id: Uuid = sqlx::query_scalar(
        "INSERT INTO patient (utilisateur_id, nip, date_naissance, lieu_naissance, sexe) \
         VALUES ($1, $2, '1995-04-12', 'Koudougou', 'feminin') RETURNING id",
    )
    .bind(utilisateur_id)
    .bind(&nip)
    .fetch_one(pool)
    .await
    .unwrap();
    PatientTest {
        patient_id,
        compte_id,
        nip,
    }
}

/// Un établissement validé, sans fin de licence, de ce type. La base
/// n'admet qu'une MUNASEB validée : pour ce type, la même est rendue.
pub(crate) async fn structure(pool: &PgPool, type_structure: &str, nom: &str) -> Uuid {
    if type_structure == "munaseb"
        && let Some(id) = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM structure WHERE type_structure = 'munaseb' AND statut = 'validee'",
        )
        .fetch_optional(pool)
        .await
        .unwrap()
    {
        return id;
    }
    let createur = compte_pro(pool).await;
    sqlx::query_scalar(
        "INSERT INTO structure (type_structure, nom, telephone, telephone_urgence, region, email, commune, province, \
                                latitude, longitude, createur_compte_id, statut) \
         VALUES ($1, $2, '+22670000000', '+22670000000', 'Région fictive', 'structure@example.org', \
                 'Commune fictive', 'Province fictive', 12.371400, -1.519700, $3, 'validee') RETURNING id",
    )
    .bind(type_structure)
    .bind(nom)
    .bind(createur)
    .fetch_one(pool)
    .await
    .unwrap()
}

static COMPTEUR: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Un compte professionnel fictif (numéro tiré d'un compteur : jamais deux
/// fois le même dans un test).
pub(crate) async fn compte_pro(pool: &PgPool) -> Uuid {
    let n = COMPTEUR.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let telephone = format!("+2266{:07}", n);
    let utilisateur: Uuid = sqlx::query_scalar(
        "INSERT INTO utilisateur (nom, prenom, telephone, email) VALUES ('SAWADOGO', 'Issa', $1, $2) RETURNING id",
    )
    .bind(&telephone)
    .bind(format!("pro{n}@example.org"))
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query_scalar(
        "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'professionnel', 'x') RETURNING id",
    )
    .bind(utilisateur)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Une affectation active de ce rôle dans l'établissement, et l'exercice
/// correspondant (ce que l'extracteur `ProfessionnelEnExercice` aurait
/// contrôlé).
pub(crate) async fn exercice(pool: &PgPool, structure_id: Uuid, role: &str) -> Exercice {
    let compte_id = compte_pro(pool).await;
    let affectation_id: Uuid = sqlx::query_scalar(
        "INSERT INTO affectation (compte_id, structure_id, role) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(compte_id)
    .bind(structure_id)
    .bind(role)
    .fetch_one(pool)
    .await
    .unwrap();
    let type_structure: String =
        sqlx::query_scalar("SELECT type_structure FROM structure WHERE id = $1")
            .bind(structure_id)
            .fetch_one(pool)
            .await
            .unwrap();
    Exercice {
        compte_id,
        affectation_id,
        structure_id,
        type_structure,
        role: role.to_string(),
    }
}
