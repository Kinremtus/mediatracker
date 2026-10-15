//! Official-source detail enricher: chapter titles/dates (COALESCE-merged, never
//! clobbering non-NULL) + free-text schedule / exact next-date on media_items,
//! plus a `media_items.chapters` counter (raise-only except a same-source
//! supersede) and MU-orphan cleanup.
//!
//! Chapter rows are written under the key of their OWN source
//! ([`chapter_key`]): a meta-capable binding wins, otherwise
//! the MangaUpdates identity key. Every `media_items` update (counter,
//! schedule) stays on the identity key, so a drawer preview — no `media_items`
//! row yet — still stores chapters and defers only the counter/schedule: the
//! drawer passes its `media_type` hint to [`enrich_official_meta_for_key`], so
//! the preview no longer needs the row to start the chain.
//!
//! The counter is raise-only with one narrow exception: a detailed meta from
//! the SAME source supersedes that source's own count-provider reading
//! (`auto:kakao` -> the kakao walk), whose coarse number counts bonus rows
//! (kakao includes its `0화` prologue in `on_sale_count`) — the walk's
//! main-only count may therefore also lower it. `chapters_manual = TRUE` is
//! never overridden.

use sqlx::PgPool;

use crate::models::media_item::OFFICIAL_SOURCE_PREFIX;
use crate::services::chapter_enrich::raised_value;
use crate::services::external::official_meta::{self, MetaFuture, OfficialMeta};
use crate::services::official_resolve::chapter_key;
use crate::services::source_ids;

/// Merge one `OfficialMeta` into the DB. Returns the number of chapter rows
/// written.
///
/// - `provider`/`external_id` is the MangaUpdates identity key. Chapter rows
///   land under `chapter_key` (the bound source key when a meta-capable
///   binding exists), while `media_items` updates keep the identity key.
/// - `source` is the official source that produced `meta`: it is stored as
///   `series_chapters.origin` and stamps `chapters_source = 'official_<source>'`.
/// - Idempotent: re-running only fills NULLs / refreshes `fetched_at`, and
///   rewrites this source's own bonus rows instead of duplicating them.
pub async fn apply_meta(
    db: &PgPool,
    provider: &str,
    external_id: &str,
    source: &str,
    meta: &OfficialMeta,
) -> anyhow::Result<usize> {
    let (ch_provider, ch_id) = chapter_key(db, provider, external_id).await;

    // Counter: raise-only (`raised_value`, same rule as `chapter_enrich`), only
    // for non-manual counts, with ONE exception: a detailed meta from the SAME
    // source supersedes that source's earlier reading when that reading counts
    // bonus rows — the coarse count provider (`chapters_source =
    // 'auto:<source>'`, kakao's `on_sale_count` includes the `0화` prologue) or
    // this meta's own earlier pass (`official_<source>`, e.g. an afterword that
    // a newer marker reclassified from main to bonus). The lowering is bounded
    // by the bonus rows the walk actually found — a larger gap means a truncated
    // answer, so the counter is left untouched.
    //
    // The row is read ONLY when the source reported a count (no point spending
    // a query otherwise); a failed read is debug-logged instead of propagated —
    // chapter rows must never be lost because the counter could not be read
    // (drawer preview, transient DB error): we simply continue without raising.
    let mut raise_to: Option<i32> = None;
    // True when `raise_to` may LOWER the stored counter (same-source supersede):
    // the write below then assigns instead of `GREATEST`.
    let mut may_lower = false;
    if let Some(count) = meta.count {
        match crate::services::chapters::get_chapter_meta(db, provider, external_id).await {
            Ok(Some((current, _, stored_source))) => {
                let auto_source = format!("auto:{source}");
                let official_source = format!("{OFFICIAL_SOURCE_PREFIX}{source}");
                let bonus_rows = meta.chapters.iter().filter(|c| c.is_bonus).count() as i32;
                // Both readings may sit above the walk's main-only count: the
                // coarse provider counts bonuses, and this meta's own earlier
                // pass counted a row that is a bonus now.
                let same_source = stored_source.as_deref() == Some(auto_source.as_str())
                    || stored_source.as_deref() == Some(official_source.as_str());
                let bounded = current
                    .is_some_and(|cur| count > 0 && count <= cur && cur - count <= bonus_rows);
                if same_source && bounded {
                    raise_to = Some(count);
                    may_lower = true;
                } else {
                    raise_to = raised_value(current, count);
                }
            }
            Ok(None) => {}
            Err(e) => tracing::debug!(provider, external_id, source, error = %e,
                "official_meta: counter read failed, raise skipped"),
        }
    }

    let mut written = 0usize;
    let mut tx = db.begin().await?;

    // The partial unique index covers main rows only, so a bonus INSERT has no
    // conflict target: a re-apply would duplicate the source's bonus rows.
    // Wipe this source's own bonuses first so a re-apply rewrites them instead.
    // `origin='manual'` bonuses and other sources' bonuses are never touched.
    //
    // Guarded by the same non-empty check as the orphan cleanup: an answer
    // without episodes (schedule-only meta, `is_useful_meta` still lets it
    // through) must not delete bonuses it never meant to replace — the source
    // changed its layout and reported only the schedule this time.
    if !meta.chapters.is_empty() {
        sqlx::query(
            "DELETE FROM series_chapters \
             WHERE provider = $1 AND external_id = $2 AND is_bonus AND origin = $3",
        )
        .bind(&ch_provider)
        .bind(&ch_id)
        .bind(source)
        .execute(&mut *tx)
        .await?;
    }

    // Bonus rows get a distinct NEGATIVE key -(total - i) * 100 in source order,
    // so `ORDER BY chapter_number ASC` lists them oldest-first, plus a display
    // label. The label the source already knows wins (a split part `479-1`,
    // including a duplicate part); otherwise the original number ('49.5') or a
    // running counter for unnumbered specials (migration 029). Main rows keep
    // their positive x100 number and the source's label (NULL unless the source
    // reported a split part, bilibili or kuaikan). A re-apply reproduces the
    // same keys/labels.
    let bonus_total = meta.chapters.iter().filter(|c| c.is_bonus).count() as i32;
    let mut bonus_i = 0i32;
    let mut unnumbered = 0i32;
    for ch in &meta.chapters {
        let (chapter_number, chapter_label): (i32, Option<String>) = if ch.is_bonus {
            let key = -(bonus_total - bonus_i) * 100;
            bonus_i += 1;
            let label = ch.label.clone().unwrap_or_else(|| {
                if ch.number_x100 > 0 {
                    crate::services::chapters::format_chapter(ch.number_x100)
                } else {
                    unnumbered += 1;
                    unnumbered.to_string()
                }
            });
            (key, Some(label))
        } else {
            (ch.number_x100, ch.label.clone())
        };
        let res = sqlx::query(
            r#"
            INSERT INTO series_chapters
                (provider, external_id, chapter_number, chapter_label, title_en, release_date, origin, is_bonus)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (provider, external_id, chapter_number) WHERE NOT is_bonus DO UPDATE
            SET title_en     = COALESCE(series_chapters.title_en,     EXCLUDED.title_en),
                release_date = COALESCE(series_chapters.release_date, EXCLUDED.release_date),
                chapter_label = COALESCE(series_chapters.chapter_label, EXCLUDED.chapter_label),
                fetched_at   = NOW()
            "#,
        )
        .bind(&ch_provider)
        .bind(&ch_id)
        .bind(chapter_number)
        .bind(chapter_label)
        .bind(ch.title.as_deref())
        .bind(ch.release_date)
        .bind(source)
        .bind(ch.is_bonus)
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() > 0 {
            written += 1;
        }
    }

    // Counter write (identity key): raise-only, skipped for `chapters_manual`
    // rows — the WHERE clause keeps a hand-set count safe even on a race.
    if let Some(next) = raise_to {
        // `GREATEST` keeps the raise race-safe: `raise_to` came from a
        // pre-transaction read, so a concurrent raise must not be lowered. The
        // same-source supersede is the one lowering branch, and it re-checks
        // the provenance INSIDE the statement: if the row no longer belongs to
        // this source (`auto:<source>` or `official_<source>`), the write falls
        // back to a plain raise.
        sqlx::query(
            "UPDATE media_items \
             SET chapters = CASE WHEN $5 AND chapters_source IN ($6, $7) THEN $3 \
                                 ELSE GREATEST(chapters, $3) END, \
                 chapters_source = $4, updated_at = NOW() \
             WHERE provider = $1 AND external_id = $2 AND chapters_manual = FALSE",
        )
        .bind(provider)
        .bind(external_id)
        .bind(next)
        .bind(format!("{OFFICIAL_SOURCE_PREFIX}{source}"))
        .bind(may_lower)
        .bind(format!("auto:{source}"))
        .bind(format!("{OFFICIAL_SOURCE_PREFIX}{source}"))
        .execute(&mut *tx)
        .await?;
    }

    // Schedule/next-date: never write an empty string over a non-empty value;
    // next_update_at only advances when the source reports a date.
    let schedule_res = sqlx::query(
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

    // Warn whenever the schedule UPDATE hit nothing while chapter rows were
    // written: the counter/schedule has nowhere to land. The gate is a
    // non-empty chapter list — with no chapters at all there is nothing
    // deferred (design :97-98).
    if schedule_res.rows_affected() == 0 && !meta.chapters.is_empty() {
        // Normal for a drawer preview: chapter rows are stored under the source
        // key, the item is not in the tracker yet -> counter/schedule deferred.
        tracing::warn!(
            provider,
            external_id,
            source,
            written,
            "official_meta: no media_items row, counter/schedule deferred"
        );
    }

    // Orphan cleanup: rows under the resolved key that the official answer does
    // not mention anymore, and that nobody ever read. Two kinds: MU-skeleton
    // rows, and THIS source's own main rows it no longer reports — a re-apply
    // can reclassify a row (an afterword that a newer marker turns into a bonus
    // would otherwise leave a stale main duplicating it). This source's bonus
    // rows are excluded: they are rewritten above with their own negative keys.
    // `read` rows, rows with a read progress entry of any user and
    // `origin='manual'` rows are kept (design :99-101). Skipped entirely for an
    // empty chapter list: an answer without episodes must never delete the
    // fallback skeleton.
    if !meta.chapters.is_empty() {
        let numbers: Vec<i32> = meta.chapters.iter().map(|c| c.number_x100).collect();
        let removed = sqlx::query(
            r#"
            DELETE FROM series_chapters sc
            WHERE sc.provider = $1 AND sc.external_id = $2
              AND (sc.origin = 'mu' OR (sc.origin = $4 AND NOT sc.is_bonus))
              AND NOT sc.read
              AND NOT (sc.chapter_number = ANY($3::int[]))
              AND NOT EXISTS (SELECT 1 FROM user_chapter_progress p
                              WHERE p.provider = sc.provider
                                AND p.external_id = sc.external_id
                                AND p.chapter_number = sc.chapter_number
                                AND p.read)
            "#,
        )
        .bind(&ch_provider)
        .bind(&ch_id)
        .bind(&numbers)
        .bind(source)
        .execute(&mut *tx)
        .await?;
        if removed.rows_affected() > 0 {
            tracing::warn!(
                provider,
                external_id,
                source,
                orphans = removed.rows_affected(),
                "official_meta: removed unread rows missing from the official answer"
            );
        }
    }

    tx.commit().await?;
    // `raised` is what was actually written to `media_items.chapters`
    // (`None` = raise-only declined it), `official_count` is what the source
    // reported — together they say "we wrote N rows, counter now at X".
    tracing::info!(provider, external_id, source, chapters = written,
        raised = ?raise_to, official_count = ?meta.count,
        "official_meta: applied");
    Ok(written)
}

/// Fold shadowed MU duplicates into the official rows of the same base.
///
/// A title that upgraded from the MU skeleton can keep MU rows under the SAME
/// base chapter as a newer official row (ToDG: MU `Гл. 82` next to official
/// parts `82-1`/`82-2`). Unread ones are already dropped by the orphan cleanup
/// in [`apply_meta`]; the ones carrying read progress survive on purpose. This
/// moves that read state onto the official rows of the base (every part of a
/// read chapter counts as read) and removes the shadowed MU rows, so the
/// chapter list stops showing both. All users are handled in one pass. A row
/// whose read flag lives only in the legacy `series_chapters.read` column (no
/// per-user progress) is left in place.
///
/// Runs on drawer open after resolve/enrich, so existing duplicates are folded
/// without a migration. Aliases that resolve to the same official key share
/// these rows by design; the fold is key-scoped and never merges two titles'
/// catalogs. Idempotent; returns the number of MU rows removed.
pub async fn merge_mu_duplicates(
    db: &PgPool,
    provider: &str,
    external_id: &str,
) -> anyhow::Result<u64> {
    let (ch_provider, ch_id) = chapter_key(db, provider, external_id).await;
    // One statement: every sub-query reads the same snapshot, so the row and
    // progress deletes cannot race the read-state transfer.
    let removed: i64 = sqlx::query_scalar(
        r#"
        WITH shadowed AS (
            SELECT mu.chapter_number, mu.read AS legacy_read
            FROM series_chapters mu
            WHERE mu.provider = $1 AND mu.external_id = $2
              AND mu.origin = 'mu'
              AND NOT mu.is_bonus
              AND mu.chapter_number >= 0
              AND EXISTS (
                  SELECT 1 FROM series_chapters o
                  WHERE o.provider = mu.provider
                    AND o.external_id = mu.external_id
                    AND o.origin <> 'mu'
                    AND NOT o.is_bonus
                    AND o.chapter_number / 100 = mu.chapter_number / 100
              )
        ),
        readable AS (
            SELECT s.chapter_number
            FROM shadowed s
            WHERE NOT s.legacy_read
               OR EXISTS (
                   SELECT 1 FROM user_chapter_progress p
                   WHERE p.provider = $1 AND p.external_id = $2
                     AND p.chapter_number = s.chapter_number
                     AND p.read
               )
        ),
        moved AS (
            INSERT INTO user_chapter_progress
                (user_id, provider, external_id, chapter_number, read, read_at)
            SELECT DISTINCT ON (p.user_id, o.chapter_number)
                   p.user_id, o.provider, o.external_id, o.chapter_number, TRUE, p.read_at
            FROM readable s
            JOIN series_chapters o
              ON o.provider = $1 AND o.external_id = $2
             AND o.origin <> 'mu'
             AND NOT o.is_bonus
             AND o.chapter_number / 100 = s.chapter_number / 100
            JOIN user_chapter_progress p
              ON p.provider = $1 AND p.external_id = $2
             AND p.chapter_number = s.chapter_number
             AND p.read
            ORDER BY p.user_id, o.chapter_number, p.read_at ASC NULLS LAST
            ON CONFLICT (user_id, provider, external_id, chapter_number) DO UPDATE
            SET read = TRUE,
                read_at = COALESCE(user_chapter_progress.read_at, EXCLUDED.read_at)
            RETURNING 1
        ),
        dropped_progress AS (
            DELETE FROM user_chapter_progress p
            USING readable s
            WHERE p.provider = $1 AND p.external_id = $2
              AND p.chapter_number = s.chapter_number
              AND p.read
            RETURNING 1
        ),
        dropped_rows AS (
            DELETE FROM series_chapters mu
            USING readable s
            WHERE mu.provider = $1 AND mu.external_id = $2
              AND mu.chapter_number = s.chapter_number
              AND mu.origin = 'mu'
            RETURNING 1
        )
        SELECT (SELECT count(*) FROM dropped_rows)
        "#,
    )
    .bind(&ch_provider)
    .bind(&ch_id)
    .fetch_one(db)
    .await?;
    if removed > 0 {
        tracing::info!(
            provider,
            external_id,
            removed,
            "official_meta: merged shadowed MU rows into official chapters"
        );
    }
    Ok(removed as u64)
}

/// True when the source actually told us something. An empty answer (no arm in
/// `fetch_source` -> `Ok(OfficialMeta::default)`, or a parse that yielded
/// nothing) is NOT a success: the chain must move on to the next source.
pub fn is_useful_meta(meta: &OfficialMeta) -> bool {
    !meta.chapters.is_empty()
        || meta.count.is_some()
        || meta.update_schedule.is_some()
        || meta.next_update_at.is_some()
}

/// One chain step: fetch a source's detail by its bound id. Injectable so the
/// whole chain (binding lookup + `apply_meta`) can run offline from tests —
/// same idea as `chapter_enrich::enrich_with_providers`.
pub trait MetaFetcher: Send + Sync {
    fn fetch<'a>(&'a self, source: &'a str, source_id: &'a str) -> MetaFuture<'a>;
}

/// Production fetcher: the shared HTTP client dispatches through
/// `official_meta::fetch_source`.
impl MetaFetcher for reqwest::Client {
    fn fetch<'a>(&'a self, source: &'a str, source_id: &'a str) -> MetaFuture<'a> {
        Box::pin(official_meta::fetch_source(source, self, source_id))
    }
}

/// The chain core with an injectable fetcher: resolve the highest-priority
/// bound source for `media_type`, fetch its detail and apply it. No binding ->
/// no network. Errors -> silent no-op. An empty answer is skipped (debug log)
/// so the next bound source is still tried. Ungated — callers that want the
/// freshness gate go through [`enrich_official_meta_for_key`].
pub async fn enrich_official_meta_with(
    db: &PgPool,
    provider: &str,
    external_id: &str,
    media_type: &str,
    fetcher: &dyn MetaFetcher,
) -> anyhow::Result<usize> {
    let sources = crate::services::official_resolve::official_source_for_media_type(media_type);
    for source in sources {
        let Some(bound) = source_ids::get_source_id(db, provider, external_id, source).await?
        else {
            continue;
        };
        let meta = match fetcher.fetch(source, &bound).await {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(provider, external_id, source, error = %e,
                    "official_meta: fetch failed");
                continue;
            }
        };
        if !is_useful_meta(&meta) {
            // Empty answer != success: keep walking the chain (an unknown
            // source name has no arm in `fetch_source` and lands here).
            tracing::debug!(
                provider,
                external_id,
                source,
                "official_meta: empty meta, trying next source"
            );
            continue;
        }
        // `apply_meta` logs the success itself (source + written rows), so this
        // stays quiet and only the chain's control flow is left here.
        return apply_meta(db, provider, external_id, source, &meta).await;
    }
    Ok(0)
}

/// How long an applied official meta is trusted before the drawer re-fetches
/// its source. The refresh worker and the manual bind bypass the gate
/// ([`enrich_official_meta_for_key_force`]), so a tracked title still re-walks
/// at least twice a day while every drawer open inside the window is instant.
pub const META_TTL_HOURS: i32 = 12;

/// True when the resolved key already carries official rows applied recently:
/// the drawer skips the whole chain walk for them. A failed check counts as
/// "not fresh" (fetch), which is always safe.
async fn official_meta_is_fresh(db: &PgPool, provider: &str, external_id: &str) -> bool {
    let (ch_provider, ch_id) = chapter_key(db, provider, external_id).await;
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM series_chapters \
         WHERE provider = $1 AND external_id = $2 AND origin <> 'mu' \
           AND fetched_at > NOW() - make_interval(hours => $3))",
    )
    .bind(&ch_provider)
    .bind(&ch_id)
    .bind(META_TTL_HOURS)
    .fetch_one(db)
    .await
    .unwrap_or(false)
}

/// Convenience wrapper for callers that only have a db + key. `media_type`
/// carries the drawer's hint: `Some(mt)` enriches immediately (drawer preview,
/// no `media_items` row yet); `None` discovers the type from `media_items` and
/// a missing row stays a no-op. Freshness-gated: a meta applied within
/// [`META_TTL_HOURS`] short-circuits without any network I/O.
pub async fn enrich_official_meta_for_key(
    db: &PgPool,
    provider: &str,
    external_id: &str,
    media_type: Option<&str>,
) -> anyhow::Result<usize> {
    let client = crate::services::external::http_client();
    enrich_official_meta_for_key_with(db, provider, external_id, media_type, &client, false).await
}

/// [`enrich_official_meta_for_key`] without the freshness gate: the refresh
/// worker and the manual bind always re-walk the bound source.
pub async fn enrich_official_meta_for_key_force(
    db: &PgPool,
    provider: &str,
    external_id: &str,
    media_type: Option<&str>,
) -> anyhow::Result<usize> {
    let client = crate::services::external::http_client();
    enrich_official_meta_for_key_with(db, provider, external_id, media_type, &client, true).await
}

/// The key-based chain with an injectable fetcher and an explicit gate switch,
/// so tests can prove that a fresh meta is not fetched again.
pub async fn enrich_official_meta_for_key_with(
    db: &PgPool,
    provider: &str,
    external_id: &str,
    media_type: Option<&str>,
    fetcher: &dyn MetaFetcher,
    force: bool,
) -> anyhow::Result<usize> {
    if !force && official_meta_is_fresh(db, provider, external_id).await {
        tracing::debug!(
            provider,
            external_id,
            ttl_hours = META_TTL_HOURS,
            "official_meta: fresh, skipping the source walk"
        );
        return Ok(0);
    }
    let media_type = match media_type {
        Some(mt) => mt.to_string(),
        None => {
            let discovered: Option<String> = sqlx::query_scalar(
                "SELECT media_type FROM media_items WHERE provider = $1 AND external_id = $2",
            )
            .bind(provider)
            .bind(external_id)
            .fetch_optional(db)
            .await?;
            match discovered {
                Some(mt) => mt,
                // Normal case: the item is not in the tracker yet (add flow).
                None => {
                    tracing::debug!(
                        provider,
                        external_id,
                        "official_meta: no media_items row, nothing to enrich"
                    );
                    return Ok(0);
                }
            }
        }
    };
    enrich_official_meta_with(db, provider, external_id, &media_type, fetcher).await
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
                is_bonus: false,
                label: None,
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

    /// A meta shaped like the `_ => Ok(OfficialMeta::default())` arm
    /// (`fetch_source` unknown source), with the binding name filled in.
    fn empty_meta() -> OfficialMeta {
        OfficialMeta {
            source: "kuaikan".into(),
            chapters: Vec::new(),
            count: None,
            update_schedule: None,
            next_update_at: None,
        }
    }

    #[test]
    fn empty_meta_is_not_useful() {
        assert!(!is_useful_meta(&empty_meta()));
        assert!(
            !is_useful_meta(&OfficialMeta::default()),
            "a default meta carries nothing and must not count as success"
        );
    }

    #[test]
    fn chapters_alone_make_meta_useful() {
        let mut m = empty_meta();
        m.chapters = meta_with(Some("Ch 1"), None).chapters;
        assert!(is_useful_meta(&m));
    }

    #[test]
    fn count_alone_makes_meta_useful() {
        let mut m = empty_meta();
        m.count = Some(12);
        assert!(is_useful_meta(&m));
    }

    #[test]
    fn update_schedule_alone_makes_meta_useful() {
        let mut m = empty_meta();
        m.update_schedule = Some("매주 월요일".into());
        assert!(is_useful_meta(&m));
    }

    #[test]
    fn next_update_at_alone_makes_meta_useful() {
        let mut m = empty_meta();
        m.next_update_at = official_meta::parse_date("2026-09-30");
        assert!(m.next_update_at.is_some(), "date must parse");
        assert!(is_useful_meta(&m));
    }
}
