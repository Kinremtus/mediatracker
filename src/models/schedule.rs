use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ReleaseEntry {
    pub provider: String,
    pub external_id: String,
    pub title: String,
    pub poster_url: Option<String>,
    /// 0 means "no season information" (Shikimori rows, legacy rows).
    pub season_number: i32,
    pub episode_number: i32,
    pub air_date: DateTime<Utc>,
}

impl ReleaseEntry {
    pub fn day(&self) -> u32 {
        self.air_date.format("%d").to_string().parse().unwrap_or(0)
    }

    pub fn month(&self) -> String {
        self.air_date.format("%b").to_string()
    }

    pub fn poster_or_placeholder(&self) -> &str {
        self.poster_url
            .as_deref()
            .unwrap_or("/static/images/placeholders/poster.svg")
    }

    /// `"S2 E5"` when the season is known, otherwise `"E12"`.
    pub fn episode_label(&self) -> String {
        if self.season_number > 0 {
            format!("S{} E{}", self.season_number, self.episode_number)
        } else {
            format!("E{}", self.episode_number)
        }
    }
}

#[derive(Debug, Clone)]
pub struct CalendarDay {
    pub date: NaiveDate,
    pub day_num: u32,
    pub is_current_month: bool,
    pub is_today: bool,
    pub releases: Vec<ReleaseEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(season: i32, episode: i32) -> ReleaseEntry {
        ReleaseEntry {
            provider: "tmdb".to_string(),
            external_id: "1".to_string(),
            title: "Show".to_string(),
            poster_url: None,
            season_number: season,
            episode_number: episode,
            air_date: Utc::now(),
        }
    }

    #[test]
    fn episode_label_uses_season_when_positive() {
        assert_eq!(entry(2, 5).episode_label(), "S2 E5");
    }

    #[test]
    fn episode_label_falls_back_to_episode_only() {
        assert_eq!(entry(0, 12).episode_label(), "E12");
    }
}
