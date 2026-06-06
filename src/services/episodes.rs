use sqlx::PgPool;
use uuid::Uuid;

use crate::services::external::mal::JikanEpisode;
use crate::services::external::mal::MalService;

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
///
/// Stores under `provider = "mal"`, `external_id = mal_id.to_string()`.
/// Jikan is our episode source of choice because Shikimori's REST
/// `/api/animes/{id}/episodes` is currently 404 on both shikimori.one
/// and shikimori.io, and Shikimori's `?mal_id=` filter is broken
/// (returns unrelated anime). Jikan's `/v4/anime/{mal_id}/episodes`
/// is paginated, unauthenticated, and not behind DDoS-Guard.
pub async fn store_episodes_mal(
    pool: &PgPool,
    mal_id: i64,
    episodes: &[JikanEpisode],
) -> Result<(), sqlx::Error> {
    for ep in episodes {
        let air_date = ep
            .aired
            .as_deref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.naive_utc().date());
        let duration_minutes = ep
            .duration
            .as_deref()
            .and_then(crate::services::external::mal::parse_duration_to_minutes);

        sqlx::query(
            r#"
            INSERT INTO anime_episodes
                (provider, external_id, episode_number, title_en, title_ru, title_jp, air_date, duration_minutes)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (provider, external_id, episode_number) DO UPDATE
            SET title_en = EXCLUDED.title_en,
                title_jp = EXCLUDED.title_jp,
                air_date = EXCLUDED.air_date,
                duration_minutes = EXCLUDED.duration_minutes,
                fetched_at = NOW()
            "#,
        )
        .bind("mal")
        .bind(mal_id.to_string())
        .bind(ep.mal_id)
        .bind(&ep.title)
        .bind(Option::<String>::None) // Jikan has no Russian episode titles
        .bind(&ep.title_japanese)
        .bind(air_date)
        .bind(duration_minutes)
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

/// Fetch episodes from Jikan and persist them.
/// Returns the number of episodes stored (0 on failure).
pub async fn fetch_and_store_mal(
    pool: PgPool,
    service: &MalService,
    mal_id: i64,
) -> Result<usize, anyhow::Error> {
    let episodes = service.fetch_episodes(mal_id).await?;
    let count = episodes.len();
    store_episodes_mal(&pool, mal_id, &episodes).await?;
    Ok(count)
}

/// Look up `mal_id` for a media item. Required because we store
/// episodes keyed on MAL id (under `provider = "mal"`) regardless
/// of which provider the user originally added the anime with.
///
/// Returns `None` for anime that don't have a known MAL id
/// (rare for Shikimori-only entries).
pub async fn lookup_mal_id(
    pool: &PgPool,
    provider: &str,
    external_id: &str,
) -> Result<Option<i64>, sqlx::Error> {
    let row: Option<(Option<i64>,)> = sqlx::query_as(
        "SELECT mal_id FROM media_items WHERE provider = $1 AND external_id = $2",
    )
    .bind(provider)
    .bind(external_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(|(v,)| v))
}

/// Set the `watched` flag on one or more episode rows.
///
/// **Bulk-fill semantics on watch**: when `watched = true`, every
/// episode with `episode_number <= episode_number` is marked watched.
/// This matches the standard "mark ep 200 watched → ep 1..200 watched"
/// UX of MAL / AniList / Shikimori — users don't want to click 200
/// checkboxes after binging a long series. Unwatch (`watched = false`)
/// only flips the one row, by design: the user explicitly marked the
/// later ones watched, and removing one earlier watched episode
/// shouldn't silently take down the rest.
///
/// Returns `true` if at least one row was updated (i.e. the target
/// episode exists in the DB for this MAL id), `false` otherwise.
pub async fn set_watched(
    pool: &PgPool,
    mal_id: i64,
    episode_number: i32,
    watched: bool,
) -> Result<bool, sqlx::Error> {
    let result = if watched {
        sqlx::query(
            r#"
            UPDATE anime_episodes
            SET watched = TRUE,
                watched_at = NOW()
            WHERE provider = 'mal'
              AND external_id = $1
              AND episode_number <= $2
            "#,
        )
        .bind(mal_id.to_string())
        .bind(episode_number)
        .execute(pool)
        .await?
    } else {
        sqlx::query(
            r#"
            UPDATE anime_episodes
            SET watched = FALSE,
                watched_at = NULL
            WHERE provider = 'mal'
              AND external_id = $1
              AND episode_number = $2
            "#,
        )
        .bind(mal_id.to_string())
        .bind(episode_number)
        .execute(pool)
        .await?
    };
    Ok(result.rows_affected() > 0)
}

/// Highest episode number currently marked watched for an anime.
/// Returns 0 if nothing is watched (or no episodes exist).
pub async fn count_watched(
    pool: &PgPool,
    mal_id: i64,
) -> Result<i32, sqlx::Error> {
    let row: (Option<i32>,) = sqlx::query_as(
        r#"
        SELECT MAX(episode_number)
        FROM anime_episodes
        WHERE provider = 'mal'
          AND external_id = $1
          AND watched = TRUE
        "#,
    )
    .bind(mal_id.to_string())
    .fetch_one(pool)
    .await?;
    Ok(row.0.unwrap_or(0))
}

/// Bumps `tracking_entries.progress` to at least `watched_count`.
/// Uses `GREATEST(progress, $1)` so it never regresses — un-checking
/// the highest episode doesn't drop your progress, you'd have to do
/// that manually with the +1/-1 buttons.
pub async fn update_progress_from_watched(
    pool: &PgPool,
    user_id: Uuid,
    media_id: Uuid,
    watched_count: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE tracking_entries
        SET progress = GREATEST(progress, $1),
            updated_at = NOW()
        WHERE user_id = $2
          AND media_id = $3
        "#,
    )
    .bind(watched_count)
    .bind(user_id)
    .bind(media_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Read a single episode by (mal_id, episode_number). Returns `None`
/// if the row doesn't exist. Used by the toggle endpoint to render
/// the updated row HTML.
pub async fn get_episode(
    pool: &PgPool,
    mal_id: i64,
    episode_number: i32,
) -> Result<Option<StoredEpisode>, sqlx::Error> {
    let row: Option<(
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
        WHERE provider = 'mal'
          AND external_id = $1
          AND episode_number = $2
        "#,
    )
    .bind(mal_id.to_string())
    .bind(episode_number)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(
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
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_jikan_episode_json() {
        // Real response shape from Jikan v4 /anime/{id}/episodes.
        let json = r#"{
            "data": [{
                "mal_id": 1,
                "url": "https://myanimelist.net/anime/21/One_Piece/episode/1",
                "title": "I'm Luffy! The Man Who's Gonna Be King of the Pirates!",
                "title_japanese": "俺はルフィ！海賊王になる男だ！",
                "aired": "1999-10-20T00:00:00+00:00",
                "score": 4.1,
                "filler": false,
                "recap": false,
                "forum_url": "https://myanimelist.net/forum/?topicid=43183"
            }],
            "pagination": {
                "last_visible_page": 12,
                "has_next_page": true
            }
        }"#;
        #[derive(serde::Deserialize)]
        struct Resp {
            data: Vec<JikanEpisode>,
        }
        let resp: Resp = serde_json::from_str(json).unwrap();
        assert_eq!(resp.data.len(), 1);
        let ep = &resp.data[0];
        assert_eq!(ep.mal_id, 1);
        assert!(ep.title.as_deref().unwrap().starts_with("I'm Luffy"));
        assert!(ep.aired.is_some());
        assert!(ep.duration.is_none(), "Jikan episodes list has no duration field");
    }
}
