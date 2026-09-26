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
    if episodes.is_empty() {
        return Ok(());
    }

    // Deduplicate by episode_number: a single INSERT ... ON CONFLICT DO
    // UPDATE may not touch the same row twice, and TMDB payloads can
    // occasionally repeat an episode (specials/announcements). Last wins.
    let mut index: std::collections::HashMap<i32, usize> = std::collections::HashMap::new();
    let mut unique: Vec<&TmdbEpisodeInfo> = Vec::with_capacity(episodes.len());
    for ep in episodes {
        match index.get(&ep.episode_number) {
            Some(&i) => unique[i] = ep,
            None => {
                index.insert(ep.episode_number, unique.len());
                unique.push(ep);
            }
        }
    }

    let episode_numbers: Vec<i32> = unique.iter().map(|e| e.episode_number).collect();
    let titles: Vec<Option<String>> = unique.iter().map(|e| e.name.clone()).collect();
    let overviews: Vec<Option<String>> = unique.iter().map(|e| e.overview.clone()).collect();
    let still_paths: Vec<Option<String>> = unique.iter().map(|e| e.still_path.clone()).collect();
    let air_dates: Vec<Option<chrono::NaiveDate>> = unique
        .iter()
        .map(|e| {
            e.air_date
                .as_deref()
                .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
        })
        .collect();
    let runtimes: Vec<Option<i32>> = unique.iter().map(|e| e.runtime).collect();

    // One statement instead of N, wrapped in a transaction so a mid-way
    // failure cannot persist a half-written season.
    let mut tx = pool.begin().await?;

    sqlx::query(
        r#"
        INSERT INTO tmdb_episodes
            (external_id, season_number, episode_number, title, overview, still_path, air_date, duration_minutes)
        SELECT $1, $2, u.episode_number, u.title, u.overview, u.still_path, u.air_date, u.duration_minutes
        FROM UNNEST($3::int[], $4::text[], $5::text[], $6::text[], $7::date[], $8::int[])
            AS u(episode_number, title, overview, still_path, air_date, duration_minutes)
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
    .bind(&episode_numbers)
    .bind(&titles)
    .bind(&overviews)
    .bind(&still_paths)
    .bind(&air_dates)
    .bind(&runtimes)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

pub async fn get_episodes(
    pool: &PgPool,
    external_id: &str,
    season_number: i32,
    user_id: Uuid,
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
        SELECT t.season_number, t.episode_number, t.title, t.overview,
               t.still_path, t.air_date, t.duration_minutes,
               COALESCE(p.watched, FALSE)
        FROM tmdb_episodes t
        LEFT JOIN user_tmdb_episode_progress p
            ON p.user_id = $3
           AND p.external_id = t.external_id
           AND p.season_number = t.season_number
           AND p.episode_number = t.episode_number
        WHERE t.external_id = $1 AND t.season_number = $2
        ORDER BY t.episode_number ASC
        "#,
    )
    .bind(external_id)
    .bind(season_number)
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(
                season_number,
                episode_number,
                title,
                overview,
                still_path,
                air_date,
                duration_minutes,
                watched,
            )| {
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
    user_id: Uuid,
    external_id: &str,
    season_number: i32,
    episode_number: i32,
    watched: bool,
) -> Result<bool, sqlx::Error> {
    let result = if watched {
        sqlx::query(
            r#"
            INSERT INTO user_tmdb_episode_progress
                (user_id, external_id, season_number, episode_number, watched, watched_at)
            SELECT $1, t.external_id, t.season_number, t.episode_number, TRUE, NOW()
            FROM tmdb_episodes t
            WHERE t.external_id = $2
              AND t.season_number = $3
              AND t.episode_number <= $4
            ON CONFLICT (user_id, external_id, season_number, episode_number) DO UPDATE
            SET watched = TRUE,
                watched_at = NOW()
            "#,
        )
        .bind(user_id)
        .bind(external_id)
        .bind(season_number)
        .bind(episode_number)
        .execute(pool)
        .await?
    } else {
        sqlx::query(
            r#"
            UPDATE user_tmdb_episode_progress
            SET watched = FALSE,
                watched_at = NULL
            WHERE user_id = $1
              AND external_id = $2
              AND season_number = $3
              AND episode_number >= $4
            "#,
        )
        .bind(user_id)
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
    user_id: Uuid,
    external_id: &str,
    season_number: i32,
) -> Result<Vec<(i32, bool)>, sqlx::Error> {
    let rows: Vec<(i32, bool)> = sqlx::query_as(
        r#"
        SELECT t.episode_number, COALESCE(p.watched, FALSE)
        FROM tmdb_episodes t
        LEFT JOIN user_tmdb_episode_progress p
            ON p.user_id = $1
           AND p.external_id = t.external_id
           AND p.season_number = t.season_number
           AND p.episode_number = t.episode_number
        WHERE t.external_id = $2 AND t.season_number = $3
        ORDER BY t.episode_number ASC
        "#,
    )
    .bind(user_id)
    .bind(external_id)
    .bind(season_number)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn get_season_counts(
    pool: &PgPool,
    external_id: &str,
    season_number: i32,
    user_id: Uuid,
) -> Result<(i32, i32), sqlx::Error> {
    let row: (Option<i64>, Option<i64>) = sqlx::query_as(
        r#"
        SELECT COUNT(*)::bigint, COUNT(*) FILTER (WHERE p.watched)::bigint
        FROM tmdb_episodes t
        LEFT JOIN user_tmdb_episode_progress p
            ON p.user_id = $3
           AND p.external_id = t.external_id
           AND p.season_number = t.season_number
           AND p.episode_number = t.episode_number
        WHERE t.external_id = $1 AND t.season_number = $2
        "#,
    )
    .bind(external_id)
    .bind(season_number)
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    Ok((row.0.unwrap_or(0) as i32, row.1.unwrap_or(0) as i32))
}

pub async fn get_season_group_counts(
    pool: &PgPool,
    external_id: &str,
    user_id: Uuid,
) -> Result<Vec<(i32, i32, i32)>, sqlx::Error> {
    let rows: Vec<(i32, Option<i64>, Option<i64>)> = sqlx::query_as(
        r#"
        SELECT t.season_number, COUNT(*)::bigint,
               COUNT(*) FILTER (WHERE p.watched)::bigint
        FROM tmdb_episodes t
        LEFT JOIN user_tmdb_episode_progress p
            ON p.user_id = $2
           AND p.external_id = t.external_id
           AND p.season_number = t.season_number
           AND p.episode_number = t.episode_number
        WHERE t.external_id = $1
        GROUP BY t.season_number
        "#,
    )
    .bind(external_id)
    .bind(user_id)
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
    user_id: Uuid,
) -> Result<Vec<(i32, i32)>, sqlx::Error> {
    let rows: Vec<(i32, Option<i64>)> = sqlx::query_as(
        r#"
        SELECT t.season_number, COUNT(*) FILTER (WHERE p.watched)::bigint AS watched_count
        FROM tmdb_episodes t
        LEFT JOIN user_tmdb_episode_progress p
            ON p.user_id = $2
           AND p.external_id = t.external_id
           AND p.season_number = t.season_number
           AND p.episode_number = t.episode_number
        WHERE t.external_id = $1
        GROUP BY t.season_number
        "#,
    )
    .bind(external_id)
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(s, c)| (s, c.unwrap_or(0) as i32))
        .collect())
}

pub async fn set_season_watched(
    pool: &PgPool,
    user_id: Uuid,
    external_id: &str,
    season_number: i32,
    watched: bool,
) -> Result<(), sqlx::Error> {
    if watched {
        sqlx::query(
            r#"
            INSERT INTO user_tmdb_episode_progress
                (user_id, external_id, season_number, episode_number, watched, watched_at)
            SELECT $1, t.external_id, t.season_number, t.episode_number, TRUE, NOW()
            FROM tmdb_episodes t
            WHERE t.external_id = $2
              AND t.season_number = $3
            ON CONFLICT (user_id, external_id, season_number, episode_number) DO UPDATE
            SET watched = TRUE,
                watched_at = NOW()
            "#,
        )
        .bind(user_id)
        .bind(external_id)
        .bind(season_number)
        .execute(pool)
        .await?;
    } else {
        sqlx::query(
            r#"
            UPDATE user_tmdb_episode_progress
            SET watched = FALSE,
                watched_at = NULL
            WHERE user_id = $1
              AND external_id = $2
              AND season_number = $3
            "#,
        )
        .bind(user_id)
        .bind(external_id)
        .bind(season_number)
        .execute(pool)
        .await?;
    }
    Ok(())
}

pub async fn get_total_watched_episodes(
    pool: &PgPool,
    user_id: Uuid,
    external_id: &str,
) -> Result<i32, sqlx::Error> {
    let row: (Option<i64>,) = sqlx::query_as(
        r#"
        SELECT COUNT(*)::bigint
        FROM user_tmdb_episode_progress
        WHERE user_id = $1
          AND external_id = $2
          AND season_number > 0
          AND watched = TRUE
        "#,
    )
    .bind(user_id)
    .bind(external_id)
    .fetch_one(pool)
    .await?;
    Ok(row.0.unwrap_or(0) as i32)
}

pub async fn set_progress_greatest(
    pool: &PgPool,
    user_id: Uuid,
    media_id: Uuid,
    progress: i32,
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
    .bind(progress)
    .bind(user_id)
    .bind(media_id)
    .execute(pool)
    .await?;
    Ok(())
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

/// Mirror a user's flat tracking `progress` count onto their own TMDB
/// episode rows: the first `progress` episodes (ordered by season then
/// episode, specials skipped) become watched, everything else unwatched.
///
/// This is the tracking-form → drawer direction, the counterpart of
/// [`set_progress_greatest`] / [`set_progress_direct`] (drawer → form).
/// Upserts `user_tmdb_episode_progress` so both representations agree.
pub async fn sync_tmdb_episodes_from_progress(
    pool: &PgPool,
    user_id: Uuid,
    external_id: &str,
    progress: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        WITH numbered AS (
            SELECT season_number, episode_number,
                   ROW_NUMBER() OVER (ORDER BY season_number, episode_number) AS seq
            FROM tmdb_episodes
            WHERE external_id = $2 AND season_number > 0
        )
        INSERT INTO user_tmdb_episode_progress
            (user_id, external_id, season_number, episode_number, watched, watched_at)
        SELECT $1, $2, n.season_number, n.episode_number,
               (n.seq <= $3::bigint),
               CASE WHEN n.seq <= $3::bigint THEN NOW() ELSE NULL END
        FROM numbered n
        ON CONFLICT (user_id, external_id, season_number, episode_number) DO UPDATE
        SET watched = EXCLUDED.watched,
            watched_at = EXCLUDED.watched_at
        "#,
    )
    .bind(user_id)
    .bind(external_id)
    .bind(progress)
    .execute(pool)
    .await?;
    Ok(())
}
