//! Garanties apportées par la base elle-même dans la migration 0012
//! (structures, pièces, registre, décisions, affectations, invitations).
//! Aucun module Rust n'existe encore pour ces tables (étape 3.7) : les
//! lignes sont posées directement en SQL, et chaque test vérifie le nom de
//! la contrainte qui refuse, pour qu'un refus venu d'ailleurs ne fasse pas
//! passer le test pour une mauvaise raison.
//!
//! Base temporaire par test, jamais la base de développement : voir
//! « Tests et environnement », section 12 du CLAUDE.md. Lancement :
//! `cargo test --test migration_0012 -- --include-ignored`.

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

async fn creer_compte(pool: &PgPool, telephone: &str, type_compte: &str) -> Uuid {
    let utilisateur_id: Uuid =
        sqlx::query_scalar("INSERT INTO utilisateur (nom, prenom, telephone) VALUES ('Test', 'Migration', $1) RETURNING id")
            .bind(telephone)
            .fetch_one(pool)
            .await
            .unwrap();
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

async fn inserer_structure(pool: &PgPool, createur: Uuid, type_structure: &str) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO structure (type_structure, nom, telephone, commune, province, latitude, longitude, createur_compte_id) \
         VALUES ($1, 'Structure de test', '+22670000000', 'Ouagadougou', 'Kadiogo', 12.371400, -1.519700, $2) RETURNING id",
    )
    .bind(type_structure)
    .bind(createur)
    .fetch_one(pool)
    .await
}

async fn changer_statut(pool: &PgPool, structure_id: Uuid, statut: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE structure SET statut = $1 WHERE id = $2")
        .bind(statut)
        .bind(structure_id)
        .execute(pool)
        .await
        .map(|_| ())
}

#[sqlx::test]
#[ignore]
async fn deuxieme_munaseb_acceptee_en_brouillon_refusee_a_la_validation(pool: PgPool) {
    let chef_a = creer_compte(&pool, "+22670100001", "professionnel").await;
    let chef_b = creer_compte(&pool, "+22670100002", "professionnel").await;

    // Deux demandes MUNASEB en brouillon : aucun blocage (M6).
    let munaseb_a = inserer_structure(&pool, chef_a, "munaseb").await.unwrap();
    let munaseb_b = inserer_structure(&pool, chef_b, "munaseb").await.unwrap();
    changer_statut(&pool, munaseb_a, "en_attente").await.unwrap();
    changer_statut(&pool, munaseb_b, "en_attente").await.unwrap();

    changer_statut(&pool, munaseb_a, "validee").await.unwrap();
    for statut in ["validee", "suspendue"] {
        let erreur = changer_statut(&pool, munaseb_b, statut).await.unwrap_err();
        assert_eq!(contrainte(erreur), "structure_une_seule_munaseb_validee", "{statut}");
    }

    // Une MUNASEB suspendue garde la place.
    changer_statut(&pool, munaseb_a, "suspendue").await.unwrap();
    let erreur = changer_statut(&pool, munaseb_b, "validee").await.unwrap_err();
    assert_eq!(contrainte(erreur), "structure_une_seule_munaseb_validee");
}

#[sqlx::test]
#[ignore]
async fn une_seule_structure_non_validee_par_personne(pool: PgPool) {
    let createur = creer_compte(&pool, "+22670100003", "professionnel").await;
    let premiere = inserer_structure(&pool, createur, "clinique").await.unwrap();

    // Brouillon, en attente ou refusée : la place reste prise (M5).
    for statut in ["brouillon", "en_attente", "refusee"] {
        changer_statut(&pool, premiere, statut).await.unwrap();
        let erreur = inserer_structure(&pool, createur, "clinique").await.unwrap_err();
        assert_eq!(contrainte(erreur), "structure_une_non_validee_par_createur", "{statut}");
    }

    // Une structure validée ne compte plus : une nouvelle demande est permise.
    changer_statut(&pool, premiere, "validee").await.unwrap();
    inserer_structure(&pool, createur, "pharmacie").await.unwrap();
}

#[sqlx::test]
#[ignore]
async fn structure_abandonnee_libere_la_place(pool: PgPool) {
    let createur = creer_compte(&pool, "+22670100004", "professionnel").await;
    let abandonnee = inserer_structure(&pool, createur, "clinique").await.unwrap();
    sqlx::query(
        "INSERT INTO piece_justificative (structure_id, type_piece, numero, date_piece, type_mime, taille, sha256) \
         VALUES ($1, 'autorisation_creation', 'A-1', '2026-01-01', 'application/pdf', 10, decode(repeat('00', 32), 'hex'))",
    )
    .bind(abandonnee)
    .execute(&pool)
    .await
    .unwrap();

    changer_statut(&pool, abandonnee, "abandonnee").await.unwrap();
    inserer_structure(&pool, createur, "clinique").await.unwrap();

    // Rien n'est effacé : la structure abandonnée et sa pièce restent.
    let pieces: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM piece_justificative p JOIN structure s ON s.id = p.structure_id \
         WHERE s.id = $1 AND s.statut = 'abandonnee'",
    )
    .bind(abandonnee)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(pieces, 1);
}

#[sqlx::test]
#[ignore]
async fn un_numero_d_autorisation_n_est_valide_qu_une_fois(pool: PgPool) {
    let createur_a = creer_compte(&pool, "+22670100005", "professionnel").await;
    let createur_b = creer_compte(&pool, "+22670100006", "professionnel").await;
    let structure_a = inserer_structure(&pool, createur_a, "clinique").await.unwrap();
    let structure_b = inserer_structure(&pool, createur_b, "clinique").await.unwrap();

    let mut pieces = Vec::new();
    for (structure_id, numero) in [(structure_a, "n° 12 ab"), (structure_b, "12AB")] {
        let (id, normalise): (Uuid, String) = sqlx::query_as(
            "INSERT INTO piece_justificative (structure_id, type_piece, numero, date_piece, type_mime, taille, sha256) \
             VALUES ($1, 'autorisation_ouverture_exploitation', $2, '2026-01-01', 'image/png', 10, decode(repeat('00', 32), 'hex')) \
             RETURNING id, numero_normalise",
        )
        .bind(structure_id)
        .bind(numero)
        .fetch_one(&pool)
        .await
        .unwrap();
        // Majuscules, sans espaces, sans « N° » initial (M3).
        assert_eq!(normalise, "12AB", "{numero}");
        pieces.push((structure_id, id));
    }

    // Le numéro normalisé ne s'écrit jamais directement (colonne générée).
    assert!(
        sqlx::query("UPDATE piece_justificative SET numero_normalise = 'AUTRE'")
            .execute(&pool)
            .await
            .is_err()
    );

    let enregistrer = |(structure_id, piece_id): (Uuid, Uuid)| {
        sqlx::query(
            "INSERT INTO autorisation_validee (type_piece, numero_normalise, structure_id, piece_id) \
             SELECT type_piece, numero_normalise, $1, id FROM piece_justificative WHERE id = $2",
        )
        .bind(structure_id)
        .bind(piece_id)
        .execute(&pool)
    };
    enregistrer(pieces[0]).await.unwrap();
    let erreur = enregistrer(pieces[1]).await.unwrap_err();
    assert_eq!(contrainte(erreur), "autorisation_validee_pkey");
}

/// Règle de normalisation du 2026-10-05 (voir le commentaire de
/// `numero_normalise` dans la migration 0012). Les caractères non ASCII
/// sont écrits en échappements Rust, pour qu'aucun ne soit invisible.
#[sqlx::test]
#[ignore]
async fn normalisation_du_numero_d_autorisation(pool: PgPool) {
    let createur = creer_compte(&pool, "+22670100015", "professionnel").await;
    let structure_id = inserer_structure(&pool, createur, "clinique").await.unwrap();

    let inserer_piece = |numero: &'static str| {
        sqlx::query_scalar::<_, String>(
            "INSERT INTO piece_justificative (structure_id, type_piece, numero, date_piece, type_mime, taille, sha256, active) \
             VALUES ($1, 'autorisation_creation', $2, '2026-01-01', 'application/pdf', 10, decode(repeat('00', 32), 'hex'), false) \
             RETURNING numero_normalise",
        )
        .bind(structure_id)
        .bind(numero)
        .fetch_one(&pool)
    };
    // `active = false` : plusieurs pièces du même type dans la même
    // structure, sans heurter l'index d'une seule pièce active par type.

    for (saisie, attendu) in [
        ("2018-628/MS/CAB", "2018-628/MS/CAB"),                       // tiret simple gardé
        ("N\u{b0} 2018-628/MS/CAB", "2018-628/MS/CAB"),               // signe degré
        ("n\u{ba}2018-628/ms/cab", "2018-628/MS/CAB"),                // indicateur ordinal, minuscules
        ("No 2018\u{2010}628/MS/CAB", "2018-628/MS/CAB"),             // « No », trait d'union
        ("N. 2018\u{2013}628/MS/CAB", "2018-628/MS/CAB"),             // « N. », demi-cadratin
        ("n \u{b0} 2018\u{2212}628 / MS / CAB", "2018-628/MS/CAB"),   // espaces autour, signe moins
        ("N\u{b0}\u{a0}2018\u{2014}628/MS/CAB", "2018-628/MS/CAB"),   // espace insécable, cadratin
        ("no2018\u{10ead}628/ms/cab", "2018-628/MS/CAB"),             // « o » minuscule, tiret hors BMP
        ("2018-628/N\u{b0}/CAB", "2018-628/N\u{b0}/CAB"),             // « N° » au milieu : gardé
        ("2018-628/ms/caf\u{e9}", "2018-628/MS/CAF\u{e9}"),           // COLLATE "C" : « é » non ASCII inchangé
    ] {
        assert_eq!(inserer_piece(saisie).await.unwrap(), attendu, "saisie : {saisie:?}");
    }

    // N2 : un numéro réduit au seul préfixe est refusé.
    let erreur = inserer_piece("N\u{b0}").await.unwrap_err();
    assert_eq!(contrainte(erreur), "piece_justificative_numero_normalise_non_vide");
}

#[sqlx::test]
#[ignore]
async fn une_affectation_exige_un_compte_professionnel(pool: PgPool) {
    let responsable = creer_compte(&pool, "+22670100007", "professionnel").await;
    let structure_id = inserer_structure(&pool, responsable, "munaseb").await.unwrap();
    let compte_patient = creer_compte(&pool, "+22670100008", "patient").await;

    let erreur = sqlx::query("INSERT INTO affectation (compte_id, structure_id, role) VALUES ($1, $2, 'agent_assurance_munaseb')")
        .bind(compte_patient)
        .bind(structure_id)
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(contrainte(erreur), "affectation_compte_professionnel");

    // Déclarer le type « patient » ne contourne pas la règle.
    let erreur = sqlx::query(
        "INSERT INTO affectation (compte_id, compte_type, structure_id, role) VALUES ($1, 'patient', $2, 'agent_assurance_munaseb')",
    )
    .bind(compte_patient)
    .bind(structure_id)
    .execute(&pool)
    .await
    .unwrap_err();
    assert_eq!(contrainte(erreur), "affectation_compte_type_check");
}

#[sqlx::test]
#[ignore]
async fn une_decision_n_est_jamais_modifiee_ni_supprimee(pool: PgPool) {
    let responsable = creer_compte(&pool, "+22670100009", "professionnel").await;
    let administrateur = creer_compte(&pool, "+22670100010", "administrateur_laaficare").await;
    let structure_id = inserer_structure(&pool, responsable, "clinique").await.unwrap();

    // Un refus sans motif est refusé.
    let erreur = sqlx::query(
        "INSERT INTO decision_structure (structure_id, administrateur_compte_id, decision) VALUES ($1, $2, 'refusee')",
    )
    .bind(structure_id)
    .bind(administrateur)
    .execute(&pool)
    .await
    .unwrap_err();
    assert_eq!(contrainte(erreur), "decision_structure_motif");

    // Une décision ne peut venir que d'un compte administrateur.
    let erreur = sqlx::query(
        "INSERT INTO decision_structure (structure_id, administrateur_compte_id, decision) VALUES ($1, $2, 'validee')",
    )
    .bind(structure_id)
    .bind(responsable)
    .execute(&pool)
    .await
    .unwrap_err();
    assert_eq!(contrainte(erreur), "decision_structure_administrateur");

    sqlx::query(
        "INSERT INTO decision_structure (structure_id, administrateur_compte_id, decision, licence_expire_le) \
         VALUES ($1, $2, 'validee', '2027-10-04')",
    )
    .bind(structure_id)
    .bind(administrateur)
    .execute(&pool)
    .await
    .unwrap();

    for requete in [
        "UPDATE decision_structure SET decision = 'suspendue', motif = 'test'",
        "DELETE FROM decision_structure",
        "TRUNCATE decision_structure",
    ] {
        let erreur = sqlx::query(requete).execute(&pool).await.unwrap_err().to_string();
        assert!(erreur.contains("jamais modifiées ni supprimées"), "{requete} : {erreur}");
    }
}

#[sqlx::test]
#[ignore]
async fn une_invitation_sans_cible_est_refusee(pool: PgPool) {
    let invitant = creer_compte(&pool, "+22670100011", "professionnel").await;
    let erreur = sqlx::query(
        "INSERT INTO invitation (telephone, invite_par_compte_id, expire_le) VALUES ('+22670100012', $1, now() + interval '7 days')",
    )
    .bind(invitant)
    .execute(&pool)
    .await
    .unwrap_err();
    assert_eq!(contrainte(erreur), "invitation_cible");
}

#[sqlx::test]
#[ignore]
async fn une_invitation_acceptee_reference_son_affectation(pool: PgPool) {
    let responsable = creer_compte(&pool, "+22670100013", "professionnel").await;
    let invite = creer_compte(&pool, "+22670100014", "professionnel").await;
    let structure_id = inserer_structure(&pool, responsable, "munaseb").await.unwrap();
    changer_statut(&pool, structure_id, "validee").await.unwrap();

    let inviter = || {
        sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO invitation (structure_id, role, telephone, invite_par_compte_id, expire_le) \
             VALUES ($1, 'agent_assurance_munaseb', '+22670100014', $2, now() + interval '7 days') RETURNING id",
        )
        .bind(structure_id)
        .bind(responsable)
        .fetch_one(&pool)
    };
    let invitation_id = inviter().await.unwrap();

    let accepter = |invitation_id: Uuid, affectation_id: Option<Uuid>| {
        sqlx::query("UPDATE invitation SET statut = 'acceptee', date_reponse = now(), affectation_id = $2 WHERE id = $1")
            .bind(invitation_id)
            .bind(affectation_id)
            .execute(&pool)
    };

    // Une acceptation sans affectation est refusée.
    let erreur = accepter(invitation_id, None).await.unwrap_err();
    assert_eq!(contrainte(erreur), "invitation_affectation_si_acceptee");

    let affecter = |role: &'static str| {
        sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO affectation (compte_id, structure_id, role) VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(invite)
        .bind(structure_id)
        .bind(role)
        .fetch_one(&pool)
    };

    // Une affectation d'un autre rôle ne peut pas être liée.
    let mauvais_role = affecter("responsable").await.unwrap();
    let erreur = accepter(invitation_id, Some(mauvais_role)).await.unwrap_err();
    assert_eq!(contrainte(erreur), "invitation_affectation");

    let affectation_id = affecter("agent_assurance_munaseb").await.unwrap();
    accepter(invitation_id, Some(affectation_id)).await.unwrap();

    // Qui a fait entrer ce professionnel, et quand.
    let (invite_par, date_reponse): (Uuid, Option<chrono::DateTime<chrono::Utc>>) =
        sqlx::query_as("SELECT invite_par_compte_id, date_reponse FROM invitation WHERE affectation_id = $1")
            .bind(affectation_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(invite_par, responsable);
    assert!(date_reponse.is_some());

    // Une affectation naît d'une seule invitation.
    let autre_invitation = inviter().await.unwrap();
    let erreur = accepter(autre_invitation, Some(affectation_id)).await.unwrap_err();
    assert_eq!(contrainte(erreur), "invitation_affectation_id_key");
}

/// Q1 (2026-10-07) : une affectation ne se supprime jamais, ni directement
/// ni par ricochet, et ne se modifie qu'une fois, pour sa clôture.
#[sqlx::test]
#[ignore]
async fn une_affectation_ne_se_supprime_ni_ne_se_reactive(pool: PgPool) {
    let responsable = creer_compte(&pool, "+22670100016", "professionnel").await;
    let professionnel = creer_compte(&pool, "+22670100017", "professionnel").await;
    let structure_id = inserer_structure(&pool, responsable, "munaseb").await.unwrap();
    // Une seconde structure, cible de la tentative de changement de structure.
    inserer_structure(&pool, professionnel, "clinique").await.unwrap();

    let affecter = || {
        sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO affectation (compte_id, structure_id, role) VALUES ($1, $2, 'agent_assurance_munaseb') RETURNING id",
        )
        .bind(professionnel)
        .bind(structure_id)
        .fetch_one(&pool)
    };
    let affectation_id = affecter().await.unwrap();
    let modifier = |requete: &'static str| sqlx::query(requete).bind(affectation_id).execute(&pool);
    let message = |erreur: sqlx::Error| erreur.to_string();

    // 1. Suppression directe.
    let erreur = modifier("DELETE FROM affectation WHERE id = $1").await.unwrap_err();
    assert!(message(erreur).contains("jamais supprimée"));

    // 2. Troncature : sans CASCADE, PostgreSQL refuse déjà (clé étrangère
    //    depuis invitation) ; avec CASCADE, le trigger refuse.
    assert!(sqlx::query("TRUNCATE affectation").execute(&pool).await.is_err());
    let erreur = sqlx::query("TRUNCATE affectation CASCADE").execute(&pool).await.unwrap_err();
    assert!(message(erreur).contains("jamais supprimée"));

    // 3. Sur une affectation active, aucune autre colonne ne change.
    for requete in [
        "UPDATE affectation SET role = 'responsable' WHERE id = $1",
        "UPDATE affectation SET date_creation = date_creation - interval '1 day' WHERE id = $1",
        "UPDATE affectation SET structure_id = (SELECT id FROM structure WHERE type_structure = 'clinique') WHERE id = $1",
        "UPDATE affectation SET compte_id = (SELECT id FROM compte WHERE id <> affectation.compte_id AND type_compte = 'professionnel' LIMIT 1) WHERE id = $1",
    ] {
        let erreur = modifier(requete).await.unwrap_err();
        assert!(message(erreur).contains("seule la clôture"), "{requete}");
    }

    // 5. Clôture sans auteur : le trigger la laisse passer, un CHECK refuse.
    //    Elle viole les deux CHECK de la clôture à la fois ; PostgreSQL ne
    //    signale que l'un d'eux.
    let erreur = modifier("UPDATE affectation SET statut = 'desactivee', desactivee_le = now() WHERE id = $1")
        .await
        .unwrap_err();
    let refusee_par = contrainte(erreur);
    assert!(
        ["affectation_desactivation_tracee", "affectation_desactivation_complete"].contains(&refusee_par.as_str()),
        "{refusee_par}"
    );

    // 4. Clôture complète : acceptée.
    sqlx::query(
        "UPDATE affectation SET statut = 'desactivee', desactivee_le = now(), desactivee_par_compte_id = $2 WHERE id = $1",
    )
    .bind(affectation_id)
    .bind(responsable)
    .execute(&pool)
    .await
    .unwrap();

    // 6 et 7. Ni réactivation, ni correction de la clôture.
    for requete in [
        "UPDATE affectation SET statut = 'active', desactivee_le = NULL, desactivee_par_compte_id = NULL WHERE id = $1",
        "UPDATE affectation SET desactivee_le = desactivee_le - interval '1 day' WHERE id = $1",
    ] {
        let erreur = modifier(requete).await.unwrap_err();
        assert!(message(erreur).contains("seule la clôture"), "{requete}");
    }

    // 8. La personne revient : une nouvelle affectation, même rôle.
    affecter().await.unwrap();
    let periodes: i64 = sqlx::query_scalar("SELECT count(*) FROM affectation WHERE compte_id = $1")
        .bind(professionnel)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(periodes, 2);

    // 9. Suppressions par ricochet : refusées par les clés étrangères
    //    (NO ACTION), ou par les triggers pour TRUNCATE ... CASCADE.
    let erreur = sqlx::query("DELETE FROM structure WHERE id = $1")
        .bind(structure_id)
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(contrainte(erreur), "affectation_structure_id_fkey");

    // Un compte que seule une affectation retient (les deux autres ont créé
    // une structure, qui les retient aussi).
    let invite = creer_compte(&pool, "+22670100018", "professionnel").await;
    sqlx::query("INSERT INTO affectation (compte_id, structure_id, role) VALUES ($1, $2, 'responsable')")
        .bind(invite)
        .bind(structure_id)
        .execute(&pool)
        .await
        .unwrap();
    let erreur = sqlx::query("DELETE FROM compte WHERE id = $1")
        .bind(invite)
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(contrainte(erreur), "affectation_compte_professionnel");

    let erreur = sqlx::query("TRUNCATE structure CASCADE").execute(&pool).await.unwrap_err();
    assert!(message(erreur).contains("jamais"), "TRUNCATE structure CASCADE");
    let restantes: i64 = sqlx::query_scalar("SELECT count(*) FROM affectation")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(restantes, 3);
}
