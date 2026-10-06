//! Code OTP du Patient (section 11) : génération, stockage haché sur
//! `utilisateur`, vérification.
//!
//! Deux règles distinctes de NIST SP 800-63B (pages.nist.gov/800-63-3/sp800-63b.html),
//! implémentées séparément -- l'une ne remplace pas l'autre :
//! 1. Remplacement par une nouvelle demande (règle du porteur : « code valide
//!    5 minutes, et une nouvelle demande invalide automatiquement l'ancien
//!    code ») -- `generer_et_enregistrer` écrase l'ancien code par un
//!    `UPDATE`, qui cesse simplement d'exister.
//! 2. « verifiers SHALL accept a given time-based OTP only once during the
//!    validity period » -- un code déjà vérifié avec succès ne doit jamais
//!    être réutilisable, même s'il reste dans sa fenêtre de 5 minutes.
//!    `verifier` efface le code en base dès qu'il a été validé une fois.
//!
//! Hachage SHA-256 plutôt qu'Argon2id (décision actée) : NIST n'impose pas
//! de hachage pour un OTP (contrairement au mot de passe, où c'est
//! explicite) -- « the symmetric keys used by authenticators [...] SHALL be
//! strongly protected against compromise », une exigence générique. Argon2id
//! serait disproportionné : l'espace de valeurs d'un code à 6 chiffres
//! (10^6) se parcourt en une fraction de seconde même haché, donc le coût
//! de calcul d'Argon2id n'apporte rien ici. La vraie protection est ailleurs
//! (fenêtre de 5 minutes, usage unique) -- le hachage n'est qu'une défense
//! en profondeur contre une fuite de base en lecture seule.
//!
//! Pas de comparaison en temps constant à la vérification : sans intérêt
//! pour un secret à 10^6 valeurs déjà trivialement énumérable, contrairement
//! à un mot de passe à haute entropie.

use chrono::{DateTime, Duration, Utc};
use rand::random_range;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use sqlx::types::Uuid;

const LONGUEUR_CODE: u32 = 6;
// Règle explicite du porteur, pas une valeur par défaut de bibliothèque.
const DUREE_VALIDITE_MIN: i64 = 5;

/// Génère un code à 6 chiffres (0 à 999999, zéros non significatifs
/// conservés par le formatage). `rand::random_range` utilise `ThreadRng`,
/// documenté comme générateur cryptographique (ChaCha 12 tours, implémente
/// `CryptoRng` -- doc rand 0.10.3) : un OTP prévisible casserait
/// l'authentification.
fn generer_code() -> String {
    format!("{:0width$}", random_range(0..10u32.pow(LONGUEUR_CODE)), width = LONGUEUR_CODE as usize)
}

// Le sel n'est pas un secret à conserver séparément : `utilisateur_id` sert
// de sel (déjà connu, pas de colonne supplémentaire), pour éviter qu'une
// seule table précalculée des 10^6 empreintes SHA-256 ne serve à retrouver
// le code de n'importe quel compte d'un coup. Bénéfice réel limité (voir
// commentaire de module), mais gratuit.
fn hacher(code: &str, utilisateur_id: Uuid) -> String {
    let mut hasheur = Sha256::new();
    hasheur.update(utilisateur_id.as_bytes());
    hasheur.update(b":");
    hasheur.update(code.as_bytes());
    hasheur
        .finalize()
        .iter()
        .map(|octet| format!("{octet:02x}"))
        .collect()
}

/// Génère un nouveau code, l'enregistre (haché) sur `utilisateur`, et
/// renvoie le code en clair -- à transmettre par SMS par l'appelant
/// (`SmsSender`, pas ce module). L'`UPDATE` écrase tout code précédent :
/// règle 1 (remplacement par une nouvelle demande).
pub async fn generer_et_enregistrer(pool: &PgPool, utilisateur_id: Uuid) -> Result<String, sqlx::Error> {
    let code = generer_code();
    let hash = hacher(&code, utilisateur_id);
    let expire_a = Utc::now() + Duration::minutes(DUREE_VALIDITE_MIN);

    sqlx::query("UPDATE utilisateur SET otp_code_hash = $1, otp_expire_a = $2 WHERE id = $3")
        .bind(&hash)
        .bind(expire_a)
        .bind(utilisateur_id)
        .execute(pool)
        .await?;

    Ok(code)
}

/// Vérifie le code fourni contre celui enregistré. `Ok(true)` efface
/// immédiatement le code en base avant de renvoyer -- règle 2 (usage
/// unique), pour qu'il ne puisse pas être rejoué même dans sa fenêtre de
/// validité. `Ok(false)` couvre : aucun code actif, code expiré, ou code
/// qui ne correspond pas -- ne distingue pas ces cas côté appelant (ne pas
/// révéler pourquoi la vérification a échoué).
pub async fn verifier(pool: &PgPool, utilisateur_id: Uuid, code: &str) -> Result<bool, sqlx::Error> {
    let ligne: Option<(Option<String>, Option<DateTime<Utc>>)> =
        sqlx::query_as("SELECT otp_code_hash, otp_expire_a FROM utilisateur WHERE id = $1")
            .bind(utilisateur_id)
            .fetch_optional(pool)
            .await?;

    // Aucune ligne, ou pas de code actif (colonnes NULL après un usage
    // précédent ou jamais demandé) : rien à vérifier.
    let (hash_stocke, expire_a) = match ligne {
        Some((Some(h), Some(e))) => (h, e),
        _ => return Ok(false),
    };

    if Utc::now() > expire_a {
        return Ok(false);
    }

    if hacher(code, utilisateur_id) != hash_stocke {
        return Ok(false);
    }

    sqlx::query("UPDATE utilisateur SET otp_code_hash = NULL, otp_expire_a = NULL WHERE id = $1")
        .bind(utilisateur_id)
        .execute(pool)
        .await?;

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn genere_un_code_a_6_chiffres() {
        let code = generer_code();
        assert_eq!(code.len(), 6);
        assert!(code.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn le_hachage_depend_de_l_utilisateur() {
        // `Uuid::new_v4()` demanderait la feature `v4` du crate `uuid`, non
        // activée (sqlx n'active que le type de base). Deux UUID fixes
        // suffisent pour ce test, pas besoin d'en générer un vrai.
        let id_a = Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        let id_b = Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap();
        assert_ne!(hacher("123456", id_a), hacher("123456", id_b));
    }

    #[test]
    fn le_hachage_est_deterministe_pour_le_meme_couple() {
        let id = Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        assert_eq!(hacher("123456", id), hacher("123456", id));
    }

    // Base temporaire par test, jamais la base de développement : voir
    // « Tests et environnement », section 12 du CLAUDE.md.
    #[sqlx::test]
    #[ignore]
    async fn regles_1_et_2_en_base(pool: PgPool) {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (nom, prenom, telephone) VALUES ('Test', 'OTP', '+22670009999') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        // Règle 1 : une nouvelle demande invalide l'ancien code.
        let premier_code = generer_et_enregistrer(&pool, id).await.unwrap();
        let second_code = generer_et_enregistrer(&pool, id).await.unwrap();
        assert!(
            !verifier(&pool, id, &premier_code).await.unwrap(),
            "le premier code doit être invalidé par la seconde demande"
        );

        // Mauvais code : refusé, et le vrai code reste utilisable ensuite.
        assert!(!verifier(&pool, id, "000000").await.unwrap());

        // Règle 2 (NIST) : un code vérifié avec succès ne doit plus être
        // réutilisable, même immédiatement après, dans sa fenêtre de 5 min.
        assert!(verifier(&pool, id, &second_code).await.unwrap());
        assert!(
            !verifier(&pool, id, &second_code).await.unwrap(),
            "un code déjà utilisé avec succès ne doit pas être vérifiable une seconde fois"
        );
    }
}
