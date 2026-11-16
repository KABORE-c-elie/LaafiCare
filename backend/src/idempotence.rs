//! Clés d'idempotence des créations de la pharmacie (demande du porteur du
//! 2026-11-16, CLAUDE.md §15 ; CDC, « aucune saisie perdue » en cas de
//! coupure). Source : draft IETF *The Idempotency-Key HTTP Header Field*
//! (httpapi) :
//! - le client envoie un en-tête `Idempotency-Key` avec une valeur unique
//!   (un UUID par exemple) ; s'il renvoie la même requête avec la même clé,
//!   le serveur renvoie le résultat déjà créé au lieu d'en créer un autre ;
//! - la même clé avec une **autre** requête est refusée (le draft propose
//!   422) ;
//! - le serveur garde une empreinte de la requête pour faire cette
//!   comparaison.
//!
//! Ici la clé est facultative (la création marche sans), unique **par
//! pharmacie**, et gardée sans limite de durée (table `operation_idempotente`,
//! migration 0031, jamais modifiée). La clé est réservée dans la
//! transaction même de la création : une requête simultanée avec la même clé
//! attend la fin de la première (index unique), puis reçoit son résultat.

use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::types::Uuid;

/// Nom de l'en-tête (draft IETF).
pub const ENTETE: &str = "Idempotency-Key";

/// Une clé contrôlée : 1 à 255 caractères ASCII visibles (même règle que le
/// CHECK de la migration 0031). Le draft définit la valeur comme une chaîne
/// de champ structuré (RFC 8941) ; on accepte aussi la forme sans
/// guillemets, plus simple pour les applications, en retirant les
/// guillemets s'il y en a.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cle(String);

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ErreurIdempotence {
    #[error("L'en-tête Idempotency-Key doit faire 1 à 255 caractères ASCII visibles.")]
    CleInvalide,
    #[error("Cette clé d'idempotence a déjà servi pour une autre requête.")]
    CleReutilisee,
}

impl Cle {
    pub fn lire(valeur: &str) -> Result<Cle, ErreurIdempotence> {
        let valeur = valeur.trim();
        let valeur = valeur.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(valeur);
        if (1..=255).contains(&valeur.len()) && valeur.bytes().all(|o| (0x21..=0x7E).contains(&o)) {
            Ok(Cle(valeur.to_string()))
        } else {
            Err(ErreurIdempotence::CleInvalide)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Empreinte SHA-256 d'une requête : l'opération et son contenu, en JSON.
pub fn empreinte<T: Serialize>(operation: &str, requete: &T) -> Vec<u8> {
    // La sérialisation d'une structure Rust en JSON ne peut pas échouer ici
    // (aucune table de hachage à clés non textuelles) ; une chaîne vide en
    // repli garde une empreinte valide.
    let json = serde_json::to_vec(&(operation, requete)).unwrap_or_default();
    Sha256::digest(&json).to_vec()
}

/// Résultat de la réservation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reservation {
    /// Première fois : l'appelant crée l'objet **avec cet identifiant**.
    Nouvelle(Uuid),
    /// Déjà faite : l'appelant annule sa transaction et renvoie l'objet
    /// existant.
    DejaFaite(Uuid),
}

/// Réserve un identifiant pour l'objet à créer, et la clé si elle est
/// donnée, dans la transaction de la création. L'identifiant est un
/// `uuidv7()` de PostgreSQL (comme toutes les tables), sauf si l'appelant
/// en impose un (`resultat`) : une annulation de vente renvoie la vente
/// elle-même.
pub async fn reserver(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    structure_id: Uuid,
    cle: Option<&Cle>,
    operation: &'static str,
    empreinte: &[u8],
    resultat: Option<Uuid>,
) -> Result<Result<Reservation, ErreurIdempotence>, sqlx::Error> {
    let id: Uuid = match resultat {
        Some(id) => id,
        None => sqlx::query_scalar("SELECT uuidv7()").fetch_one(&mut **tx).await?,
    };
    let Some(cle) = cle else {
        return Ok(Ok(Reservation::Nouvelle(id)));
    };
    let inseree: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO operation_idempotente (structure_id, cle, operation, empreinte, resultat_id) \
         VALUES ($1, $2, $3, $4, $5) ON CONFLICT (structure_id, cle) DO NOTHING RETURNING resultat_id",
    )
    .bind(structure_id)
    .bind(cle.as_str())
    .bind(operation)
    .bind(empreinte)
    .bind(id)
    .fetch_optional(&mut **tx)
    .await?;
    if inseree.is_some() {
        return Ok(Ok(Reservation::Nouvelle(id)));
    }
    let (operation_existante, empreinte_existante, resultat_id): (String, Vec<u8>, Uuid) = sqlx::query_as(
        "SELECT operation, empreinte, resultat_id FROM operation_idempotente WHERE structure_id = $1 AND cle = $2",
    )
    .bind(structure_id)
    .bind(cle.as_str())
    .fetch_one(&mut **tx)
    .await?;
    if operation_existante == operation && empreinte_existante == empreinte {
        Ok(Ok(Reservation::DejaFaite(resultat_id)))
    } else {
        Ok(Err(ErreurIdempotence::CleReutilisee))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lecture_des_cles() {
        assert_eq!(Cle::lire(" 8e03978e-40d5-43e8-bc93-6894a57f9324 ").unwrap().as_str(), "8e03978e-40d5-43e8-bc93-6894a57f9324");
        assert_eq!(Cle::lire("\"abc\"").unwrap().as_str(), "abc", "forme chaîne du draft");
        for mauvaise in ["", "  ", "a b", "é", &"x".repeat(256), "\"\""] {
            assert_eq!(Cle::lire(mauvaise), Err(ErreurIdempotence::CleInvalide), "{mauvaise:?}");
        }
    }

    #[test]
    fn empreintes() {
        assert_eq!(empreinte("vente", &1), empreinte("vente", &1));
        assert_ne!(empreinte("vente", &1), empreinte("vente", &2));
        assert_ne!(empreinte("vente", &1), empreinte("perte", &1));
    }
}
