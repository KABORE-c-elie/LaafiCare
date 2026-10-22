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

// ---------------------------------------------------------------------
// Lecture par le patient (étape 6, décisions E1, E2 et E6 du 2026-10-21)
// ---------------------------------------------------------------------
//
// Le destinataire est l'identité ; le jeton porte le compte patient. Chaque
// requête passe donc du compte à son identité, dans la requête elle-même :
// une notification d'une autre personne est introuvable, exactement comme
// une notification inexistante (OWASP API Security Top 10 2023, API1).

/// Une notification telle que l'application l'affiche.
#[derive(Debug, serde::Serialize, sqlx::FromRow)]
pub struct NotificationPatient {
    pub id: Uuid,
    pub titre: String,
    pub message: String,
    pub lu: bool,
    pub date_creation: chrono::DateTime<chrono::Utc>,
    /// Demande concernée, pour ouvrir son détail.
    pub demande_id: Option<Uuid>,
}

/// Page de notifications, les plus récentes d'abord (curseur `avant` sur
/// l'identifiant `uuidv7`), et le nombre de notifications non lues pour le
/// badge de l'application.
pub async fn lister_pour_patient(
    pool: &PgPool,
    compte_id: Uuid,
    avant: Option<Uuid>,
    limite: i64,
) -> Result<(Vec<NotificationPatient>, i64), sqlx::Error> {
    let notifications = sqlx::query_as(
        "SELECT n.id, n.titre, n.message, n.lu, n.date_creation, n.demande_remboursement_id AS demande_id \
         FROM notification n JOIN compte c ON c.utilisateur_id = n.utilisateur_id \
         WHERE c.id = $1 AND c.type_compte = 'patient' AND ($2::uuid IS NULL OR n.id < $2) \
         ORDER BY n.id DESC LIMIT $3",
    )
    .bind(compte_id)
    .bind(avant)
    .bind(limite)
    .fetch_all(pool)
    .await?;
    let non_lues: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM notification n JOIN compte c ON c.utilisateur_id = n.utilisateur_id \
         WHERE c.id = $1 AND c.type_compte = 'patient' AND NOT n.lu",
    )
    .bind(compte_id)
    .fetch_one(pool)
    .await?;
    Ok((notifications, non_lues))
}

/// Marque une notification comme lue. Faux si elle n'existe pas ou
/// appartient à une autre personne (404). Une notification déjà lue reste
/// trouvée : rejouer la requête n'a pas d'effet (réseau instable, E6).
pub async fn marquer_lue(pool: &PgPool, compte_id: Uuid, notification_id: Uuid) -> Result<bool, sqlx::Error> {
    let trouvee: Option<Uuid> = sqlx::query_scalar(
        "UPDATE notification n SET lu = true FROM compte c \
         WHERE n.id = $2 AND c.id = $1 AND c.type_compte = 'patient' AND c.utilisateur_id = n.utilisateur_id \
         RETURNING n.id",
    )
    .bind(compte_id)
    .bind(notification_id)
    .fetch_optional(pool)
    .await?;
    Ok(trouvee.is_some())
}

/// Marque toutes les notifications de la personne comme lues.
pub async fn marquer_toutes_lues(pool: &PgPool, compte_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE notification n SET lu = true FROM compte c \
         WHERE c.id = $1 AND c.type_compte = 'patient' AND c.utilisateur_id = n.utilisateur_id AND NOT n.lu",
    )
    .bind(compte_id)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn compte_patient(pool: &PgPool, telephone: &str) -> (Uuid, Uuid) {
        let u: Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (nom, prenom, telephone) VALUES ('OUEDRAOGO', 'Awa', $1) RETURNING id",
        )
        .bind(telephone)
        .fetch_one(pool)
        .await
        .unwrap();
        let c: Uuid = sqlx::query_scalar(
            "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'patient', 'x') RETURNING id",
        )
        .bind(u)
        .fetch_one(pool)
        .await
        .unwrap();
        (c, u)
    }

    /// E2, E6 : liste paginée, non lues, marquer une ou toutes ; une
    /// notification d'une autre personne est introuvable.
    #[sqlx::test]
    #[ignore]
    async fn lecture_par_le_patient(pool: PgPool) {
        let (a, ua) = compte_patient(&pool, "+22670020031").await;
        let (b, ub) = compte_patient(&pool, "+22670020032").await;
        let mut ids = Vec::new();
        for i in 0..3 {
            ids.push(
                creer(&pool, ua, TypeNotification::RemboursementStatut, "Remboursement MUNASEB", &format!("Message {i}"), None)
                    .await
                    .unwrap(),
            );
        }
        let de_b = creer(&pool, ub, TypeNotification::RemboursementStatut, "Remboursement MUNASEB", "Message B", None)
            .await
            .unwrap();

        let (page1, non_lues) = lister_pour_patient(&pool, a, None, 2).await.unwrap();
        assert_eq!(non_lues, 3);
        assert_eq!(page1.iter().map(|n| n.id).collect::<Vec<_>>(), [ids[2], ids[1]]);
        let (page2, _) = lister_pour_patient(&pool, a, Some(ids[1]), 2).await.unwrap();
        assert_eq!(page2.iter().map(|n| n.id).collect::<Vec<_>>(), [ids[0]]);

        // Celle de B n'existe pas pour A ; rien n'est modifié.
        assert!(!marquer_lue(&pool, a, de_b).await.unwrap());
        assert!(!marquer_lue(&pool, a, Uuid::nil()).await.unwrap());
        assert_eq!(lister_pour_patient(&pool, b, None, 50).await.unwrap().1, 1);

        // Idempotent.
        assert!(marquer_lue(&pool, a, ids[0]).await.unwrap());
        assert!(marquer_lue(&pool, a, ids[0]).await.unwrap());
        assert_eq!(lister_pour_patient(&pool, a, None, 50).await.unwrap().1, 2);

        marquer_toutes_lues(&pool, a).await.unwrap();
        assert_eq!(lister_pour_patient(&pool, a, None, 50).await.unwrap().1, 0);
        assert_eq!(lister_pour_patient(&pool, b, None, 50).await.unwrap().1, 1, "celles de B ne changent pas");
    }

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
