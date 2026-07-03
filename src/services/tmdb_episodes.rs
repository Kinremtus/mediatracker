use sqlx::PgPool;
use uuid::Uuid;

use crate::services::external::tmdb::TmdbEpisodeInfo;

#[derive(Debug, Clone)]
pub struct TmdbStoredEpisode {
    pub season_number: i32,
    pub episode_number: i32,
    pub title: Option<String>,
    pub overview: Option<String>,
    pub still_path: Option<String>,
    pub air_date: Option<chrono::NaiveDate>,
    pub duration_minutes: Option<i32>,
    pub watched: bool,
}

pub async fn store_episodes(
    pool: &PgPool,
    external_id: &str,
    season_number: i32,
    episodes: &[TmdbEpisodeInfo],
) -> Result<(), sqlx::Error> {
    for ep in episodes {
        let air_date = ep
            .air_date
            .as_deref()
            .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());

        sqlx::query(
            r#"
            INSERT INTO tmdb_episodes
                (external_id, season_number, episode_number, title, overview, still_path, air_date, duration_minutes)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (external_id, season_number, episode_number) DO UPDATE
            SET title = EXCLUDED.title,
                overview = EXCLUDED.overview,
                still_path = EXCLUDED.still_path,
                air_date = EXCLUDED.air_date,
                duration_minutes = EXCLUDED.duration_minutes,
                fetched_at = NOW()
            "#,
        )
        .bind(external_id)
        .bind(season_number)
        .bind(ep.episode_number)
        .bind(&ep.name)
        .bind(&ep.overview)
        .bind(&ep.still_path)
        .bind(air_date)
        .bind(ep.runtime)
        .execute(pool)
        .await?;
    }
    Ok(())
}

pub async fn get_episodes(
    pool: &PgPool,
    external_id: &str,
    season_number: i32,
) -> Result<Vec<TmdbStoredEpisode>, sqlx::Error> {
    #[allow(clippy::type_complexity)]
    let rows: Vec<(
        i32,
        i32,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<chrono::NaiveDate>,
        Option<i32>,
        bool,
    )> = sqlx::query_as(
        r#"
        SELECT season_number, episode_number, title, overview, still_path, air_date, duration_minutes, watched
        FROM tmdb_episodes
        WHERE external_id = $1 AND season_number = $2
        ORDER BY episode_number ASC
        "#,
    )
    .bind(external_id)
    .bind(season_number)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(season_number, episode_number, title, overview, still_path, air_date, duration_minutes, watched)| {
                TmdbStoredEpisode {
                    season_number,
                    episode_number,
                    title,
                    overview,
                    still_path,
                    air_date,
                    duration_minutes,
                    watched,
                }
            },
        )
        .collect())
}

pub async fn set_watched(
    pool: &PgPool,
    external_id: &str,
    season_number: i32,
    episode_number: i32,
    watched: bool,
) -> Result<bool, sqlx::Error> {
    let result = if watched {
        sqlx::query(
            r#"
            UPDATE tmdb_episodes
            SET watched = TRUE,
                watched_at = NOW()
            WHERE external_id = $1
              AND season_number = $2
              AND episode_number <= $3
            "#,
        )
        .bind(external_id)
        .bind(season_number)
        .bind(episode_number)
        .execute(pool)
        .await?
    } else {
        sqlx::query(
            r#"
            UPDATE tmdb_episodes
            SET watched = FALSE,
                watched_at = NULL
            WHERE external_id = $1
              AND season_number = $2
              AND episode_number >= $3
            "#,
        )
        .bind(external_id)
        .bind(season_number)
        .bind(episode_number)
        .execute(pool)
        .await?
    };
    Ok(result.rows_affected() > 0)
}

pub async fn count_watched(
    pool: &PgPool,
    external_id: &str,
    season_number: i32,
) -> Result<i32, sqlx::Error> {
    let row: (Option<i32>,) = sqlx::query_as(
        r#"
        SELECT MAX(episode_number)
        FROM tmdb_episodes
        WHERE external_id = $1
          AND season_number = $2
          AND watched = TRUE
        "#,
    )
    .bind(external_id)
    .bind(season_number)
    .fetch_one(pool)
    .await?;
    Ok(row.0.unwrap_or(0))
}

pub async fn get_episode_states(
    pool: &PgPool,
    external_id: &str,
    season_number: i32,
) -> Result<Vec<(i32, bool)>, sqlx::Error> {
    let rows: Vec<(i32, bool)> = sqlx::query_as(
        r#"
        SELECT episode_number, watched
        FROM tmdb_episodes
        WHERE external_id = $1 AND season_number = $2
        ORDER BY episode_number ASC
        "#,
    )
    .bind(external_id)
    .bind(season_number)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

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

pub async fn get_season_counts(
    pool: &PgPool,
    external_id: &str,
    season_number: i32,
) -> Result<(i32, i32), sqlx::Error> {
    let row: (Option<i64>, Option<i64>) = sqlx::query_as(
        r#"
        SELECT COUNT(*)::bigint, COUNT(*) FILTER (WHERE watched)::bigint
        FROM tmdb_episodes
        WHERE external_id = $1 AND season_number = $2
        "#,
    )
    .bind(external_id)
    .bind(season_number)
    .fetch_one(pool)
    .await?;
    Ok((row.0.unwrap_or(0) as i32, row.1.unwrap_or(0) as i32))
}

pub async fn get_season_group_counts(
    pool: &PgPool,
    external_id: &str,
) -> Result<Vec<(i32, i32, i32)>, sqlx::Error> {
    let rows: Vec<(i32, Option<i64>, Option<i64>)> = sqlx::query_as(
        r#"
        SELECT season_number, COUNT(*)::bigint, COUNT(*) FILTER (WHERE watched)::bigint
        FROM tmdb_episodes
        WHERE external_id = $1
        GROUP BY season_number
        "#,
    )
    .bind(external_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(s, t, w)| (s, t.unwrap_or(0) as i32, w.unwrap_or(0) as i32))
        .collect())
}

pub async fn get_season_watched_counts(
    pool: &PgPool,
    external_id: &str,
) -> Result<Vec<(i32, i32)>, sqlx::Error> {
    let rows: Vec<(i32, Option<i64>)> = sqlx::query_as(
        r#"
        SELECT season_number, COUNT(*) FILTER (WHERE watched)::bigint AS watched_count
        FROM tmdb_episodes
        WHERE external_id = $1
        GROUP BY season_number
        "#,
    )
    .bind(external_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(s, c)| (s, c.unwrap_or(0) as i32)).collect())
}

pub async fn set_season_watched(
    pool: &PgPool,
    external_id: &str,
    season_number: i32,
    watched: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE tmdb_episodes
        SET watched = $1,
            watched_at = CASE WHEN $1 THEN NOW() ELSE NULL END
        WHERE external_id = $2
          AND season_number = $3
        "#,
    )
    .bind(watched)
    .bind(external_id)
    .bind(season_number)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_total_watched_episodes(
    pool: &PgPool,
    external_id: &str,
) -> Result<i32, sqlx::Error> {
    let row: (Option<i64>,) = sqlx::query_as(
        r#"
        SELECT COUNT(*) FILTER (WHERE watched)::bigint
        FROM tmdb_episodes
        WHERE external_id = $1
        "#,
    )
    .bind(external_id)
    .fetch_one(pool)
    .await?;
    Ok(row.0.unwrap_or(0) as i32)
}

pub async fn set_progress_direct(
    pool: &PgPool,
    user_id: Uuid,
    media_id: Uuid,
    progress: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE tracking_entries
        SET progress = $1,
            updated_at = NOW()
        WHERE user_id = $2
          AND media_id = $3
        "#,
    )
    .bind(progress)
    .bind(user_id)
    .bind(media_id)
    .execute(pool)
    .await?;
    Ok(())
}
