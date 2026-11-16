//! Catalogue national des médicaments (pharmacie, étape 3 ; CLAUDE.md §15,
//! D6 ; migration 0030). Écrit seulement par l'outil local
//! `importer_catalogue` (rôle `laaficare_outils`), lu par les pharmacies.
//!
//! **Aucune donnée réelle n'est livrée** : le catalogue reste vide tant que
//! le fichier officiel (liste nationale des médicaments essentiels 2023,
//! ministère de la Santé) n'est pas dans `docs/Reglementation, loi et
//! reference/`. Les tests n'utilisent que des médicaments fictifs.
//!
//! **Format du fichier d'import** (le fichier officiel est un PDF : l'équipe
//! en tire un fichier texte, relu ligne à ligne, avant l'import) :
//! - texte UTF-8 ; première ligne exactement `denomination;dci;forme;dosage` ;
//! - une ligne par médicament, quatre champs séparés par `;` (aucun champ ne
//!   peut contenir `;`, il n'y a pas de guillemets) ;
//! - `dci` : une ou plusieurs molécules séparées par `+` (association) ;
//! - lignes vides ignorées.

use std::collections::BTreeSet;

use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use sqlx::types::Uuid;

/// En-tête obligatoire du fichier d'import.
pub const ENTETE: &str = "denomination;dci;forme;dosage";

// Garde-fous techniques (mêmes bornes que les CHECK de la 0030) ; 20 000
// lignes au plus par fichier (la liste 2023 en compte un peu plus de mille,
// selon le résumé trouvé le 2026-11-16).
const MAX_LIGNES: usize = 20_000;
const MAX_DENOMINATION: usize = 200;
const MAX_DCI: usize = 200;
const MAX_FORME_DOSAGE: usize = 100;
const MAX_SOURCE: usize = 300;

#[derive(Debug, thiserror::Error)]
pub enum ErreurCatalogue {
    #[error("Le fichier n'est pas un texte UTF-8.")]
    PasUtf8,
    #[error("La première ligne doit être exactement : {ENTETE}")]
    EnteteInvalide,
    #[error("Ligne {ligne} : {probleme}")]
    LigneInvalide { ligne: usize, probleme: String },
    #[error("Le fichier ne contient aucun médicament.")]
    Vide,
    #[error("Le fichier dépasse {MAX_LIGNES} lignes.")]
    TropDeLignes,
    #[error("La source doit être décrite (titre, édition), en {MAX_SOURCE} caractères au plus.")]
    SourceInvalide,
    #[error("erreur interne : {0}")]
    Interne(String),
}

impl From<sqlx::Error> for ErreurCatalogue {
    fn from(erreur: sqlx::Error) -> Self {
        ErreurCatalogue::Interne(erreur.to_string())
    }
}

/// Une ligne lue, nettoyée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LigneCatalogue {
    pub denomination: String,
    pub dcis: Vec<String>,
    pub forme: String,
    pub dosage: String,
}

/// Forme de comparaison : minuscules (Unicode, `str::to_lowercase`) et
/// espaces réduites à une seule. Deux écritures d'un même nom (« Paracétamol »,
/// « paracétamol ») ne créent pas deux lignes.
pub fn cle(texte: &str) -> String {
    texte.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

fn champ(valeur: &str, nom: &str, max: usize, ligne: usize) -> Result<String, ErreurCatalogue> {
    let propre = valeur.split_whitespace().collect::<Vec<_>>().join(" ");
    let probleme = if propre.is_empty() {
        format!("{nom} vide")
    } else if propre.chars().count() > max {
        format!("{nom} de plus de {max} caractères")
    } else if propre.chars().any(char::is_control) {
        format!("{nom} avec un caractère de contrôle")
    } else {
        return Ok(propre);
    };
    Err(ErreurCatalogue::LigneInvalide { ligne, probleme })
}

/// Lit le fichier d'import (fonction pure, testée sans base). Les numéros
/// de ligne commencent à 1 (l'en-tête est la ligne 1).
pub fn lire_fichier(contenu: &str) -> Result<Vec<LigneCatalogue>, ErreurCatalogue> {
    // Marque d'ordre des octets UTF-8 éventuelle (tableurs).
    let contenu = contenu.strip_prefix('\u{feff}').unwrap_or(contenu);
    let mut lignes = contenu.lines();
    if lignes.next().map(str::trim) != Some(ENTETE) {
        return Err(ErreurCatalogue::EnteteInvalide);
    }
    let mut resultat = Vec::new();
    for (index, texte) in lignes.enumerate() {
        let numero = index + 2;
        if numero > MAX_LIGNES + 1 {
            return Err(ErreurCatalogue::TropDeLignes);
        }
        if texte.trim().is_empty() {
            continue;
        }
        let champs: Vec<&str> = texte.split(';').collect();
        let [denomination, dci, forme, dosage] = champs[..] else {
            return Err(ErreurCatalogue::LigneInvalide { ligne: numero, probleme: format!("{} champs au lieu de 4", champs.len()) });
        };
        let mut dcis = Vec::new();
        for molecule in dci.split('+') {
            let molecule = champ(molecule, "dci", MAX_DCI, numero)?;
            if !dcis.iter().any(|d: &String| cle(d) == cle(&molecule)) {
                dcis.push(molecule);
            }
        }
        resultat.push(LigneCatalogue {
            denomination: champ(denomination, "denomination", MAX_DENOMINATION, numero)?,
            dcis,
            forme: champ(forme, "forme", MAX_FORME_DOSAGE, numero)?,
            dosage: champ(dosage, "dosage", MAX_FORME_DOSAGE, numero)?,
        });
    }
    if resultat.is_empty() {
        return Err(ErreurCatalogue::Vide);
    }
    Ok(resultat)
}

fn cle_medicament(ligne: &LigneCatalogue) -> String {
    format!("{}|{}|{}", cle(&ligne.denomination), cle(&ligne.forme), cle(&ligne.dosage))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BilanImport {
    pub import_id: Uuid,
    pub lignes_lues: usize,
    pub medicaments_ajoutes: usize,
    pub deja_presents: usize,
}

/// Import d'un fichier, par un administrateur authentifié par l'outil, en
/// une seule transaction : tout ou rien. Un médicament déjà présent (même
/// dénomination, forme et dosage) est laissé tel quel. Les imports sont
/// sérialisés par un verrou consultatif de transaction (doc PostgreSQL 18,
/// `pg_advisory_xact_lock`) : l'outil est local et rare, deux imports
/// simultanés compteraient mal les ajouts.
pub async fn importer(
    pool: &PgPool,
    administrateur_compte_id: Uuid,
    source: &str,
    fichier: &[u8],
) -> Result<BilanImport, ErreurCatalogue> {
    let source = source.trim();
    if source.is_empty() || source.chars().count() > MAX_SOURCE || source.chars().any(char::is_control) {
        return Err(ErreurCatalogue::SourceInvalide);
    }
    let texte = std::str::from_utf8(fichier).map_err(|_| ErreurCatalogue::PasUtf8)?;
    let lignes = lire_fichier(texte)?;
    // Une même ligne répétée dans le fichier ne compte qu'une fois.
    let mut vues = BTreeSet::new();
    let lignes: Vec<LigneCatalogue> = lignes.into_iter().filter(|l| vues.insert(cle_medicament(l))).collect();

    let mut tx = pool.begin().await?;
    // Clé du verrou : un nombre fixe propre à cet import (« catalogue »).
    sqlx::query("SELECT pg_advisory_xact_lock(1_330_2026)").execute(&mut *tx).await?;
    let cles: Vec<String> = lignes.iter().map(cle_medicament).collect();
    let deja: i64 = sqlx::query_scalar("SELECT count(*) FROM medicament WHERE cle = ANY($1)")
        .bind(&cles)
        .fetch_one(&mut *tx)
        .await?;
    let deja = deja as usize;

    let import_id: Uuid = sqlx::query_scalar(
        "INSERT INTO import_catalogue (administrateur_compte_id, source, fichier_sha256, lignes_lues, medicaments_ajoutes) \
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(administrateur_compte_id)
    .bind(source)
    .bind(Sha256::digest(fichier).to_vec())
    .bind(lignes.len() as i32)
    .bind((lignes.len() - deja) as i32)
    .fetch_one(&mut *tx)
    .await?;

    for ligne in &lignes {
        let medicament: Option<Uuid> = sqlx::query_scalar(
            "INSERT INTO medicament (denomination, forme, dosage, cle, import_id) VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (cle) DO NOTHING RETURNING id",
        )
        .bind(&ligne.denomination)
        .bind(&ligne.forme)
        .bind(&ligne.dosage)
        .bind(cle_medicament(ligne))
        .bind(import_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(medicament_id) = medicament else { continue };
        for molecule in &ligne.dcis {
            sqlx::query("INSERT INTO dci (nom, cle, import_id) VALUES ($1, $2, $3) ON CONFLICT (cle) DO NOTHING")
                .bind(molecule)
                .bind(cle(molecule))
                .bind(import_id)
                .execute(&mut *tx)
                .await?;
            sqlx::query("INSERT INTO medicament_dci (medicament_id, dci_id) SELECT $1, id FROM dci WHERE cle = $2")
                .bind(medicament_id)
                .bind(cle(molecule))
                .execute(&mut *tx)
                .await?;
        }
    }
    tx.commit().await?;
    Ok(BilanImport { import_id, lignes_lues: lignes.len(), medicaments_ajoutes: lignes.len() - deja, deja_presents: deja })
}

/// Un médicament tel que la pharmacie le voit.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Medicament {
    pub id: Uuid,
    pub denomination: String,
    pub forme: String,
    pub dosage: String,
    /// Les DCI, dans l'ordre alphabétique.
    pub dci: Vec<String>,
}

// Macro : sqlx 0.9 n'accepte que des chaînes SQL littérales.
macro_rules! select_medicament {
    () => {
        "SELECT m.id, m.denomination, m.forme, m.dosage, \
                COALESCE((SELECT array_agg(d.nom ORDER BY d.cle) FROM medicament_dci md JOIN dci d ON d.id = md.dci_id \
                          WHERE md.medicament_id = m.id), '{}') AS dci \
         FROM medicament m"
    };
}
pub(crate) use select_medicament;

/// Échappe `%`, `_` et `\` pour un motif `ILIKE` (doc PostgreSQL 18,
/// §9.7.1 : `\` est le caractère d'échappement par défaut de LIKE).
fn motif_like(texte: &str) -> String {
    let mut motif = String::from("%");
    for c in texte.chars() {
        if matches!(c, '%' | '_' | '\\') {
            motif.push('\\');
        }
        motif.push(c);
    }
    motif.push('%');
    motif
}

/// Recherche par dénomination ou par DCI (contient, sans tenir compte des
/// majuscules), dans l'ordre alphabétique de la dénomination.
pub async fn rechercher(pool: &PgPool, texte: &str, limite: i64) -> Result<Vec<Medicament>, sqlx::Error> {
    sqlx::query_as(concat!(
        select_medicament!(),
        " WHERE m.denomination ILIKE $1 \
              OR EXISTS (SELECT 1 FROM medicament_dci md JOIN dci d ON d.id = md.dci_id \
                         WHERE md.medicament_id = m.id AND d.nom ILIKE $1) \
         ORDER BY m.cle, m.id LIMIT $2"
    ))
    .bind(motif_like(texte.trim()))
    .bind(limite)
    .fetch_all(pool)
    .await
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn lecture_du_fichier() {
        let fichier = "\u{feff}denomination;dci;forme;dosage\n\
                       Doliprane fictif ; Paracétamol ; comprimé ; 500 mg\n\
                       \n\
                       Générique fictif;paracétamol + Codéine + PARACÉTAMOL;comprimé;500 mg/30 mg\n";
        let lignes = lire_fichier(fichier).unwrap();
        assert_eq!(lignes.len(), 2);
        assert_eq!(lignes[0].denomination, "Doliprane fictif");
        assert_eq!(lignes[1].dcis, ["paracétamol", "Codéine"], "doublon de DCI retiré, majuscules ignorées");

        assert!(matches!(lire_fichier("nom;dci\nx;y"), Err(ErreurCatalogue::EnteteInvalide)));
        assert!(matches!(lire_fichier(ENTETE), Err(ErreurCatalogue::Vide)));
        assert!(matches!(
            lire_fichier("denomination;dci;forme;dosage\na;b;c"),
            Err(ErreurCatalogue::LigneInvalide { ligne: 2, .. })
        ));
        assert!(matches!(
            lire_fichier("denomination;dci;forme;dosage\na;b;c;d\na; + ;c;d"),
            Err(ErreurCatalogue::LigneInvalide { ligne: 3, .. })
        ));
        assert!(matches!(
            lire_fichier(&format!("denomination;dci;forme;dosage\n{};b;c;d", "x".repeat(201))),
            Err(ErreurCatalogue::LigneInvalide { ligne: 2, .. })
        ));
    }

    #[test]
    fn cles_et_motifs() {
        assert_eq!(cle("  Paracétamol   500  "), "paracétamol 500");
        assert_eq!(motif_like("50%_a\\"), "%50\\%\\_a\\\\%");
    }

    // ------------------------------------------------------------------
    // En base (`#[sqlx::test]`, données fictives)
    // ------------------------------------------------------------------

    pub(crate) async fn administrateur(pool: &PgPool) -> Uuid {
        sqlx::query_scalar(
            "WITH u AS (INSERT INTO utilisateur (nom, prenom, telephone) VALUES ('ZONGO', 'Admin', '+22670530001') RETURNING id) \
             INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) \
             SELECT id, 'administrateur_laaficare', 'x' FROM u RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// Import tout ou rien, tracé ; second import sans doublon ; recherche
    /// par nom ou par DCI ; catalogue jamais modifié.
    #[sqlx::test]
    #[ignore]
    async fn import_et_recherche(pool: PgPool) {
        let admin = administrateur(&pool).await;
        let fichier = "denomination;dci;forme;dosage\n\
                       Antalgique fictif;Molécule-A fictive;comprimé;500 mg\n\
                       Générique fictif;molécule-a FICTIVE;comprimé;500 mg\n\
                       Association fictive;Molécule-A fictive + Molécule-B fictive;comprimé;500 mg/30 mg\n";
        assert!(matches!(importer(&pool, admin, "  ", fichier.as_bytes()).await, Err(ErreurCatalogue::SourceInvalide)));
        assert!(matches!(importer(&pool, admin, "Source fictive", &[0xFF, 0xFE]).await, Err(ErreurCatalogue::PasUtf8)));
        // Une ligne fausse : rien n'est écrit.
        let faux = format!("{fichier}Ligne fausse;;comprimé;1 mg\n");
        assert!(importer(&pool, admin, "Source fictive", faux.as_bytes()).await.is_err());
        let total = || async { sqlx::query_scalar::<_, i64>("SELECT count(*) FROM medicament").fetch_one(&pool).await.unwrap() };
        assert_eq!(total().await, 0);

        let bilan = importer(&pool, admin, "Liste fictive, édition de test", fichier.as_bytes()).await.unwrap();
        assert_eq!((bilan.lignes_lues, bilan.medicaments_ajoutes, bilan.deja_presents), (3, 3, 0));
        let dci: i64 = sqlx::query_scalar("SELECT count(*) FROM dci").fetch_one(&pool).await.unwrap();
        assert_eq!(dci, 2, "« Molécule-A fictive » une seule fois, quelle que soit l'écriture");
        let encore = importer(&pool, admin, "Liste fictive, édition de test", fichier.as_bytes()).await.unwrap();
        assert_eq!((encore.medicaments_ajoutes, encore.deja_presents), (0, 3));
        assert_eq!(total().await, 3);

        let trouves = rechercher(&pool, "MOLÉCULE-B", 50).await.unwrap();
        assert_eq!(trouves.len(), 1);
        assert_eq!(trouves[0].dci, ["Molécule-A fictive", "Molécule-B fictive"]);
        assert_eq!(rechercher(&pool, "générique", 50).await.unwrap().len(), 1);
        assert_eq!(rechercher(&pool, "%", 50).await.unwrap().len(), 0, "% cherché comme un caractère");

        for requete in ["UPDATE medicament SET dosage = 'x'", "DELETE FROM dci", "TRUNCATE medicament_dci"] {
            let erreur = sqlx::query(requete).execute(&pool).await.unwrap_err();
            assert!(erreur.to_string().contains("jamais modifiees"), "{requete} : {erreur}");
        }
    }
}
