//! Code OTP du Patient (section 11) : génération, stockage haché sur
//! `utilisateur`, vérification, limites d'envoi et d'essais.
//!
//! Règles de NIST SP 800-63B, implémentées séparément -- l'une ne remplace
//! pas l'autre :
//! 1. Remplacement par une nouvelle demande (règle du porteur : « code valide
//!    5 minutes, et une nouvelle demande invalide automatiquement l'ancien
//!    code ») -- `generer_et_enregistrer` écrase l'ancien code par un
//!    `UPDATE`, qui cesse simplement d'exister.
//! 2. « verifiers SHALL accept a given time-based OTP only once during the
//!    validity period » -- un code déjà vérifié avec succès ne doit jamais
//!    être réutilisable, même s'il reste dans sa fenêtre de 5 minutes.
//!    `verifier` efface le code en base dès qu'il a été validé une fois.
//! 3. Limitation des essais (rév. 4, §3.1.3) : un secret de moins de 64 bits
//!    impose « a rate-limiting mechanism », et « Generating a new
//!    authentication secret SHALL NOT reset the failed authentication
//!    count ». Décision O3 (lecture A, 2026-10-08) : 5 essais sur un code ;
//!    après 5 échecs cumulés, chaque nouveau code n'a qu'un essai, jusqu'au
//!    premier succès.
//! 4. Limites d'envoi (O1, O2) : 60 s entre deux envois pour un même
//!    numéro, 5 envois par heure glissante. OWASP MFA Cheat Sheet : « enforce
//!    per-account rate limits », et se protéger contre des demandes en masse
//!    qui épuisent le budget SMS. Ni NIST ni l'OWASP ne donnent de valeur :
//!    celles-ci sont du porteur.
//!
//! Hachage SHA-256 plutôt qu'Argon2id (décision actée) : NIST n'impose pas
//! de hachage pour un OTP (contrairement au mot de passe, où c'est
//! explicite) -- « the symmetric keys used by authenticators [...] SHALL be
//! strongly protected against compromise », une exigence générique. Argon2id
//! serait disproportionné : l'espace de valeurs d'un code à 6 chiffres
//! (10^6) se parcourt en une fraction de seconde même haché, donc le coût
//! de calcul d'Argon2id n'apporte rien ici. La vraie protection est ailleurs
//! (fenêtre de 5 minutes, usage unique, essais limités) -- le hachage n'est
//! qu'une défense en profondeur contre une fuite de base en lecture seule.
//! (Empreinte HMAC avec une clé du serveur : point ouvert.)
//!
//! Pas de comparaison en temps constant à la vérification : sans intérêt
//! pour un secret à 10^6 valeurs déjà trivialement énumérable, contrairement
//! à un mot de passe à haute entropie.

use rand::random_range;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use sqlx::types::Uuid;

const LONGUEUR_CODE: u32 = 6;
// Règle explicite du porteur, pas une valeur par défaut de bibliothèque.
const DUREE_VALIDITE_MIN: i32 = 5;
// Valeurs du porteur (O1, O2, O3) : aucune source n'en donne (voir l'en-tête).
const DELAI_MIN_ENTRE_ENVOIS_SECS: f64 = 60.0;
const ENVOIS_MAX_PAR_HEURE: i64 = 5;
const ECHECS_AVANT_UN_SEUL_ESSAI: i16 = 5;

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

#[derive(Debug, thiserror::Error)]
pub enum ErreurDemandeOtp {
    /// O1 ou O2 : délai en secondes avant qu'une nouvelle demande puisse
    /// passer (en-tête `Retry-After` de la réponse 429).
    #[error("trop de demandes de code, réessayer dans {reessayer_dans_secs} s")]
    TropDeDemandes { reessayer_dans_secs: u64 },
    /// Compte patient bloqué par l'équipe (`intervenir_compte`, P2) : aucun
    /// code n'est créé et aucun SMS ne doit partir. Pas une erreur pour la
    /// route : elle répond exactement comme pour un numéro inconnu, pour ne
    /// rien révéler (décision du porteur, 2026-10-22).
    #[error("compte patient bloqué : aucun code")]
    CompteBloque,
    #[error(transparent)]
    Base(#[from] sqlx::Error),
}

/// Génère un nouveau code, l'enregistre (haché) sur `utilisateur`, et
/// renvoie le code en clair -- à transmettre par SMS par l'appelant
/// (`SmsSender`, pas ce module). L'`UPDATE` écrase tout code précédent :
/// règle 1 (remplacement par une nouvelle demande).
///
/// Tout se passe dans une transaction, avec la ligne `utilisateur`
/// verrouillée : deux demandes simultanées pour un même numéro passent
/// l'une après l'autre, et ne peuvent pas dépasser ensemble les limites.
/// `now()` est l'heure de début de la transaction (doc PostgreSQL 18,
/// « Current Date/Time ») : la même pour le nettoyage, le comptage et
/// l'expiration.
pub async fn generer_et_enregistrer(pool: &PgPool, utilisateur_id: Uuid) -> Result<String, ErreurDemandeOtp> {
    let mut tx = pool.begin().await?;

    sqlx::query("SELECT id FROM utilisateur WHERE id = $1 FOR UPDATE")
        .bind(utilisateur_id)
        .fetch_one(&mut *tx)
        .await?;

    // O7 b : les envois de plus d'une heure ne servent plus au comptage.
    // La purge de masse n'est plus exécutée sur le chemin chaud OLTP (décision P0)
    // afin d'éviter la contention de verrous ; elle est déportée en tâche de fond.
    // Le filtre ci-dessous restreint strictement la fenêtre d'observation à la dernière heure.
    let (envois, attente_delai, attente_fenetre): (i64, Option<i64>, Option<i64>) = sqlx::query_as(
        "SELECT count(*), \
                ceil(extract(epoch FROM max(envoye_le) + make_interval(secs => $2) - now()))::bigint, \
                ceil(extract(epoch FROM min(envoye_le) + interval '1 hour' - now()))::bigint \
         FROM envoi_otp WHERE utilisateur_id = $1 AND envoye_le >= now() - interval '1 hour'",
    )
    .bind(utilisateur_id)
    .bind(DELAI_MIN_ENTRE_ENVOIS_SECS)
    .fetch_one(&mut *tx)
    .await?;

    let mut attente: i64 = 0;
    if let Some(secs) = attente_delai.filter(|secs| *secs > 0) {
        attente = attente.max(secs);
    }
    if envois >= ENVOIS_MAX_PAR_HEURE {
        // Au moins 1 s : un envoi pile à la limite de l'heure compte encore.
        attente = attente.max(attente_fenetre.unwrap_or(1).max(1));
    }
    if attente > 0 {
        // La transaction est annulée à sa destruction : rien n'est écrit,
        // pas même le nettoyage, sans conséquence.
        return Err(ErreurDemandeOtp::TropDeDemandes { reessayer_dans_secs: attente as u64 });
    }

    // Compte patient bloqué (P2) : les limites ci-dessus s'appliquent et
    // l'envoi est compté comme pour tout numéro (sinon un 429 qui ne vient
    // jamais trahirait le blocage) ; mais tout code en cours est effacé,
    // aucun nouveau n'est créé, et l'appelant n'envoie aucun SMS.
    let bloque: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM compte WHERE utilisateur_id = $1 AND type_compte = 'patient' AND statut = 'desactive')",
    )
    .bind(utilisateur_id)
    .fetch_one(&mut *tx)
    .await?;
    if bloque {
        sqlx::query("UPDATE utilisateur SET otp_code_hash = NULL, otp_expire_a = NULL WHERE id = $1")
            .bind(utilisateur_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO envoi_otp (utilisateur_id) VALUES ($1)")
            .bind(utilisateur_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        return Err(ErreurDemandeOtp::CompteBloque);
    }

    let code = generer_code();
    // Expiration calculée par la base, comme sa vérification : une seule
    // horloge décide des 5 minutes. Essais du nouveau code selon O3
    // (lecture A) : 5 moins les échecs cumulés, et 1 seul une fois le cumul
    // à 5.
    sqlx::query(
        "UPDATE utilisateur SET otp_code_hash = $1, otp_expire_a = now() + make_interval(mins => $2), \
         otp_essais_restants = CASE WHEN otp_echecs >= $3 THEN 1 ELSE $3 - otp_echecs END \
         WHERE id = $4",
    )
    .bind(hacher(&code, utilisateur_id))
    .bind(DUREE_VALIDITE_MIN)
    .bind(ECHECS_AVANT_UN_SEUL_ESSAI)
    .bind(utilisateur_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query("INSERT INTO envoi_otp (utilisateur_id) VALUES ($1)")
        .bind(utilisateur_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(code)
}

/// Résultat d'une vérification. `Invalide` couvre : aucun code en cours,
/// code expiré, plus d'essai, ou mauvais code avec des essais restants --
/// ne distingue pas ces cas côté appelant.
#[derive(Debug, PartialEq, Eq)]
pub enum VerificationOtp {
    Valide,
    Invalide,
    /// Mauvais code, et c'était le dernier essai permis : le code est
    /// effacé, il faut en demander un nouveau (O6 : 429).
    EssaisEpuises,
}

/// Vérifie le code fourni contre celui enregistré.
///
/// L'essai est retiré AVANT la comparaison, en une seule requête qui ne
/// passe que s'il reste des essais : le verrou de ligne pris par l'`UPDATE`
/// sérialise les essais simultanés, qui ne peuvent donc jamais dépasser le
/// nombre permis (même principe que le verrouillage BF-01). L'échec est
/// compté d'avance et annulé en cas de succès.
pub async fn verifier(pool: &PgPool, utilisateur_id: Uuid, code: &str) -> Result<VerificationOtp, sqlx::Error> {
    let essai: Option<(String, i16)> = sqlx::query_as(
        "UPDATE utilisateur SET otp_essais_restants = otp_essais_restants - 1, otp_echecs = otp_echecs + 1 \
         WHERE id = $1 AND otp_code_hash IS NOT NULL AND otp_expire_a > now() AND otp_essais_restants > 0 \
         RETURNING otp_code_hash, otp_essais_restants",
    )
    .bind(utilisateur_id)
    .fetch_optional(pool)
    .await?;

    let Some((hash_stocke, essais_restants)) = essai else {
        return Ok(VerificationOtp::Invalide);
    };

    if hacher(code, utilisateur_id) == hash_stocke {
        // Règle 2 (usage unique) et O3 : succès, le cumul revient à zéro.
        sqlx::query(
            "UPDATE utilisateur SET otp_code_hash = NULL, otp_expire_a = NULL, otp_echecs = 0, \
             otp_essais_restants = 0 WHERE id = $1",
        )
        .bind(utilisateur_id)
        .execute(pool)
        .await?;
        return Ok(VerificationOtp::Valide);
    }

    if essais_restants == 0 {
        sqlx::query("UPDATE utilisateur SET otp_code_hash = NULL, otp_expire_a = NULL WHERE id = $1")
            .bind(utilisateur_id)
            .execute(pool)
            .await?;
        return Ok(VerificationOtp::EssaisEpuises);
    }

    Ok(VerificationOtp::Invalide)
}

/// Purge les envois d'OTP vieux de plus d'une heure.
/// Exécutée par la tâche de fond de maintenance périodique (décision P0 / DBA).
pub async fn purger_expires(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let suppr = sqlx::query("DELETE FROM envoi_otp WHERE envoye_le < now() - interval '1 hour'")
        .execute(pool)
        .await?
        .rows_affected();
    Ok(suppr)
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
    // « Tests et environnement », section 12 du CLAUDE.md. Numéros fictifs.

    async fn utilisateur(pool: &PgPool, telephone: &str) -> Uuid {
        sqlx::query_scalar("INSERT INTO utilisateur (telephone) VALUES ($1) RETURNING id")
            .bind(telephone)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /// Simule le temps qui passe : recule les envois de ce numéro.
    async fn reculer_envois(pool: &PgPool, utilisateur_id: Uuid, secondes: f64) {
        sqlx::query("UPDATE envoi_otp SET envoye_le = envoye_le - make_interval(secs => $2) WHERE utilisateur_id = $1")
            .bind(utilisateur_id)
            .bind(secondes)
            .execute(pool)
            .await
            .unwrap();
    }

    async fn compteurs(pool: &PgPool, utilisateur_id: Uuid) -> (i16, i16) {
        sqlx::query_as("SELECT otp_echecs, otp_essais_restants FROM utilisateur WHERE id = $1")
            .bind(utilisateur_id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /// Un code à coup sûr différent de `code`.
    fn autre_code(code: &str) -> &'static str {
        if code == "000000" { "000001" } else { "000000" }
    }

    fn attente(resultat: Result<String, ErreurDemandeOtp>) -> u64 {
        match resultat {
            Err(ErreurDemandeOtp::TropDeDemandes { reessayer_dans_secs }) => reessayer_dans_secs,
            autre => panic!("attendu TropDeDemandes, obtenu {autre:?}"),
        }
    }

    #[sqlx::test]
    #[ignore]
    async fn regles_1_et_2_en_base(pool: PgPool) {
        let id = utilisateur(&pool, "+22670009999").await;

        // Règle 1 : une nouvelle demande invalide l'ancien code.
        let premier_code = generer_et_enregistrer(&pool, id).await.unwrap();
        reculer_envois(&pool, id, 61.0).await;
        let second_code = generer_et_enregistrer(&pool, id).await.unwrap();
        if premier_code != second_code {
            assert_eq!(
                verifier(&pool, id, &premier_code).await.unwrap(),
                VerificationOtp::Invalide,
                "le premier code doit être invalidé par la seconde demande"
            );
        }

        // Mauvais code : refusé, et le vrai code reste utilisable ensuite.
        assert_eq!(verifier(&pool, id, autre_code(&second_code)).await.unwrap(), VerificationOtp::Invalide);

        // Règle 2 (NIST) : un code vérifié avec succès ne doit plus être
        // réutilisable, même immédiatement après, dans sa fenêtre de 5 min.
        assert_eq!(verifier(&pool, id, &second_code).await.unwrap(), VerificationOtp::Valide);
        assert_eq!(
            verifier(&pool, id, &second_code).await.unwrap(),
            VerificationOtp::Invalide,
            "un code déjà utilisé avec succès ne doit pas être vérifiable une seconde fois"
        );
    }

    #[sqlx::test]
    #[ignore]
    async fn delai_de_60_s_entre_deux_envois(pool: PgPool) {
        let id = utilisateur(&pool, "+22670009001").await;
        generer_et_enregistrer(&pool, id).await.unwrap();

        let secs = attente(generer_et_enregistrer(&pool, id).await);
        assert!((1..=60).contains(&secs), "attente {secs}");

        reculer_envois(&pool, id, 61.0).await;
        generer_et_enregistrer(&pool, id).await.unwrap();
    }

    #[sqlx::test]
    #[ignore]
    async fn cinq_envois_par_heure_glissante(pool: PgPool) {
        let id = utilisateur(&pool, "+22670009002").await;
        generer_et_enregistrer(&pool, id).await.unwrap();
        for _ in 0..4 {
            reculer_envois(&pool, id, 61.0).await;
            generer_et_enregistrer(&pool, id).await.unwrap();
        }
        reculer_envois(&pool, id, 61.0).await;

        // Le plus ancien envoi a 5 × 61 s : il reste environ 3 600 - 305 s
        // avant qu'il sorte de la fenêtre.
        let secs = attente(generer_et_enregistrer(&pool, id).await);
        assert!((3_200..=3_300).contains(&secs), "attente {secs}");

        // Le plus ancien sort de la fenêtre (il a alors plus d'une heure) :
        // le sixième envoi passe.
        reculer_envois(&pool, id, 3_300.0).await;
        generer_et_enregistrer(&pool, id).await.unwrap();
    }

    #[sqlx::test]
    #[ignore]
    async fn purge_des_envois_de_plus_d_une_heure(pool: PgPool) {
        let autre = utilisateur(&pool, "+22670009003").await;
        sqlx::query("INSERT INTO envoi_otp (utilisateur_id, envoye_le) VALUES ($1, now() - interval '2 hours')")
            .bind(autre)
            .execute(&pool)
            .await
            .unwrap();

        let id = utilisateur(&pool, "+22670009004").await;
        generer_et_enregistrer(&pool, id).await.unwrap();

        let restants: i64 = sqlx::query_scalar("SELECT count(*) FROM envoi_otp WHERE utilisateur_id = $1")
            .bind(autre)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(restants, 0, "l'envoi de plus d'une heure d'un autre numéro doit être effacé");
    }

    #[sqlx::test]
    #[ignore]
    async fn lecture_a_un_seul_essai_apres_cinq_echecs(pool: PgPool) {
        let id = utilisateur(&pool, "+22670009005").await;

        // Premier code : 5 essais, le 5e échec l'efface.
        let code = generer_et_enregistrer(&pool, id).await.unwrap();
        assert_eq!(compteurs(&pool, id).await, (0, 5));
        for _ in 0..4 {
            assert_eq!(verifier(&pool, id, autre_code(&code)).await.unwrap(), VerificationOtp::Invalide);
        }
        assert_eq!(verifier(&pool, id, autre_code(&code)).await.unwrap(), VerificationOtp::EssaisEpuises);
        assert_eq!(verifier(&pool, id, &code).await.unwrap(), VerificationOtp::Invalide, "code effacé");
        assert_eq!(compteurs(&pool, id).await, (5, 0));

        // Nouveau code : le cumul n'est pas remis à zéro, un seul essai.
        reculer_envois(&pool, id, 61.0).await;
        let code = generer_et_enregistrer(&pool, id).await.unwrap();
        assert_eq!(compteurs(&pool, id).await, (5, 1));
        assert_eq!(verifier(&pool, id, autre_code(&code)).await.unwrap(), VerificationOtp::EssaisEpuises);

        // Encore un seul essai ; un succès remet le cumul à zéro.
        reculer_envois(&pool, id, 61.0).await;
        let code = generer_et_enregistrer(&pool, id).await.unwrap();
        assert_eq!(compteurs(&pool, id).await, (6, 1));
        assert_eq!(verifier(&pool, id, &code).await.unwrap(), VerificationOtp::Valide);
        assert_eq!(compteurs(&pool, id).await.0, 0);

        // Le code suivant a de nouveau 5 essais.
        reculer_envois(&pool, id, 61.0).await;
        generer_et_enregistrer(&pool, id).await.unwrap();
        assert_eq!(compteurs(&pool, id).await, (0, 5));
    }

    #[sqlx::test]
    #[ignore]
    async fn essais_simultanes_limites(pool: PgPool) {
        let id = utilisateur(&pool, "+22670009006").await;
        let code = generer_et_enregistrer(&pool, id).await.unwrap();
        let faux = autre_code(&code);

        let mut essais = tokio::task::JoinSet::new();
        for _ in 0..10 {
            let pool = pool.clone();
            essais.spawn(async move { verifier(&pool, id, faux).await.unwrap() });
        }
        let mut epuises = 0;
        while let Some(resultat) = essais.join_next().await {
            if resultat.unwrap() == VerificationOtp::EssaisEpuises {
                epuises += 1;
            }
        }

        // Exactement 5 essais comptés, malgré 10 lancés en même temps.
        assert_eq!(compteurs(&pool, id).await, (5, 0));
        assert_eq!(epuises, 1);
    }
}
