//! Official-source detail enricher: chapter titles/dates (COALESCE-merged, never
//! clobbering non-NULL) + free-text schedule / exact next-date on media_items.
//!
//! Raise-only count semantics are untouched — this module never writes
//! `media_items.chapters`. Every failure is a silent no-op.

use sqlx::PgPool;

use crate::services::external::official_meta::{self, OfficialMeta};
use crate::services::source_ids;

/// Merge one `OfficialMeta` into the DB. Returns the number of chapter rows
/// written. Idempotent: re-running only fills NULLs / refreshes fetched_at.
pub async fn apply_meta(
    db: &PgPool,
    provider: &str,
    external_id: &str,
    meta: &OfficialMeta,
) -> anyhow::Result<usize> {
    let mut written = 0usize;
    let mut tx = db.begin().await?;

    for ch in &meta.chapters {
        let res = sqlx::query(
            r#"
            INSERT INTO series_chapters
                (provider, external_id, chapter_number, title_en, release_date)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (provider, external_id, chapter_number) DO UPDATE
            SET title_en     = COALESCE(series_chapters.title_en,     EXCLUDED.title_en),
                release_date = COALESCE(series_chapters.release_date, EXCLUDED.release_date),
                fetched_at   = NOW()
            "#,
        )
        .bind(provider)
        .bind(external_id)
        .bind(ch.number_x100)
        .bind(ch.title.as_deref())
        .bind(ch.release_date)
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() > 0 {
            written += 1;
        }
    }

    // Schedule/next-date: never write an empty string over a non-empty value;
    // next_update_at only advances when the source reports a date.
    sqlx::query(
        r#"
        UPDATE media_items
        SET update_schedule = CASE
                WHEN COALESCE($3, '') = '' THEN update_schedule
                ELSE $3
            END,
            next_update_at = COALESCE($4, next_update_at),
            updated_at = NOW()
        WHERE provider = $1 AND external_id = $2
        "#,
    )
    .bind(provider)
    .bind(external_id)
    .bind(meta.update_schedule.as_deref())
    .bind(meta.next_update_at)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(written)
}

/// Resolve the highest-priority bound source for `media_type`, fetch its detail
/// and apply it. No binding -> no network. Errors -> silent no-op.
pub async fn enrich_official_meta(
    db: &PgPool,
    provider: &str,
    external_id: &str,
    media_type: &str,
) -> anyhow::Result<usize> {
    let client = crate::services::external::http_client();
    let sources = crate::services::official_resolve::official_source_for_media_type(media_type);
    for source in sources {
        let Some(bound) = source_ids::get_source_id(db, provider, external_id, source).await?
        else {
            continue;
        };
        let meta = match official_meta::fetch_source(source, &client, &bound).await {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(provider, external_id, source, error = %e,
                    "official_meta: fetch failed");
                continue;
            }
        };
        return apply_meta(db, provider, external_id, &meta).await;
    }
    Ok(0)
}

/// Convenience wrapper for callers that only have a db + key (drawer/add hook).
/// Discovers media_type from `media_items`; a missing row is a no-op.
pub async fn enrich_official_meta_for_key(
    db: &PgPool,
    provider: &str,
    external_id: &str,
) -> anyhow::Result<usize> {
    let media_type: Option<String> = sqlx::query_scalar(
        "SELECT media_type FROM media_items WHERE provider = $1 AND external_id = $2",
    )
    .bind(provider)
    .bind(external_id)
    .fetch_optional(db)
    .await?;
    match media_type {
        Some(mt) => enrich_official_meta(db, provider, external_id, &mt).await,
        None => Ok(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::external::official_meta::ChapterMeta;

    fn meta_with(title: Option<&str>, date: Option<&str>) -> OfficialMeta {
        OfficialMeta {
            source: "test".into(),
            chapters: vec![ChapterMeta {
                number_x100: 100,
                title: title.map(str::to_string),
                release_date: date.and_then(official_meta::parse_date),
            }],
            count: Some(1),
            update_schedule: Some("매주 월요일".into()),
            next_update_at: None,
        }
    }

    #[test]
    fn coerce_free_and_expected() {
        // Pure sanity that the DTO used by the SQL binds is well-formed.
        let m = meta_with(Some("Ch 1"), Some("2026-09-30"));
        assert_eq!(m.chapters[0].number_x100, 100);
        assert_eq!(m.update_schedule.as_deref(), Some("매주 월요일"));
    }
}
