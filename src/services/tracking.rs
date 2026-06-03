use sqlx::PgPool;
use uuid::Uuid;

use crate::models::tracking_entry::{TrackingEntry, TrackingEntryWithMedia, UpdateTracking};
use crate::models::media_item::CreateMediaItem;

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
        // First, ensure media item exists in DB
        let media_id = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM media_items WHERE provider = $1 AND external_id = $2",
        )
        .bind(&media.provider)
        .bind(&media.external_id)
        .fetch_optional(&self.db)
        .await?;

        let media_id = match media_id {
            Some(id) => id,
            None => {
                let new_id = sqlx::query_scalar::<_, Uuid>(
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
                        genres, themes, demographics, categories
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
                        $41, $42, $43, $44
                    )
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
                .fetch_one(&self.db)
                .await?;
                new_id
            }
        };

        // Create tracking entry
        let entry = sqlx::query_as::<_, TrackingEntry>(
            "INSERT INTO tracking_entries (user_id, media_id, status) VALUES ($1, $2, $3) ON CONFLICT (user_id, media_id) DO UPDATE SET status = $3, updated_at = NOW() RETURNING *",
        )
        .bind(user_id)
        .bind(media_id)
        .bind(status)
        .fetch_one(&self.db)
        .await?;

        let _ = sqlx::query(
            "INSERT INTO activity_log (user_id, action, media_id) VALUES ($1, 'added', $2)",
        )
        .bind(user_id)
        .bind(media_id)
        .execute(&self.db)
        .await;

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
        qb.push(" RETURNING *");

        let entry = qb
            .build_query_as::<TrackingEntry>()
            .fetch_one(&self.db)
            .await?;

        let _ = sqlx::query(
            "INSERT INTO activity_log (user_id, action, media_id) VALUES ($1, 'updated', $2)",
        )
        .bind(user_id)
        .bind(entry.media_id)
        .execute(&self.db)
        .await;

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

    pub async fn get_status_counts(&self, user_id: Uuid) -> Result<(i32, i32, i32, i32), anyhow::Error> {
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

    pub async fn get_user_entries(
        &self,
        user_id: Uuid,
        status: Option<&str>,
        media_type: Option<&str>,
        search_query: Option<&str>,
    ) -> Result<Vec<TrackingEntryWithMedia>, anyhow::Error> {
        let mut query = String::from(
            "SELECT tracking_entries.id, tracking_entries.user_id, tracking_entries.media_id, \
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
             JOIN media_items ON tracking_entries.media_id = media_items.id \
             WHERE tracking_entries.user_id = $1",
        );
        let mut param_idx = 2;

        if let Some(s) = status {
            query.push_str(&format!(" AND tracking_entries.status = ${}", param_idx));
            param_idx += 1;
        }
        if let Some(mt) = media_type {
            query.push_str(&format!(" AND media_items.media_type = ${}", param_idx));
            param_idx += 1;
        }
        if let Some(sq) = search_query {
            if !sq.is_empty() {
                query.push_str(&format!(" AND media_items.title ILIKE '%' || ${} || '%'", param_idx));
                param_idx += 1;
            }
        }

        query.push_str(" ORDER BY tracking_entries.updated_at DESC");

        let mut q = sqlx::query_as::<_, TrackingEntryWithMedia>(&query).bind(user_id);
        if let Some(s) = status {
            q = q.bind(s);
        }
        if let Some(mt) = media_type {
            q = q.bind(mt);
        }
        if let Some(sq) = search_query {
            if !sq.is_empty() {
                q = q.bind(sq);
            }
        }

        let entries = q.fetch_all(&self.db).await?;
        Ok(entries)
    }
}
