//! Vérification d'un mot de passe avec verrouillage 5 tentatives / 15 min
//! (BF-01) : la seule implémentation de cette règle, pour tous les types de
//! compte (décision K1, section 14 du CLAUDE.md). L'appelant retrouve le
//! compte avec ses propres critères (téléphone pour un patient, email pour
//! un professionnel) et passe `None` si rien ne correspond.
//!
//! Aucune réponse ne révèle si un compte existe (section 11, décisions P1 et
//! L1) : compte inconnu, verrouillé, désactivé ou mauvais mot de passe
//! donnent tous `IdentifiantsInvalides`, et un hachage Argon2id est toujours
//! vérifié, pour que le temps de réponse soit le même dans tous les cas.
//!
//! Second facteur (décision X1, 2026-10-06) : chaque essai de code TOTP ou
//! de code de secours passe par le même compteur (`compter_tentative`).
//! Quand un TOTP est actif, le bon mot de passe ne remet PAS le compteur à
//! zéro : seul le bon code le fait. Sinon, quelqu'un qui connaît le mot de
//! passe pourrait alterner sans fin « bon mot de passe, 4 codes au
//! hasard ». Conséquence acceptée : 5 essais au total, mot de passe compris.

use std::sync::OnceLock;

use sqlx::PgPool;
use sqlx::types::Uuid;

use crate::mot_de_passe;

// BF-01 du CDC -- pas une valeur arbitraire de bibliothèque.
pub const MAX_TENTATIVES: i16 = 5;
pub const DUREE_VERROUILLAGE_MIN: i32 = 15;

#[derive(Debug, thiserror::Error)]
pub enum ErreurConnexion {
    /// Compte inconnu, verrouillé, désactivé, ou mauvais mot de passe :
    /// jamais distingués (décisions P1 et L1).
    #[error("identifiants invalides")]
    IdentifiantsInvalides,

    #[error("erreur interne : {0}")]
    Interne(String),
}

impl From<sqlx::Error> for ErreurConnexion {
    fn from(erreur: sqlx::Error) -> Self {
        ErreurConnexion::Interne(erreur.to_string())
    }
}

/// Faux hachage, vérifié quand aucun mot de passe réel ne peut l'être
/// (compte inconnu ou verrouillé), pour égaliser le temps de réponse
/// (décision K4). Calculé une seule fois, avec `mot_de_passe::hacher` : il
/// a donc toujours les paramètres Argon2id des vrais hachages, jamais une
/// valeur recopiée en dur qui divergerait le jour où ces paramètres
/// changent.
static FAUX_HACHAGE: OnceLock<String> = OnceLock::new();

fn verifier_faux_hachage(mot_de_passe_saisi: &str) -> Result<(), ErreurConnexion> {
    let faux = match FAUX_HACHAGE.get() {
        Some(faux) => faux,
        None => {
            let calcule = mot_de_passe::hacher("faux-hachage-egalisation-du-temps")
                .map_err(|e| ErreurConnexion::Interne(e.to_string()))?;
            FAUX_HACHAGE.get_or_init(|| calcule)
        }
    };
    // Le résultat n'a pas d'importance : seul compte le temps passé.
    let _ = mot_de_passe::verifier(mot_de_passe_saisi, faux);
    Ok(())
}

/// Tentative comptée sur un compte non verrouillé.
pub(crate) struct TentativeComptee {
    pub mot_de_passe_hash: String,
    pub statut: String,
    pub totp_actif: bool,
}

/// Compte une tentative (mot de passe, code TOTP ou code de secours) et
/// pose le verrouillage au 5ᵉ échec. `None` : compte verrouillé ou
/// inexistant, la tentative ne doit pas être vérifiée.
///
/// La tentative est comptée AVANT toute vérification, en une seule requête,
/// et seulement si le compte n'est pas verrouillé (décision K2). PostgreSQL
/// applique les mises à jour d'une même ligne l'une après l'autre : au plus
/// MAX_TENTATIVES tentatives passent, même lancées en même temps. Vérifier
/// d'abord et compter ensuite laisserait dix tentatives simultanées tester
/// dix mots de passe. Au 5ᵉ échec, le compteur repart à 0 et le verrouillage
/// est posé : à son expiration, 5 nouveaux essais sont possibles.
///
/// `totp_actif` est lu dans la même requête (sous-requête du `RETURNING`) :
/// aucun écart possible entre le comptage et la lecture de l'état du TOTP.
pub(crate) async fn compter_tentative(pool: &PgPool, compte_id: Uuid) -> Result<Option<TentativeComptee>, sqlx::Error> {
    let ligne: Option<(String, String, bool)> = sqlx::query_as(
        "UPDATE compte SET \
           tentatives_echouees = CASE WHEN tentatives_echouees + 1 >= $2 THEN 0 ELSE tentatives_echouees + 1 END, \
           verrouille_jusqua = CASE WHEN tentatives_echouees + 1 >= $2 \
                                    THEN now() + make_interval(mins => $3) ELSE verrouille_jusqua END \
         WHERE id = $1 AND (verrouille_jusqua IS NULL OR verrouille_jusqua <= now()) \
         RETURNING mot_de_passe_hash, statut, \
                   EXISTS (SELECT 1 FROM second_facteur_totp t WHERE t.compte_id = compte.id AND t.statut = 'actif')",
    )
    .bind(compte_id)
    .bind(MAX_TENTATIVES)
    .bind(DUREE_VERROUILLAGE_MIN)
    .fetch_optional(pool)
    .await?;

    Ok(ligne.map(|(mot_de_passe_hash, statut, totp_actif)| TentativeComptee {
        mot_de_passe_hash,
        statut,
        totp_actif,
    }))
}

/// Authentification complète : compteur remis à zéro et verrouillage levé
/// -- y compris celui que la dernière tentative venait de poser, si c'était
/// la 5ᵉ et la bonne. Appelée après le mot de passe sans TOTP, ou après le
/// bon code quand un TOTP est actif (X1).
pub(crate) async fn remettre_a_zero(pool: &PgPool, compte_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE compte SET tentatives_echouees = 0, verrouille_jusqua = NULL WHERE id = $1")
        .bind(compte_id)
        .execute(pool)
        .await
        .map(|_| ())
}

/// Résultat d'un mot de passe correct. Le type oblige chaque appelant à
/// traiter le cas du second facteur (décision V1).
#[derive(Debug, PartialEq, Eq)]
pub enum Verification {
    /// Aucun TOTP actif : authentification complète, compteur remis à zéro.
    Complete(Uuid),
    /// TOTP actif : le code reste à vérifier, compteur NON remis à zéro.
    SecondFacteurRequis(Uuid),
}

/// Vérifie le mot de passe d'un compte et applique le verrouillage.
pub async fn verifier(
    pool: &PgPool,
    compte_id: Option<Uuid>,
    mot_de_passe_saisi: &str,
) -> Result<Verification, ErreurConnexion> {
    let Some(compte_id) = compte_id else {
        verifier_faux_hachage(mot_de_passe_saisi)?;
        return Err(ErreurConnexion::IdentifiantsInvalides);
    };

    // Aucune ligne : compte verrouillé (ou disparu). Même réponse et même
    // temps qu'un compte inconnu (décision L1).
    let Some(tentative) = compter_tentative(pool, compte_id).await? else {
        verifier_faux_hachage(mot_de_passe_saisi)?;
        return Err(ErreurConnexion::IdentifiantsInvalides);
    };

    let correct = mot_de_passe::verifier(mot_de_passe_saisi, &tentative.mot_de_passe_hash)
        .map_err(|e| ErreurConnexion::Interne(e.to_string()))?;

    // Compte désactivé : le mot de passe a été vérifié quand même, pour que
    // le temps de réponse ne le distingue pas d'un mauvais mot de passe.
    if !correct || tentative.statut != "actif" {
        return Err(ErreurConnexion::IdentifiantsInvalides);
    }

    // V1 : la décision est prise ici, aucun appelant ne peut l'oublier.
    if tentative.totp_actif {
        return Ok(Verification::SecondFacteurRequis(compte_id));
    }

    remettre_a_zero(pool, compte_id).await?;
    Ok(Verification::Complete(compte_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BON: &str = "Bon-mot-de-passe-1";
    const MAUVAIS: &str = "Mauvais-mot-de-passe-2";

    async fn compte_patient(pool: &PgPool) -> Uuid {
        let utilisateur_id: Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (nom, prenom, telephone) VALUES ('Test', 'Verrouillage', '+22670015001') RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query_scalar(
            "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'patient', $2) RETURNING id",
        )
        .bind(utilisateur_id)
        .bind(mot_de_passe::hacher(BON).unwrap())
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn etat(pool: &PgPool, compte_id: Uuid) -> (i16, bool) {
        sqlx::query_as(
            "SELECT tentatives_echouees, verrouille_jusqua IS NOT NULL AND verrouille_jusqua > now() \
             FROM compte WHERE id = $1",
        )
        .bind(compte_id)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    fn refuse(resultat: Result<Verification, ErreurConnexion>) -> bool {
        matches!(resultat, Err(ErreurConnexion::IdentifiantsInvalides))
    }

    // Base temporaire par test, jamais la base de développement : voir
    // « Tests et environnement », section 12 du CLAUDE.md.
    #[sqlx::test]
    #[ignore]
    async fn verrouillage_en_base(pool: PgPool) {
        let compte = compte_patient(&pool).await;

        // --- aucun compte ---
        assert!(refuse(verifier(&pool, None, BON).await));

        // --- 4 échecs puis le bon : accepté, compteur remis à zéro ---
        for _ in 0..4 {
            assert!(refuse(verifier(&pool, Some(compte), MAUVAIS).await));
        }
        assert_eq!(etat(&pool, compte).await, (4, false));
        assert_eq!(verifier(&pool, Some(compte), BON).await.unwrap(), Verification::Complete(compte));
        assert_eq!(etat(&pool, compte).await, (0, false));

        // --- 5 échecs : verrouillé, le bon mot de passe est refusé ---
        for _ in 0..5 {
            assert!(refuse(verifier(&pool, Some(compte), MAUVAIS).await));
        }
        assert_eq!(etat(&pool, compte).await, (0, true));
        assert!(refuse(verifier(&pool, Some(compte), BON).await), "refusé pendant le verrouillage");

        // --- verrouillage expiré : de nouveau accepté ---
        sqlx::query("UPDATE compte SET verrouille_jusqua = now() - interval '1 minute' WHERE id = $1")
            .bind(compte)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(verifier(&pool, Some(compte), BON).await.unwrap(), Verification::Complete(compte));

        // --- 5ᵉ tentative correcte : elle réussit et lève le verrouillage
        // qu'elle venait de poser ---
        for _ in 0..4 {
            assert!(refuse(verifier(&pool, Some(compte), MAUVAIS).await));
        }
        assert_eq!(verifier(&pool, Some(compte), BON).await.unwrap(), Verification::Complete(compte));
        assert_eq!(etat(&pool, compte).await, (0, false));

        // --- compte désactivé : refusé, même avec le bon mot de passe ---
        sqlx::query("UPDATE compte SET statut = 'desactive' WHERE id = $1")
            .bind(compte)
            .execute(&pool)
            .await
            .unwrap();
        assert!(refuse(verifier(&pool, Some(compte), BON).await));
    }

    /// Dix tentatives simultanées avec un mauvais mot de passe : toutes
    /// refusées, et le compte est verrouillé ensuite. Toutes les réponses
    /// étant identiques (décision L1), le test ne peut pas compter combien
    /// de mots de passe ont réellement été vérifiés : la limite de 5 vient
    /// de la requête unique avec sa condition `WHERE`, que PostgreSQL
    /// applique ligne par ligne.
    #[sqlx::test]
    #[ignore]
    async fn tentatives_simultanees_en_base(pool: PgPool) {
        let compte = compte_patient(&pool).await;

        let mut taches = tokio::task::JoinSet::new();
        for _ in 0..10 {
            let pool = pool.clone();
            taches.spawn(async move { verifier(&pool, Some(compte), MAUVAIS).await });
        }
        while let Some(resultat) = taches.join_next().await {
            assert!(refuse(resultat.unwrap()));
        }

        assert_eq!(etat(&pool, compte).await, (0, true));
        assert!(refuse(verifier(&pool, Some(compte), BON).await));
    }

    /// TOTP actif, posé directement en SQL (le module `totp` n'existe pas
    /// encore) : le contenu n'est jamais déchiffré ici, des zéros suffisent.
    async fn activer_totp(pool: &PgPool, compte_id: Uuid) {
        sqlx::query(
            "INSERT INTO second_facteur_totp (compte_id, secret_chiffre, nonce, version_cle, statut, date_activation) \
             VALUES ($1, decode(repeat('00', 36), 'hex'), decode(repeat('00', 12), 'hex'), 1, 'actif', now())",
        )
        .bind(compte_id)
        .execute(pool)
        .await
        .unwrap();
    }

    /// X1 : avec un TOTP actif, le bon mot de passe ne suffit pas et ne
    /// remet pas le compteur à zéro.
    #[sqlx::test]
    #[ignore]
    async fn totp_actif_le_mot_de_passe_ne_remet_pas_a_zero(pool: PgPool) {
        let compte = compte_patient(&pool).await;
        activer_totp(&pool, compte).await;

        assert_eq!(
            verifier(&pool, Some(compte), BON).await.unwrap(),
            Verification::SecondFacteurRequis(compte)
        );
        assert_eq!(etat(&pool, compte).await, (1, false));

        // Un TOTP seulement en attente ne compte pas : connexion complète.
        sqlx::query("DELETE FROM second_facteur_totp WHERE compte_id = $1")
            .bind(compte)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO second_facteur_totp (compte_id, secret_chiffre, nonce, version_cle) \
             VALUES ($1, decode(repeat('00', 36), 'hex'), decode(repeat('00', 12), 'hex'), 1)",
        )
        .bind(compte)
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(verifier(&pool, Some(compte), BON).await.unwrap(), Verification::Complete(compte));
        assert_eq!(etat(&pool, compte).await, (0, false));
    }

    /// X1, attaque par alternance : avec un TOTP actif, 5 connexions avec le
    /// BON mot de passe et sans code verrouillent le compte (5 essais au
    /// total, mot de passe compris).
    #[sqlx::test]
    #[ignore]
    async fn totp_actif_cinq_bons_mots_de_passe_sans_code_verrouillent(pool: PgPool) {
        let compte = compte_patient(&pool).await;
        activer_totp(&pool, compte).await;

        for essai in 1..=5 {
            assert_eq!(
                verifier(&pool, Some(compte), BON).await.unwrap(),
                Verification::SecondFacteurRequis(compte),
                "essai {essai}"
            );
        }
        assert_eq!(etat(&pool, compte).await, (0, true));
        assert!(refuse(verifier(&pool, Some(compte), BON).await), "6ᵉ refusé : compte verrouillé");
    }
}
