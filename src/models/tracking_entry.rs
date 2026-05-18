use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use super::media_item::MediaItem;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct TrackingEntry {
    pub id: Uuid,
    pub user_id: Uuid,
    pub media_id: Uuid,
    pub status: String,
    pub rating: Option<f64>,
    pub progress: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct TrackingEntryWithMedia {
    #[sqlx(flatten)]
    pub entry: TrackingEntry,
    #[sqlx(flatten)]
    pub media: MediaItem,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTracking {
    pub status: Option<String>,
    pub rating: Option<f64>,
    pub progress: Option<i32>,
}
