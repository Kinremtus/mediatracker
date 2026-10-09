//! Backfill chapter titles from MangaDex for manga added before Stage 1.
//!
//! Usage:
//!   cargo run --bin backfill_chapters [--force] [--provider mangaupdates] [--external-id <id>]
//!
//! Flags:
//!   --force          Re-enrich even if titles already exist
//!   --provider       Filter by provider (default: mangaupdates)
//!   --external-id    Enrich single manga by external_id
//!   --limit          Max items to process (default: 100)

use anyhow::{Result, anyhow};
use clap::Parser;
use mediatracker::config::Config;
use mediatracker::services::chapters::{
    ChapterTitleGap, enrich_from_mangadex, list_titles_needing_chapter_titles,
};
use sqlx::{PgPool, Row};
use std::time::Duration;
use tracing::{info, warn};

#[derive(Parser, Debug)]
#[command(author, version, about = "Backfill chapter titles from MangaDex")]
struct Args {
    /// Re-enrich chapters even if they already have titles
    #[arg(long)]
    force: bool,

    /// Filter by provider (default: mangaupdates)
    #[arg(long, default_value = "mangaupdates")]
    provider: String,

    /// Enrich single manga by external_id (MangaUpdates series_id)
    #[arg(long)]
    external_id: Option<String>,

    /// Max items to process
    #[arg(long, default_value = "100")]
    limit: usize,

    /// Delay between requests (ms) to respect rate limits
    #[arg(long, default_value = "500")]
    delay_ms: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let args = Args::parse();
    let config = Config::from_env().map_err(|e| anyhow!("config error: {}", e))?;

    let pool = PgPool::connect(&config.database_url).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;

    info!(
        "Starting chapter backfill (force={}, provider={}, limit={})",
        args.force, args.provider, args.limit
    );

    let rows: Vec<(String, String, String)> = if let Some(ext_id) = args.external_id {
        // Single manga
        sqlx::query(
            "SELECT provider, external_id, title FROM media_items \
             WHERE provider = $1 AND external_id = $2 \
             AND media_type IN ('manga','manhwa','manhua','novel','other-comics','comic')",
        )
        .bind(&args.provider)
        .bind(&ext_id)
        .fetch_all(&pool)
        .await?
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect()
    } else if args.force {
        // All manga-like items from provider
        sqlx::query(
            "SELECT provider, external_id, title FROM media_items \
             WHERE provider = $1 \
             AND media_type IN ('manga','manhwa','manhua','novel','other-comics','comic') \
             ORDER BY created_at ASC LIMIT $2",
        )
        .bind(&args.provider)
        .bind(args.limit as i64)
        .fetch_all(&pool)
        .await?
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect()
    } else {
        // Only items whose chapter rows still have untitled entries. The
        // predicate runs on the RESOLVED key (see
        // `chapters::list_titles_needing_chapter_titles`): a bound title keeps
        // its rows under the official source key, so matching them against the
        // `media_items` key would skip exactly those titles. `limit` is applied
        // after the predicate, not to the raw candidate list.
        list_titles_needing_chapter_titles(
            &pool,
            Some(&args.provider),
            ChapterTitleGap::UntitledRows,
            Some(args.limit),
        )
        .await?
    };

    if rows.is_empty() {
        info!("No items to process");
        return Ok(());
    }

    info!("Found {} items to enrich", rows.len());

    let mut enriched = 0usize;
    let mut failed = 0usize;

    for (provider, external_id, title) in &rows {
        info!("Enriching: {} ({})", title, external_id);

        match enrich_from_mangadex(&pool, provider, external_id).await {
            Ok(count) => {
                enriched += count;
                info!("  -> enriched {} chapters", count);
            }
            Err(e) => {
                failed += 1;
                warn!("  -> failed: {}", e);
            }
        }

        tokio::time::sleep(Duration::from_millis(args.delay_ms)).await;
    }

    info!(
        "Backfill complete: enriched {} chapters across {} items, {} failed",
        enriched,
        rows.len(),
        failed
    );

    Ok(())
}
