//! `AuthProfessionnelService` (section 11 du CLAUDE.md) : identifiant
//! `email` + `motDePasseHash` (Argon2id) + verrouillage 5 tentatives / 15
//! min (BF-01), plus la récupération de mot de passe (deux voies au choix
//! de la personne : OTP ou lien par email -- section 11, révision
//! 2026-09-23).
//!
//! **`se_connecter` ne connaît aucun rôle.** Ce service vérifie une
//! identité (`utilisateur`) seule -- il n'existe encore aucune table de
//! rôle non-Patient (`AgentAssurance` est l'étape suivante du plan, section
//! 12). Il renvoie l'`utilisateur_id` vérifié, jamais un JWT : c'est au
//! futur service spécifique au rôle de confirmer l'appartenance à ce rôle
//! et d'appeler `JwtService::emettre(id, role)` avec le bon `role` --
//! exactement comme `jwt.rs` accepte déjà un `role: &str` sans le connaître
//! à l'avance. Bâtir `AuthProfessionnelService` avant `AgentAssurance` n'a
//! de sens que si ce découplage est respecté.
//!
//! **Voie de récupération par OTP** : implémentée ici, réutilise `otp.rs`
//! tel quel (jamais exclusif au Patient). **Voie par email (lien de
//! réinitialisation)** : décision actée mais **non implémentée dans ce
//! fichier** -- aucune dépendance d'envoi d'email n'a encore été choisie
//! (section 13 : à présenter avec sources officielles avant tout ajout au
//! Cargo.toml, pas fait ici). Choisir entre les deux voies revient à la
//! personne au moment de la demande, pas imposé par le système.

use chrono::{DateTime, Duration, Utc};
use sqlx::PgPool;
use sqlx::types::Uuid;

use crate::{mot_de_passe, otp};

// BF-01, section 11 du CLAUDE.md -- pas une valeur arbitraire de bibliothèque.
const MAX_TENTATIVES: i16 = 5;
const DUREE_VERROUILLAGE_MIN: i64 = 15;

// Constante unique de `mot_de_passe.rs` (décision W2).
use crate::mot_de_passe::LONGUEUR_MOT_DE_PASSE_MIN;

#[derive(Debug, thiserror::Error)]
pub enum ErreurAuthProfessionnel {
    /// Couvre à la fois « email inconnu » et « mot de passe incorrect » --
    /// volontairement non distingués sur `se_connecter`, pour ne pas
    /// permettre à un attaquant de découvrir quels emails sont enregistrés
    /// en sondant la connexion (pratique standard, pas propre à ce projet).
    #[error("Email ou mot de passe incorrect.")]
    IdentifiantsInvalides,

    #[error("compte verrouillé, réessayer plus tard")]
    CompteVerrouille,

    #[error("code invalide ou expiré")]
    CodeInvalide,

    #[error("mot de passe trop court : {0} caractères, {LONGUEUR_MOT_DE_PASSE_MIN} minimum")]
    MotDePasseTropCourt(usize),

    #[error("erreur interne : {0}")]
    Interne(String),
}

impl From<sqlx::Error> for ErreurAuthProfessionnel {
    fn from(erreur: sqlx::Error) -> Self {
        ErreurAuthProfessionnel::Interne(erreur.to_string())
    }
}

/// Connexion par email + mot de passe, avec verrouillage BF-01. Renvoie
/// l'`utilisateur_id` vérifié (voir doc de module -- pas de JWT ici).
pub async fn se_connecter(
    pool: &PgPool,
    email: &str,
    mot_de_passe: &str,
) -> Result<Uuid, ErreurAuthProfessionnel> {
    let ligne: Option<(Uuid, Option<String>, i16, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT id, mot_de_passe_hash, tentatives_echouees, verrouille_jusqua \
         FROM utilisateur WHERE lower(email) = lower($1)",
    )
    .bind(email)
    .fetch_optional(pool)
    .await?;

    // Email inconnu : même erreur que « mauvais mot de passe » (voir
    // ErreurAuthProfessionnel::IdentifiantsInvalides).
    let Some((utilisateur_id, hash, tentatives, verrouille_jusqua)) = ligne else {
        return Err(ErreurAuthProfessionnel::IdentifiantsInvalides);
    };

    if let Some(jusqua) = verrouille_jusqua {
        if Utc::now() < jusqua {
            return Err(ErreurAuthProfessionnel::CompteVerrouille);
        }
    }

    // Pas de mot de passe défini (ex. identité créée mais jamais complétée) :
    // aucun mot de passe ne peut jamais correspondre.
    let Some(hash) = hash else {
        return Err(ErreurAuthProfessionnel::IdentifiantsInvalides);
    };

    let correct = mot_de_passe::verifier(mot_de_passe, &hash)
        .map_err(|e| ErreurAuthProfessionnel::Interne(e.to_string()))?;

    if !correct {
        let nouvelles_tentatives = tentatives + 1;
        if nouvelles_tentatives >= MAX_TENTATIVES {
            let jusqua = Utc::now() + Duration::minutes(DUREE_VERROUILLAGE_MIN);
            sqlx::query("UPDATE utilisateur SET tentatives_echouees = 0, verrouille_jusqua = $1 WHERE id = $2")
                .bind(jusqua)
                .bind(utilisateur_id)
                .execute(pool)
                .await?;
        } else {
            sqlx::query("UPDATE utilisateur SET tentatives_echouees = $1 WHERE id = $2")
                .bind(nouvelles_tentatives)
                .bind(utilisateur_id)
                .execute(pool)
                .await?;
        }
        return Err(ErreurAuthProfessionnel::IdentifiantsInvalides);
    }

    // Connexion réussie : remet le compteur à zéro (aussi utile si le
    // compte n'était plus verrouillé mais gardait un compteur partiel).
    sqlx::query("UPDATE utilisateur SET tentatives_echouees = 0, verrouille_jusqua = NULL WHERE id = $1")
        .bind(utilisateur_id)
        .execute(pool)
        .await?;

    Ok(utilisateur_id)
}

/// Demande un OTP de récupération pour un email. `Ok(None)` si l'email est
/// inconnu -- **volontairement pas une erreur distincte** : l'appelant
/// (future route HTTP) doit répondre de façon identique dans les deux cas
/// (email existant ou non), pour ne pas révéler quels emails sont
/// enregistrés. Renvoie aussi le téléphone associé : c'est l'appelant qui
/// envoie le SMS (`SmsSender`), ce module reste indépendant du canal.
pub async fn demander_otp_recuperation(
    pool: &PgPool,
    email: &str,
) -> Result<Option<(String, String)>, ErreurAuthProfessionnel> {
    let ligne: Option<(Uuid, String)> =
        sqlx::query_as("SELECT id, telephone FROM utilisateur WHERE lower(email) = lower($1)")
            .bind(email)
            .fetch_optional(pool)
            .await?;

    let Some((utilisateur_id, telephone)) = ligne else {
        return Ok(None);
    };

    let code = otp::generer_et_enregistrer(pool, utilisateur_id).await?;
    Ok(Some((code, telephone)))
}

/// Réinitialise le mot de passe après validation de l'OTP de récupération.
/// Efface aussi tout verrouillage en cours : prouver le contrôle du
/// téléphone est un motif légitime de déblocage.
pub async fn reinitialiser_via_otp(
    pool: &PgPool,
    email: &str,
    code_otp: &str,
    nouveau_mot_de_passe: &str,
) -> Result<(), ErreurAuthProfessionnel> {
    if mot_de_passe::longueur(nouveau_mot_de_passe) < LONGUEUR_MOT_DE_PASSE_MIN {
        return Err(ErreurAuthProfessionnel::MotDePasseTropCourt(
            mot_de_passe::longueur(nouveau_mot_de_passe),
        ));
    }

    let ligne: Option<(Uuid,)> =
        sqlx::query_as("SELECT id FROM utilisateur WHERE lower(email) = lower($1)")
            .bind(email)
            .fetch_optional(pool)
            .await?;
    // Aucune ligne : aucun OTP n'a pu être généré pour cet email
    // (otp::generer_et_enregistrer exige un utilisateur_id existant), donc
    // code_otp ne peut de toute façon pas être valide.
    let (utilisateur_id,) = ligne.ok_or(ErreurAuthProfessionnel::CodeInvalide)?;

    if !otp::verifier(pool, utilisateur_id, code_otp).await? {
        return Err(ErreurAuthProfessionnel::CodeInvalide);
    }

    let hash = mot_de_passe::hacher(nouveau_mot_de_passe)
        .map_err(|e| ErreurAuthProfessionnel::Interne(e.to_string()))?;

    sqlx::query(
        "UPDATE utilisateur SET mot_de_passe_hash = $1, tentatives_echouees = 0, verrouille_jusqua = NULL WHERE id = $2",
    )
    .bind(&hash)
    .bind(utilisateur_id)
    .execute(pool)
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Base temporaire par test, jamais la base de développement : voir
    // « Tests et environnement », section 12 du CLAUDE.md. Ce service ne
    // connaît aucun rôle : la ligne `utilisateur` de test est créée
    // directement en SQL.
    #[sqlx::test]
    #[ignore]
    async fn se_connecter_et_recuperation_en_base(pool: PgPool) {
        let email = "test.auth.pro@example.org";
        let telephone = "+22670002002";
        let hash = mot_de_passe::hacher("motdepasseinitial123").unwrap();

        let utilisateur_id: Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (nom, prenom, telephone, email, mot_de_passe_hash) \
             VALUES ('Pro', 'Test', $1, $2, $3) RETURNING id",
        )
        .bind(telephone)
        .bind(email)
        .bind(&hash)
        .fetch_one(&pool)
        .await
        .unwrap();

        // --- connexion ---
        assert!(matches!(
            se_connecter(&pool, email, "mauvais-mot-de-passe").await,
            Err(ErreurAuthProfessionnel::IdentifiantsInvalides)
        ));
        assert!(matches!(
            se_connecter(&pool, "inconnu@example.org", "peu-importe").await,
            Err(ErreurAuthProfessionnel::IdentifiantsInvalides)
        ));

        let id_verifie = se_connecter(&pool, email, "motdepasseinitial123").await.unwrap();
        assert_eq!(id_verifie, utilisateur_id);

        // --- verrouillage (5 échecs) ---
        for _ in 0..5 {
            let _ = se_connecter(&pool, email, "mauvais-mot-de-passe").await;
        }
        assert!(matches!(
            se_connecter(&pool, email, "motdepasseinitial123").await,
            Err(ErreurAuthProfessionnel::CompteVerrouille)
        ), "le bon mot de passe doit aussi être refusé pendant le verrouillage");

        // débloque manuellement pour la suite du test (pas d'attente de 15 min)
        sqlx::query("UPDATE utilisateur SET tentatives_echouees = 0, verrouille_jusqua = NULL WHERE id = $1")
            .bind(utilisateur_id)
            .execute(&pool)
            .await
            .unwrap();

        // --- récupération par OTP ---
        assert!(
            demander_otp_recuperation(&pool, "inconnu@example.org").await.unwrap().is_none(),
            "email inconnu : aucune information renvoyée"
        );
        let (code, telephone_recu) = demander_otp_recuperation(&pool, email).await.unwrap().unwrap();
        assert_eq!(telephone_recu, telephone);

        assert!(matches!(
            reinitialiser_via_otp(&pool, email, "000000", "nouveaumdp123").await,
            Err(ErreurAuthProfessionnel::CodeInvalide)
        ));

        reinitialiser_via_otp(&pool, email, &code, "nouveaumdp123").await.unwrap();
        se_connecter(&pool, email, "nouveaumdp123")
            .await
            .expect("le nouveau mot de passe doit fonctionner");
    }
}
