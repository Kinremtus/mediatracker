use sqlx::PgPool;
use uuid::Uuid;

use crate::models::stats::{ActivityEntry, StatsOverview, StatusCount, TitleProgress};

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

        // Count by status
        let status_counts: Vec<StatusCount> = sqlx::query_as(
            "SELECT status, COUNT(*)::int as count, 0 as percentage FROM tracking_entries WHERE user_id = $1 GROUP BY status",
        )
        .bind(user_id)
        .fetch_all(&self.db)
        .await?;

        // Top category (media_type)
        let top_category: Option<(String, i32)> = sqlx::query_as(
            "SELECT m.media_type, COUNT(*)::int as count FROM tracking_entries t JOIN media_items m ON t.media_id = m.id WHERE t.user_id = $1 GROUP BY m.media_type ORDER BY count DESC LIMIT 1",
        )
        .bind(user_id)
        .fetch_optional(&self.db)
        .await?;

        // Completion rate
        let completed: (i32,) = sqlx::query_as(
            "SELECT COUNT(*)::int FROM tracking_entries WHERE user_id = $1 AND status = 'completed'",
        )
        .bind(user_id)
        .fetch_one(&self.db)
        .await?;

        let completion_rate = if total.0 > 0 {
            (completed.0 as f64 / total.0 as f64) * 100.0
        } else {
            0.0
        };

        Ok(StatsOverview {
            total_titles: total.0,
            status_counts,
            top_category: top_category.map(|(t, _)| t),
            completion_rate,
        })
    }

    pub async fn get_activity(&self, user_id: Uuid) -> Result<Vec<ActivityEntry>, anyhow::Error> {
        // Get activity log entries for the last year
        let entries: Vec<ActivityEntry> = sqlx::query_as(
            "SELECT action, created_at FROM activity_log WHERE user_id = $1 AND created_at > NOW() - INTERVAL '1 year' ORDER BY created_at ASC",
        )
        .bind(user_id)
        .fetch_all(&self.db)
        .await?;

        Ok(entries)
    }

    pub async fn get_title_progress(&self, user_id: Uuid) -> Result<Vec<TitleProgress>, anyhow::Error> {
        let mut progress: Vec<TitleProgress> = sqlx::query_as(
            "SELECT t.id, m.title, t.progress, m.episodes, t.status, 0 as percentage FROM tracking_entries t JOIN media_items m ON t.media_id = m.id WHERE t.user_id = $1 AND m.episodes IS NOT NULL AND m.episodes > 0 ORDER BY t.updated_at DESC LIMIT 10",
        )
        .bind(user_id)
        .fetch_all(&self.db)
        .await?;
        
        // Calculate percentages
        for p in &mut progress {
            if let Some(ep) = p.episodes {
                if ep > 0 {
                    p.percentage = (p.progress * 100 / ep).min(100);
                }
            }
        }

        Ok(progress)
    }
}
