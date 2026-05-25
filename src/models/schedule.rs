use chrono::{NaiveDate, DateTime, Utc};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ReleaseEntry {
    pub provider: String,
    pub external_id: String,
    pub title: String,
    pub poster_url: Option<String>,
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
        self.poster_url.as_deref().unwrap_or("/static/images/placeholders/poster.svg")
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
