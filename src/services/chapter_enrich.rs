//! Raise-only chapter-count enrichment (design Layer 2 + Layer 3 binding).
//!
//! Never lowers `media_items.chapters`. Providers are tried in order; the first
//! usable reading wins. Every error path is a silent no-op (debug log only), so a
//! flaky source can never blank a progress denominator.
//!
//! Layer 3: providers use their OWN ids, not the skeleton id. The binding is
//! resolved per provider via `services::source_ids`; a missing binding means the
//! provider is skipped entirely (never fall back to the skeleton `external_id`).

use std::sync::Arc;

use sqlx::PgPool;

/// Pure decision: the value a raise-only update should write, if any.
/// Returns `Some(next)` only when `reported > current`.
pub fn raised_value(current: Option<i32>, reported: i32) -> Option<i32> {
    if reported > 0 && reported > current.unwrap_or(0) {
        Some(reported)
    } else {
        None
    }
}

/// Public entry point — unchanged signature, kept for `refresh_counts.rs`.
/// Resolves the provider set from `media_type` and delegates to
/// [`enrich_with_providers`].
pub async fn enrich_chapter_count(
    db: &PgPool,
    provider: &str,
    external_id: &str,
    media_type: &str,
) -> Option<i32> {
    use crate::services::external::chapter_count::providers_for;
    enrich_with_providers(db, provider, external_id, providers_for(media_type)).await
}

/// Inner, test-injectable core. `pub` so `tests/source_id_mapping.rs` can pass a
/// mock provider. Returns the applied value, if any.
pub async fn enrich_with_providers(
    db: &PgPool,
    provider: &str,
    external_id: &str,
    providers: Vec<Arc<dyn crate::services::external::chapter_count::ChapterCountProvider>>,
) -> Option<i32> {
    use crate::services::external::chapter_count::usable;

    let (current, manual) = match crate::services::chapters::get_chapter_meta(
        db,
        provider,
        external_id,
    )
    .await
    {
        Ok(Some((ch, m, _))) => (ch, m),
        Ok(None) => (None, false),
        Err(e) => {
            tracing::debug!(provider, external_id, error = %e, "chapter_enrich: meta lookup failed");
            return None;
        }
    };
    if manual {
        return None;
    }

    for p in providers {
        // Layer 3: providers use their OWN ids. Never fall back to the skeleton
        // external_id — a missing binding means "do not query this provider".
        let bound =
            match crate::services::source_ids::get_source_id(db, provider, external_id, p.name())
                .await
            {
                Ok(Some(id)) => id,
                Ok(None) => continue,
                Err(e) => {
                    tracing::debug!(
                        provider,
                        external_id,
                        source = p.name(),
                        error = %e,
                        "chapter_enrich: source-id lookup failed"
                    );
                    continue;
                }
            };
        let fetched = match p.fetch(&bound).await {
            Ok(fetched) => fetched,
            Err(e) => {
                tracing::warn!(
                    provider,
                    external_id,
                    source = p.name(),
                    error = %e,
                    "chapter_enrich: fetch failed"
                );
                None
            }
        };
        let reading = match usable(fetched) {
            Some(r) => r,
            None => continue,
        };
        let Some(next) = raised_value(current, reading.total) else {
            continue;
        };
        let mut tx = match db.begin().await {
            Ok(tx) => tx,
            Err(e) => {
                tracing::debug!(provider, external_id, error = %e, "chapter_enrich: begin tx failed");
                continue;
            }
        };
        let updated = sqlx::query(
            "UPDATE media_items SET chapters = $3, chapters_source = $4, updated_at = NOW() \
             WHERE provider = $1 AND external_id = $2 AND chapters_manual = FALSE",
        )
        .bind(provider)
        .bind(external_id)
        .bind(next)
        .bind(reading.source)
        .execute(&mut *tx)
        .await;
        match updated {
            Ok(res) if res.rows_affected() == 0 => {
                // Became manual (or row vanished) between read and write: no-op.
                let _ = tx.rollback().await;
                continue;
            }
            Ok(_) => {}
            Err(e) => {
                tracing::debug!(provider, external_id, error = %e, "chapter_enrich: write failed");
                continue;
            }
        }
        if let Err(e) =
            crate::services::chapters::ensure_chapter_skeleton(&mut tx, provider, external_id, next)
                .await
        {
            tracing::debug!(provider, external_id, error = %e, "chapter_enrich: skeleton extend failed");
            continue;
        }
        if let Err(e) = tx.commit().await {
            tracing::debug!(provider, external_id, error = %e, "chapter_enrich: commit failed");
            continue;
        }
        tracing::info!(provider, external_id, from = ?current, to = next, source = reading.source, "chapter_enrich: raised");
        return Some(next);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::raised_value;

    #[test]
    fn raised_value_only_raises() {
        assert_eq!(raised_value(Some(145), 157), Some(157));
        assert_eq!(raised_value(Some(157), 157), None);
        assert_eq!(raised_value(Some(157), 145), None, "must never lower");
        assert_eq!(raised_value(None, 157), Some(157));
        assert_eq!(raised_value(Some(10), 0), None, "zero is not a reading");
    }
}
