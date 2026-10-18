//! Periodic retention sweeps for tables that would otherwise grow forever:
//! expired sessions, spent/expired password-reset tokens, old
//! notification-log rows, release-schedule entries that already aired,
//! fruitless official-meta attempts, and idle search queries (with the
//! catalog rows no remaining query references).
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

/// release_schedule rows whose air date is older than this are deleted.
const RELEASE_SCHEDULE_RETENTION_DAYS: i32 = 30;

/// official_meta_attempts rows older than this are deleted: the marker only
/// matters for the 12-hour freshness gate.
const OFFICIAL_ATTEMPT_RETENTION_DAYS: i32 = 30;

/// search_queries idle for longer than this are pruned, and with them the
/// catalog rows no remaining query references. The catalog deliberately keeps
/// last-known data for anything still being searched (migration 030); only
/// the cold tail expires.
const SEARCH_RETENTION_DAYS: i32 = 180;

/// Number of rows removed by one cleanup pass.
#[derive(Debug, Default, Clone, Copy)]
pub struct CleanupStats {
    pub sessions: u64,
    pub reset_tokens: u64,
    pub notifications: u64,
    pub release_schedule: u64,
    pub official_attempts: u64,
    pub search_catalog: u64,
    pub search_queries: u64,
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

    // The calendar only renders recent and upcoming entries, so rows that
    // aired more than a month ago are dead weight.
    match sqlx::query(
        "DELETE FROM release_schedule WHERE air_date < NOW() - make_interval(days => $1)",
    )
    .bind(RELEASE_SCHEDULE_RETENTION_DAYS)
    .execute(db)
    .await
    {
        Ok(result) => stats.release_schedule = result.rows_affected(),
        Err(error) => warn!(error = %error, "cleanup: failed to delete release schedule rows"),
    }

    match sqlx::query(
        "DELETE FROM official_meta_attempts \
         WHERE attempted_at < NOW() - make_interval(days => $1)",
    )
    .bind(OFFICIAL_ATTEMPT_RETENTION_DAYS)
    .execute(db)
    .await
    {
        Ok(result) => stats.official_attempts = result.rows_affected(),
        Err(error) => warn!(error = %error, "cleanup: failed to delete official-meta attempts"),
    }

    // Search retention, in order: first the catalog rows whose query is gone
    // or itself idle, then the idle queries. `last_query` stores the
    // normalized query; a row survives while ANY type still references the
    // query inside the retention window (the catalog keeps last-known data
    // for anything still being searched — see migration 030).
    match sqlx::query(
        "DELETE FROM search_catalog c \
         WHERE c.fetched_at < NOW() - make_interval(days => $1) \
           AND NOT EXISTS (SELECT 1 FROM search_queries q \
                           WHERE q.query_norm = c.last_query \
                             AND q.fetched_at >= NOW() - make_interval(days => $1))",
    )
    .bind(SEARCH_RETENTION_DAYS)
    .execute(db)
    .await
    {
        Ok(result) => stats.search_catalog = result.rows_affected(),
        Err(error) => warn!(error = %error, "cleanup: failed to delete idle catalog rows"),
    }

    match sqlx::query(
        "DELETE FROM search_queries WHERE fetched_at < NOW() - make_interval(days => $1)",
    )
    .bind(SEARCH_RETENTION_DAYS)
    .execute(db)
    .await
    {
        Ok(result) => stats.search_queries = result.rows_affected(),
        Err(error) => warn!(error = %error, "cleanup: failed to delete idle search queries"),
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
            release_schedule = stats.release_schedule,
            official_attempts = stats.official_attempts,
            search_catalog = stats.search_catalog,
            search_queries = stats.search_queries,
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
