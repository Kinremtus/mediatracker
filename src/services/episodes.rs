use sqlx::PgPool;

use crate::services::external::shikimori::{ShikimoriEpisode, ShikimoriService};

/// Episode as returned to the template layer.
#[derive(Debug, Clone)]
pub struct StoredEpisode {
    pub episode_number: i32,
    pub title_en: Option<String>,
    pub title_ru: Option<String>,
    pub title_jp: Option<String>,
    pub air_date: Option<chrono::NaiveDate>,
    pub duration_minutes: Option<i32>,
    pub watched: bool,
}

/// Insert or update episodes in the DB. UNIQUE (provider, external_id,
/// episode_number) makes the operation idempotent — re-fetching the
/// same anime just refreshes titles and air dates in place.
pub async fn store_episodes(
    pool: &PgPool,
    provider: &str,
    external_id: &str,
    episodes: &[ShikimoriEpisode],
) -> Result<(), sqlx::Error> {
    for ep in episodes {
        sqlx::query(
            r#"
            INSERT INTO anime_episodes
                (provider, external_id, episode_number, title_en, title_ru, air_date, duration_minutes)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT (provider, external_id, episode_number) DO UPDATE
            SET title_en = EXCLUDED.title_en,
                title_ru = EXCLUDED.title_ru,
                air_date = EXCLUDED.air_date,
                duration_minutes = EXCLUDED.duration_minutes,
                fetched_at = NOW()
            "#,
        )
        .bind(provider)
        .bind(external_id)
        .bind(ep.number)
        .bind(&ep.name_en)
        .bind(&ep.name_ru)
        .bind(ep.airdate)
        .bind(ep.duration)
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// Read all episodes for one anime, sorted by number ascending.
pub async fn get_episodes(
    pool: &PgPool,
    provider: &str,
    external_id: &str,
) -> Result<Vec<StoredEpisode>, sqlx::Error> {
    let rows: Vec<(
        i32,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<chrono::NaiveDate>,
        Option<i32>,
        bool,
    )> = sqlx::query_as(
        r#"
        SELECT episode_number, title_en, title_ru, title_jp, air_date, duration_minutes, watched
        FROM anime_episodes
        WHERE provider = $1 AND external_id = $2
        ORDER BY episode_number ASC
        "#,
    )
    .bind(provider)
    .bind(external_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(episode_number, title_en, title_ru, title_jp, air_date, duration_minutes, watched)| {
                StoredEpisode {
                    episode_number,
                    title_en,
                    title_ru,
                    title_jp,
                    air_date,
                    duration_minutes,
                    watched,
                }
            },
        )
        .collect())
}

/// Fetch episodes from Shikimori and persist them.
/// Returns the number of episodes stored (0 on failure or unsupported provider).
pub async fn fetch_and_store(
    pool: PgPool,
    service: &ShikimoriService,
    shikimori_id: i64,
) -> Result<usize, anyhow::Error> {
    let episodes = service.fetch_episodes(shikimori_id).await?;
    let count = episodes.len();
    store_episodes(&pool, "shikimori", &shikimori_id.to_string(), &episodes).await?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_shikimori_episode_json() {
        let json = r#"[{
            "id": 1,
            "number": 1,
            "name_en": "Enter: Naruto Uzumaki!",
            "name_ru": "Появляется Наруто Узумаки!",
            "airdate": "2002-10-03",
            "duration": 23
        }]"#;
        let eps: Vec<ShikimoriEpisode> = serde_json::from_str(json).unwrap();
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].number, 1);
        assert_eq!(eps[0].name_ru.as_deref(), Some("Появляется Наруто Узумаки!"));
        assert_eq!(eps[0].duration, Some(23));
    }

    #[test]
    fn parse_shikimori_episode_with_missing_fields() {
        // Some episodes have no name, no airdate, no duration
        let json = r#"[{"id": 5, "number": 5}]"#;
        let eps: Vec<ShikimoriEpisode> = serde_json::from_str(json).unwrap();
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].number, 5);
        assert!(eps[0].name_en.is_none());
        assert!(eps[0].name_ru.is_none());
        assert!(eps[0].airdate.is_none());
        assert!(eps[0].duration.is_none());
    }
}
