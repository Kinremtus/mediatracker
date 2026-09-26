use crate::services::external::mangadex::{
    MangaDexChapter, MangaDexMangaAttributes, MangaDexService,
};
use sqlx::PgPool;
use std::collections::HashMap;

/// Chapter as returned to the template layer.
#[derive(Debug, Clone)]
pub struct StoredChapter {
    pub chapter_number: i32,
    pub volume: Option<i32>,
    pub title_en: Option<String>,
    pub title_ru: Option<String>,
    pub release_date: Option<chrono::NaiveDate>,
    pub read: bool,
}

impl StoredChapter {
    pub fn formatted(&self) -> String {
        format_chapter(self.chapter_number)
    }
}

// Chapter number stored in DB as integer * 100 (1050 = chapter 10.5,
// 1 = chapter 0.01, 100 = chapter 1). UI helpers format it back.

/// Format chapter number for display:
/// 100 -> "1", 1050 -> "10.5", 1 -> "0.01", 10110 -> "101.1".
pub fn format_chapter(chapter_number: i32) -> String {
    let whole = chapter_number / 100;
    let frac = chapter_number % 100;
    if frac == 0 {
        format!("{}", whole)
    } else if frac % 10 == 0 {
        format!("{}.{}", whole, frac / 10)
    } else {
        format!("{}.{:02}", whole, frac)
    }
}

/// Parse a chapter string like "10", "10.5" or "0.01" into the stored integer
/// (scale x100). The fractional part is normalized to exactly two digits:
/// "10.5" -> 1050, "0.01" -> 1, "101.1" -> 10110, "10.05" -> 1005.
/// Digits beyond the second are ignored. Negative numbers are rejected.
pub fn parse_chapter(s: &str) -> Option<i32> {
    let s = s.trim();
    if s.starts_with('-') {
        return None;
    }
    let parts: Vec<&str> = s.split('.').collect();
    match parts.len() {
        1 => {
            let whole: i32 = parts[0].parse().ok()?;
            if whole < 0 {
                return None;
            }
            Some(whole * 100)
        }
        2 => {
            let whole: i32 = parts[0].parse().ok()?;
            if whole < 0 {
                return None;
            }
            let frac_str = parts[1];
            let mut chars = frac_str.chars();
            let d1 = chars.next()?.to_digit(10)? as i32;
            let d2 = match chars.next() {
                Some(c) => c.to_digit(10)? as i32,
                None => 0,
            };
            Some(whole * 100 + d1 * 10 + d2)
        }
        _ => None,
    }
}

/// Insert or update chapter skeleton in the DB. UNIQUE (provider,
/// external_id, chapter_number) makes the operation idempotent.
///
/// This creates the "skeleton" 1..N chapters from MangaUpdates'
/// `latest_chapter` field. Titles and detailed metadata can be
/// filled later from MangaDex (Stage 2). For now we store bare
/// chapter numbers so the checkbox UI works.
///
/// Skips if `latest_chapter` is 0 or None (e.g. for manga that
/// haven't started publishing yet).
pub async fn store_chapters_mu(
    pool: &PgPool,
    series_id: i64,
    latest_chapter: i32,
) -> Result<usize, sqlx::Error> {
    if latest_chapter <= 0 {
        return Ok(0);
    }

    let external_id = series_id.to_string();
    // Chapter 1 -> 100, chapter 2 -> 200, … (fraction goes in the low digits).
    let chapter_numbers: Vec<i32> = (1..=latest_chapter).map(|ch| ch * 100).collect();

    // One statement instead of N: a mid-way failure can no longer leave a
    // partially populated skeleton. The media_items sync runs in the same
    // transaction so the card denominator can never disagree with the rows.
    let mut tx = pool.begin().await?;

    let result = sqlx::query(
        r#"
        INSERT INTO series_chapters
            (provider, external_id, chapter_number)
        SELECT 'mangaupdates', $1, u.chapter_number
        FROM UNNEST($2::int[]) AS u(chapter_number)
        ON CONFLICT (provider, external_id, chapter_number) DO NOTHING
        "#,
    )
    .bind(&external_id)
    .bind(&chapter_numbers)
    .execute(&mut *tx)
    .await?;

    let count = result.rows_affected();

    // Sync media_items.chapters = ceil(MAX(chapter_number) / 100) so the
    // tracking card denominator matches. Never lower an existing value.
    if count > 0 {
        sqlx::query(
            r#"
            UPDATE media_items
            SET chapters = GREATEST(COALESCE(media_items.chapters, 0), sub.max_ch)
            FROM (
                SELECT ((MAX(chapter_number) + 99) / 100) AS max_ch
                FROM series_chapters
                WHERE provider = 'mangaupdates' AND external_id = $1
            ) AS sub
            WHERE media_items.provider = 'mangaupdates'
              AND media_items.external_id = $1
              AND sub.max_ch > COALESCE(media_items.chapters, 0)
            "#,
        )
        .bind(&external_id)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(count as usize)
}

/// Read all chapters for one manga, sorted by number ascending. The
/// `read` flag comes from the per-user progress table for `user_id`.
pub async fn get_chapters(
    pool: &PgPool,
    provider: &str,
    external_id: &str,
    user_id: uuid::Uuid,
) -> Result<Vec<StoredChapter>, sqlx::Error> {
    #[allow(clippy::type_complexity)]
    let rows: Vec<(
        i32,
        Option<i32>,
        Option<String>,
        Option<String>,
        Option<chrono::NaiveDate>,
        bool,
    )> = sqlx::query_as(
        r#"
        SELECT sc.chapter_number, sc.volume, sc.title_en, sc.title_ru,
               sc.release_date, COALESCE(p.read, FALSE)
        FROM series_chapters sc
        LEFT JOIN user_chapter_progress p
            ON p.user_id = $3
           AND p.provider = sc.provider
           AND p.external_id = sc.external_id
           AND p.chapter_number = sc.chapter_number
        WHERE sc.provider = $1 AND sc.external_id = $2
        ORDER BY sc.chapter_number ASC
        "#,
    )
    .bind(provider)
    .bind(external_id)
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(chapter_number, volume, title_en, title_ru, release_date, read)| StoredChapter {
                chapter_number,
                volume,
                title_en,
                title_ru,
                release_date,
                read,
            },
        )
        .collect())
}

/// Read a single chapter by (provider, external_id, chapter_number),
/// with the per-user `read` flag joined from `user_chapter_progress`.
pub async fn get_chapter(
    pool: &PgPool,
    provider: &str,
    external_id: &str,
    chapter_number: i32,
    user_id: uuid::Uuid,
) -> Result<Option<StoredChapter>, sqlx::Error> {
    #[allow(clippy::type_complexity)]
    let row: Option<(
        i32,
        Option<i32>,
        Option<String>,
        Option<String>,
        Option<chrono::NaiveDate>,
        bool,
    )> = sqlx::query_as(
        r#"
        SELECT sc.chapter_number, sc.volume, sc.title_en, sc.title_ru,
               sc.release_date, COALESCE(p.read, FALSE)
        FROM series_chapters sc
        LEFT JOIN user_chapter_progress p
            ON p.user_id = $4
           AND p.provider = sc.provider
           AND p.external_id = sc.external_id
           AND p.chapter_number = sc.chapter_number
        WHERE sc.provider = $1 AND sc.external_id = $2 AND sc.chapter_number = $3
        "#,
    )
    .bind(provider)
    .bind(external_id)
    .bind(chapter_number)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(
        |(chapter_number, volume, title_en, title_ru, release_date, read)| StoredChapter {
            chapter_number,
            volume,
            title_en,
            title_ru,
            release_date,
            read,
        },
    ))
}

/// Writes go to `user_chapter_progress` keyed by `(user_id, provider,
/// external_id, chapter_number)`. The catalog table `series_chapters`
/// is only read as the source of real chapter numbers.
///
/// Returns `true` if at least one progress row was inserted/updated.
pub async fn set_read(
    pool: &PgPool,
    user_id: uuid::Uuid,
    provider: &str,
    external_id: &str,
    chapter_number: i32,
    read: bool,
) -> Result<bool, sqlx::Error> {
    let result = if read {
        sqlx::query(
            r#"
            INSERT INTO user_chapter_progress
                (user_id, provider, external_id, chapter_number, read, read_at)
            SELECT $1, sc.provider, sc.external_id, sc.chapter_number, TRUE, NOW()
            FROM series_chapters sc
            WHERE sc.provider = $2
              AND sc.external_id = $3
              AND sc.chapter_number <= $4
            ON CONFLICT (user_id, provider, external_id, chapter_number) DO UPDATE
            SET read = TRUE,
                read_at = NOW()
            "#,
        )
        .bind(user_id)
        .bind(provider)
        .bind(external_id)
        .bind(chapter_number)
        .execute(pool)
        .await?
    } else {
        sqlx::query(
            r#"
            UPDATE user_chapter_progress
            SET read = FALSE,
                read_at = NULL
            WHERE user_id = $1
              AND provider = $2
              AND external_id = $3
              AND chapter_number >= $4
            "#,
        )
        .bind(user_id)
        .bind(provider)
        .bind(external_id)
        .bind(chapter_number)
        .execute(pool)
        .await?
    };
    Ok(result.rows_affected() > 0)
}

/// Highest chapter_number currently marked read for this user.
/// Returns 0 if nothing is read.
pub async fn count_read(
    pool: &PgPool,
    user_id: uuid::Uuid,
    provider: &str,
    external_id: &str,
) -> Result<i32, sqlx::Error> {
    let row: (Option<i32>,) = sqlx::query_as(
        r#"
        SELECT MAX(sc.chapter_number)
        FROM series_chapters sc
        JOIN user_chapter_progress p
            ON p.user_id = $1
           AND p.provider = sc.provider
           AND p.external_id = sc.external_id
           AND p.chapter_number = sc.chapter_number
        WHERE sc.provider = $2
          AND sc.external_id = $3
          AND p.read = TRUE
        "#,
    )
    .bind(user_id)
    .bind(provider)
    .bind(external_id)
    .fetch_one(pool)
    .await?;
    // chapter_number is stored as ch * 100, so divide back for display
    // (integer division floors fractional progress, e.g. 10.5 -> 10).
    Ok(row.0.map(|v| v / 100).unwrap_or(0))
}

/// Get all (chapter_number, read) pairs for a manga and user, ordered
/// ASC. Used by the toggle endpoint to broadcast authoritative state
/// via HX-Trigger.
pub async fn get_chapter_states(
    pool: &PgPool,
    user_id: uuid::Uuid,
    provider: &str,
    external_id: &str,
) -> Result<Vec<(i32, bool)>, sqlx::Error> {
    let rows: Vec<(i32, bool)> = sqlx::query_as(
        r#"
        SELECT sc.chapter_number, COALESCE(p.read, FALSE)
        FROM series_chapters sc
        LEFT JOIN user_chapter_progress p
            ON p.user_id = $1
           AND p.provider = sc.provider
           AND p.external_id = sc.external_id
           AND p.chapter_number = sc.chapter_number
        WHERE sc.provider = $2 AND sc.external_id = $3
        ORDER BY sc.chapter_number ASC
        "#,
    )
    .bind(user_id)
    .bind(provider)
    .bind(external_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Mirrors the read chapters onto `tracking_entries.progress`:
/// sets it to `read_count`, so un-checking lowers the counter.
pub async fn update_progress_from_read(
    pool: &PgPool,
    user_id: uuid::Uuid,
    media_id: uuid::Uuid,
    read_count: i32,
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
    .bind(read_count)
    .bind(user_id)
    .bind(media_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Resolve the media_items.id from provider + external_id.
pub async fn lookup_media_id(
    pool: &PgPool,
    provider: &str,
    external_id: &str,
) -> Result<Option<uuid::Uuid>, sqlx::Error> {
    let row: Option<(uuid::Uuid,)> =
        sqlx::query_as("SELECT id FROM media_items WHERE provider = $1 AND external_id = $2")
            .bind(provider)
            .bind(external_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|(v,)| v))
}

/// One chapter row destined for `series_chapters`, produced from a MangaDex
/// feed entry after dedup by chapter number.
#[derive(Debug, Clone)]
pub struct MdChapterRow {
    pub chapter_number: i32,
    pub title_en: Option<String>,
    pub title_ru: Option<String>,
    pub volume: Option<i32>,
    pub release_date: Option<chrono::NaiveDate>,
}

/// Collapse a MangaDex feed into one row per chapter number.
///
/// Multiple translations/groups of the same chapter (en + ru, or two
/// scanlation groups) merge into one row; the first non-null value wins. This
/// matches the storage semantics where a filled column is never overwritten.
/// Entries with `chapter: null` (no number) are skipped with a debug log
/// instead of failing the whole feed.
fn build_md_rows(chapters: &[MangaDexChapter]) -> Vec<MdChapterRow> {
    let mut map: HashMap<i32, MdChapterRow> = HashMap::new();
    for ch in chapters {
        let Some(number) = ch.chapter_number_10() else {
            tracing::debug!(title = ?ch.title, "MangaDex: chapter without a number, skipping");
            continue;
        };
        let volume = ch.volume.as_ref().and_then(|v| v.parse::<i32>().ok());
        let release_date = ch
            .publish_at
            .as_deref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.date_naive());
        let (title_en, title_ru) = match ch.translated_language.as_str() {
            "en" => (ch.title.clone(), None),
            "ru" => (None, ch.title.clone()),
            _ => (None, None),
        };
        map.entry(number)
            .and_modify(|row| {
                if row.title_en.is_none() {
                    row.title_en = title_en.clone();
                }
                if row.title_ru.is_none() {
                    row.title_ru = title_ru.clone();
                }
                if row.volume.is_none() {
                    row.volume = volume;
                }
                if row.release_date.is_none() {
                    row.release_date = release_date;
                }
            })
            .or_insert(MdChapterRow {
                chapter_number: number,
                title_en,
                title_ru,
                volume,
                release_date,
            });
    }
    map.into_values().collect()
}

/// Insert missing chapters and COALESCE-fill existing ones in one statement.
/// `(provider, external_id, chapter_number)` is the dedup key, so repeated
/// enrichment is idempotent and never overwrites a filled column with NULL.
pub async fn upsert_series_chapters(
    pool: &PgPool,
    provider: &str,
    external_id: &str,
    rows: &[MdChapterRow],
) -> Result<usize, sqlx::Error> {
    if rows.is_empty() {
        return Ok(0);
    }
    let chapter_numbers: Vec<i32> = rows.iter().map(|r| r.chapter_number).collect();
    let title_en: Vec<Option<String>> = rows.iter().map(|r| r.title_en.clone()).collect();
    let title_ru: Vec<Option<String>> = rows.iter().map(|r| r.title_ru.clone()).collect();
    let volumes: Vec<Option<i32>> = rows.iter().map(|r| r.volume).collect();
    let release_dates: Vec<Option<chrono::NaiveDate>> =
        rows.iter().map(|r| r.release_date).collect();

    let mut tx = pool.begin().await?;
    let result = sqlx::query(
        r#"
        INSERT INTO series_chapters
            (provider, external_id, chapter_number, title_en, title_ru, volume, release_date)
        SELECT $1, $2, u.chapter_number, u.title_en, u.title_ru, u.volume, u.release_date
        FROM UNNEST($3::int[], $4::text[], $5::text[], $6::int[], $7::date[])
            AS u(chapter_number, title_en, title_ru, volume, release_date)
        ON CONFLICT (provider, external_id, chapter_number) DO UPDATE
        SET title_en = COALESCE(series_chapters.title_en, EXCLUDED.title_en),
            title_ru = COALESCE(series_chapters.title_ru, EXCLUDED.title_ru),
            volume = COALESCE(series_chapters.volume, EXCLUDED.volume),
            release_date = COALESCE(series_chapters.release_date, EXCLUDED.release_date)
        "#,
    )
    .bind(provider)
    .bind(external_id)
    .bind(&chapter_numbers)
    .bind(&title_en)
    .bind(&title_ru)
    .bind(&volumes)
    .bind(&release_dates)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    Ok(result.rows_affected() as usize)
}

/// Enrich chapter titles/dates from MangaDex.
/// Searches MangaDex by the manga title from media_items, fetches the chapter
/// list, and upserts every chapter (integer + fractional) into
/// `series_chapters`.
pub async fn enrich_from_mangadex(
    pool: &PgPool,
    provider: &str,
    external_id: &str,
) -> Result<usize, anyhow::Error> {
    // Get manga title from media_items
    let title: Option<String> = sqlx::query_scalar(
        "SELECT title FROM media_items WHERE provider = $1 AND external_id = $2",
    )
    .bind(provider)
    .bind(external_id)
    .fetch_optional(pool)
    .await?;

    let Some(title) = title else {
        return Ok(0);
    };

    // Search MangaDex
    let md = MangaDexService::new();
    let search_results = md.search_manga(&title).await?;
    if search_results.is_empty() {
        return Ok(0);
    }

    // MangaDex search is fuzzy and `search_results[0]` can be an unrelated
    // series: verify title similarity before writing another title's data.
    // Exact normalized matches win across the whole list, so a fuzzy hit
    // ranked first can never shadow a later exact hit.
    let attributes: Vec<MangaDexMangaAttributes> = search_results
        .iter()
        .map(|r| r.attributes.clone())
        .collect();
    let Some(idx) = select_mangadex_candidate(&title, &attributes) else {
        tracing::info!(
            title = %title,
            "MangaDex: no close title match, skipping enrichment"
        );
        return Ok(0);
    };
    let candidate = &search_results[idx];
    let md_chapters = md.get_chapters(&candidate.id).await?;

    let rows = build_md_rows(&md_chapters);
    if rows.is_empty() {
        return Ok(0);
    }

    // `provider` stays 'mangaupdates' even for MangaDex-sourced rows: it is the
    // storage key (provider='mangaupdates', external_id=MU series id).
    let affected = upsert_series_chapters(pool, provider, external_id, &rows).await?;
    Ok(affected)
}

/// Normalize a title for comparison: lowercase, strip apostrophes (so
/// "Extra's" and "Extras" collapse to the same token stream), then map every
/// other non-alphanumeric to a space and collapse whitespace.
fn normalize_title(raw: &str) -> String {
    raw.to_lowercase()
        .chars()
        .filter(|c| !matches!(c, '\'' | '\u{2019}' | '`'))
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Jaccard similarity over whitespace tokens.
fn token_overlap(a: &str, b: &str) -> f32 {
    let set_a: std::collections::HashSet<&str> = a.split(' ').collect();
    let set_b: std::collections::HashSet<&str> = b.split(' ').collect();
    if set_a.is_empty() || set_b.is_empty() {
        return 0.0;
    }
    let intersection = set_a.intersection(&set_b).count() as f32;
    let union = set_a.union(&set_b).count() as f32;
    intersection / union
}

/// Token overlap required in addition to a prefix relationship. "Solo Leveling"
/// vs "Solo Leveling: Ragnarok" is 2/3 (~0.667) and must match, while "Naruto"
/// vs "NARUTO: Sasuke Retsuden ..." is ~1/9 and must not.
const PREFIX_MIN_OVERLAP: f32 = 0.66;

/// True when ANY MangaDex title variant (primary `title` or any `altTitles`
/// value) plausibly denotes the local title. Deliberately conservative: a false
/// negative only skips enrichment, while a false positive would write chapter
/// titles from an unrelated series (the "Naruto" vs "Renge to Naruto!" bug).
///
/// A candidate matches only on:
///   1. exact equality after normalization, or
///   2. a whole-word prefix relationship AND token overlap >= 0.66.
///
/// Bare containment is intentionally NOT enough: 'naruto' is contained in
/// 'renge to naruto', which previously polluted the wrong series.
fn mangadex_title_matches(local: &str, attributes: &MangaDexMangaAttributes) -> bool {
    let local_norm = normalize_title(local);
    if local_norm.is_empty() {
        return false;
    }
    attributes.title_candidates().any(|candidate| {
        let candidate_norm = normalize_title(candidate);
        if candidate_norm.is_empty() {
            return false;
        }
        if candidate_norm == local_norm {
            return true;
        }
        let (shorter, longer) = if candidate_norm.len() <= local_norm.len() {
            (candidate_norm.as_str(), local_norm.as_str())
        } else {
            (local_norm.as_str(), candidate_norm.as_str())
        };
        is_word_prefix_of(shorter, longer)
            && token_overlap(&local_norm, &candidate_norm) >= PREFIX_MIN_OVERLAP
    })
}

/// True when `shorter` is `longer` or a whole-word prefix of it. The word
/// boundary stops "solo" from prefixing "sololeveling".
fn is_word_prefix_of(shorter: &str, longer: &str) -> bool {
    match longer.strip_prefix(shorter) {
        Some(rest) => rest.is_empty() || rest.starts_with(' '),
        None => false,
    }
}

/// True when the local title normalizes to exactly the same string as ANY
/// MangaDex title candidate (primary `title` or any `altTitles` value). This is
/// the strict half of `mangadex_title_matches`, exposed separately so exact
/// hits can be ranked above fuzzy ones. Empty titles never match.
fn mangadex_exact_match(local: &str, attributes: &MangaDexMangaAttributes) -> bool {
    let local_norm = normalize_title(local);
    if local_norm.is_empty() {
        return false;
    }
    attributes
        .title_candidates()
        .any(|candidate| normalize_title(candidate) == local_norm)
}

/// Pick the best MangaDex search result for `local`, returning its index.
///
/// Two passes: an exact normalized match anywhere in the list wins outright,
/// so a fuzzy hit that MangaDex happened to rank first cannot shadow a later
/// exact hit ("Solo Leveling": fuzzy "Solo Leveling: Ragnarok" at index 0 must
/// lose to the exact match at index 2). Only when no exact match exists do we
/// fall back to the fuzzy matcher, in result order.
fn select_mangadex_candidate(local: &str, results: &[MangaDexMangaAttributes]) -> Option<usize> {
    if let Some(idx) = results
        .iter()
        .position(|attributes| mangadex_exact_match(local, attributes))
    {
        return Some(idx);
    }
    results
        .iter()
        .position(|attributes| mangadex_title_matches(local, attributes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_chapter_integer() {
        assert_eq!(format_chapter(100), "1");
        assert_eq!(format_chapter(200), "2");
        assert_eq!(format_chapter(1000), "10");
    }

    #[test]
    fn format_chapter_fractional() {
        assert_eq!(format_chapter(1050), "10.5");
        assert_eq!(format_chapter(250), "2.5");
        assert_eq!(format_chapter(10110), "101.1");
        assert_eq!(format_chapter(1), "0.01");
        assert_eq!(format_chapter(1005), "10.05");
        assert_eq!(format_chapter(50), "0.5");
    }

    #[test]
    fn parse_chapter_integer() {
        assert_eq!(parse_chapter("1"), Some(100));
        assert_eq!(parse_chapter("10"), Some(1000));
        assert_eq!(parse_chapter("25"), Some(2500));
    }

    #[test]
    fn parse_chapter_fractional() {
        assert_eq!(parse_chapter("10.5"), Some(1050));
        assert_eq!(parse_chapter("1.1"), Some(110));
        assert_eq!(parse_chapter("25.0"), Some(2500));
        assert_eq!(parse_chapter("0.01"), Some(1));
        assert_eq!(parse_chapter("101.1"), Some(10110));
    }

    #[test]
    fn parse_chapter_invalid() {
        assert_eq!(parse_chapter("abc"), None);
        assert_eq!(parse_chapter(""), None);
        assert_eq!(parse_chapter("1.2.3"), None);
    }

    #[test]
    fn parse_chapter_edge_cases() {
        // Digits beyond the second are ignored (normalized to 2 places).
        assert_eq!(parse_chapter("10.50"), Some(1050));
        assert_eq!(parse_chapter("10.05"), Some(1005));
        assert_eq!(parse_chapter("10.99"), Some(1099));
        assert_eq!(parse_chapter("1.05"), Some(105));
        // No whole part
        assert_eq!(parse_chapter(".5"), None);
        // Negative not supported
        assert_eq!(parse_chapter("-1"), None);
    }

    #[test]
    fn stored_chapter_formatted() {
        let ch = StoredChapter {
            chapter_number: 1050,
            volume: Some(10),
            title_en: Some("Test".to_string()),
            title_ru: None,
            release_date: None,
            read: false,
        };
        assert_eq!(ch.formatted(), "10.5");

        let ch2 = StoredChapter {
            chapter_number: 200,
            volume: None,
            title_en: None,
            title_ru: None,
            release_date: None,
            read: true,
        };
        assert_eq!(ch2.formatted(), "2");
    }

    #[test]
    fn build_md_rows_skips_null_and_merges_duplicates() {
        let mk = |number: Option<&str>,
                  lang: &str,
                  title: Option<&str>,
                  volume: Option<&str>,
                  published: Option<&str>| MangaDexChapter {
            title: title.map(str::to_string),
            chapter: number.map(str::to_string),
            volume: volume.map(str::to_string),
            translated_language: lang.to_string(),
            publish_at: published.map(str::to_string),
        };

        let feed = vec![
            mk(None, "en", Some("No number"), None, None),
            mk(
                Some("101.1"),
                "en",
                Some("Extra"),
                Some("12"),
                Some("2021-03-05T09:00:00+00:00"),
            ),
            mk(Some("101.1"), "ru", Some("Экстра"), Some("12"), None),
            mk(Some("5.5"), "en", Some("Special"), None, None),
        ];

        let mut rows = build_md_rows(&feed);
        rows.sort_by_key(|r| r.chapter_number);

        assert_eq!(rows.len(), 2, "null-number skipped; 101.1 merged");
        assert_eq!(rows[0].chapter_number, 550);
        assert_eq!(rows[0].title_en.as_deref(), Some("Special"));
        assert_eq!(rows[1].chapter_number, 10110);
        assert_eq!(rows[1].title_en.as_deref(), Some("Extra"));
        assert_eq!(rows[1].title_ru.as_deref(), Some("Экстра"));
        assert_eq!(rows[1].volume, Some(12));
        assert_eq!(
            rows[1].release_date,
            chrono::NaiveDate::from_ymd_opt(2021, 3, 5)
        );
    }

    /// Build MangaDex attributes from JSON so the tests exercise the real
    /// deserialization path (including the `altTitles` default).
    fn md_attrs(json: &str) -> MangaDexMangaAttributes {
        serde_json::from_str(json).expect("valid MangaDex attributes JSON")
    }

    #[test]
    fn title_match_accepts_identical_and_normalized_variants() {
        let attrs = md_attrs(r#"{"title":{"en":"Solo Leveling"}}"#);
        assert!(mangadex_title_matches("Solo Leveling", &attrs));
        assert!(mangadex_title_matches("solo leveling!", &attrs));
    }

    #[test]
    fn title_match_accepts_prefix_extension() {
        // "Solo Leveling: Ragnarok" shares both tokens (jaccard 2/3) and extends
        // the local title -> same franchise, safe to enrich.
        let attrs = md_attrs(r#"{"title":{"en":"Solo Leveling: Ragnarok"}}"#);
        assert!(mangadex_title_matches("Solo Leveling", &attrs));
    }

    #[test]
    fn title_match_accepts_romaji_variant() {
        let attrs = md_attrs(r#"{"title":{"ja":"Ore dake Level Up na Ken"}}"#);
        assert!(mangadex_title_matches("Ore dake Level Up na Ken", &attrs));
    }

    #[test]
    fn title_match_rejects_unrelated_and_empty() {
        let attrs = md_attrs(r#"{"title":{"en":"Berserk"}}"#);
        assert!(!mangadex_title_matches("Solo Leveling", &attrs));
        assert!(!mangadex_title_matches("", &attrs));
        let empty = md_attrs(r#"{"title":{}}"#);
        assert!(!mangadex_title_matches("Solo Leveling", &empty));
    }

    #[test]
    fn title_match_rejects_containment_regression() {
        // Prod incident: "Naruto" must NOT match "Renge to Naruto!" just because
        // the shorter string is contained in the longer one (jaccard 1/3).
        let attrs = md_attrs(r#"{"title":{"en":"Renge to Naruto!"}}"#);
        assert!(!mangadex_title_matches("Naruto", &attrs));
    }

    #[test]
    fn title_match_rejects_long_prefix_with_low_overlap() {
        // "Naruto" IS a whole-word prefix here, but only 1 of 9 tokens overlap.
        let attrs = md_attrs(
            r#"{"title":{"en":"NARUTO: Sasuke Retsuden—Uchiha no Matsuei to Tenkyuu no Hoshikuzu"}}"#,
        );
        assert!(!mangadex_title_matches("Naruto", &attrs));
    }

    #[test]
    fn title_match_uses_alt_titles() {
        // The exact English name lives only in altTitles; the primary map is Korean.
        let attrs = md_attrs(
            r#"{"title":{"ko-ro":"Academy eseo Saranamgi"},"altTitles":[{"en":"The Extra's Academy Survival Guide"}]}"#,
        );
        assert!(mangadex_title_matches(
            "The Extra's Academy Survival Guide",
            &attrs
        ));
    }

    #[test]
    fn title_match_strips_apostrophes() {
        // "Extra's" vs "Extras" must collapse to the same normalized tokens.
        let attrs = md_attrs(r#"{"title":{"en":"The Extras Academy Survival Guide"}}"#);
        assert!(mangadex_title_matches(
            "The Extra's Academy Survival Guide",
            &attrs
        ));
    }

    #[test]
    fn title_match_rejects_no_shared_tokens() {
        let attrs = md_attrs(r#"{"title":{"en":"Only I Level Up"}}"#);
        assert!(!mangadex_title_matches("Solo Leveling", &attrs));
    }

    #[test]
    fn select_candidate_prefers_exact_over_fuzzy() {
        // Fuzzy Ragnarok at index 0 must not shadow the exact match at index 2.
        let results = vec![
            md_attrs(r#"{"title":{"en":"Solo Leveling: Ragnarok"}}"#),
            md_attrs(r#"{"title":{"en":"Berserk"}}"#),
            md_attrs(r#"{"title":{"en":"Solo Leveling"}}"#),
        ];
        assert_eq!(
            select_mangadex_candidate("Solo Leveling", &results),
            Some(2)
        );
    }

    #[test]
    fn select_candidate_falls_back_to_fuzzy() {
        let results = vec![
            md_attrs(r#"{"title":{"en":"Solo Leveling: Ragnarok"}}"#),
            md_attrs(r#"{"title":{"en":"Berserk"}}"#),
        ];
        assert_eq!(
            select_mangadex_candidate("Solo Leveling", &results),
            Some(0)
        );
    }

    #[test]
    fn select_candidate_returns_none_without_match() {
        let results = vec![md_attrs(r#"{"title":{"en":"Berserk"}}"#)];
        assert_eq!(select_mangadex_candidate("Solo Leveling", &results), None);
    }

    #[test]
    fn exact_match_requires_normalized_equality() {
        let fuzzy = md_attrs(r#"{"title":{"en":"Solo Leveling: Ragnarok"}}"#);
        assert!(!mangadex_exact_match("Solo Leveling", &fuzzy));
        let exact = md_attrs(
            r#"{"title":{"ko-ro":"Academy eseo Saranamgi"},"altTitles":[{"en":"The Extra's Academy Survival Guide"}]}"#,
        );
        assert!(mangadex_exact_match(
            "The Extra's Academy Survival Guide",
            &exact
        ));
        assert!(!mangadex_exact_match("", &exact));
    }
}
