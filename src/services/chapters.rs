use crate::services::external::mangadex::MangaDexService;
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

// Chapter number stored in DB as integer * 10 (105 = chapter 10.5).
// UI helpers below format it back to human-readable form.

/// Format chapter number for display: 10 → "1", 105 → "10.5", 120 → "12".
pub fn format_chapter(chapter_number: i32) -> String {
    if chapter_number % 10 == 0 {
        format!("{}", chapter_number / 10)
    } else {
        let whole = chapter_number / 10;
        let frac = chapter_number % 10;
        format!("{}.{}", whole, frac)
    }
}

/// Parse a chapter string like "10" or "10.5" into the stored integer.
/// Only first digit after decimal is kept (tenths): "10.5" → 105, "10.05" → 100.
/// Negative numbers are rejected.
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
            Some(whole * 10)
        }
        2 => {
            let whole: i32 = parts[0].parse().ok()?;
            if whole < 0 {
                return None;
            }
            let frac_str = parts[1];
            // Take only first digit (tenths). "5"→5, "50"→5, "05"→0.
            let frac: i32 = frac_str.chars().next()?.to_digit(10)? as i32;
            Some(whole * 10 + frac)
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
    // Chapter 1 → 10, chapter 2 → 20, … (tenths are stored in the low digit).
    let chapter_numbers: Vec<i32> = (1..=latest_chapter).map(|ch| ch * 10).collect();

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

    // Sync media_items.chapters = MAX(chapter_number) / 10 of what
    // we just stored, so the tracking card denominator matches.
    if count > 0 {
        sqlx::query(
            r#"
            UPDATE media_items
            SET chapters = sub.max_ch
            FROM (
                SELECT MAX(chapter_number) / 10 AS max_ch
                FROM series_chapters
                WHERE provider = 'mangaupdates' AND external_id = $1
            ) AS sub
            WHERE media_items.provider = 'mangaupdates'
              AND media_items.external_id = $1
              AND (media_items.chapters IS NULL OR media_items.chapters = 0)
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
    // chapter_number is stored as ch * 10, so divide back for display
    Ok(row.0.map(|v| v / 10).unwrap_or(0))
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

/// Bumps `tracking_entries.progress` to at least `read_count`.
/// Uses GREATEST semantics — manual progress never regresses.
pub async fn update_progress_from_read(
    pool: &PgPool,
    user_id: uuid::Uuid,
    media_id: uuid::Uuid,
    read_count: i32,
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

/// Enrich chapter titles from MangaDex.
/// Searches MangaDex by the manga title from media_items,
/// fetches chapter list, and updates title_en/title_ru/volume
/// for matching chapter numbers.
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
    // series: verify title similarity before overwriting chapter titles with
    // another title's data.
    let Some(candidate) = search_results
        .iter()
        .find(|r| mangadex_title_matches(&title, &r.attributes.title))
    else {
        tracing::info!(
            title = %title,
            "MangaDex: no close title match, skipping enrichment"
        );
        return Ok(0);
    };
    let md_chapters = md.get_chapters(&candidate.id).await?;

    // Build lookup: chapter_number_10 -> (title_en, title_ru, volume)
    #[allow(clippy::type_complexity)]
    let mut md_map: HashMap<i32, (Option<String>, Option<String>, Option<i32>)> = HashMap::new();
    for ch in md_chapters {
        if let Some(ch_num_10) = ch.chapter_number_10() {
            let vol = ch.volume.as_ref().and_then(|v| v.parse::<i32>().ok());
            let (en, ru) = match ch.translated_language.as_str() {
                "en" => (ch.title.clone(), None),
                "ru" => (None, ch.title.clone()),
                _ => (None, None),
            };
            md_map
                .entry(ch_num_10)
                .and_modify(|(e, r, v)| {
                    if e.is_none() {
                        *e = en.clone();
                    }
                    if r.is_none() {
                        *r = ru.clone();
                    }
                    if v.is_none() {
                        *v = vol;
                    }
                })
                .or_insert((en, ru, vol));
        }
    }

    // Update series_chapters in one batched statement (single round-trip and
    // atomic) instead of one UPDATE per chapter.
    if md_map.is_empty() {
        return Ok(0);
    }

    let mut chapter_numbers: Vec<i32> = Vec::with_capacity(md_map.len());
    let mut title_en: Vec<Option<String>> = Vec::with_capacity(md_map.len());
    let mut title_ru: Vec<Option<String>> = Vec::with_capacity(md_map.len());
    let mut volumes: Vec<Option<i32>> = Vec::with_capacity(md_map.len());
    for (ch_num_10, (en, ru, vol)) in md_map {
        chapter_numbers.push(ch_num_10);
        title_en.push(en);
        title_ru.push(ru);
        volumes.push(vol);
    }

    let mut tx = pool.begin().await?;
    let result = sqlx::query(
        r#"
        UPDATE series_chapters sc
        SET title_en = COALESCE(sc.title_en, u.title_en),
            title_ru = COALESCE(sc.title_ru, u.title_ru),
            volume = COALESCE(sc.volume, u.volume)
        FROM UNNEST($3::int[], $4::text[], $5::text[], $6::int[])
            AS u(chapter_number, title_en, title_ru, volume)
        WHERE sc.provider = $1
          AND sc.external_id = $2
          AND sc.chapter_number = u.chapter_number
        "#,
    )
    .bind(provider)
    .bind(external_id)
    .bind(&chapter_numbers)
    .bind(&title_en)
    .bind(&title_ru)
    .bind(&volumes)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    Ok(result.rows_affected() as usize)
}

/// Normalize a title for comparison: lowercase, non-alphanumeric -> space,
/// collapsed whitespace.
fn normalize_title(raw: &str) -> String {
    raw.to_lowercase()
        .chars()
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

/// True when ANY MangaDex title variant plausibly denotes the local title.
/// Deliberately conservative: a false negative only skips enrichment, while a
/// false positive would overwrite chapter titles from an unrelated series.
fn mangadex_title_matches(local: &str, titles: &HashMap<String, String>) -> bool {
    let local_norm = normalize_title(local);
    if local_norm.is_empty() {
        return false;
    }
    titles.values().any(|candidate| {
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
        if shorter.chars().count() >= 4 && longer.contains(shorter) {
            return true;
        }
        token_overlap(&local_norm, &candidate_norm) >= 0.8
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_chapter_integer() {
        assert_eq!(format_chapter(10), "1");
        assert_eq!(format_chapter(20), "2");
        assert_eq!(format_chapter(100), "10");
    }

    #[test]
    fn format_chapter_fractional() {
        assert_eq!(format_chapter(105), "10.5");
        assert_eq!(format_chapter(250), "25");
        assert_eq!(format_chapter(101), "10.1");
    }

    #[test]
    fn parse_chapter_integer() {
        assert_eq!(parse_chapter("1"), Some(10));
        assert_eq!(parse_chapter("10"), Some(100));
        assert_eq!(parse_chapter("25"), Some(250));
    }

    #[test]
    fn parse_chapter_fractional() {
        assert_eq!(parse_chapter("10.5"), Some(105));
        assert_eq!(parse_chapter("1.1"), Some(11));
        assert_eq!(parse_chapter("25.0"), Some(250));
    }

    #[test]
    fn parse_chapter_invalid() {
        assert_eq!(parse_chapter("abc"), None);
        assert_eq!(parse_chapter(""), None);
        assert_eq!(parse_chapter("1.2.3"), None);
    }

    #[test]
    fn stored_chapter_formatted() {
        let ch = StoredChapter {
            chapter_number: 105,
            volume: Some(10),
            title_en: Some("Test".to_string()),
            title_ru: None,
            release_date: None,
            read: false,
        };
        assert_eq!(ch.formatted(), "10.5");

        let ch2 = StoredChapter {
            chapter_number: 20,
            volume: None,
            title_en: None,
            title_ru: None,
            release_date: None,
            read: true,
        };
        assert_eq!(ch2.formatted(), "2");
    }

    #[test]
    fn parse_chapter_edge_cases() {
        // Multiple digits after decimal - only first counts
        assert_eq!(parse_chapter("10.50"), Some(105));
        assert_eq!(parse_chapter("10.05"), Some(100));
        assert_eq!(parse_chapter("10.99"), Some(109));
        // Leading zeros in fractional part
        assert_eq!(parse_chapter("1.05"), Some(10));
        // No whole part
        assert_eq!(parse_chapter(".5"), None);
        // Negative not supported
        assert_eq!(parse_chapter("-1"), None);
    }

    #[test]
    fn title_match_accepts_identical_and_containment() {
        let mut titles = std::collections::HashMap::new();
        titles.insert("en".to_string(), "Solo Leveling".to_string());
        assert!(mangadex_title_matches("Solo Leveling", &titles));
        assert!(mangadex_title_matches("solo leveling!", &titles));
        assert!(mangadex_title_matches("Solo Leveling: Ragnarok", &titles));
    }

    #[test]
    fn title_match_accepts_romaji_variant() {
        let mut titles = std::collections::HashMap::new();
        titles.insert("ja".to_string(), "Ore dake Level Up na Ken".to_string());
        assert!(mangadex_title_matches("Ore dake Level Up na Ken", &titles));
    }

    #[test]
    fn title_match_rejects_unrelated_and_empty() {
        let mut titles = std::collections::HashMap::new();
        titles.insert("en".to_string(), "Berserk".to_string());
        assert!(!mangadex_title_matches("Solo Leveling", &titles));
        assert!(!mangadex_title_matches("", &titles));
        assert!(!mangadex_title_matches(
            "Solo Leveling",
            &std::collections::HashMap::new()
        ));
    }
}
