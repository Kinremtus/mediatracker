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

    // === Расширенные метаданные (миграция 008) ===
    pub format_type: Option<String>,
    #[sqlx(json)]
    pub details: serde_json::Value,
    pub chapters: Option<i32>,
    pub volumes: Option<i32>,
    pub pages: Option<i32>,
    pub runtime_minutes: Option<i32>,
    pub playtime_hours: Option<i32>,
    pub year: Option<i16>,
    pub aired_from: Option<chrono::NaiveDate>,
    pub aired_to: Option<chrono::NaiveDate>,
    pub premiered_season: Option<String>,
    pub premiered_year: Option<i16>,
    pub broadcast: Option<String>,
    pub completed: Option<bool>,
    pub licensed: Option<bool>,
    pub source: Option<String>,
    pub duration: Option<String>,
    pub rating: Option<String>,
    pub rating_votes: Option<i32>,
    pub authors: Vec<String>,
    pub artists: Vec<String>,
    pub studios: Vec<String>,
    pub producers: Vec<String>,
    pub licensors: Vec<String>,
    pub publishers: Vec<String>,
    pub serialized_in: Vec<String>,
    pub networks: Vec<String>,
    pub platforms: Vec<String>,
    pub genres: Vec<String>,
    pub themes: Vec<String>,
    pub demographics: Vec<String>,
    pub categories: Vec<String>,
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

    // === Расширенные метаданные ===
    #[serde(default)]
    pub format_type: Option<String>,
    #[serde(default)]
    pub details: Option<serde_json::Value>,
    #[serde(default)]
    pub chapters: Option<i32>,
    #[serde(default)]
    pub volumes: Option<i32>,
    #[serde(default)]
    pub pages: Option<i32>,
    #[serde(default)]
    pub runtime_minutes: Option<i32>,
    #[serde(default)]
    pub playtime_hours: Option<i32>,
    #[serde(default)]
    pub year: Option<i16>,
    #[serde(default)]
    pub aired_from: Option<chrono::NaiveDate>,
    #[serde(default)]
    pub aired_to: Option<chrono::NaiveDate>,
    #[serde(default)]
    pub premiered_season: Option<String>,
    #[serde(default)]
    pub premiered_year: Option<i16>,
    #[serde(default)]
    pub broadcast: Option<String>,
    #[serde(default)]
    pub completed: Option<bool>,
    #[serde(default)]
    pub licensed: Option<bool>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub duration: Option<String>,
    #[serde(default)]
    pub rating: Option<String>,
    #[serde(default)]
    pub rating_votes: Option<i32>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub artists: Vec<String>,
    #[serde(default)]
    pub studios: Vec<String>,
    #[serde(default)]
    pub producers: Vec<String>,
    #[serde(default)]
    pub licensors: Vec<String>,
    #[serde(default)]
    pub publishers: Vec<String>,
    #[serde(default)]
    pub serialized_in: Vec<String>,
    #[serde(default)]
    pub networks: Vec<String>,
    #[serde(default)]
    pub platforms: Vec<String>,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub themes: Vec<String>,
    #[serde(default)]
    pub demographics: Vec<String>,
    #[serde(default)]
    pub categories: Vec<String>,
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

    // Поля, нужные для компактного отображения в карточках/drawer
    pub format_type: Option<String>,
    pub chapters: Option<i32>,
    pub volumes: Option<i32>,
    pub pages: Option<i32>,
    pub runtime_minutes: Option<i32>,
    pub playtime_hours: Option<i32>,
    pub authors: Vec<String>,
    pub artists: Vec<String>,
    pub studios: Vec<String>,
    pub publishers: Vec<String>,
    pub genres: Vec<String>,
    pub themes: Vec<String>,
    pub year: Option<i16>,
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

    /// Возвращает "total count" для прогресса в зависимости от media_type.
    /// Используется в UI для подписи "X / Y (эпизодов/глав/страниц/часов)".
    pub fn total_count(&self) -> Option<i32> {
        match self.media_type.as_str() {
            "manga" | "manhwa" | "manhua" | "novel" | "other-comics" => self.chapters,
            "anime" | "series" | "cartoons" | "animated-movies" => self.episodes,
            "book" => self.pages,
            "game" => self.playtime_hours,
            "movie" | "dramas" => self.runtime_minutes,
            _ => self.episodes.or(self.chapters),
        }
    }

    /// Подпись единицы прогресса (для UI на русском).
    pub fn progress_unit_ru(&self) -> &'static str {
        match self.media_type.as_str() {
            "manga" | "manhwa" | "manhua" | "novel" | "other-comics" => "гл.",
            "anime" | "series" | "cartoons" | "animated-movies" => "эп.",
            "book" => "стр.",
            "game" => "ч.",
            "movie" | "dramas" => "мин.",
            _ => "ед.",
        }
    }

    /// true если progress достиг total.
    pub fn progress_complete(&self, progress: i32) -> bool {
        self.total_count().map(|t| progress >= t).unwrap_or(false)
    }
}

impl CreateMediaItem {
    /// true если progress достиг total (по ссылке).
    pub fn progress_complete_ref(&self, progress: &i32) -> bool {
        self.total_count().map(|t| *progress >= t).unwrap_or(false)
    }
}

impl MediaItemSlim {
    /// true если progress достиг total (по ссылке).
    pub fn progress_complete_ref(&self, progress: &i32) -> bool {
        self.total_count().map(|t| *progress >= t).unwrap_or(false)
    }
}

impl MediaItemSlim {
    /// Форматирует длительность (runtime_minutes) в "Хч Хмин" / "Х мин".
    pub fn runtime_human(&self) -> String {
        match self.runtime_minutes {
            Some(m) if m >= 60 => format!("{}ч {}мин", m / 60, m % 60),
            Some(m) => format!("{} мин", m),
            None => String::new(),
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

    /// Возвращает "total count" — см. MediaItemSlim.
    pub fn total_count(&self) -> Option<i32> {
        match self.media_type.as_str() {
            "manga" | "manhwa" | "manhua" | "novel" | "other-comics" => self.chapters,
            "anime" | "series" | "cartoons" | "animated-movies" => self.episodes,
            "book" => self.pages,
            "game" => self.playtime_hours,
            "movie" | "dramas" => self.runtime_minutes,
            _ => self.episodes.or(self.chapters),
        }
    }

    /// true если progress достиг total.
    pub fn progress_complete(&self, progress: i32) -> bool {
        self.total_count().map(|t| progress >= t).unwrap_or(false)
    }

    /// Подпись единицы прогресса (для UI на русском).
    pub fn progress_unit_ru(&self) -> &'static str {
        match self.media_type.as_str() {
            "manga" | "manhwa" | "manhua" | "novel" | "other-comics" => "гл.",
            "anime" | "series" | "cartoons" | "animated-movies" => "эп.",
            "book" => "стр.",
            "game" => "ч.",
            "movie" | "dramas" => "мин.",
            _ => "ед.",
        }
    }

    /// Форматирует длительность (runtime_minutes) в "Хч Хмин" / "Х мин".
    pub fn runtime_human(&self) -> String {
        match self.runtime_minutes {
            Some(m) if m >= 60 => format!("{}ч {}мин", m / 60, m % 60),
            Some(m) => format!("{} мин", m),
            None => String::new(),
        }
    }
}
