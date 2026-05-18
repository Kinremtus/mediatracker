use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct StatusCount {
    pub status: String,
    pub count: i32,
    pub percentage: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StatsOverview {
    pub total_titles: i32,
    pub status_counts: Vec<StatusCount>,
    pub top_category: Option<String>,
    pub completion_rate: f64,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct ActivityEntry {
    pub action: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct TitleProgress {
    pub id: Uuid,
    pub title: String,
    pub progress: i32,
    pub episodes: Option<i32>,
    pub status: String,
    pub percentage: i32,
}
