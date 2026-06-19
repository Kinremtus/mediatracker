use chrono::{Datelike, NaiveDate, Utc};
use sqlx::PgPool;
use std::collections::HashMap;
use uuid::Uuid;

use crate::models::stats::{StatsOverview, StatusCount, TitleProgress};

#[derive(Clone)]
pub struct StatsService {
    db: PgPool,
}

impl StatsService {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    pub async fn get_overview(&self, user_id: Uuid) -> Result<StatsOverview, anyhow::Error> {
        // Total titles
        let total: (i32,) = sqlx::query_as(
            "SELECT COUNT(*)::int FROM tracking_entries WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_one(&self.db)
        .await?;

        // Completed count
        let completed: (i32,) = sqlx::query_as(
            "SELECT COUNT(*)::int FROM tracking_entries WHERE user_id = $1 AND status = 'completed'",
        )
        .bind(user_id)
        .fetch_one(&self.db)
        .await?;

        // Completion rate
        let completion_rate = if total.0 > 0 {
            (completed.0 as f64 / total.0 as f64) * 100.0
        } else {
            0.0
        };

        // Top category (media_type)
        let top_category: Option<(String, i32)> = sqlx::query_as(
            "SELECT m.media_type, COUNT(*)::int as count FROM tracking_entries t JOIN media_items m ON t.media_id = m.id WHERE t.user_id = $1 GROUP BY m.media_type ORDER BY count DESC LIMIT 1",
        )
        .bind(user_id)
        .fetch_optional(&self.db)
        .await?;

        // Count by status
        let status_counts: Vec<StatusCount> = sqlx::query_as(
            "SELECT status, COUNT(*)::int as count, 0 as percentage FROM tracking_entries WHERE user_id = $1 GROUP BY status",
        )
        .bind(user_id)
        .fetch_all(&self.db)
        .await?;

        Ok(StatsOverview {
            total_titles: total.0,
            completed_count: completed.0,
            completion_rate,
            top_category: top_category.map(|(t, _)| t),
            status_counts,
        })
    }

    /// Daily action counts from activity_log (journal; survives tracking entry deletion).
    pub async fn get_activity_by_day(
        &self,
        user_id: Uuid,
    ) -> Result<HashMap<NaiveDate, i32>, anyhow::Error> {
        let year = Utc::now().year();
        let year_start = NaiveDate::from_ymd_opt(year, 1, 1).expect("valid year");
        let year_end = NaiveDate::from_ymd_opt(year + 1, 1, 1).expect("valid next year");

        let rows: Vec<(NaiveDate, i32)> = sqlx::query_as(
            r#"
            SELECT created_at::date AS activity_date, COUNT(*)::int AS count
            FROM activity_log
            WHERE user_id = $1
              AND created_at >= $2::date
              AND created_at < $3::date
            GROUP BY activity_date
            "#,
        )
        .bind(user_id)
        .bind(year_start)
        .bind(year_end)
        .fetch_all(&self.db)
        .await?;

        Ok(rows.into_iter().collect())
    }

    pub async fn get_title_progress(&self, user_id: Uuid) -> Result<Vec<TitleProgress>, anyhow::Error> {
        let mut progress: Vec<TitleProgress> = sqlx::query_as(
            "SELECT t.id, m.title, t.progress, m.episodes, t.status, 0 as percentage FROM tracking_entries t JOIN media_items m ON t.media_id = m.id WHERE t.user_id = $1 ORDER BY t.updated_at DESC LIMIT 10",
        )
        .bind(user_id)
        .fetch_all(&self.db)
        .await?;
        
        // Calculate percentages
        for p in &mut progress {
            if let Some(ep) = p.episodes
                && ep > 0 {
                    p.percentage = (p.progress * 100 / ep).min(100);
                }
        }

        Ok(progress)
    }
}
