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

use anyhow::{anyhow, Result};
use clap::Parser;
use mediatracker::config::Config;
use mediatracker::services::chapters::enrich_from_mangadex;
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
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "info".into()))
        .init();

    let args = Args::parse();
    let config = Config::from_env().map_err(|e| anyhow!("config error: {}", e))?;

    let pool = PgPool::connect(&config.database_url).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;

    info!("Starting chapter backfill (force={}, provider={}, limit={})",
        args.force, args.provider, args.limit);

    let rows = if let Some(ext_id) = args.external_id {
        // Single manga
        sqlx::query(
            "SELECT provider, external_id, title FROM media_items \
             WHERE provider = $1 AND external_id = $2 \
             AND media_type IN ('manga','manhwa','manhua','novel','other-comics')"
        )
        .bind(&args.provider)
        .bind(&ext_id)
        .fetch_all(&pool)
        .await?
    } else if args.force {
        // All manga-like items from provider
        sqlx::query(
            "SELECT provider, external_id, title FROM media_items \
             WHERE provider = $1 \
             AND media_type IN ('manga','manhwa','manhua','novel','other-comics') \
             ORDER BY created_at ASC LIMIT $2"
        )
        .bind(&args.provider)
        .bind(args.limit as i64)
        .fetch_all(&pool)
        .await?
    } else {
        // Only items where chapters exist but lack titles
        sqlx::query(
            r#"
            SELECT mi.provider, mi.external_id, mi.title FROM media_items mi
            WHERE mi.provider = $1
              AND mi.media_type IN ('manga','manhwa','manhua','novel','other-comics')
              AND EXISTS (
                  SELECT 1 FROM series_chapters sc
                  WHERE sc.provider = mi.provider AND sc.external_id = mi.external_id
              )
              AND NOT EXISTS (
                  SELECT 1 FROM series_chapters sc
                  WHERE sc.provider = mi.provider AND sc.external_id = mi.external_id
                  AND (sc.title_en IS NOT NULL OR sc.title_ru IS NOT NULL)
                  LIMIT 1
              )
            ORDER BY mi.created_at ASC LIMIT $2
            "#,
        )
        .bind(&args.provider)
        .bind(args.limit as i64)
        .fetch_all(&pool)
        .await?
    };

    if rows.is_empty() {
        info!("No items to process");
        return Ok(());
    }

    info!("Found {} items to enrich", rows.len());

    let mut enriched = 0usize;
    let mut failed = 0usize;

    for row in rows {
        let provider: String = row.get(0);
        let external_id: String = row.get(1);
        let title: String = row.get(2);

        info!("Enriching: {} ({})", title, external_id);

        match enrich_from_mangadex(&pool, &provider, &external_id).await {
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

    info!("Backfill complete: enriched {} chapters across {} items, {} failed",
        enriched, rows.len(), failed);

    Ok(())
}