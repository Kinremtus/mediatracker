use sqlx::PgPool;



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

/// Chapter number stored in DB as integer * 10 (105 = chapter 10.5).
/// UI helpers below format it back to human-readable form.

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
pub fn parse_chapter(s: &str) -> Option<i32> {
    let s = s.trim();
    let parts: Vec<&str> = s.split('.').collect();
    match parts.len() {
        1 => {
            let whole: i32 = parts[0].parse().ok()?;
            Some(whole * 10)
        }
        2 => {
            let whole: i32 = parts[0].parse().ok()?;
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

    let mut count = 0;
    for ch in 1..=latest_chapter {
        let chapter_number = ch * 10; // chapter 1 → 10, chapter 2 → 20, etc.
        let result = sqlx::query(
            r#"
            INSERT INTO series_chapters
                (provider, external_id, chapter_number)
            VALUES ($1, $2, $3)
            ON CONFLICT (provider, external_id, chapter_number) DO NOTHING
            "#,
        )
        .bind("mangaupdates")
        .bind(series_id.to_string())
        .bind(chapter_number)
        .execute(pool)
        .await?;
        count += result.rows_affected();
    }

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
        .bind(series_id.to_string())
        .execute(pool)
        .await?;
    }

    Ok(count as usize)
}

/// Read all chapters for one manga, sorted by number ascending.
pub async fn get_chapters(
    pool: &PgPool,
    provider: &str,
    external_id: &str,
) -> Result<Vec<StoredChapter>, sqlx::Error> {
    let rows: Vec<(
        i32,
        Option<i32>,
        Option<String>,
        Option<String>,
        Option<chrono::NaiveDate>,
        bool,
    )> = sqlx::query_as(
        r#"
        SELECT chapter_number, volume, title_en, title_ru, release_date, read
        FROM series_chapters
        WHERE provider = $1 AND external_id = $2
        ORDER BY chapter_number ASC
        "#,
    )
    .bind(provider)
    .bind(external_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(chapter_number, volume, title_en, title_ru, release_date, read)| {
                StoredChapter {
                    chapter_number,
                    volume,
                    title_en,
                    title_ru,
                    release_date,
                    read,
                }
            },
        )
        .collect())
}

/// Read a single chapter by (provider, external_id, chapter_number).
pub async fn get_chapter(
    pool: &PgPool,
    provider: &str,
    external_id: &str,
    chapter_number: i32,
) -> Result<Option<StoredChapter>, sqlx::Error> {
    let row: Option<(
        i32,
        Option<i32>,
        Option<String>,
        Option<String>,
        Option<chrono::NaiveDate>,
        bool,
    )> = sqlx::query_as(
        r#"
        SELECT chapter_number, volume, title_en, title_ru, release_date, read
        FROM series_chapters
        WHERE provider = $1 AND external_id = $2 AND chapter_number = $3
        "#,
    )
    .bind(provider)
    .bind(external_id)
    .bind(chapter_number)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(
        |(chapter_number, volume, title_en, title_ru, release_date, read)| {
            StoredChapter {
                chapter_number,
                volume,
                title_en,
                title_ru,
                release_date,
                read,
            }
        },
    ))
}

/// Set the `read` flag on one or more chapter rows.
///
/// **Bulk-fill on read**: when `read = true`, every chapter with
/// `chapter_number <= N` is marked read (same semantics as anime
/// episodes: "mark ch 25 read → ch 1..25 read").
///
/// **Reverse bulk-fill on unread**: when `read = false`, every
/// chapter with `chapter_number >= N` is marked unread (the mirror:
/// "if I haven't read ch 25, I haven't read anything past it").
///
/// Returns `true` if at least one row was updated.
pub async fn set_read(
    pool: &PgPool,
    provider: &str,
    external_id: &str,
    chapter_number: i32,
    read: bool,
) -> Result<bool, sqlx::Error> {
    let result = if read {
        sqlx::query(
            r#"
            UPDATE series_chapters
            SET read = TRUE,
                read_at = NOW()
            WHERE provider = $1
              AND external_id = $2
              AND chapter_number <= $3
            "#,
        )
        .bind(provider)
        .bind(external_id)
        .bind(chapter_number)
        .execute(pool)
        .await?
    } else {
        sqlx::query(
            r#"
            UPDATE series_chapters
            SET read = FALSE,
                read_at = NULL
            WHERE provider = $1
              AND external_id = $2
              AND chapter_number >= $3
            "#,
        )
        .bind(provider)
        .bind(external_id)
        .bind(chapter_number)
        .execute(pool)
        .await?
    };
    Ok(result.rows_affected() > 0)
}

/// Highest chapter_number currently marked read for a manga.
/// Returns 0 if nothing is read.
pub async fn count_read(
    pool: &PgPool,
    provider: &str,
    external_id: &str,
) -> Result<i32, sqlx::Error> {
    let row: (Option<i32>,) = sqlx::query_as(
        r#"
        SELECT MAX(chapter_number)
        FROM series_chapters
        WHERE provider = $1
          AND external_id = $2
          AND read = TRUE
        "#,
    )
    .bind(provider)
    .bind(external_id)
    .fetch_one(pool)
    .await?;
    // chapter_number is stored as ch * 10, so divide back for display
    Ok(row.0.map(|v| v / 10).unwrap_or(0))
}

/// Get all (chapter_number, read) pairs for a manga, ordered ASC.
/// Used by the toggle endpoint to broadcast authoritative state
/// via HX-Trigger.
pub async fn get_chapter_states(
    pool: &PgPool,
    provider: &str,
    external_id: &str,
) -> Result<Vec<(i32, bool)>, sqlx::Error> {
    let rows: Vec<(i32, bool)> = sqlx::query_as(
        r#"
        SELECT chapter_number, read
        FROM series_chapters
        WHERE provider = $1 AND external_id = $2
        ORDER BY chapter_number ASC
        "#,
    )
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
    let row: Option<(uuid::Uuid,)> = sqlx::query_as(
        "SELECT id FROM media_items WHERE provider = $1 AND external_id = $2",
    )
    .bind(provider)
    .bind(external_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(v,)| v))
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
}
