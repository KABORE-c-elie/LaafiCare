//! Garanties apportées par la base elle-même dans la migration 0013
//! (second facteur TOTP, codes de secours, désactivations, profil
//! administrateur). Aucun module Rust n'existe encore pour ces tables
//! (`totp.rs` vient au fichier suivant) : les lignes sont posées
//! directement en SQL, et chaque test vérifie le nom de la contrainte qui
//! refuse, pour qu'un refus venu d'ailleurs ne fasse pas passer le test
//! pour une mauvaise raison.
//!
//! Base temporaire par test, jamais la base de développement : voir
//! « Tests et environnement », section 12 du CLAUDE.md. Lancement :
//! `cargo test --test migration_0013 -- --include-ignored`.

use sqlx::PgPool;
use sqlx::types::Uuid;

/// Nom de la contrainte (ou de l'index unique) qui a refusé l'écriture.
fn contrainte(erreur: sqlx::Error) -> String {
    erreur
        .as_database_error()
        .and_then(|e| e.constraint())
        .unwrap_or_else(|| panic!("pas une violation de contrainte : {erreur}"))
        .to_string()
}

async fn creer_utilisateur(pool: &PgPool, telephone: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO utilisateur (nom, prenom, telephone) VALUES ('Test', 'Migration', $1) RETURNING id")
        .bind(telephone)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Un compte rattaché à une identité existante : une même personne peut
/// avoir un compte patient et un compte administrateur (section 14).
async fn creer_compte_pour(pool: &PgPool, utilisateur_id: Uuid, type_compte: &str) -> Uuid {
    // Le hachage n'est jamais vérifié ici : une valeur quelconque suffit.
    sqlx::query_scalar(
        "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, $2, 'hachage-de-test') RETURNING id",
    )
    .bind(utilisateur_id)
    .bind(type_compte)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn creer_compte(pool: &PgPool, telephone: &str, type_compte: &str) -> Uuid {
    let utilisateur_id = creer_utilisateur(pool, telephone).await;
    creer_compte_pour(pool, utilisateur_id, type_compte).await
}

/// TOTP bien formé (36 octets chiffrés, nonce de 12 octets), daté s'il est
/// actif. Le contenu n'est jamais déchiffré ici : des zéros suffisent.
async fn inserer_totp(pool: &PgPool, compte_id: Uuid, statut: &str) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO second_facteur_totp (compte_id, secret_chiffre, nonce, version_cle, statut, date_activation) \
         VALUES ($1, decode(repeat('00', 36), 'hex'), decode(repeat('00', 12), 'hex'), 1, $2, \
                 CASE WHEN $2 = 'actif' THEN now() END) \
         RETURNING id",
    )
    .bind(compte_id)
    .bind(statut)
    .fetch_one(pool)
    .await
}

async fn desactiver(
    pool: &PgPool,
    compte_id: Uuid,
    administrateur_compte_id: Uuid,
    motif: &str,
    cnib_verifiee: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO desactivation_totp (compte_id, administrateur_compte_id, motif, cnib_verifiee) VALUES ($1, $2, $3, $4)",
    )
    .bind(compte_id)
    .bind(administrateur_compte_id)
    .bind(motif)
    .bind(cnib_verifiee)
    .execute(pool)
    .await
    .map(|_| ())
}

async fn inserer_profil(pool: &PgPool, compte_id: Uuid, numero_cnib: &str) -> Result<String, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO profil_administrateur (compte_id, numero_cnib) VALUES ($1, $2) RETURNING numero_cnib_normalise",
    )
    .bind(compte_id)
    .bind(numero_cnib)
    .fetch_one(pool)
    .await
}

#[sqlx::test]
#[ignore]
async fn un_seul_totp_actif_et_un_seul_en_attente_par_compte(pool: PgPool) {
    let compte = creer_compte(&pool, "+22670200001", "administrateur_laaficare").await;

    // Pendant un remplacement : l'ancien actif et le nouveau en attente (Z7).
    inserer_totp(&pool, compte, "actif").await.unwrap();
    inserer_totp(&pool, compte, "en_attente").await.unwrap();

    let erreur = inserer_totp(&pool, compte, "actif").await.unwrap_err();
    assert_eq!(contrainte(erreur), "second_facteur_totp_un_actif");
    let erreur = inserer_totp(&pool, compte, "en_attente").await.unwrap_err();
    assert_eq!(contrainte(erreur), "second_facteur_totp_un_en_attente");

    // La limite est par compte : un autre compte n'est pas gêné.
    let autre = creer_compte(&pool, "+22670200002", "patient").await;
    inserer_totp(&pool, autre, "actif").await.unwrap();
}

#[sqlx::test]
#[ignore]
async fn un_totp_mal_forme_est_refuse(pool: PgPool) {
    let compte = creer_compte(&pool, "+22670200003", "patient").await;

    // (taille du secret chiffré, taille du nonce, statut, date d'activation,
    // dernier pas, contrainte attendue)
    for (secret, nonce, statut, date_activation, dernier_pas, attendue) in [
        (35, 12, "en_attente", false, None, "second_facteur_totp_secret_chiffre_check"),
        (37, 12, "en_attente", false, None, "second_facteur_totp_secret_chiffre_check"),
        (36, 11, "en_attente", false, None, "second_facteur_totp_nonce_check"),
        (36, 16, "en_attente", false, None, "second_facteur_totp_nonce_check"),
        (36, 12, "actif", false, None, "second_facteur_totp_activation_datee"),
        (36, 12, "en_attente", true, None, "second_facteur_totp_activation_datee"),
        (36, 12, "en_attente", false, Some(1_i64), "second_facteur_totp_pas_si_actif"),
    ] {
        let erreur = sqlx::query(
            "INSERT INTO second_facteur_totp (compte_id, secret_chiffre, nonce, version_cle, statut, date_activation, dernier_pas) \
             VALUES ($1, decode(repeat('00', $2), 'hex'), decode(repeat('00', $3), 'hex'), 1, $4, \
                     CASE WHEN $5 THEN now() END, $6)",
        )
        .bind(compte)
        .bind(secret)
        .bind(nonce)
        .bind(statut)
        .bind(date_activation)
        .bind(dernier_pas)
        .execute(&pool)
        .await
        .unwrap_err();
        assert_eq!(contrainte(erreur), attendue, "secret {secret}, nonce {nonce}, {statut}");
    }
}

#[sqlx::test]
#[ignore]
async fn supprimer_un_totp_supprime_ses_codes_de_secours(pool: PgPool) {
    let compte = creer_compte(&pool, "+22670200004", "professionnel").await;
    let totp = inserer_totp(&pool, compte, "actif").await.unwrap();
    for _ in 0..2 {
        sqlx::query("INSERT INTO code_secours (second_facteur_id, hash) VALUES ($1, 'hachage-de-test')")
            .bind(totp)
            .execute(&pool)
            .await
            .unwrap();
    }

    // Y1 : le TOTP désactivé ou remplacé est supprimé, ses codes avec lui.
    sqlx::query("DELETE FROM second_facteur_totp WHERE id = $1")
        .bind(totp)
        .execute(&pool)
        .await
        .unwrap();
    let restants: i64 = sqlx::query_scalar("SELECT count(*) FROM code_secours")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(restants, 0);
}

#[sqlx::test]
#[ignore]
async fn une_desactivation_exige_motif_cnib_et_administrateur(pool: PgPool) {
    let patient = creer_compte(&pool, "+22670200005", "patient").await;
    let administrateur = creer_compte(&pool, "+22670200006", "administrateur_laaficare").await;
    let professionnel = creer_compte(&pool, "+22670200007", "professionnel").await;

    for motif in ["", "   "] {
        let erreur = desactiver(&pool, patient, administrateur, motif, true).await.unwrap_err();
        assert_eq!(contrainte(erreur), "desactivation_totp_motif_check", "motif {motif:?}");
    }

    // Y2 : l'attestation de vérification de la CNIB est obligatoire.
    let erreur = desactiver(&pool, patient, administrateur, "Téléphone perdu", false).await.unwrap_err();
    assert_eq!(contrainte(erreur), "desactivation_totp_cnib_verifiee_check");

    // Seul un compte administrateur LaafiCare peut désactiver.
    let erreur = desactiver(&pool, patient, professionnel, "Téléphone perdu", true).await.unwrap_err();
    assert_eq!(contrainte(erreur), "desactivation_totp_administrateur");

    // Déclarer un autre type de compte ne contourne pas la règle.
    let erreur = sqlx::query(
        "INSERT INTO desactivation_totp (compte_id, administrateur_compte_id, administrateur_type_compte, motif, cnib_verifiee) \
         VALUES ($1, $2, 'professionnel', 'Téléphone perdu', true)",
    )
    .bind(patient)
    .bind(professionnel)
    .execute(&pool)
    .await
    .unwrap_err();
    assert_eq!(contrainte(erreur), "desactivation_totp_administrateur_type_compte_check");

    desactiver(&pool, patient, administrateur, "Téléphone perdu", true).await.unwrap();
}

#[sqlx::test]
#[ignore]
async fn un_administrateur_ne_desactive_aucun_de_ses_propres_comptes(pool: PgPool) {
    // Une même personne : compte patient, professionnel et administrateur.
    let personne = creer_utilisateur(&pool, "+22670200008").await;
    let son_patient = creer_compte_pour(&pool, personne, "patient").await;
    let son_professionnel = creer_compte_pour(&pool, personne, "professionnel").await;
    let son_administrateur = creer_compte_pour(&pool, personne, "administrateur_laaficare").await;

    // Y3 et Y3-bis : refusé par le trigger, quel que soit le compte visé.
    for compte in [son_administrateur, son_patient, son_professionnel] {
        let erreur = desactiver(&pool, compte, son_administrateur, "Téléphone perdu", true)
            .await
            .unwrap_err()
            .to_string();
        assert!(erreur.contains("ses propres comptes"), "{erreur}");
    }

    // Un autre membre de l'équipe le peut.
    let collegue = creer_compte(&pool, "+22670200009", "administrateur_laaficare").await;
    for compte in [son_administrateur, son_patient, son_professionnel] {
        desactiver(&pool, compte, collegue, "Téléphone perdu", true).await.unwrap();
    }
}

#[sqlx::test]
#[ignore]
async fn une_desactivation_n_est_jamais_modifiee_ni_supprimee(pool: PgPool) {
    let patient = creer_compte(&pool, "+22670200010", "patient").await;
    let administrateur = creer_compte(&pool, "+22670200011", "administrateur_laaficare").await;
    desactiver(&pool, patient, administrateur, "Téléphone perdu", true).await.unwrap();

    for requete in [
        "UPDATE desactivation_totp SET motif = 'autre motif'",
        "DELETE FROM desactivation_totp",
        "TRUNCATE desactivation_totp",
    ] {
        let erreur = sqlx::query(requete).execute(&pool).await.unwrap_err().to_string();
        assert!(erreur.contains("jamais modifiées ni supprimées"), "{requete} : {erreur}");
    }
}

#[sqlx::test]
#[ignore]
async fn un_profil_exige_un_compte_administrateur(pool: PgPool) {
    let patient = creer_compte(&pool, "+22670200012", "patient").await;

    let erreur = inserer_profil(&pool, patient, "B12345678").await.unwrap_err();
    assert_eq!(contrainte(erreur), "profil_administrateur_compte");

    // Déclarer le type « patient » ne contourne pas la règle.
    let erreur = sqlx::query("INSERT INTO profil_administrateur (compte_id, compte_type, numero_cnib) VALUES ($1, 'patient', 'B12345678')")
        .bind(patient)
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(contrainte(erreur), "profil_administrateur_compte_type_check");
}

/// Format de la CNIB actuelle, fourni par le porteur : B suivi de 8
/// chiffres, contrôlé après normalisation. Les caractères non ASCII sont
/// écrits en échappements Rust, pour qu'aucun ne soit invisible.
#[sqlx::test]
#[ignore]
async fn numero_de_cnib_normalise_au_format_et_unique(pool: PgPool) {
    let administrateur = creer_compte(&pool, "+22670200013", "administrateur_laaficare").await;

    // Toutes ces saisies donnent le même numéro : chaque profil est
    // supprimé après lecture, l'unicité est vérifiée plus bas.
    for (saisie, attendu) in [
        ("b 1234 5678", "B12345678"),
        ("B12345678", "B12345678"),
        ("\u{a0}b1234\u{2009}5678\u{3000}", "B12345678"), // insécable, fine, idéographique
        ("b\t1234\u{2028}5678", "B12345678"),              // tabulation, séparateur de ligne
    ] {
        assert_eq!(inserer_profil(&pool, administrateur, saisie).await.unwrap(), attendu, "saisie : {saisie:?}");
        sqlx::query("DELETE FROM profil_administrateur").execute(&pool).await.unwrap();
    }

    // Refusés : 7 chiffres, autre lettre, 9 chiffres, vide, espaces seules,
    // chiffres pleine largeur (seuls 0 à 9 ASCII sont des chiffres ici).
    for saisie in ["B1234567", "A12345678", "B123456789", "", "   ", "\u{3000}", "B\u{ff11}2345678"] {
        let erreur = inserer_profil(&pool, administrateur, saisie).await.unwrap_err();
        assert_eq!(contrainte(erreur), "profil_administrateur_cnib_format", "saisie : {saisie:?}");
    }

    // K3 : deux administrateurs ne peuvent pas avoir le même numéro normalisé.
    inserer_profil(&pool, administrateur, "b 1234 5678").await.unwrap();
    let collegue = creer_compte(&pool, "+22670200014", "administrateur_laaficare").await;
    let erreur = inserer_profil(&pool, collegue, "B12345678").await.unwrap_err();
    assert_eq!(contrainte(erreur), "profil_administrateur_cnib_unique");

    // Le numéro normalisé ne s'écrit jamais directement (colonne générée).
    assert!(
        sqlx::query("UPDATE profil_administrateur SET numero_cnib_normalise = 'B00000000'")
            .execute(&pool)
            .await
            .is_err()
    );
}
