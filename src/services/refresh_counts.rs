use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use sqlx::PgPool;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::models::media_item::CreateMediaItem;
use crate::services::external::anilist::AniListService;
use crate::services::external::comicvine::ComicVineService;
use crate::services::external::dispatch::{Provider, ProviderClients};
use crate::services::external::google_books::GoogleBooksService;
use crate::services::external::igdb::IgdbService;
use crate::services::external::mal::MalService;
use crate::services::external::mangadex::MangaDexService;
use crate::services::external::mangaupdates::MangaUpdatesService;
use crate::services::external::openlibrary::OpenLibraryService;
use crate::services::external::rawg::RawgService;
use crate::services::external::shikimori::ShikimoriService;
use crate::services::external::tmdb::TmdbService;

const REFRESH_INTERVAL: Duration = Duration::from_secs(6 * 3600);
const LOCK_ID: i64 = 42;

#[derive(Debug, Clone, sqlx::FromRow)]
#[expect(dead_code)]
struct MediaItemRow {
    id: Uuid,
    provider: String,
    external_id: String,
    media_type: String,
    episodes: Option<i32>,
    chapters: Option<i32>,
    volumes: Option<i32>,
    pages: Option<i32>,
    runtime_minutes: Option<i32>,
    playtime_hours: Option<i32>,
    status: Option<String>,
    score: Option<f64>,
}

pub struct RefreshCtx {
    pub db: PgPool,
    pub shikimori: ShikimoriService,
    pub mal: MalService,
    pub mangaupdates: MangaUpdatesService,
    pub tmdb: TmdbService,
    pub rawg: RawgService,
    pub igdb: IgdbService,
    pub google_books: GoogleBooksService,
    pub openlibrary: OpenLibraryService,
    pub mangadex: MangaDexService,
    pub anilist: AniListService,
    pub comicvine: ComicVineService,
}

pub async fn run_refresh_loop(ctx: RefreshCtx, cancel: CancellationToken) {
    let mut interval = tokio::time::interval(REFRESH_INTERVAL);
    // Consume the immediate first tick so the first cycle runs after a full
    // period (preserves the previous refresh behaviour).
    interval.tick().await;

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                info!("refresh_counts: cancelled, shutting down");
                break;
            }
            _ = interval.tick() => {
                if let Err(e) = try_refresh(&ctx).await {
                    warn!(error = %e, "refresh_counts: cycle failed");
                }
            }
        }
    }
}

/// Acquire the cycle advisory lock on a dedicated connection.
///
/// Advisory locks are session-scoped, so `pg_try_advisory_lock` and
/// `pg_advisory_unlock` MUST run on the same physical connection. Checking out
/// one pooled connection for the whole cycle also guarantees the lock is held
/// for the entire duration of the work (previously it was taken and released
/// possibly on different pooled connections, which made it useless).
async fn try_refresh(ctx: &RefreshCtx) -> Result<(), anyhow::Error> {
    let mut conn = ctx.db.acquire().await?;

    let (locked,): (bool,) = sqlx::query_as("SELECT pg_try_advisory_lock($1)")
        .bind(LOCK_ID)
        .fetch_one(&mut *conn)
        .await?;

    if !locked {
        info!("refresh_counts: lock not acquired, another replica is running");
        return Ok(());
    }

    let result = do_refresh(ctx).await;

    if let Err(e) = sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(LOCK_ID)
        .execute(&mut *conn)
        .await
    {
        warn!(error = %e, "refresh_counts: failed to release advisory lock");
    }

    result
}

async fn do_refresh(ctx: &RefreshCtx) -> Result<(), anyhow::Error> {
    let rows: Vec<MediaItemRow> = sqlx::query_as(
        r#"
        SELECT id, provider, external_id, media_type,
               episodes, chapters, volumes, pages,
               runtime_minutes, playtime_hours,
               status, score
        FROM media_items
        ORDER BY provider
        "#,
    )
    .fetch_all(&ctx.db)
    .await?;

    if rows.is_empty() {
        info!("refresh_counts: no media items to refresh");
        return Ok(());
    }

    info!(total = rows.len(), "refresh_counts: loaded media items");

    let by_provider = group_by_provider(&rows);

    for (provider_name, items) in &by_provider {
        let clients = ProviderClients {
            shikimori: &ctx.shikimori,
            mal: &ctx.mal,
            mangaupdates: &ctx.mangaupdates,
            tmdb: &ctx.tmdb,
            rawg: &ctx.rawg,
            igdb: &ctx.igdb,
            google_books: &ctx.google_books,
            openlibrary: &ctx.openlibrary,
            mangadex: &ctx.mangadex,
            anilist: &ctx.anilist,
            comicvine: &ctx.comicvine,
        };
        let provider = match Provider::from_name(&clients, provider_name) {
            Some(provider) => Some(provider),
            None => {
                debug!(
                    provider = provider_name,
                    n = items.len(),
                    "refresh_counts: unknown provider, skipping"
                );
                None
            }
        };

        if let Some(provider) = provider {
            let items: Vec<MediaItemRow> = items.iter().map(|r| (*r).clone()).collect();
            refresh_group(&ctx.db, items, provider).await;
        }
    }

    Ok(())
}

fn group_by_provider(rows: &[MediaItemRow]) -> std::collections::HashMap<&str, Vec<&MediaItemRow>> {
    let mut map: std::collections::HashMap<&str, Vec<&MediaItemRow>> =
        std::collections::HashMap::new();
    for row in rows {
        map.entry(row.provider.as_str()).or_default().push(row);
    }
    map
}

async fn refresh_group(db: &PgPool, items: Vec<MediaItemRow>, provider: Provider) {
    let total = items.len();
    let delay = provider.delay();
    let concurrency = provider.concurrency();
    let sem = Arc::new(Semaphore::new(concurrency));
    let updated = Arc::new(AtomicU32::new(0));
    let mut handles = Vec::with_capacity(items.len());

    for item in items {
        let permit = match Arc::clone(&sem).acquire_owned().await {
            Ok(p) => p,
            Err(_) => {
                warn!("refresh_counts: semaphore closed");
                break;
            }
        };
        let db = db.clone();
        let provider = provider.clone();
        let updated = Arc::clone(&updated);
        let ext_id = item.external_id.clone();
        let media_type = item.media_type.clone();

        handles.push(tokio::spawn(async move {
            let _permit = permit;
            tokio::time::sleep(delay).await;
            // TMDB expects "movie" or "tv"; the other providers ignore it.
            let fetch_media_type = match media_type.as_str() {
                "movie" | "dramas" => "movie",
                _ => "tv",
            };
            match provider.fetch(&ext_id, fetch_media_type).await {
                Ok(details) => match update_item(&db, &item, &details).await {
                    Ok(true) => {
                        updated.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(false) => {}
                    Err(e) => {
                        warn!(external_id = %ext_id, error = %e, "refresh: update_item failed");
                    }
                },
                Err(e) => {
                    warn!(external_id = %ext_id, error = %e, "refresh: fetch failed");
                }
            }
        }));
    }

    for h in handles {
        let _ = h.await;
    }

    let updated = updated.load(Ordering::Relaxed) as usize;
    let unchanged = total - updated;
    info!(
        provider = provider.name(),
        total, updated, unchanged, "refresh_counts: provider done"
    );
}

async fn update_item(
    db: &PgPool,
    old: &MediaItemRow,
    new: &CreateMediaItem,
) -> Result<bool, anyhow::Error> {
    let result = sqlx::query(
        r#"
        UPDATE media_items
        SET
            episodes = COALESCE($2, episodes),
            chapters = COALESCE($3, chapters),
            volumes = COALESCE($4, volumes),
            pages = COALESCE($5, pages),
            runtime_minutes = COALESCE($6, runtime_minutes),
            playtime_hours = COALESCE($7, playtime_hours),
            status = COALESCE($8, status),
            score = COALESCE($9, score),
            updated_at = now()
        WHERE id = $1
          AND (
            ($2 IS NOT NULL AND episodes IS DISTINCT FROM $2)
            OR ($3 IS NOT NULL AND chapters IS DISTINCT FROM $3)
            OR ($4 IS NOT NULL AND volumes IS DISTINCT FROM $4)
            OR ($5 IS NOT NULL AND pages IS DISTINCT FROM $5)
            OR ($6 IS NOT NULL AND runtime_minutes IS DISTINCT FROM $6)
            OR ($7 IS NOT NULL AND playtime_hours IS DISTINCT FROM $7)
            OR ($8 IS NOT NULL AND status IS DISTINCT FROM $8)
            OR ($9 IS NOT NULL AND score IS DISTINCT FROM $9)
          )
        "#,
    )
    .bind(old.id)
    .bind(new.episodes)
    .bind(new.chapters)
    .bind(new.volumes)
    .bind(new.pages)
    .bind(new.runtime_minutes)
    .bind(new.playtime_hours)
    .bind(&new.status)
    .bind(new.score)
    .execute(db)
    .await?;

    Ok(result.rows_affected() > 0)
}
