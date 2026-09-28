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

#[derive(Debug, Clone, Default, Deserialize)]
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
    pub shikimori_id: Option<i64>,
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

#[derive(Debug, Clone, Serialize)]
pub struct SearchSuggestion {
    pub provider: String,
    pub external_id: String,
    pub media_type: String,
    pub title: String,
    pub title_english: Option<String>,
    pub poster_url: Option<String>,
    pub year: Option<i16>,
    pub score: Option<f64>,
    pub is_tracked: bool,
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

/// Маппит свободный текст статуса выпуска (от провайдеров) в CSS-класс,
/// совпадающий с цветом соответствующего трекинг-статуса.
/// Используется для бэйджа в drawer/detail, чтобы цвет текста и фона
/// совпадал со смыслом: "Завершено" → зелёный, "В процессе" → персиковый и т.д.
pub fn status_release_class(raw: Option<&str>) -> &'static str {
    let s = match raw {
        Some(s) if !s.is_empty() => s.to_lowercase(),
        _ => return "",
    };
    if s.contains("complete")
        || s.contains("finished")
        || s.contains("released")
        || s.contains("ended")
    {
        "status-completed"
    } else if s.contains("ongoing")
        || s.contains("airing")
        || s.contains("publishing")
        || s.contains("in production")
        || s.contains("returning")
    {
        "status-in_progress"
    } else if s.contains("not yet")
        || s.contains("announced")
        || s.contains("planned")
        || s.contains("anons")
        || s.contains("pending")
    {
        "status-planned"
    } else if s.contains("hiatus") || s.contains("paused") {
        "status-paused"
    } else if s.contains("discontinued")
        || s.contains("cancelled")
        || s.contains("canceled")
        || s.contains("dropped")
    {
        "status-dropped"
    } else {
        ""
    }
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

    pub fn status_class(&self) -> &'static str {
        status_release_class(self.status.as_deref())
    }

    /// Human-readable label для `media_type` (slug) — fallback когда `format_type` пуст.
    pub fn media_type_display(&self) -> &'static str {
        match self.media_type.as_str() {
            "anime" => "Anime",
            "manga" => "Manga",
            "manhwa" => "Manhwa",
            "manhua" => "Manhua",
            "novel" => "Novel",
            "movie" => "Movie",
            "series" => "TV Series",
            "dramas" => "Drama",
            "cartoons" => "Cartoon",
            "animated-movies" => "Animated Movie",
            "game" => "Game",
            "book" => "Book",
            "other-comics" => "Other Comics",
            "comic" => "Comic",
            _ => "Other",
        }
    }

    /// Лейбл для UI: `format_type` (от провайдера) → fallback на `media_type_display()`.
    pub fn primary_label(&self) -> &str {
        self.format_type
            .as_deref()
            .unwrap_or_else(|| self.media_type_display())
    }

    /// Год для UI: `year` от API → fallback на `aired_from.year` (для Movie,
    /// где MAL/Shikimori не возвращают `year`).
    pub fn display_year(&self) -> Option<i16> {
        self.year.or_else(|| {
            self.aired_from
                .and_then(|d| d.format("%Y").to_string().parse::<i16>().ok())
        })
    }
}

impl From<MediaItem> for CreateMediaItem {
    /// Переиспользовать сохранённую строку `media_items` как DTO карточки.
    ///
    /// Всё, что рендерит карточка, приходит из БД. Поля, которые может дать
    /// только живой fetch (`comparison_key`, `is_tracked`), сбрасываются;
    /// `mal_id`/`shikimori_id` в `MediaItem` отсутствуют и у manual-строк
    /// всегда пусты.
    fn from(m: MediaItem) -> Self {
        Self {
            provider: m.provider,
            external_id: m.external_id,
            media_type: m.media_type,
            title: m.title,
            title_english: m.title_english,
            title_native: m.title_native,
            title_russian: m.title_russian,
            poster_url: m.poster_url,
            episodes: m.episodes,
            description: m.description,
            status: m.status,
            score: m.score,
            is_tracked: false,
            mal_id: None,
            shikimori_id: None,
            comparison_key: None,
            format_type: m.format_type,
            details: Some(m.details),
            chapters: m.chapters,
            volumes: m.volumes,
            pages: m.pages,
            runtime_minutes: m.runtime_minutes,
            playtime_hours: m.playtime_hours,
            year: m.year,
            aired_from: m.aired_from,
            aired_to: m.aired_to,
            premiered_season: m.premiered_season,
            premiered_year: m.premiered_year,
            broadcast: m.broadcast,
            completed: m.completed,
            licensed: m.licensed,
            source: m.source,
            duration: m.duration,
            rating: m.rating,
            rating_votes: m.rating_votes,
            authors: m.authors,
            artists: m.artists,
            studios: m.studios,
            producers: m.producers,
            licensors: m.licensors,
            publishers: m.publishers,
            serialized_in: m.serialized_in,
            networks: m.networks,
            platforms: m.platforms,
            genres: m.genres,
            themes: m.themes,
            demographics: m.demographics,
            categories: m.categories,
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

    pub fn status_class(&self) -> &'static str {
        status_release_class(self.status.as_deref())
    }

    /// Возвращает "total count" для прогресса в зависимости от media_type.
    /// Используется в UI для подписи "X / Y (эпизодов/глав/страниц/часов)".
    pub fn total_count(&self) -> Option<i32> {
        match self.media_type.as_str() {
            "manga" | "manhwa" | "manhua" | "novel" | "other-comics" | "comic" => self.chapters,
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
            "comic" => "вып.",
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

    /// Human-readable label для `media_type` (slug) — fallback когда `format_type` пуст.
    pub fn media_type_display(&self) -> &'static str {
        match self.media_type.as_str() {
            "anime" => "Anime",
            "manga" => "Manga",
            "manhwa" => "Manhwa",
            "manhua" => "Manhua",
            "novel" => "Novel",
            "movie" => "Movie",
            "series" => "TV Series",
            "dramas" => "Drama",
            "cartoons" => "Cartoon",
            "animated-movies" => "Animated Movie",
            "game" => "Game",
            "book" => "Book",
            "other-comics" => "Other Comics",
            "comic" => "Comic",
            _ => "Other",
        }
    }

    /// Лейбл для UI: `format_type` (от провайдера) → fallback на `media_type_display()`.
    pub fn primary_label(&self) -> &str {
        self.format_type
            .as_deref()
            .unwrap_or_else(|| self.media_type_display())
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
    /// Human-readable label для `media_type` (slug) — fallback когда `format_type` пуст.
    pub fn media_type_display(&self) -> &'static str {
        match self.media_type.as_str() {
            "anime" => "Anime",
            "manga" => "Manga",
            "manhwa" => "Manhwa",
            "manhua" => "Manhua",
            "novel" => "Novel",
            "movie" => "Movie",
            "series" => "TV Series",
            "dramas" => "Drama",
            "cartoons" => "Cartoon",
            "animated-movies" => "Animated Movie",
            "game" => "Game",
            "book" => "Book",
            "other-comics" => "Other Comics",
            "comic" => "Comic",
            _ => "Other",
        }
    }

    /// Лейбл для UI: `format_type` (от провайдера) → fallback на `media_type_display()`.
    pub fn primary_label(&self) -> &str {
        self.format_type
            .as_deref()
            .unwrap_or_else(|| self.media_type_display())
    }

    /// Год для UI: `year` от API → fallback на `aired_from.year` (для Movie,
    /// где MAL/Shikimori не возвращают `year`).
    pub fn display_year(&self) -> Option<i16> {
        self.year.or_else(|| {
            self.aired_from
                .and_then(|d| d.format("%Y").to_string().parse::<i16>().ok())
        })
    }

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
            "manga" | "manhwa" | "manhua" | "novel" | "other-comics" | "comic" => self.chapters,
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
            "comic" => "вып.",
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

    pub fn status_class(&self) -> &'static str {
        status_release_class(self.status.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> MediaItem {
        MediaItem {
            id: Uuid::nil(),
            provider: "manual".to_string(),
            external_id: "11111111-1111-1111-1111-111111111111".to_string(),
            media_type: "movie".to_string(),
            title: "Handmade".to_string(),
            title_english: Some("Handmade EN".to_string()),
            title_native: None,
            title_russian: Some("Ручное".to_string()),
            poster_url: Some("https://example.com/p.jpg".to_string()),
            color_hex: Some("#123456".to_string()),
            episodes: None,
            description: Some("desc".to_string()),
            status: Some("Released".to_string()),
            score: Some(7.5),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            format_type: Some("Movie".to_string()),
            details: serde_json::json!({}),
            chapters: None,
            volumes: None,
            pages: None,
            runtime_minutes: Some(120),
            playtime_hours: None,
            year: Some(1999),
            aired_from: None,
            aired_to: None,
            premiered_season: None,
            premiered_year: None,
            broadcast: None,
            completed: None,
            licensed: None,
            source: None,
            duration: None,
            rating: None,
            rating_votes: None,
            authors: vec![],
            artists: vec![],
            studios: vec!["Studio".to_string()],
            producers: vec![],
            licensors: vec![],
            publishers: vec![],
            serialized_in: vec![],
            networks: vec![],
            platforms: vec![],
            genres: vec!["Drama".to_string()],
            themes: vec![],
            demographics: vec![],
            categories: vec![],
        }
    }

    #[test]
    fn media_item_converts_to_card_dto() {
        let c: CreateMediaItem = sample().into();
        assert_eq!(c.provider, "manual");
        assert_eq!(c.media_type, "movie");
        assert_eq!(c.title, "Handmade");
        assert_eq!(c.title_english.as_deref(), Some("Handmade EN"));
        assert_eq!(c.title_russian.as_deref(), Some("Ручное"));
        assert_eq!(c.poster_url.as_deref(), Some("https://example.com/p.jpg"));
        assert_eq!(c.year, Some(1999));
        assert_eq!(c.runtime_minutes, Some(120));
        assert_eq!(c.score, Some(7.5));
        assert_eq!(c.studios, vec!["Studio".to_string()]);
        assert_eq!(c.genres, vec!["Drama".to_string()]);
        // Upstream-only поля сбрасываются.
        assert!(!c.is_tracked);
        assert_eq!(c.comparison_key, None);
        assert_eq!(c.mal_id, None);
        assert_eq!(c.shikimori_id, None);
        // details переносится.
        assert!(c.details.is_some());
    }
}
