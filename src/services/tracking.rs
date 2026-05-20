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
                // Insert media item
                let new_id = sqlx::query_scalar::<_, Uuid>(
                    "INSERT INTO media_items (provider, external_id, media_type, title, title_english, title_native, title_russian, poster_url, episodes, description, status, score) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12) RETURNING id",
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

        Ok(entry)
    }

    pub async fn update_entry(
        &self,
        entry_id: Uuid,
        user_id: Uuid,
        data: &UpdateTracking,
    ) -> Result<TrackingEntry, anyhow::Error> {
        let mut query = "UPDATE tracking_entries SET ".to_string();
        let mut params = vec![];
        let mut idx = 1;

        if let Some(status) = &data.status {
            query.push_str(&format!("status = ${}, ", idx));
            params.push(status.clone());
            idx += 1;
        }
        if let Some(rating) = data.rating {
            query.push_str(&format!("rating = ${}, ", idx));
            params.push(rating.to_string());
            idx += 1;
        }
        if let Some(progress) = data.progress {
            query.push_str(&format!("progress = ${}, ", idx));
            params.push(progress.to_string());
            idx += 1;
        }

        if params.is_empty() {
            return Err(anyhow::anyhow!("No fields to update"));
        }

        // Remove trailing comma and space
        query.pop();
        query.pop();

        query.push_str(&format!(
            ", updated_at = NOW() WHERE id = ${} AND user_id = ${} RETURNING *",
            idx,
            idx + 1
        ));

        let entry = sqlx::query_as::<_, TrackingEntry>(&query)
            .bind(params)
            .bind(entry_id)
            .bind(user_id)
            .fetch_one(&self.db)
            .await?;

        Ok(entry)
    }

    pub async fn delete_entry(&self, entry_id: Uuid, user_id: Uuid) -> Result<(), anyhow::Error> {
        sqlx::query("DELETE FROM tracking_entries WHERE id = $1 AND user_id = $2")
            .bind(entry_id)
            .bind(user_id)
            .execute(&self.db)
            .await?;
        Ok(())
    }

    pub async fn get_user_entries(
        &self,
        user_id: Uuid,
        status: Option<&str>,
        media_type: Option<&str>,
    ) -> Result<Vec<TrackingEntryWithMedia>, anyhow::Error> {
        let mut query = "SELECT tracking_entries.*, media_items.* FROM tracking_entries JOIN media_items ON tracking_entries.media_id = media_items.id WHERE tracking_entries.user_id = $1".to_string();
        let mut param_idx = 2;

        if let Some(s) = status {
            query.push_str(&format!(" AND tracking_entries.status = ${}", param_idx));
            param_idx += 1;
        }
        if let Some(mt) = media_type {
            query.push_str(&format!(" AND media_items.media_type = ${}", param_idx));
            param_idx += 1;
        }

        query.push_str(" ORDER BY tracking_entries.updated_at DESC");

        let mut q = sqlx::query_as::<_, TrackingEntryWithMedia>(&query).bind(user_id);
        if let Some(s) = status {
            q = q.bind(s);
        }
        if let Some(mt) = media_type {
            q = q.bind(mt);
        }

        let entries = q.fetch_all(&self.db).await?;
        Ok(entries)
    }
}
