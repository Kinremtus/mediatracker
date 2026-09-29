use sqlx::PgPool;
use uuid::Uuid;

use crate::models::media_item::{CreateMediaItem, MediaItem};
use crate::models::tracking_entry::{TrackingEntry, TrackingEntryWithMedia, UpdateTracking};
use crate::services::episodes;
use crate::services::tmdb_episodes;

#[derive(Clone)]
pub struct TrackingService {
    db: PgPool,
}

impl TrackingService {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    pub async fn add_to_list(
        &self,
        user_id: Uuid,
        media: &CreateMediaItem,
        status: &str,
    ) -> Result<TrackingEntry, anyhow::Error> {
        // Find-or-create media, tracking entry and activity row atomically.
        // A plain SELECT-then-INSERT would let two concurrent adds for the
        // same title both miss and then collide on UNIQUE(provider,
        // external_id); a single upsert closes that window. The no-op update
        // keeps whatever metadata already exists and only bumps updated_at.
        let mut tx = self.db.begin().await?;

        let media_id = sqlx::query_scalar::<_, Uuid>(
            r#"
            INSERT INTO media_items (
                provider, external_id, media_type, title, title_english, title_native, title_russian,
                poster_url, episodes, description, status, score,
                format_type, details,
                chapters, volumes, pages, runtime_minutes, playtime_hours,
                year, aired_from, aired_to, premiered_season, premiered_year, broadcast,
                completed, licensed,
                source, duration, rating, rating_votes,
                authors, artists, studios, producers, licensors, publishers,
                serialized_in, networks, platforms,
                genres, themes, demographics, categories,
                mal_id, shikimori_id
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7,
                $8, $9, $10, $11, $12,
                $13, $14,
                $15, $16, $17, $18, $19,
                $20, $21, $22, $23, $24, $25,
                $26, $27,
                $28, $29, $30, $31,
                $32, $33, $34, $35, $36, $37,
                $38, $39, $40,
                $41, $42, $43, $44,
                $45, $46
            )
            ON CONFLICT (provider, external_id) DO UPDATE
            SET updated_at = NOW()
            RETURNING id
            "#,
        )
        .bind(&media.provider)
        .bind(&media.external_id)
        .bind(&media.media_type)
        .bind(&media.title)
        .bind(&media.title_english)
        .bind(&media.title_native)
        .bind(&media.title_russian)
        .bind(&media.poster_url)
        .bind(media.episodes)
        .bind(&media.description)
        .bind(&media.status)
        .bind(media.score)
        .bind(&media.format_type)
        .bind(media.details.clone().unwrap_or(serde_json::Value::Object(Default::default())))
        .bind(media.chapters)
        .bind(media.volumes)
        .bind(media.pages)
        .bind(media.runtime_minutes)
        .bind(media.playtime_hours)
        .bind(media.year)
        .bind(media.aired_from)
        .bind(media.aired_to)
        .bind(&media.premiered_season)
        .bind(media.premiered_year)
        .bind(&media.broadcast)
        .bind(media.completed)
        .bind(media.licensed)
        .bind(&media.source)
        .bind(&media.duration)
        .bind(&media.rating)
        .bind(media.rating_votes)
        .bind(&media.authors)
        .bind(&media.artists)
        .bind(&media.studios)
        .bind(&media.producers)
        .bind(&media.licensors)
        .bind(&media.publishers)
        .bind(&media.serialized_in)
        .bind(&media.networks)
        .bind(&media.platforms)
        .bind(&media.genres)
        .bind(&media.themes)
        .bind(&media.demographics)
        .bind(&media.categories)
        .bind(media.mal_id)
        .bind(media.shikimori_id)
        .fetch_one(&mut *tx)
        .await?;

        // Create tracking entry
        let entry = sqlx::query_as::<_, TrackingEntry>(
            "INSERT INTO tracking_entries (user_id, media_id, status) VALUES ($1, $2, $3) ON CONFLICT (user_id, media_id) DO UPDATE SET status = $3, updated_at = NOW() RETURNING *",
        )
        .bind(user_id)
        .bind(media_id)
        .bind(status)
        .fetch_one(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO activity_log (user_id, action, media_id) VALUES ($1, 'added', $2)",
        )
        .bind(user_id)
        .bind(media_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(entry)
    }

    pub async fn update_entry(
        &self,
        entry_id: Uuid,
        user_id: Uuid,
        data: &UpdateTracking,
    ) -> Result<TrackingEntry, anyhow::Error> {
        let mut qb = sqlx::QueryBuilder::new("UPDATE tracking_entries SET ");
        {
            let mut sep = qb.separated(", ");
            let mut has_field = false;
            if let Some(status) = &data.status {
                sep.push("status = ");
                sep.push_bind_unseparated(status);
                has_field = true;
            }
            if let Some(rating) = data.rating {
                sep.push("rating = ");
                sep.push_bind_unseparated(rating);
                has_field = true;
            }
            if let Some(progress) = data.progress {
                sep.push("progress = ");
                sep.push_bind_unseparated(progress);
                has_field = true;
            }
            if !has_field {
                return Err(anyhow::anyhow!("No fields to update"));
            }
            sep.push("updated_at = NOW()");
        }
        qb.push(" WHERE id = ");
        qb.push_bind(entry_id);
        qb.push(" AND user_id = ");
        qb.push_bind(user_id);
        // Cast rating to float8 so sqlx can decode it into TrackingEntry's
        // Option<f64>. tracking_entries.rating is NUMERIC(2,1) by the
        // migration; decoding NUMERIC -> f64 needs the bigdecimal feature
        // which we don't enable. get_user_entries already does this cast
        // (see the SELECT in that fn) — we forgot it here, so an update on
        // any entry with a rating set would 500 with 'mismatched types'
        // even though the UPDATE itself succeeded. The 500 then caused
        // htmx_update_tracking to issue a 303 Redirect, leaving UI and DB
        // inconsistent.
        qb.push(
            " RETURNING id, user_id, media_id, status, \
             rating::double precision AS rating, \
             progress, created_at, updated_at",
        );

        let mut tx = self.db.begin().await?;

        let entry = qb
            .build_query_as::<TrackingEntry>()
            .fetch_one(&mut *tx)
            .await?;

        sqlx::query(
            "INSERT INTO activity_log (user_id, action, media_id) VALUES ($1, 'updated', $2)",
        )
        .bind(user_id)
        .bind(entry.media_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        // Best-effort: mirror the new progress onto this user's own per-episode
        // rows so the drawer checkboxes stay consistent with the tracking
        // card/form (the reverse of set_watched + update_progress_from_watched).
        // TMDB rows live in user_tmdb_episode_progress; anime rows live in
        // user_episode_progress under the card's own (provider, external_id)
        // key. Logged, never fatal — the tracking update is already committed.
        if let Some(progress) = data.progress {
            match sqlx::query_as::<_, (String, String, String, Option<i64>)>(
                "SELECT provider, external_id, media_type, mal_id \
                 FROM media_items WHERE id = $1",
            )
            .bind(entry.media_id)
            .fetch_optional(&self.db)
            .await
            {
                Ok(Some((provider, external_id, media_type, mal_id))) => {
                    if media_type == "anime" {
                        // Prefer the dedicated mal_id; MAL-sourced cards also
                        // store the MAL id as external_id, so fall back to it
                        // when the column is empty. Skip when neither is
                        // available — the catalog cannot be located.
                        let resolved_mal_id = mal_id.or_else(|| {
                            if provider == "mal" {
                                external_id.parse::<i64>().ok()
                            } else {
                                None
                            }
                        });
                        if let Some(mal_id) = resolved_mal_id
                            && let Err(e) = episodes::sync_watched_from_progress(
                                &self.db,
                                user_id,
                                &provider,
                                &external_id,
                                mal_id,
                                progress,
                            )
                            .await
                        {
                            tracing::warn!(
                                provider = %provider,
                                external_id = %external_id,
                                mal_id,
                                error = %e,
                                "anime progress sync failed"
                            );
                        }
                    } else if provider == "tmdb"
                        && let Err(e) = tmdb_episodes::sync_tmdb_episodes_from_progress(
                            &self.db,
                            user_id,
                            &external_id,
                            progress,
                        )
                        .await
                    {
                        tracing::warn!(
                            external_id = %external_id,
                            error = %e,
                            "tmdb progress sync failed"
                        );
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(
                        media_id = %entry.media_id,
                        error = %e,
                        "media lookup for progress sync failed"
                    );
                }
            }
        }

        Ok(entry)
    }

    pub async fn delete_entry(&self, entry_id: Uuid, user_id: Uuid) -> Result<(), anyhow::Error> {
        let media_id: Option<Uuid> = sqlx::query_scalar(
            "SELECT media_id FROM tracking_entries WHERE id = $1 AND user_id = $2",
        )
        .bind(entry_id)
        .bind(user_id)
        .fetch_optional(&self.db)
        .await?;

        if let Some(media_id) = media_id {
            let _ = sqlx::query(
                "INSERT INTO activity_log (user_id, action, media_id) VALUES ($1, 'deleted', $2)",
            )
            .bind(user_id)
            .bind(media_id)
            .execute(&self.db)
            .await;

            sqlx::query("DELETE FROM tracking_entries WHERE id = $1 AND user_id = $2")
                .bind(entry_id)
                .bind(user_id)
                .execute(&self.db)
                .await?;
        }

        Ok(())
    }

    pub async fn get_status_counts(
        &self,
        user_id: Uuid,
    ) -> Result<(i32, i32, i32, i32), anyhow::Error> {
        let rows: Vec<(String, i32)> = sqlx::query_as(
            "SELECT status, COUNT(*)::int as count FROM tracking_entries WHERE user_id = $1 GROUP BY status",
        )
        .bind(user_id)
        .fetch_all(&self.db)
        .await?;

        let mut in_progress = 0i32;
        let mut completed = 0i32;
        let mut planned = 0i32;
        let mut dropped = 0i32;
        for (status, count) in rows {
            match status.as_str() {
                "in_progress" => in_progress = count,
                "completed" => completed = count,
                "planned" => planned = count,
                "dropped" => dropped = count,
                _ => {}
            }
        }
        Ok((in_progress, completed, planned, dropped))
    }

    pub async fn find_entry_by_media(
        &self,
        user_id: Uuid,
        provider: &str,
        external_id: &str,
    ) -> Result<Option<(Uuid, String, i32, Option<f64>)>, anyhow::Error> {
        let row: Option<(Uuid, String, i32, Option<f64>)> = sqlx::query_as(
            "SELECT te.id, te.status, te.progress, te.rating::double precision FROM tracking_entries te
             JOIN media_items mi ON te.media_id = mi.id
             WHERE te.user_id = $1 AND mi.provider = $2 AND mi.external_id = $3",
        )
        .bind(user_id)
        .bind(provider)
        .bind(external_id)
        .fetch_optional(&self.db)
        .await?;
        Ok(row)
    }

    /// Загрузить локальную строку `media_items` по `(provider, external_id)`.
    ///
    /// Это источник истины для карточки: читаем её первой, чтобы тайтл
    /// рендерился, даже если провайдер удалил запись (этап 4), и чтобы
    /// рендерились `provider = "manual"` строки, у которых провайдера нет
    /// вообще.
    ///
    /// SELECT алиасит `id`/`created_at`/`updated_at` под имена, которые ждут
    /// `#[sqlx(rename = ...)]` в `MediaItem`, и кастит `score` в
    /// `double precision` (иначе NUMERIC -> f64 требует bigdecimal — см.
    /// `update_entry`).
    pub async fn find_media_item(
        &self,
        provider: &str,
        external_id: &str,
    ) -> Result<Option<MediaItem>, anyhow::Error> {
        let row = sqlx::query_as::<_, MediaItem>(
            r#"
            SELECT
                id                      AS media_id,
                provider, external_id, media_type, title,
                title_english, title_native, title_russian, poster_url, color_hex,
                episodes, description, status,
                score::double precision AS score,
                created_at              AS media_created_at,
                updated_at              AS media_updated_at,
                format_type, details,
                chapters, volumes, pages, runtime_minutes, playtime_hours,
                year, aired_from, aired_to, premiered_season, premiered_year, broadcast,
                completed, licensed, source, duration, rating, rating_votes,
                authors, artists, studios, producers, licensors, publishers,
                serialized_in, networks, platforms, genres, themes, demographics, categories
            FROM media_items
            WHERE provider = $1 AND external_id = $2
            "#,
        )
        .bind(provider)
        .bind(external_id)
        .fetch_optional(&self.db)
        .await?;
        Ok(row)
    }

    /// Find an existing `media_items` row with the same media type and a
    /// case-insensitive, whitespace-trimmed title. Used to block manual
    /// duplicates before insert (design workstream 2). `media_type` is part
    /// of the key on purpose: the same title may legitimately exist as two
    /// different media types.
    pub async fn find_duplicate(
        &self,
        media_type: &str,
        title: &str,
    ) -> Result<Option<(String, String)>, anyhow::Error> {
        let row: Option<(String, String)> = sqlx::query_as(
            r#"
            SELECT provider, external_id
            FROM media_items
            WHERE media_type = $1
              AND lower(btrim(title)) = lower(btrim($2))
            LIMIT 1
            "#,
        )
        .bind(media_type)
        .bind(title)
        .fetch_optional(&self.db)
        .await?;
        Ok(row)
    }

    /// Return the `(provider, external_id)` pairs the user is tracking.
    ///
    /// Batched alternative to calling [`Self::find_entry_by_media`] per search
    /// result (N+1). One round-trip covers any number of pairs.
    pub async fn find_tracked_media(
        &self,
        user_id: Uuid,
        pairs: &[(String, String)],
    ) -> Result<std::collections::HashSet<(String, String)>, anyhow::Error> {
        if pairs.is_empty() {
            return Ok(std::collections::HashSet::new());
        }

        let providers: Vec<String> = pairs.iter().map(|(p, _)| p.clone()).collect();
        let external_ids: Vec<String> = pairs.iter().map(|(_, e)| e.clone()).collect();

        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT mi.provider, mi.external_id \
             FROM tracking_entries te \
             JOIN media_items mi ON mi.id = te.media_id \
             WHERE te.user_id = $1 \
               AND (mi.provider, mi.external_id) IN (\
                   SELECT * FROM unnest($2::text[], $3::text[])\
               )",
        )
        .bind(user_id)
        .bind(&providers)
        .bind(&external_ids)
        .fetch_all(&self.db)
        .await?;

        Ok(rows.into_iter().collect())
    }

    /// Shared SELECT column list for tracking entries joined with media items.
    const ENTRY_SELECT: &'static str = "SELECT tracking_entries.id, tracking_entries.user_id, tracking_entries.media_id, \
         tracking_entries.status, tracking_entries.rating::double precision AS rating, \
         tracking_entries.progress, tracking_entries.created_at, tracking_entries.updated_at, \
         media_items.provider, media_items.external_id, media_items.media_type, \
         media_items.title, media_items.title_english, media_items.title_native, \
         media_items.title_russian, media_items.poster_url, media_items.episodes, \
         media_items.description, media_items.status AS media_status, \
         media_items.score::double precision AS score, \
         media_items.format_type, media_items.chapters, media_items.volumes, media_items.pages, \
         media_items.runtime_minutes, media_items.playtime_hours, \
         media_items.authors, media_items.artists, media_items.studios, media_items.publishers, \
         media_items.genres, media_items.themes, media_items.year \
         FROM tracking_entries \
         JOIN media_items ON tracking_entries.media_id = media_items.id";

    pub async fn get_user_entries(
        &self,
        user_id: Uuid,
        status: Option<&str>,
        media_type: Option<&str>,
        search_query: Option<&str>,
    ) -> Result<Vec<TrackingEntryWithMedia>, anyhow::Error> {
        let mut qb = sqlx::QueryBuilder::new(Self::ENTRY_SELECT);
        qb.push(" WHERE tracking_entries.user_id = ")
            .push_bind(user_id);

        if let Some(s) = status {
            qb.push(" AND tracking_entries.status = ").push_bind(s);
        }
        if let Some(mt) = media_type {
            qb.push(" AND media_items.media_type = ").push_bind(mt);
        }
        if let Some(sq) = search_query
            && !sq.is_empty()
        {
            qb.push(" AND media_items.title ILIKE '%' || ")
                .push_bind(sq)
                .push(" || '%'");
        }

        qb.push(" ORDER BY tracking_entries.updated_at DESC");

        let entries = qb
            .build_query_as::<TrackingEntryWithMedia>()
            .fetch_all(&self.db)
            .await?;
        Ok(entries)
    }

    /// Fetch a single tracked entry (with its media) scoped to its owner.
    /// Returns `Ok(None)` when the entry does not exist or belongs to another user.
    pub async fn get_entry_with_media(
        &self,
        user_id: Uuid,
        entry_id: Uuid,
    ) -> Result<Option<TrackingEntryWithMedia>, anyhow::Error> {
        let mut qb = sqlx::QueryBuilder::new(Self::ENTRY_SELECT);
        qb.push(" WHERE tracking_entries.user_id = ")
            .push_bind(user_id)
            .push(" AND tracking_entries.id = ")
            .push_bind(entry_id);

        let entry = qb
            .build_query_as::<TrackingEntryWithMedia>()
            .fetch_optional(&self.db)
            .await?;
        Ok(entry)
    }
}
