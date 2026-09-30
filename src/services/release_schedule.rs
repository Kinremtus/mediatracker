use std::future::Future;

use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDate, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::schedule::{
    CalendarEvent, CalendarEventKind, CalendarMonthView, FamilyTab, canonical_key,
    filter_by_family, group_into_weeks, merge_and_dedup,
};
use crate::models::schedule::ReleaseEntry;
use crate::services::anime_identity;
use crate::services::external::shikimori::ShikimoriService;
use crate::services::external::tmdb::TmdbService;
use crate::services::notifications::TelegramNotifier;
use crate::services::search_families;

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
    tmdb: &TmdbService,
    telegram: &TelegramNotifier,
) {
    // `ensure_fresh` only hits the provider when the stored rows are stale, so
    // a frequent schedule here does not hammer Shikimori.
    if let Err(error) = service.ensure_fresh(shikimori).await {
        tracing::error!(error = %error, "release_schedule: calendar refresh failed");
    }

    // Phase 2: write TMDB next-episode rows. Best effort.
    if let Err(error) = service.refresh_from_tmdb(tmdb).await {
        tracing::error!(error = %error, "release_schedule: tmdb refresh failed");
    }

    // Phase 1: fill in shikimori_id for tracked MAL anime so the OR-arm below
    // can match their releases. Best effort, no-op when nothing is unresolved.
    if let Err(error) = anime_identity::resolve_tracked_anime(service.pool(), shikimori).await {
        tracing::error!(error = %error, "release_schedule: anime identity resolution failed");
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

    /// Read-only access to the pool for collaborators that run their own SQL
    /// (the anime identity resolver).
    pub fn pool(&self) -> &PgPool {
        &self.db
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
                INSERT INTO release_schedule
                    (provider, external_id, season_number, episode_number, air_date, title, poster_url, fetched_at)
                VALUES ($1, $2, 0, $3, $4, $5, $6, NOW())
                ON CONFLICT (provider, external_id, season_number, episode_number)
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

    /// Phase 2: write `next_episode_to_air` rows for tracked TMDB series.
    ///
    /// Best effort per series: a TMDB failure is logged and skipped. Rows are
    /// stored at 12:00 UTC of the air date (TMDB only supplies a date). The
    /// title/poster come from the tracked `media_items` row, so no metadata is
    /// re-fetched and `CreateMediaItem` is untouched.
    pub async fn refresh_from_tmdb(&self, tmdb: &TmdbService) -> Result<(), anyhow::Error> {
        let candidates: Vec<(String, String, Option<String>)> = sqlx::query_as(
            r#"
            SELECT DISTINCT m.external_id, m.title, m.poster_url
            FROM media_items m
            JOIN tracking_entries t ON t.media_id = m.id
            WHERE m.provider = 'tmdb'
              AND m.media_type IN ('series', 'dramas', 'cartoons')
              AND t.status IN ('in_progress', 'planned')
            LIMIT 50
            "#,
        )
        .fetch_all(&self.db)
        .await?;

        for (external_id, title, poster) in &candidates {
            match tmdb.fetch_next_episode(external_id).await {
                Ok(Some(next)) => {
                    let Some(air_date) = next
                        .air_date
                        .as_deref()
                        .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
                    else {
                        continue;
                    };
                    let air_ts = air_date
                        .and_hms_opt(12, 0, 0)
                        .expect("12:00 is a valid time")
                        .and_utc();

                    if let Err(error) = sqlx::query(
                        r#"
                        INSERT INTO release_schedule
                            (provider, external_id, season_number, episode_number, air_date, title, poster_url, fetched_at)
                        VALUES ('tmdb', $1, $2, $3, $4, $5, $6, NOW())
                        ON CONFLICT (provider, external_id, season_number, episode_number)
                        DO UPDATE SET air_date = EXCLUDED.air_date,
                                      title = EXCLUDED.title,
                                      poster_url = EXCLUDED.poster_url,
                                      fetched_at = NOW()
                        "#,
                    )
                    .bind(external_id)
                    .bind(next.season_number)
                    .bind(next.episode_number)
                    .bind(air_ts)
                    .bind(title)
                    .bind(poster)
                    .execute(&self.db)
                    .await
                    {
                        tracing::warn!(
                            external_id,
                            error = %error,
                            "release_schedule: failed to upsert tmdb episode"
                        );
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(
                        external_id,
                        error = %error,
                        "release_schedule: tmdb next episode fetch failed"
                    );
                }
            }

            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
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
        type Row = (
            String,
            String,
            String,
            Option<String>,
            i32,
            i32,
            DateTime<Utc>,
        );
        #[allow(clippy::type_complexity)]
        let rows: Vec<Row> = sqlx::query_as(
            r#"
            SELECT DISTINCT
                r.provider, r.external_id, r.title, r.poster_url,
                r.season_number, r.episode_number, r.air_date
            FROM release_schedule r
            JOIN tracking_entries t ON t.user_id = $1
            JOIN media_items m ON m.id = t.media_id
                AND (
                    (m.provider = r.provider AND m.external_id = r.external_id)
                    OR (r.provider = 'shikimori'
                        AND m.shikimori_id IS NOT NULL
                        AND m.shikimori_id::text = r.external_id)
                )
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
            .map(|(p, eid, title, poster, season, ep, date)| ReleaseEntry {
                provider: p,
                external_id: eid,
                title,
                poster_url: poster,
                season_number: season,
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
        type Row = (
            String,
            String,
            String,
            Option<String>,
            i32,
            i32,
            DateTime<Utc>,
        );
        #[allow(clippy::type_complexity)]
        let rows: Vec<Row> = sqlx::query_as(
            r#"
            SELECT DISTINCT
                r.provider, r.external_id, r.title, r.poster_url,
                r.season_number, r.episode_number, r.air_date
            FROM release_schedule r
            JOIN tracking_entries t ON t.user_id = $1
            JOIN media_items m ON m.id = t.media_id
                AND (
                    (m.provider = r.provider AND m.external_id = r.external_id)
                    OR (r.provider = 'shikimori'
                        AND m.shikimori_id IS NOT NULL
                        AND m.shikimori_id::text = r.external_id)
                )
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
            .map(|(p, eid, title, poster, season, ep, date)| ReleaseEntry {
                provider: p,
                external_id: eid,
                title,
                poster_url: poster,
                season_number: season,
                episode_number: ep,
                air_date: date,
            })
            .collect())
    }

    /// Unified tracked-release feed for one month.
    ///
    /// Merges episodes (`release_schedule`) with premieres
    /// (`media_items.aired_from`), dedups (episode wins), keeps every tracking
    /// status except `dropped`, computes per-family counts over the whole month,
    /// groups visible events into ISO weeks clamped to the month, and appends a
    /// year section for `planned` media that has a year but no exact date.
    ///
    /// This is the ONLY place the calendar status filter is widened; the home
    /// widget (`get_upcoming_for_user`) and notifications keep their original
    /// statuses.
    pub async fn get_calendar_month(
        &self,
        user_id: Uuid,
        year: i32,
        month: u32,
        family: &str,
    ) -> Result<CalendarMonthView, anyhow::Error> {
        let first = NaiveDate::from_ymd_opt(year, month, 1)
            .ok_or_else(|| anyhow::anyhow!("invalid year/month {year}-{month}"))?;
        let first_next = if month == 12 {
            NaiveDate::from_ymd_opt(year + 1, 1, 1)
        } else {
            NaiveDate::from_ymd_opt(year, month + 1, 1)
        }
        .ok_or_else(|| anyhow::anyhow!("invalid month {month}"))?;
        let from = first.and_hms_opt(0, 0, 0).expect("valid time").and_utc();
        let to = first_next.and_hms_opt(0, 0, 0).expect("valid time").and_utc();
        let today = Utc::now().date_naive();

        // --- Episodes -------------------------------------------------------
        type EpisodeRow = (
            String,
            String,
            String,
            Option<String>,
            i32,
            i32,
            DateTime<Utc>,
            String,
            String,
            Option<i64>,
            String,
        );
        #[allow(clippy::type_complexity)]
        let episode_rows: Vec<EpisodeRow> = sqlx::query_as(
            r#"
            SELECT DISTINCT
                r.provider, r.external_id, m.title, r.poster_url,
                r.season_number, r.episode_number, r.air_date,
                m.provider, m.external_id, m.shikimori_id, m.media_type
            FROM release_schedule r
            JOIN tracking_entries t ON t.user_id = $1
            JOIN media_items m ON m.id = t.media_id
                AND (
                    (m.provider = r.provider AND m.external_id = r.external_id)
                    OR (r.provider = 'shikimori'
                        AND m.shikimori_id IS NOT NULL
                        AND m.shikimori_id::text = r.external_id)
                )
            WHERE t.status <> 'dropped'
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

        let episodes: Vec<CalendarEvent> = episode_rows
            .into_iter()
            .map(
                |(
                    r_provider,
                    r_external_id,
                    m_title,
                    r_poster,
                    season,
                    episode,
                    air_date,
                    m_provider,
                    m_external_id,
                    shikimori_id,
                    media_type,
                )| {
                    let date = air_date.date_naive();
                    CalendarEvent {
                        // Drawer must target the tracked media_items row.
                        provider: m_provider,
                        external_id: m_external_id,
                        family: search_families::family_of(&media_type),
                        canonical_key: canonical_key(
                            shikimori_id,
                            &r_provider,
                            &r_external_id,
                        ),
                        title: m_title,
                        poster_url: r_poster,
                        date: Some(date),
                        kind: CalendarEventKind::Episode {
                            season_number: season,
                            episode_number: episode,
                        },
                        media_type,
                        is_past: date < today,
                    }
                },
            )
            .collect();

        // --- Premieres ------------------------------------------------------
        type PremiereRow = (String, String, String, String, Option<String>, NaiveDate, Option<i64>);
        #[allow(clippy::type_complexity)]
        let premiere_rows: Vec<PremiereRow> = sqlx::query_as(
            r#"
            SELECT DISTINCT
                m.provider, m.external_id, m.media_type, m.title,
                m.poster_url, m.aired_from, m.shikimori_id
            FROM media_items m
            JOIN tracking_entries t ON t.media_id = m.id
            WHERE t.user_id = $1
              AND t.status <> 'dropped'
              AND m.aired_from IS NOT NULL
              AND m.aired_from >= $2
              AND m.aired_from < $3
            ORDER BY m.aired_from ASC
            "#,
        )
        .bind(user_id)
        .bind(first)
        .bind(first_next)
        .fetch_all(&self.db)
        .await?;

        let premieres: Vec<CalendarEvent> = premiere_rows
            .into_iter()
            .map(
                |(provider, external_id, media_type, title, poster_url, aired_from, shikimori_id)| {
                    CalendarEvent {
                        family: search_families::family_of(&media_type),
                        canonical_key: canonical_key(shikimori_id, &provider, &external_id),
                        provider,
                        external_id,
                        media_type,
                        title,
                        poster_url,
                        date: Some(aired_from),
                        kind: CalendarEventKind::Premiere,
                        is_past: aired_from < today,
                    }
                },
            )
            .collect();

        let all_events = merge_and_dedup(episodes, premieres);

        // --- Year section ---------------------------------------------------
        type YearRow = (String, String, String, String, Option<String>, Option<i64>);
        #[allow(clippy::type_complexity)]
        let year_rows: Vec<YearRow> = sqlx::query_as(
            r#"
            SELECT DISTINCT m.provider, m.external_id, m.media_type, m.title,
                            m.poster_url, m.shikimori_id
            FROM media_items m
            JOIN tracking_entries t ON t.media_id = m.id
            WHERE t.user_id = $1
              AND t.status = 'planned'
              AND m.year = $2
              AND m.aired_from IS NULL
              AND NOT EXISTS (
                  SELECT 1 FROM release_schedule r
                  WHERE (r.provider = m.provider AND r.external_id = m.external_id)
                     OR (r.provider = 'shikimori'
                         AND m.shikimori_id IS NOT NULL
                         AND m.shikimori_id::text = r.external_id)
              )
            ORDER BY m.title ASC
            "#,
        )
        .bind(user_id)
        .bind(year as i16)
        .fetch_all(&self.db)
        .await?;

        let year_all: Vec<CalendarEvent> = year_rows
            .into_iter()
            .map(|(provider, external_id, media_type, title, poster_url, shikimori_id)| {
                CalendarEvent {
                    family: search_families::family_of(&media_type),
                    canonical_key: canonical_key(shikimori_id, &provider, &external_id),
                    provider,
                    external_id,
                    media_type,
                    title,
                    poster_url,
                    date: None,
                    kind: CalendarEventKind::Premiere,
                    is_past: false,
                }
            })
            .collect();

        // --- Counts (whole month, before family filtering) ------------------
        let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
        for event in &all_events {
            *counts.entry(event.family).or_insert(0) += 1;
        }

        let month_qs = format!("?year={year}&month={month}");
        let mut family_tabs: Vec<FamilyTab> =
            Vec::with_capacity(search_families::families().len() + 1);
        family_tabs.push(FamilyTab {
            key: search_families::ALL_TAB,
            label: search_families::all_label().to_string(),
            count: all_events.len(),
            href: format!("/calendar{month_qs}"),
            active: family.is_empty(),
        });
        for f in search_families::families() {
            family_tabs.push(FamilyTab {
                key: f.key,
                label: f.label.to_string(),
                count: counts.get(f.key).copied().unwrap_or(0),
                href: format!("/calendar{month_qs}&family={}", f.key),
                active: f.key == family,
            });
        }

        // --- Filter + group -------------------------------------------------
        let visible = filter_by_family(&all_events, family);
        let weeks = group_into_weeks(visible);
        let year_section = filter_by_family(&year_all, family);
        let is_empty = weeks.is_empty() && year_section.is_empty();

        Ok(CalendarMonthView {
            weeks,
            year_section,
            year_section_title: format!("Выйдет когда-то в {year}"),
            family_tabs,
            is_empty,
        })
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
            // currently watching. DISTINCT collapses rows duplicated when the
            // same title is tracked through both MAL and Shikimori. Dedup
            // across passes is still enforced by the atomic claim below, whose
            // key stays (user, provider, external_id, episode_number).
            let recent: Vec<(String, String, i32, i32, String)> = sqlx::query_as(
                r#"
                SELECT DISTINCT
                    r.provider, r.title, r.season_number, r.episode_number, r.external_id
                FROM release_schedule r
                JOIN tracking_entries t ON t.user_id = $1
                JOIN media_items m ON m.id = t.media_id
                    AND (
                        (m.provider = r.provider AND m.external_id = r.external_id)
                        OR (r.provider = 'shikimori'
                            AND m.shikimori_id IS NOT NULL
                            AND m.shikimori_id::text = r.external_id)
                    )
                WHERE t.status = 'in_progress'
                  AND r.air_date >= NOW() - INTERVAL '2 hours'
                  AND r.air_date < NOW()
                "#,
            )
            .bind(user_id)
            .fetch_all(&self.db)
            .await?;

            for (provider, title, season, episode, external_id) in &recent {
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
                    .send_new_episode_notification(chat_id, title, *season, *episode)
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
