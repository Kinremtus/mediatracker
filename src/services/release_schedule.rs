use std::future::Future;

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::schedule::ReleaseEntry;
use crate::services::external::shikimori::ShikimoriService;
use crate::services::notifications::TelegramNotifier;

/// Advisory-lock key guarding release-schedule refreshes across replicas,
/// the in-process periodic worker and `refresh_counts`' notify cycle.
///
/// Intentionally equal to `refresh_counts::LOCK_ID` (42): all of them mutate
/// the same `release_schedule` / `notification_log` tables, so serialising
/// them on one lock is exactly what we want. If `refresh_counts` ever changes
/// its key, update this one too.
pub const REFRESH_LOCK_ID: i64 = 42;

/// Run `f` while holding a Postgres session-level advisory lock.
///
/// The lock is taken on a single pooled connection, so lock and unlock cannot
/// be split across connections (which would silently break exclusion, the bug
/// in the old `refresh_counts` implementation). `f` is executed in a spawned
/// task so a panic inside it is contained and the unlock below still runs on
/// the same connection.
///
/// Returns `Ok(None)` when another instance already holds the lock; in that
/// case `f` is not run.
pub async fn with_refresh_lock<F, Fut, T>(pool: &PgPool, f: F) -> Result<Option<T>, anyhow::Error>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Result<T, anyhow::Error>> + Send + 'static,
    T: Send + 'static,
{
    let mut conn = pool.acquire().await?;

    let (locked,): (bool,) = sqlx::query_as("SELECT pg_try_advisory_lock($1)")
        .bind(REFRESH_LOCK_ID)
        .fetch_one(&mut *conn)
        .await?;

    if !locked {
        tracing::debug!(
            lock_id = REFRESH_LOCK_ID,
            "release_schedule: refresh lock held by another instance, skipping"
        );
        return Ok(None);
    }

    // Contain panics so the unlock below always runs on this connection.
    let result = match tokio::spawn(f()).await {
        Ok(result) => result,
        Err(join_error) => Err(anyhow::anyhow!(
            "release_schedule refresh task panicked: {join_error}"
        )),
    };

    if let Err(error) = sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(REFRESH_LOCK_ID)
        .execute(&mut *conn)
        .await
    {
        tracing::warn!(
            error = %error,
            lock_id = REFRESH_LOCK_ID,
            "release_schedule: failed to release refresh lock"
        );
    }

    result.map(Some)
}

/// One refresh pass with failure isolation between logical units.
///
/// The schedule refresh and the notification pass are independent: a failure
/// in one is logged and does not prevent the other. Errors are not returned,
/// so the periodic loop keeps running.
pub async fn refresh_release_schedule(
    service: &ReleaseScheduleService,
    shikimori: &ShikimoriService,
    telegram: &TelegramNotifier,
) {
    // `ensure_fresh` only hits the provider when the stored rows are stale, so
    // a frequent schedule here does not hammer Shikimori.
    if let Err(error) = service.ensure_fresh(shikimori).await {
        tracing::error!(error = %error, "release_schedule: calendar refresh failed");
    }

    if let Err(error) = service.notify_new_episodes(telegram).await {
        tracing::error!(error = %error, "release_schedule: notification pass failed");
    }
}

#[derive(Clone)]
pub struct ReleaseScheduleService {
    db: PgPool,
}

impl ReleaseScheduleService {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    pub async fn refresh_from_shikimori(
        &self,
        shikimori: &ShikimoriService,
    ) -> Result<(), anyhow::Error> {
        let entries = shikimori.fetch_calendar().await?;

        for entry in &entries {
            let poster = entry.anime.image.original.as_ref().map(|url| {
                if url.starts_with("http") {
                    url.clone()
                } else {
                    format!("https://shikimori.one{}", url)
                }
            });

            sqlx::query(
                r#"
                INSERT INTO release_schedule (provider, external_id, episode_number, air_date, title, poster_url, fetched_at)
                VALUES ($1, $2, $3, $4, $5, $6, NOW())
                ON CONFLICT (provider, external_id, episode_number)
                DO UPDATE SET air_date = $4, title = $5, poster_url = $6, fetched_at = NOW()
                "#,
            )
            .bind("shikimori")
            .bind(entry.anime.id.to_string())
            .bind(entry.next_episode)
            .bind(entry.next_episode_at)
            .bind(entry.anime.russian.as_deref().unwrap_or(&entry.anime.name))
            .bind(poster)
            .execute(&self.db)
            .await?;
        }

        Ok(())
    }

    pub async fn ensure_fresh(&self, shikimori: &ShikimoriService) -> Result<(), anyhow::Error> {
        let stale: Option<(i32,)> = sqlx::query_as(
            "SELECT COUNT(*)::int FROM release_schedule WHERE fetched_at > NOW() - INTERVAL '6 hours'",
        )
        .fetch_optional(&self.db)
        .await?;

        let has_fresh = stale.map(|(c,)| c > 0).unwrap_or(false);
        if !has_fresh {
            self.refresh_from_shikimori(shikimori).await?;
        }
        Ok(())
    }

    pub async fn get_upcoming_for_user(
        &self,
        user_id: Uuid,
        limit: i64,
    ) -> Result<Vec<ReleaseEntry>, anyhow::Error> {
        type Row = (String, String, String, Option<String>, i32, DateTime<Utc>);
        #[allow(clippy::type_complexity)]
        let rows: Vec<Row> = sqlx::query_as(
            r#"
            SELECT
                r.provider, r.external_id, r.title, r.poster_url,
                r.episode_number, r.air_date
            FROM release_schedule r
            JOIN tracking_entries t ON t.user_id = $1
            JOIN media_items m ON m.id = t.media_id
                AND m.provider = r.provider
                AND m.external_id = r.external_id
            WHERE t.status IN ('in_progress', 'planned')
              AND r.air_date >= NOW()
            ORDER BY r.air_date ASC
            LIMIT $2
            "#,
        )
        .bind(user_id)
        .bind(limit)
        .fetch_all(&self.db)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(p, eid, title, poster, ep, date)| ReleaseEntry {
                provider: p,
                external_id: eid,
                title,
                poster_url: poster,
                episode_number: ep,
                air_date: date,
            })
            .collect())
    }

    pub async fn get_by_date_range(
        &self,
        user_id: Uuid,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<ReleaseEntry>, anyhow::Error> {
        type Row = (String, String, String, Option<String>, i32, DateTime<Utc>);
        #[allow(clippy::type_complexity)]
        let rows: Vec<Row> = sqlx::query_as(
            r#"
            SELECT
                r.provider, r.external_id, r.title, r.poster_url,
                r.episode_number, r.air_date
            FROM release_schedule r
            JOIN tracking_entries t ON t.user_id = $1
            JOIN media_items m ON m.id = t.media_id
                AND m.provider = r.provider
                AND m.external_id = r.external_id
            WHERE t.status IN ('in_progress', 'planned')
              AND r.air_date >= $2
              AND r.air_date < $3
            ORDER BY r.air_date ASC
            "#,
        )
        .bind(user_id)
        .bind(from)
        .bind(to)
        .fetch_all(&self.db)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(p, eid, title, poster, ep, date)| ReleaseEntry {
                provider: p,
                external_id: eid,
                title,
                poster_url: poster,
                episode_number: ep,
                air_date: date,
            })
            .collect())
    }

    /// Send Telegram notifications for episodes that aired in the last window.
    ///
    /// Deliberately does NOT refresh the schedule itself: the refresh loop is
    /// the single writer and calls `ensure_fresh` before invoking this. The
    /// `notification_log` row is claimed atomically via `ON CONFLICT DO NOTHING`
    /// BEFORE sending, so concurrent replicas cannot double-notify; if the send
    /// fails the claim is released so the next cycle retries.
    pub async fn notify_new_episodes(
        &self,
        telegram: &TelegramNotifier,
    ) -> Result<u32, anyhow::Error> {
        if !telegram.is_configured() {
            return Ok(0);
        }

        // Users with Telegram notifications enabled
        let users: Vec<(Uuid, String)> = sqlx::query_as(
            "SELECT id, telegram_chat_id FROM users WHERE telegram_notifications_enabled = true AND telegram_chat_id IS NOT NULL"
        )
        .fetch_all(&self.db)
        .await?;

        if users.is_empty() {
            return Ok(0);
        }

        let mut notified = 0u32;

        for (user_id, chat_id) in &users {
            // Episodes that aired in the last 2 hours for media the user is
            // currently watching. Dedup is enforced by the atomic claim below,
            // not by a NOT EXISTS check, so this stays race-safe.
            let recent: Vec<(String, String, i32, String)> = sqlx::query_as(
                r#"
                SELECT r.provider, r.title, r.episode_number, r.external_id
                FROM release_schedule r
                JOIN tracking_entries t ON t.user_id = $1
                JOIN media_items m ON m.id = t.media_id
                    AND m.provider = r.provider
                    AND m.external_id = r.external_id
                WHERE t.status = 'in_progress'
                  AND r.air_date >= NOW() - INTERVAL '2 hours'
                  AND r.air_date < NOW()
                "#,
            )
            .bind(user_id)
            .fetch_all(&self.db)
            .await?;

            for (provider, title, episode, external_id) in &recent {
                // Atomic claim: only the first writer of the unique
                // (user, provider, external_id, episode) row proceeds to send.
                let claimed = sqlx::query(
                    r#"
                    INSERT INTO notification_log (user_id, provider, external_id, episode_number)
                    VALUES ($1, $2, $3, $4)
                    ON CONFLICT (user_id, provider, external_id, episode_number) DO NOTHING
                    "#,
                )
                .bind(user_id)
                .bind(provider)
                .bind(external_id)
                .bind(episode)
                .execute(&self.db)
                .await?
                .rows_affected();

                if claimed == 0 {
                    continue;
                }

                if let Err(e) = telegram
                    .send_new_episode_notification(chat_id, title, *episode)
                    .await
                {
                    tracing::error!("Failed to send Telegram notification: {}", e);
                    // Release the claim so the next cycle retries instead of
                    // silently dropping the notification forever.
                    let _ = sqlx::query(
                        "DELETE FROM notification_log WHERE user_id = $1 AND provider = $2 AND external_id = $3 AND episode_number = $4",
                    )
                    .bind(user_id)
                    .bind(provider)
                    .bind(external_id)
                    .bind(episode)
                    .execute(&self.db)
                    .await;
                    continue;
                }

                notified += 1;
            }
        }

        Ok(notified)
    }
}
