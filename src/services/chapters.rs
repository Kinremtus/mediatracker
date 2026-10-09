use crate::services::external::mangadex::{
    MangaDexChapter, MangaDexMangaAttributes, MangaDexService,
};
use crate::services::official_resolve::chapter_key;
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
    /// Bonus rows ('00' prologue, non-numeric special, duplicate number). They
    /// are excluded from the read-progress denominator (design :111).
    pub is_bonus: bool,
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

/// Sync `media_items.chapters` from the stored chapter rows: the value is
/// `ceil(MAX(chapter_number) / 100)` (chapter numbers are stored at x100
/// scale) and an existing value is never lowered. Runs inside the caller's
/// transaction via the passed connection so the card denominator can never
/// disagree with the rows.
///
/// The two keys differ on purpose: `media_items` is keyed by the incoming
/// (MangaUpdates) identity, while the chapter rows live under the resolved key
/// (design :73-78). Updating `media_items` by the resolved key would match no
/// row at all and silently leave the denominator stale.
async fn sync_media_items_chapters(
    conn: &mut sqlx::PgConnection,
    media_provider: &str,
    media_external_id: &str,
    rows_provider: &str,
    rows_external_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE media_items
        SET chapters = GREATEST(COALESCE(media_items.chapters, 0), sub.max_ch)
        FROM (
            SELECT ((MAX(chapter_number) + 99) / 100) AS max_ch
            FROM series_chapters
            WHERE provider = $3 AND external_id = $4
        ) AS sub
        WHERE media_items.provider = $1
          AND media_items.external_id = $2
          AND NOT COALESCE(media_items.chapters_manual, FALSE)
          AND sub.max_ch > COALESCE(media_items.chapters, 0)
        "#,
    )
    .bind(media_provider)
    .bind(media_external_id)
    .bind(rows_provider)
    .bind(rows_external_id)
    .execute(conn)
    .await?;
    Ok(())
}

/// `(chapters, chapters_manual, chapters_source)` for one media item, or `None`
/// when the item is unknown. The drawer uses this to surface the manual override
/// over a live provider fetch.
///
/// No key resolution here on purpose: this reads the `media_items` row itself,
/// which is always keyed by the incoming (MangaUpdates) identity. There is
/// nothing to resolve — the official key would point at a row that does not
/// exist (design :73-78).
#[allow(clippy::type_complexity)]
pub async fn get_chapter_meta(
    pool: &PgPool,
    provider: &str,
    external_id: &str,
) -> Result<Option<(Option<i32>, bool, Option<String>)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT chapters, chapters_manual, chapters_source \
         FROM media_items WHERE provider = $1 AND external_id = $2",
    )
    .bind(provider)
    .bind(external_id)
    .fetch_optional(pool)
    .await
}

/// Insert skeleton rows for every integer chapter `1..=n` (stored x100) under
/// `(provider, external_id)`. Idempotent (`ON CONFLICT DO NOTHING`). Used so a
/// manual count larger than the MangaUpdates skeleton can be rendered and ticked.
///
/// The caller is responsible for passing the **resolved** key (see
/// `official_resolve::chapter_key`): this function only takes a connection, not a
/// pool, so it cannot resolve by itself.
pub(crate) async fn ensure_chapter_skeleton(
    conn: &mut sqlx::PgConnection,
    provider: &str,
    external_id: &str,
    n: i32,
) -> Result<(), sqlx::Error> {
    if n <= 0 {
        return Ok(());
    }
    let nums: Vec<i32> = (1..=n).map(|c| c * 100).collect();
    sqlx::query(
        "INSERT INTO series_chapters (provider, external_id, chapter_number) \
         SELECT $1, $2, u.chapter_number FROM UNNEST($3::int[]) AS u(chapter_number) \
           ON CONFLICT (provider, external_id, chapter_number) WHERE NOT is_bonus DO NOTHING",
    )
    .bind(provider)
    .bind(external_id)
    .bind(&nums)
    .execute(conn)
    .await?;
    Ok(())
}

/// Store a user-supplied count as ground truth. Also extends the skeleton so the
/// drawer can display/tick chapters beyond the auto count. Transactional: the
/// value and its skeleton move together.
pub async fn set_manual_chapters(
    pool: &PgPool,
    provider: &str,
    external_id: &str,
    chapters: i32,
) -> Result<(), sqlx::Error> {
    let chapters = chapters.max(0);
    // Resolve before opening the transaction: `chapter_key` needs a pool, and a
    // read outside the tx is cheaper than pinning a connection for it. The
    // manual value itself still lands on the `media_items` row, which is keyed
    // by the incoming (MangaUpdates) identity.
    let (key_provider, key_external_id) = chapter_key(pool, provider, external_id).await;
    let mut tx = pool.begin().await?;
    sqlx::query(
        "UPDATE media_items \
         SET chapters = $3, chapters_manual = TRUE, chapters_source = 'manual', \
             updated_at = NOW() \
         WHERE provider = $1 AND external_id = $2",
    )
    .bind(provider)
    .bind(external_id)
    .bind(chapters)
    .execute(&mut *tx)
    .await?;
    // The skeleton must join the resolved rows, otherwise ticking a chapter the
    // manual count added would find no `series_chapters` row to attach progress
    // to.
    ensure_chapter_skeleton(&mut tx, &key_provider, &key_external_id, chapters).await?;
    tx.commit().await?;
    Ok(())
}

/// Drop the manual override. `chapters` is intentionally left untouched until the
/// next automatic refresh rewrites it (a transient provider failure must not blank
/// the denominator).
pub async fn clear_manual_chapters(
    pool: &PgPool,
    provider: &str,
    external_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE media_items \
         SET chapters_manual = FALSE, chapters_source = NULL, updated_at = NOW() \
         WHERE provider = $1 AND external_id = $2",
    )
    .bind(provider)
    .bind(external_id)
    .execute(pool)
    .await?;
    Ok(())
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
    // Chapter rows live under the resolved key: with a meta-capable binding the
    // official source owns that key, so the MU skeleton is a fallback written
    // into the same rows (design :73-78).
    let (key_provider, key_external_id) = chapter_key(pool, "mangaupdates", &external_id).await;
    // Chapter 1 -> 100, chapter 2 -> 200, … (fraction goes in the low digits).
    let chapter_numbers: Vec<i32> = (1..=latest_chapter).map(|ch| ch * 100).collect();

    // One statement instead of N: a mid-way failure can no longer leave a
    // partially populated skeleton. The media_items sync runs in the same
    // transaction so the card denominator can never disagree with the rows.
    let mut tx = pool.begin().await?;

    // Official enrichment runs asynchronously and writes `origin='official_*'`
    // rows under the same resolved key. If they are already there, the fallback
    // skeleton must not overwrite/pad them with `origin='mu'` rows
    // (design :102-103). The check happens inside the transaction so a
    // concurrent apply_meta either wins before this point or loses after it.
    let (official_rows,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM series_chapters \
         WHERE provider = $1 AND external_id = $2 AND origin <> 'mu'",
    )
    .bind(&key_provider)
    .bind(&key_external_id)
    .fetch_one(&mut *tx)
    .await?;
    if official_rows > 0 {
        // Dropping the transaction rolls back: nothing was written.
        return Ok(0);
    }

    let result = sqlx::query(
        r#"
        INSERT INTO series_chapters
            (provider, external_id, chapter_number)
        SELECT $1, $2, u.chapter_number
        FROM UNNEST($3::int[]) AS u(chapter_number)
          ON CONFLICT (provider, external_id, chapter_number) WHERE NOT is_bonus DO NOTHING
        "#,
    )
    .bind(&key_provider)
    .bind(&key_external_id)
    .bind(&chapter_numbers)
    .execute(&mut *tx)
    .await?;

    let count = result.rows_affected();

    // Sync media_items.chapters so the tracking card denominator matches the
    // stored rows (same helper the MangaDex enrichment path uses). The
    // `media_items` row is addressed by the ORIGINAL MangaUpdates identity
    // (there is no row under the official key), while the row source is the
    // resolved key the skeleton was just written to.
    if count > 0 {
        sync_media_items_chapters(
            &mut tx,
            "mangaupdates",
            &external_id,
            &key_provider,
            &key_external_id,
        )
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
    // Callers always pass the MangaUpdates identity key; the rows themselves
    // live under the resolved key (official source when bound).
    let (key_provider, key_external_id) = chapter_key(pool, provider, external_id).await;
    #[allow(clippy::type_complexity)]
    let rows: Vec<(
        i32,
        Option<i32>,
        Option<String>,
        Option<String>,
        Option<chrono::NaiveDate>,
        bool,
        bool,
    )> = sqlx::query_as(
        r#"
        SELECT sc.chapter_number, sc.volume, sc.title_en, sc.title_ru,
               sc.release_date, COALESCE(p.read, FALSE), sc.is_bonus
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
    .bind(&key_provider)
    .bind(&key_external_id)
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(chapter_number, volume, title_en, title_ru, release_date, read, is_bonus)| {
                StoredChapter {
                    chapter_number,
                    volume,
                    title_en,
                    title_ru,
                    release_date,
                    read,
                    is_bonus,
                }
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
    let (key_provider, key_external_id) = chapter_key(pool, provider, external_id).await;
    #[allow(clippy::type_complexity)]
    let row: Option<(
        i32,
        Option<i32>,
        Option<String>,
        Option<String>,
        Option<chrono::NaiveDate>,
        bool,
        bool,
    )> = sqlx::query_as(
        r#"
        SELECT sc.chapter_number, sc.volume, sc.title_en, sc.title_ru,
               sc.release_date, COALESCE(p.read, FALSE), sc.is_bonus
        FROM series_chapters sc
        LEFT JOIN user_chapter_progress p
            ON p.user_id = $4
           AND p.provider = sc.provider
           AND p.external_id = sc.external_id
           AND p.chapter_number = sc.chapter_number
        WHERE sc.provider = $1 AND sc.external_id = $2 AND sc.chapter_number = $3
        "#,
    )
    .bind(&key_provider)
    .bind(&key_external_id)
    .bind(chapter_number)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(
        |(chapter_number, volume, title_en, title_ru, release_date, read, is_bonus)| {
            StoredChapter {
                chapter_number,
                volume,
                title_en,
                title_ru,
                release_date,
                read,
                is_bonus,
            }
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
    // Progress rows must sit on the same key as the `series_chapters` rows they
    // mirror, otherwise the tick would create a progress row the chapter list
    // never joins against.
    let (key_provider, key_external_id) = chapter_key(pool, provider, external_id).await;
    let result = if read {
        // Bulk-fill every skeleton chapter <= N (existing behaviour). The
        // inserted key comes from `sc.*`, i.e. the already-resolved one.
        let bulk = sqlx::query(
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
        .bind(&key_provider)
        .bind(&key_external_id)
        .bind(chapter_number)
        .execute(pool)
        .await?;

        // Manual override raises the ceiling past the auto skeleton: record the
        // exact clicked chapter even when no `series_chapters` row exists for it.
        // `media_items` is keyed by the incoming identity, NOT the resolved key.
        let manual: bool = sqlx::query_scalar(
            "SELECT COALESCE(chapters_manual, FALSE) FROM media_items \
             WHERE provider = $1 AND external_id = $2",
        )
        .bind(provider)
        .bind(external_id)
        .fetch_optional(pool)
        .await?
        .unwrap_or(false);

        let direct = if manual {
            sqlx::query(
                r#"
                INSERT INTO user_chapter_progress
                    (user_id, provider, external_id, chapter_number, read, read_at)
                VALUES ($1, $2, $3, $4, TRUE, NOW())
                ON CONFLICT (user_id, provider, external_id, chapter_number) DO UPDATE
                SET read = TRUE,
                    read_at = NOW()
                "#,
            )
            .bind(user_id)
            .bind(&key_provider)
            .bind(&key_external_id)
            .bind(chapter_number)
            .execute(pool)
            .await?
        } else {
            Default::default()
        };
        (bulk, direct)
    } else {
        let r = sqlx::query(
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
        .bind(&key_provider)
        .bind(&key_external_id)
        .bind(chapter_number)
        .execute(pool)
        .await?;
        (r, Default::default())
    };
    Ok(result.0.rows_affected() > 0 || result.1.rows_affected() > 0)
}

/// Highest chapter_number currently marked read for this user.
/// Returns 0 if nothing is read.
pub async fn count_read(
    pool: &PgPool,
    user_id: uuid::Uuid,
    provider: &str,
    external_id: &str,
) -> Result<i32, sqlx::Error> {
    let (key_provider, key_external_id) = chapter_key(pool, provider, external_id).await;
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
          -- Read progress is measured over main chapters only: bonus rows
          -- ('00' prologue, special, duplicate number) would push the ratio
          -- past the real main-chapter count (design :111).
          AND NOT sc.is_bonus
        "#,
    )
    .bind(user_id)
    .bind(&key_provider)
    .bind(&key_external_id)
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
    let (key_provider, key_external_id) = chapter_key(pool, provider, external_id).await;
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
    .bind(&key_provider)
    .bind(&key_external_id)
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
///
/// The incoming key is the MangaUpdates identity; the rows are written under the
/// resolved key so MangaDex titles join the same rows the reader sees
/// (resolution happens here, inside the function, so no caller has to remember
/// it).
pub async fn upsert_series_chapters(
    pool: &PgPool,
    provider: &str,
    external_id: &str,
    rows: &[MdChapterRow],
) -> Result<usize, sqlx::Error> {
    if rows.is_empty() {
        return Ok(0);
    }
    let (key_provider, key_external_id) = chapter_key(pool, provider, external_id).await;
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
        ON CONFLICT (provider, external_id, chapter_number) WHERE NOT is_bonus DO UPDATE
        SET title_en = COALESCE(series_chapters.title_en, EXCLUDED.title_en),
            title_ru = COALESCE(series_chapters.title_ru, EXCLUDED.title_ru),
            volume = COALESCE(series_chapters.volume, EXCLUDED.volume),
            release_date = COALESCE(series_chapters.release_date, EXCLUDED.release_date)
        "#,
    )
    .bind(&key_provider)
    .bind(&key_external_id)
    .bind(&chapter_numbers)
    .bind(&title_en)
    .bind(&title_ru)
    .bind(&volumes)
    .bind(&release_dates)
    .execute(&mut *tx)
    .await?;
    // Enrichment inserts/merges rows but historically never refreshed the
    // counter; sync it in the same transaction so it can never drift. The
    // `media_items` row is addressed by the ORIGINAL identity (no row exists
    // under the resolved key), the row source is the resolved key.
    sync_media_items_chapters(
        &mut tx,
        provider,
        external_id,
        &key_provider,
        &key_external_id,
    )
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

    // `provider` here is still the incoming MangaUpdates identity: the title
    // lookup above and the counter sync inside `upsert_series_chapters` are both
    // `media_items` queries and must stay on it. The chapter rows themselves are
    // written under the resolved key — the resolution happens INSIDE
    // `upsert_series_chapters`, deliberately, so no caller can forget it.
    let affected = upsert_series_chapters(pool, provider, external_id, &rows).await?;
    Ok(affected)
}

/// Media types that own a chapter list, i.e. the candidate set of the MangaDex
/// chapter-title passes. Same list as the hand-written queries this selection
/// replaced (`routes/admin.rs`, `bin/backfill_chapters.rs`).
const CHAPTER_MEDIA_TYPES: [&str; 6] = [
    "manga",
    "manhwa",
    "manhua",
    "novel",
    "other-comics",
    "comic",
];

/// Which chapter rows of a title are still missing provider metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChapterTitleGap {
    /// No stored row carries `title_en`/`title_ru` (this includes a title with
    /// no rows at all) — the admin bulk pass, which enriches from scratch.
    NoTitledRows,
    /// At least one stored row has no title — the MangaDex backfill pass, which
    /// only fills the gaps. Implies that rows exist.
    UntitledRows,
}

/// `(provider, external_id, title)` of one title queued for chapter enrichment.
pub type ChapterTitleCandidate = (String, String, String);

/// Titles that still need a MangaDex chapter-title pass, oldest first.
///
/// The predicate cannot stay a `NOT EXISTS`/`EXISTS` subquery against
/// `media_items.provider/external_id`: the rows of a bound title live under the
/// resolved key (`official_resolve::chapter_key`, design :73-78), so comparing
/// them to the incoming MangaUpdates key matches nothing once migration 028 has
/// re-keyed them. The mismatch is silent in both directions — the admin pass
/// would re-enrich already titled titles, and the backfill would skip exactly
/// the titles that still have untitled rows. Candidates are therefore resolved
/// one by one through the same resolver every reader/writer uses.
///
/// Cost: one `chapter_key` (two indexed queries, one for a non-MU provider) plus
/// one predicate query per candidate, so roughly three round trips per manga-like
/// title instead of a single statement. That is the price of keeping a single
/// resolver; both call sites are manual, infrequent operations.
///
/// `limit` is applied **after** the predicate, and that placement is unchanged:
/// the old single-statement query put both `EXISTS` subqueries in the `WHERE`
/// clause ahead of its `LIMIT`, so the limit truncated an already filtered set
/// in exactly the same way. What actually changed is the *key* the predicate is
/// evaluated against — it used to be the MangaUpdates-derived key of the bound
/// title, now it is the resolved `chapter_key` — and that changes which
/// `media_items` rows pass the "does this title already have such chapters"
/// test, not where the `LIMIT` sits.
pub async fn list_titles_needing_chapter_titles(
    pool: &PgPool,
    provider: Option<&str>,
    gap: ChapterTitleGap,
    limit: Option<usize>,
) -> Result<Vec<ChapterTitleCandidate>, sqlx::Error> {
    let media_types: Vec<String> = CHAPTER_MEDIA_TYPES
        .iter()
        .map(|t| (*t).to_string())
        .collect();
    let candidates: Vec<ChapterTitleCandidate> = sqlx::query_as(
        r#"
        SELECT provider, external_id, title
        FROM media_items
        WHERE ($1::text IS NULL OR provider = $1)
          AND media_type = ANY($2::text[])
        ORDER BY created_at ASC, external_id ASC
        "#,
    )
    .bind(provider)
    .bind(&media_types)
    .fetch_all(pool)
    .await?;

    let mut selected: Vec<ChapterTitleCandidate> = Vec::new();
    for (provider, external_id, title) in candidates {
        if limit.is_some_and(|l| selected.len() >= l) {
            break;
        }
        // Resolve inside the loop: the chapter rows belong to the resolved key,
        // not to the incoming identity key.
        let (key_provider, key_external_id) = chapter_key(pool, &provider, &external_id).await;
        if chapter_title_gap_present(pool, &key_provider, &key_external_id, gap).await? {
            selected.push((provider, external_id, title));
        }
    }
    Ok(selected)
}

/// Whether the rows stored under the resolved key match `gap`. See
/// [`ChapterTitleGap`].
async fn chapter_title_gap_present(
    pool: &PgPool,
    provider: &str,
    external_id: &str,
    gap: ChapterTitleGap,
) -> Result<bool, sqlx::Error> {
    match gap {
        ChapterTitleGap::NoTitledRows => {
            // The old `NOT EXISTS (titled row)`: a title with no rows at all
            // qualifies, and so does one whose rows are all bare numbers.
            let titled: bool = sqlx::query_scalar(
                r#"
                SELECT EXISTS (
                    SELECT 1 FROM series_chapters
                    WHERE provider = $1 AND external_id = $2
                      AND (title_en IS NOT NULL OR title_ru IS NOT NULL)
                )
                "#,
            )
            .bind(provider)
            .bind(external_id)
            .fetch_one(pool)
            .await?;
            Ok(!titled)
        }
        // The old `EXISTS rows AND EXISTS untitled rows`: an untitled row cannot
        // exist without rows, so this single probe carries both halves. It is
        // true both when no row carries a title and when only some do.
        ChapterTitleGap::UntitledRows => {
            let untitled: bool = sqlx::query_scalar(
                r#"
                SELECT EXISTS (
                    SELECT 1 FROM series_chapters
                    WHERE provider = $1 AND external_id = $2
                      AND title_en IS NULL AND title_ru IS NULL
                )
                "#,
            )
            .bind(provider)
            .bind(external_id)
            .fetch_one(pool)
            .await?;
            Ok(untitled)
        }
    }
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
            is_bonus: false,
        };
        assert_eq!(ch.formatted(), "10.5");

        let ch2 = StoredChapter {
            chapter_number: 200,
            volume: None,
            title_en: None,
            title_ru: None,
            release_date: None,
            read: true,
            is_bonus: true,
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
