use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct MediaItem {
    #[sqlx(rename = "media_id")]
    pub id: Uuid,
    pub provider: String,
    pub external_id: String,
    pub media_type: String,
    pub title: String,
    pub title_english: Option<String>,
    pub title_native: Option<String>,
    pub title_russian: Option<String>,
    pub poster_url: Option<String>,
    pub color_hex: Option<String>,
    pub episodes: Option<i32>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub score: Option<f64>,
    #[sqlx(rename = "media_created_at")]
    pub created_at: DateTime<Utc>,
    #[sqlx(rename = "media_updated_at")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateMediaItem {
    pub provider: String,
    pub external_id: String,
    pub media_type: String,
    pub title: String,
    pub title_english: Option<String>,
    pub title_native: Option<String>,
    pub title_russian: Option<String>,
    pub poster_url: Option<String>,
    pub episodes: Option<i32>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub score: Option<f64>,
    #[serde(default)]
    pub is_tracked: bool,
    #[serde(default)]
    pub mal_id: Option<i64>,
    #[serde(default)]
    pub comparison_key: Option<String>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct MediaItemSlim {
    pub provider: String,
    pub external_id: String,
    pub media_type: String,
    pub title: String,
    pub title_english: Option<String>,
    pub title_native: Option<String>,
    pub title_russian: Option<String>,
    pub poster_url: Option<String>,
    pub episodes: Option<i32>,
    pub description: Option<String>,
    #[sqlx(rename = "media_status")]
    pub status: Option<String>,
    pub score: Option<f64>,
}

impl MediaItem {
    pub fn score_class(&self) -> &'static str {
        match self.score {
            Some(s) if s <= 3.0 => "score-1",
            Some(s) if s <= 5.0 => "score-4",
            Some(s) if s <= 7.0 => "score-6",
            Some(s) if s <= 9.0 => "score-8",
            Some(_) => "score-10",
            None => "",
        }
    }
}

impl MediaItemSlim {
    pub fn score_class(&self) -> &'static str {
        match self.score {
            Some(s) if s <= 3.0 => "score-1",
            Some(s) if s <= 5.0 => "score-4",
            Some(s) if s <= 7.0 => "score-6",
            Some(s) if s <= 9.0 => "score-8",
            Some(_) => "score-10",
            None => "",
        }
    }
}

impl CreateMediaItem {
    pub fn score_class(&self) -> &'static str {
        match self.score {
            Some(s) if s <= 3.0 => "score-1",
            Some(s) if s <= 5.0 => "score-4",
            Some(s) if s <= 7.0 => "score-6",
            Some(s) if s <= 9.0 => "score-8",
            Some(_) => "score-10",
            None => "",
        }
    }
}
