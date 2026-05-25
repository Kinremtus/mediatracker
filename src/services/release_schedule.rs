use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::schedule::ReleaseEntry;
use crate::services::external::shikimori::ShikimoriService;

#[derive(Clone)]
pub struct ReleaseScheduleService {
    db: PgPool,
}

impl ReleaseScheduleService {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    pub async fn refresh_from_shikimori(
        &self,
        shikimori: &ShikimoriService,
    ) -> Result<(), anyhow::Error> {
        let entries = shikimori.fetch_calendar().await?;

        for entry in &entries {
            let poster = entry
                .anime
                .image
                .original
                .as_ref()
                .map(|url| {
                    if url.starts_with("http") {
                        url.clone()
                    } else {
                        format!("https://shikimori.one{}", url)
                    }
                });

            sqlx::query(
                r#"
                INSERT INTO release_schedule (provider, external_id, episode_number, air_date, title, poster_url, fetched_at)
                VALUES ($1, $2, $3, $4, $5, $6, NOW())
                ON CONFLICT (provider, external_id, episode_number)
                DO UPDATE SET air_date = $4, title = $5, poster_url = $6, fetched_at = NOW()
                "#,
            )
            .bind("shikimori")
            .bind(entry.anime.id.to_string())
            .bind(entry.next_episode)
            .bind(entry.next_episode_at)
            .bind(entry.anime.russian.as_deref().unwrap_or(&entry.anime.name))
            .bind(poster)
            .execute(&self.db)
            .await?;
        }

        Ok(())
    }

    pub async fn ensure_fresh(
        &self,
        shikimori: &ShikimoriService,
    ) -> Result<(), anyhow::Error> {
        let stale: Option<(i32,)> = sqlx::query_as(
            "SELECT COUNT(*)::int FROM release_schedule WHERE fetched_at > NOW() - INTERVAL '6 hours'",
        )
        .fetch_optional(&self.db)
        .await?;

        let has_fresh = stale.map(|(c,)| c > 0).unwrap_or(false);
        if !has_fresh {
            self.refresh_from_shikimori(shikimori).await?;
        }
        Ok(())
    }

    pub async fn get_upcoming_for_user(
        &self,
        user_id: Uuid,
        limit: i64,
    ) -> Result<Vec<ReleaseEntry>, anyhow::Error> {
        let rows: Vec<(String, String, String, Option<String>, i32, DateTime<Utc>)> = sqlx::query_as(
            r#"
            SELECT DISTINCT ON (r.id)
                r.provider, r.external_id, r.title, r.poster_url,
                r.episode_number, r.air_date
            FROM release_schedule r
            JOIN tracking_entries t ON t.user_id = $1
            JOIN media_items m ON m.id = t.media_id
                AND m.provider = r.provider
                AND m.external_id = r.external_id
            WHERE t.status IN ('in_progress', 'planned')
              AND r.air_date >= NOW()
            ORDER BY r.air_date ASC
            LIMIT $2
            "#,
        )
        .bind(user_id)
        .bind(limit)
        .fetch_all(&self.db)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(p, eid, title, poster, ep, date)| ReleaseEntry {
                provider: p,
                external_id: eid,
                title,
                poster_url: poster,
                episode_number: ep,
                air_date: date,
            })
            .collect())
    }

    pub async fn get_by_date_range(
        &self,
        user_id: Uuid,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<ReleaseEntry>, anyhow::Error> {
        let rows: Vec<(String, String, String, Option<String>, i32, DateTime<Utc>)> = sqlx::query_as(
            r#"
            SELECT DISTINCT ON (r.id)
                r.provider, r.external_id, r.title, r.poster_url,
                r.episode_number, r.air_date
            FROM release_schedule r
            JOIN tracking_entries t ON t.user_id = $1
            JOIN media_items m ON m.id = t.media_id
                AND m.provider = r.provider
                AND m.external_id = r.external_id
            WHERE t.status IN ('in_progress', 'planned')
              AND r.air_date >= $2
              AND r.air_date < $3
            ORDER BY r.air_date ASC
            "#,
        )
        .bind(user_id)
        .bind(from)
        .bind(to)
        .fetch_all(&self.db)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(p, eid, title, poster, ep, date)| ReleaseEntry {
                provider: p,
                external_id: eid,
                title,
                poster_url: poster,
                episode_number: ep,
                air_date: date,
            })
            .collect())
    }
}
