use reqwest::Client;
use serde::Deserialize;
use url::Url;

use crate::models::media_item::CreateMediaItem;

const BASE_URL: &str = "https://shikimori.one/api";
const USER_AGENT: &str = "MediaTracker/0.1 (+https://github.com/Kinremtus/mediatracker)";

#[derive(Debug, Deserialize)]
struct ShikimoriSearchResult {
    id: i64,
    name: String,
    name_en: Option<String>,
    russian: Option<String>,
    image: Option<ShikimoriImage>,
    kind: Option<String>,
    #[serde(deserialize_with = "deserialize_optional_f64")]
    score: Option<f64>,
    status: Option<String>,
    episodes: Option<i32>,
    description: Option<String>,
    mal_id: Option<i64>,
}

fn deserialize_optional_f64<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value: Option<serde_json::Value> = Option::deserialize(deserializer)?;
    match value {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::Number(n)) => n
            .as_f64()
            .map(Some)
            .ok_or_else(|| serde::de::Error::custom("invalid number")),
        Some(serde_json::Value::String(s)) => s
            .parse()
            .map(Some)
            .map_err(serde::de::Error::custom),
        _ => Err(serde::de::Error::custom("expected number or string")),
    }
}

#[derive(Debug, Deserialize)]
pub struct ShikimoriImage {
    pub original: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ShikiCalendarEntry {
    pub next_episode: i32,
    pub next_episode_at: chrono::DateTime<chrono::Utc>,
    pub anime: ShikiCalendarAnime,
}

#[derive(Debug, Deserialize)]
pub struct ShikiCalendarAnime {
    pub id: i64,
    pub name: String,
    pub russian: Option<String>,
    pub image: ShikimoriImage,
}

fn poster_url(original: Option<String>) -> Option<String> {
    original.map(|url| {
        if url.starts_with("http") {
            url
        } else {
            format!("https://shikimori.one{}", url)
        }
    })
}

#[derive(Clone)]
pub struct ShikimoriService {
    client: Client,
}

impl ShikimoriService {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .user_agent(USER_AGENT)
                .build()
                .expect("reqwest client"),
        }
    }

    pub async fn search(&self, query: &str) -> Result<Vec<CreateMediaItem>, anyhow::Error> {
        let mut url = Url::parse(&format!("{}/animes", BASE_URL))?;
        url.query_pairs_mut()
            .append_pair("search", query)
            .append_pair("limit", "50");
        let response = self.client.get(url).send().await?;
        let results: Vec<ShikimoriSearchResult> = response.json().await?;

        let items = results
            .into_iter()
            .map(|r| CreateMediaItem {
                provider: "shikimori".to_string(),
                external_id: r.id.to_string(),
                media_type: match r.kind.as_deref() {
                    Some("anime") => "anime".to_string(),
                    Some("manga") => "manga".to_string(),
                    _ => "anime".to_string(),
                },
                title: r.name,
                title_english: r.name_en,
                title_native: None,
                title_russian: r.russian,
                poster_url: poster_url(r.image.and_then(|img| img.original)),
                episodes: r.episodes,
                description: r.description,
                status: r.status,
                score: r.score,
                is_tracked: false,
                mal_id: r.mal_id,
            })
            .collect();

        Ok(items)
    }

    pub async fn fetch_calendar(&self) -> Result<Vec<ShikiCalendarEntry>, anyhow::Error> {
        let url = format!("{}/calendar", BASE_URL);
        let response = self.client.get(&url).send().await?;
        let entries: Vec<ShikiCalendarEntry> = response.json().await?;
        Ok(entries)
    }

    pub async fn get_details(&self, id: &str) -> Result<CreateMediaItem, anyhow::Error> {
        let url = format!("{}/animes/{}", BASE_URL, id);
        let response = self.client.get(&url).send().await?;
        let r: ShikimoriSearchResult = response.json().await?;

        Ok(CreateMediaItem {
            provider: "shikimori".to_string(),
            external_id: r.id.to_string(),
            media_type: match r.kind.as_deref() {
                Some("anime") => "anime".to_string(),
                Some("manga") => "manga".to_string(),
                _ => "anime".to_string(),
            },
            title: r.name,
            title_english: r.name_en,
            title_native: None,
            title_russian: r.russian,
            poster_url: poster_url(r.image.and_then(|img| img.original)),
            episodes: r.episodes,
            description: r.description,
            status: r.status,
            score: r.score,
            is_tracked: false,
            mal_id: r.mal_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_search_response_with_string_score() {
        let json = r#"[{"id":20,"name":"Naruto","russian":"Наруто","kind":"tv","score":"8.02","status":"released","episodes":220}]"#;
        let results: Vec<ShikimoriSearchResult> = serde_json::from_str(json).unwrap();
        assert_eq!(results[0].score, Some(8.02));
        assert_eq!(results[0].russian.as_deref(), Some("Наруто"));
    }
}
