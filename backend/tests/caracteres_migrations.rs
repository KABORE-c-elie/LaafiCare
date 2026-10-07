//! Aucun espace ni tiret trompeur dans les migrations SQL (règle de la
//! section 13 du CLAUDE.md, 2026-10-06). Un outil d'édition a déjà
//! remplacé des échappements `\uXXXX` par les caractères eux-mêmes
//! (migrations 0012 et 0013) : invisibles à la relecture, un éditeur
//! pourrait les modifier sans que personne le voie, et la normalisation
//! des numéros cesserait de fonctionner.
//!
//! Sans base de données : `cargo test --test caracteres_migrations`.

use std::path::Path;

use laaficare_backend::telephone::est_tiret;

/// Caractère refusé : (numéro de ligne, à partir de 1 ; caractère).
fn caracteres_interdits(texte: &str) -> Vec<(usize, char)> {
    let mut trouves = Vec::new();
    for (indice, ligne) in texte.split('\n').enumerate() {
        // Aucun `\r`, même en fin de ligne : SQLx garde l'empreinte de chaque
        // migration, fins de ligne comprises, et une migration en CRLF ne
        // correspondrait plus à celle appliquée en production. Le
        // `.gitattributes` impose LF à l'extraction ; ce test vérifie le
        // résultat.
        for c in ligne.chars() {
            // `is_whitespace` = propriété Unicode White_Space (doc de
            // `char::is_whitespace`) ; `est_tiret` = propriété Unicode Dash.
            let espace_interdit = c.is_whitespace() && c != ' ' && c != '\t';
            let tiret_interdit = est_tiret(c) && c != '-';
            if espace_interdit || tiret_interdit {
                trouves.push((indice + 1, c));
            }
        }
    }
    trouves
}

#[test]
fn aucune_migration_ne_contient_d_espace_ni_de_tiret_trompeur() {
    let dossier = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut fichiers: Vec<_> = std::fs::read_dir(&dossier)
        .unwrap()
        .map(|entree| entree.unwrap().path())
        .filter(|chemin| chemin.extension().is_some_and(|e| e == "sql"))
        .collect();
    fichiers.sort();
    assert!(!fichiers.is_empty(), "aucune migration trouvée dans {}", dossier.display());

    let mut erreurs = Vec::new();
    for chemin in &fichiers {
        let texte = std::fs::read_to_string(chemin).unwrap();
        for (ligne, c) in caracteres_interdits(&texte) {
            erreurs.push(format!(
                "{}, ligne {ligne} : U+{:04X} -- écrire \\U{:08X}",
                chemin.file_name().unwrap().to_string_lossy(),
                c as u32,
                c as u32
            ));
        }
    }
    assert!(erreurs.is_empty(), "caractères interdits :\n{}", erreurs.join("\n"));
}

/// Le détecteur lui-même : sans ce test, une erreur dans
/// `caracteres_interdits` ferait passer le test précédent sans rien
/// vérifier. Les caractères sont écrits en échappements Rust.
#[test]
fn le_detecteur_trouve_les_caracteres_interdits() {
    // Permis : espace, tabulation, fin de ligne LF, tiret simple, et les
    // caractères visibles des commentaires (accents, guillemets, °).
    assert!(caracteres_interdits("a b\tc-d\n« é ° º »\n").is_empty());

    // Refusé : tout `\r`, en fin de ligne (CRLF) comme ailleurs.
    assert_eq!(
        caracteres_interdits("ligne 1\r\nx\u{a0}y\u{2011}z\n\u{3000}\nfin\r\u{2212}"),
        vec![(1, '\r'), (2, '\u{a0}'), (2, '\u{2011}'), (3, '\u{3000}'), (4, '\r'), (4, '\u{2212}')]
    );
}
