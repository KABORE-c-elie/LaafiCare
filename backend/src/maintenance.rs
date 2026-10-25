//! Tâche de fond périodique de maintenance (nettoyage des sessions expirées,
//! des anciens OTP et des jetons d'email obsolètes).
//!
//! Exécutée en tâche asynchrone d'arrière-plan (`tokio::spawn`) plutôt que sur
//! le chemin chaud OLTP des requêtes utilisateurs (décision P0 / DBA), afin
//! d'éviter la contention de verrous et l'amplification d'écritures (WAL).

use std::time::Duration;
use sqlx::PgPool;

use crate::{otp, session};

/// Fréquence d'exécution de la purge en tâche de fond (10 minutes).
pub const INTERVALLE_MAINTENANCE: Duration = Duration::from_secs(600);

/// Démarre la boucle de maintenance périodique en tâche d'arrière-plan Tokio.
pub fn demarrer_maintenance_arriere_plan(pool: PgPool) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(INTERVALLE_MAINTENANCE);
        // Le premier tick se déclenche immédiatement à l'initialisation ; on l'attend pour laisser le serveur démarrer.
        interval.tick().await;

        loop {
            interval.tick().await;
            if let Err(erreur) = executer_cycle_maintenance(&pool).await {
                tracing::warn!(%erreur, "échec du cycle de maintenance d'arrière-plan");
            }
        }
    });
}

/// Exécute un cycle complet de purge des données expirées.
pub async fn executer_cycle_maintenance(pool: &PgPool) -> Result<(), sqlx::Error> {
    // 1. Purge des envois d'OTP expirés (> 1 heure)
    let suppr_otp = otp::purger_expires(pool).await?;

    // 2. Purge des sessions expirées (> 30 jours, ou > 6 mois pour rejeu détecté)
    let suppr_sessions = session::purger_sessions_expirees(pool).await?;

    // 3. Purge des jetons d'email expirés ou utilisés depuis plus de 30 jours
    let suppr_jetons = sqlx::query(
        "DELETE FROM jeton_email WHERE (utilise_le IS NOT NULL AND utilise_le < now() - interval '30 days') \
         OR (annule_le IS NOT NULL AND annule_le < now() - interval '30 days') \
         OR expire_le < now() - interval '30 days'",
    )
    .execute(pool)
    .await?
    .rows_affected();

    if suppr_otp > 0 || suppr_sessions > 0 || suppr_jetons > 0 {
        tracing::info!(
            suppr_otp,
            suppr_sessions,
            suppr_jetons,
            "cycle de maintenance d'arrière-plan terminé"
        );
    }

    Ok(())
}
