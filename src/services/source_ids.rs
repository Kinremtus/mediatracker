//! Layer 3: per-source id binding for a skeleton media item.
//!
//! A skeleton item is keyed by `(provider, external_id)` (e.g. the MangaUpdates
//! id). Providers such as kakao/mangaplus/syosetu use their OWN ids, so
//! enrichment must look up the bound id before querying them; a missing binding
//! means "do not query this provider".

use sqlx::PgPool;

/// Whitelist of id-bindable original-language sources. Adding a source here is
/// enough for the UI + validation; the DB column is free-form TEXT.
pub const KNOWN_SOURCES: &[&str] = &["kakao", "mangaplus", "syosetu", "mh5"];

/// True when `source` is a whitelisted bindable source.
pub fn is_known_source(source: &str) -> bool {
    KNOWN_SOURCES.contains(&source)
}

/// All whitelisted sources, as owned static strs.
pub fn known_sources() -> Vec<&'static str> {
    KNOWN_SOURCES.to_vec()
}

/// One binding row, as rendered in the drawer panel.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct SourceId {
    pub source: String,
    pub source_id: String,
}

/// All bindings for a skeleton item, ordered by `source`.
pub async fn list_source_ids(
    pool: &PgPool,
    provider: &str,
    external_id: &str,
) -> Result<Vec<SourceId>, sqlx::Error> {
    sqlx::query_as::<_, SourceId>(
        "SELECT source, source_id FROM media_source_ids \
         WHERE provider = $1 AND external_id = $2 ORDER BY source",
    )
    .bind(provider)
    .bind(external_id)
    .fetch_all(pool)
    .await
}

/// `Some(source_id)` when bound, `None` when not. `chapter_enrich` keys on this.
pub async fn get_source_id(
    pool: &PgPool,
    provider: &str,
    external_id: &str,
    source: &str,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar::<_, String>(
        "SELECT source_id FROM media_source_ids \
         WHERE provider = $1 AND external_id = $2 AND source = $3",
    )
    .bind(provider)
    .bind(external_id)
    .bind(source)
    .fetch_optional(pool)
    .await
}

/// UPSERT a binding (re-binding overwrites + bumps updated_at).
pub async fn set_source_id(
    pool: &PgPool,
    provider: &str,
    external_id: &str,
    source: &str,
    source_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO media_source_ids (provider, external_id, source, source_id) \
         VALUES ($1, $2, $3, $4) \
         ON CONFLICT (provider, external_id, source) \
         DO UPDATE SET source_id = EXCLUDED.source_id, updated_at = NOW()",
    )
    .bind(provider)
    .bind(external_id)
    .bind(source)
    .bind(source_id)
    .execute(pool)
    .await
    .map(|_| ())
}

/// Idempotent delete; `true` when a row was removed.
pub async fn delete_source_id(
    pool: &PgPool,
    provider: &str,
    external_id: &str,
    source: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "DELETE FROM media_source_ids \
         WHERE provider = $1 AND external_id = $2 AND source = $3",
    )
    .bind(provider)
    .bind(external_id)
    .bind(source)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use super::is_known_source;

    #[test]
    fn known_source_whitelist() {
        assert!(is_known_source("kakao"));
        assert!(is_known_source("mangaplus"));
        assert!(is_known_source("syosetu"));
        assert!(is_known_source("mh5"));
        assert!(!is_known_source("mangaupdates"));
        assert!(!is_known_source(""));
    }
}
