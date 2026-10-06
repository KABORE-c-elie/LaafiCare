//! Notifications enregistrées en base, affichées par l'app patient (BF-11,
//! avec l'exception assumée de la section 12 du CLAUDE.md : pas de SMS pour
//! les notifications de remboursement, le SMS reste réservé à l'OTP).
//!
//! La ligne en base est le contrat de cette brique. Le push futur
//! s'ajoutera par-dessus (colonne `push_envoye_le` + traitement qui lit les
//! notifications non poussées) sans changer les appelants de `creer`.
//!
//! `creer` ne décide pas quoi faire d'un échec : c'est l'appelant qui sait
//! si la notification est accessoire. Pour une demande de remboursement,
//! l'échec est journalisé et n'annule jamais la demande (section 12).

use sqlx::PgPool;
use sqlx::types::Uuid;

/// Même liste fermée que le CHECK de la colonne `notification.type`
/// (migration 0008) : un nouveau type = une variante ici + une migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeNotification {
    RemboursementStatut,
}

impl TypeNotification {
    fn as_str(self) -> &'static str {
        match self {
            TypeNotification::RemboursementStatut => "remboursement_statut",
        }
    }
}

/// Enregistre une notification pour un utilisateur. `utilisateur_id` est
/// l'identité (`utilisateur.id`, le `sub` du JWT), pas l'id d'une table de
/// rôle. Le texte doit rester minimal, sans information de santé : un push
/// futur s'afficherait sur l'écran verrouillé.
pub async fn creer(
    pool: &PgPool,
    utilisateur_id: Uuid,
    type_: TypeNotification,
    titre: &str,
    message: &str,
    demande_remboursement_id: Option<Uuid>,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO notification (utilisateur_id, type, titre, message, demande_remboursement_id) \
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(utilisateur_id)
    .bind(type_.as_str())
    .bind(titre)
    .bind(message)
    .bind(demande_remboursement_id)
    .fetch_one(pool)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_type_correspond_a_la_valeur_du_check_sql() {
        assert_eq!(TypeNotification::RemboursementStatut.as_str(), "remboursement_statut");
    }

    // Base temporaire par test, jamais la base de développement : voir
    // « Tests et environnement », section 12 du CLAUDE.md.
    #[sqlx::test]
    #[ignore]
    async fn creer_en_base(pool: PgPool) {
        let utilisateur_id: Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (nom, prenom, telephone) VALUES ('Test', 'Notification', '+22670009019') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        let id = creer(
            &pool,
            utilisateur_id,
            TypeNotification::RemboursementStatut,
            "Remboursement MUNASEB",
            "Une demande de remboursement a été enregistrée à votre nom.",
            None,
        )
        .await
        .unwrap();

        let (type_lu, titre, lu, demande): (String, String, bool, Option<Uuid>) = sqlx::query_as(
            "SELECT type, titre, lu, demande_remboursement_id FROM notification WHERE id = $1",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(type_lu, "remboursement_statut");
        assert_eq!(titre, "Remboursement MUNASEB");
        assert!(!lu, "une nouvelle notification doit être non lue");
        assert_eq!(demande, None);

        // Un lien vers une demande inexistante est refusé par la clé
        // étrangère : l'erreur remonte à l'appelant, qui décide.
        assert!(
            creer(&pool, utilisateur_id, TypeNotification::RemboursementStatut, "t", "m", Some(Uuid::nil()))
                .await
                .is_err()
        );
    }
}
