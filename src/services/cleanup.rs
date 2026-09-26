//! Periodic retention sweeps for tables that would otherwise grow forever:
//! expired sessions, spent/expired password-reset tokens, and old
//! notification-log rows.
//!
//! The notification log doubles as the idempotency key for episode
//! notifications, so rows are kept for a generous window before deletion.
//! Like the other background jobs this runs only in the dedicated refresh
//! Deployment (`REFRESH_LOOP_ENABLED=true`), never in web replicas.

use std::time::Duration;

use sqlx::PgPool;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// Interval between cleanup passes (6 hours).
const CLEANUP_INTERVAL: Duration = Duration::from_secs(6 * 3600);

/// notification_log rows older than this are deleted.
const NOTIFICATION_RETENTION_DAYS: i32 = 90;

/// Number of rows removed by one cleanup pass.
#[derive(Debug, Default, Clone, Copy)]
pub struct CleanupStats {
    pub sessions: u64,
    pub reset_tokens: u64,
    pub notifications: u64,
}

/// Delete expired sessions, spent reset tokens, and old notification rows.
///
/// Each statement is independent: a failure in one is logged and does not
/// prevent the others from running, so a single locked table cannot stall
/// retention entirely.
pub async fn run_cleanup_once(db: &PgPool) -> CleanupStats {
    let mut stats = CleanupStats::default();

    match sqlx::query("DELETE FROM sessions WHERE expires_at < NOW()")
        .execute(db)
        .await
    {
        Ok(result) => stats.sessions = result.rows_affected(),
        Err(error) => warn!(error = %error, "cleanup: failed to delete expired sessions"),
    }

    // Spent tokens are useless the moment they are used; keep them for one
    // day so a support question can still be answered from the row.
    match sqlx::query(
        "DELETE FROM password_reset_tokens \
         WHERE expires_at < NOW() \
            OR (used_at IS NOT NULL AND used_at < NOW() - INTERVAL '1 day')",
    )
    .execute(db)
    .await
    {
        Ok(result) => stats.reset_tokens = result.rows_affected(),
        Err(error) => warn!(error = %error, "cleanup: failed to delete reset tokens"),
    }

    match sqlx::query(
        "DELETE FROM notification_log WHERE created_at < NOW() - make_interval(days => $1)",
    )
    .bind(NOTIFICATION_RETENTION_DAYS)
    .execute(db)
    .await
    {
        Ok(result) => stats.notifications = result.rows_affected(),
        Err(error) => warn!(error = %error, "cleanup: failed to delete notification rows"),
    }

    stats
}

/// Run [`run_cleanup_once`] every [`CLEANUP_INTERVAL`] until cancelled.
/// The first pass runs immediately.
pub async fn run_cleanup_loop(db: PgPool, cancel: CancellationToken) {
    let mut interval = tokio::time::interval(CLEANUP_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        let stats = run_cleanup_once(&db).await;
        info!(
            sessions = stats.sessions,
            reset_tokens = stats.reset_tokens,
            notifications = stats.notifications,
            "cleanup pass finished"
        );

        tokio::select! {
            _ = cancel.cancelled() => {
                info!("cleanup loop: cancelled, shutting down");
                break;
            }
            _ = interval.tick() => {}
        }
    }
}
