//! `backfill_anime` — one-shot CLI for anime entries that were
//! added to `media_items` before migration 011 added the `mal_id`
//! column and Stage B switched episode storage to Jikan.
//!
//! For every `media_items` row where `media_type = 'anime'` and
//! `mal_id IS NULL`, the script tries to resolve a `mal_id` and
//! then pulls the episode list from Jikan v4.
//!
//! Resolution strategies, in order:
//!   1. provider = "mal" + numeric external_id  →  no HTTP
//!   2. shikimori_id is known                  →  GET shikimori.one/api/animes/{id}
//!   3. fallback                               →  Jikan search by title (fuzzy)
//!
//! Pure helpers (`choose_strategy`, `mal_id_from_shikimori_anime_response`,
//! `try_external_id_as_mal`) live in `services::backfill` so they're
//! unit-tested without a DB or network.
//!
//! USAGE:
//!   backfill_anime [--dry-run] [--limit N] [--provider <name>]
//!
//! ENV:
//!   DATABASE_URL  required (loaded via Config::from_env)
//!   RUST_LOG      optional, e.g. "info,sqlx=warn,reqwest=warn"

use std::env;
use std::process::ExitCode;
use std::time::Duration;

use mediatracker::config::Config;
use mediatracker::services::backfill::{
    choose_strategy, mal_id_from_shikimori_anime_response, try_external_id_as_mal, BackfillCandidate,
    Strategy,
};
use mediatracker::services::episodes;
use mediatracker::services::external::mal::MalService;
use sqlx::PgPool;
use sqlx::Row;
use uuid::Uuid;

const SHIKIMORI_USER_AGENT: &str = "MediaTracker/0.1 (+https://github.com/Kinremtus/mediatracker) backfill";
// Shikimori public API: 5 req/sec per IP is the published guidance.
// We pace to 1 req / 1000 ms = 1 req/sec to leave headroom for the
// Jikan calls in the same loop.
const SHIKIMORI_PAUSE: Duration = Duration::from_millis(1000);
// Jikan public API: 3 req/sec, 60 req/min. We pace to ~1.5 req/sec
// between items so even big lists stay comfortably under the limit.
const JIKAN_SEARCH_PAUSE: Duration = Duration::from_millis(700);
// Soft pause between items even when we don't hit an external API
// (MAL rows resolve from external_id alone). Keeps the loop quiet
// and gives the DB a moment to breathe.
const BETWEEN_ITEMS_PAUSE: Duration = Duration::from_millis(150);

#[derive(Debug)]
struct Cli {
    dry_run: bool,
    limit: Option<i64>,
    provider: Option<String>,
    /// `--force`: re-fetch episodes for rows that already have `mal_id`.
    /// Default mode targets only `mal_id IS NULL` rows. With `--force`
    /// the target is `mal_id IS NOT NULL` rows (we skip the resolve +
    /// UPDATE steps and go straight to `fetch_and_store_mal`). Useful
    /// for refetching after a buggy Jikan fetch truncated a long series
    /// (e.g. One Piece getting 200 of 1100+ episodes because of an
    /// early 429). The UPSERT in `store_episodes_mal` is idempotent —
    /// existing rows are updated in place, missing rows are inserted.
    force: bool,
}

fn parse_args() -> Cli {
    let mut cli = Cli {
        dry_run: false,
        limit: None,
        provider: None,
        force: false,
    };
    let mut args = env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--dry-run" => cli.dry_run = true,
            "--limit" => {
                let v = args.next().expect("--limit requires a value");
                match v.parse() {
                    Ok(n) if n >= 0 => cli.limit = Some(n),
                    _ => {
                        eprintln!("--limit value must be a non-negative integer, got {v:?}");
                        std::process::exit(2);
                    }
                }
            }
            "--provider" => {
                let v = args.next().expect("--provider requires a value");
                cli.provider = Some(v);
            }
            "--force" => cli.force = true,
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => {
                eprintln!("unknown arg: {other}");
                print_help();
                std::process::exit(2);
            }
        }
    }
    cli
}

fn print_help() {
    eprintln!(
        "backfill_anime — backfill mal_id and episodes for anime added before the Jikan migration\n\
         \n\
         USAGE:\n  \
             backfill_anime [--dry-run] [--force] [--limit N] [--provider <name>]\n\
         \n\
         OPTIONS:\n  \
             --dry-run        Show what would change without writing to DB or external APIs\n  \
             --force          Re-fetch episodes for rows that already have a mal_id (UPSERT, idempotent)\n  \
             --limit N        Only process the first N rows (for smoke-testing)\n  \
             --provider NAME  Filter by provider (e.g. shikimori, mal)\n  \
             -h, --help       Show this message\n\
         \n\
         ENV:\n  \
             DATABASE_URL     required\n  \
             RUST_LOG         optional, e.g. \"info,sqlx=warn,reqwest=warn\""
    );
}

#[derive(Debug, Default)]
struct Summary {
    total: i64,
    resolved: i64,
    episodes_stored: i64,
    episodes_failed: i64,
    skipped: i64,
    failed: i64,
    force_refetched: i64,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,sqlx=warn,reqwest=warn".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = parse_args();

    let config = match Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("config error: {e}");
            return ExitCode::from(2);
        }
    };

    let pool = match PgPool::connect(&config.database_url).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("db connect: {e}");
            return ExitCode::from(2);
        }
    };

    let mal = MalService::new();
    let shikimori_http = match reqwest::Client::builder()
        .user_agent(SHIKIMORI_USER_AGENT)
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("reqwest client build: {e}");
            return ExitCode::from(2);
        }
    };

    let summary = run(&pool, &mal, &shikimori_http, &cli).await;
    print_summary(&summary, cli.dry_run);

    if summary.failed > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

async fn run(
    pool: &PgPool,
    mal: &MalService,
    shikimori_http: &reqwest::Client,
    cli: &Cli,
) -> Summary {
    let mut summary = Summary::default();

    // Build the SELECT. Provider filter is bound; LIMIT is inlined
    // (sqlx doesn't allow parameterised LIMIT, but we validated it
    // as a non-negative integer in parse_args).
    //
    // Default mode targets `mal_id IS NULL` rows that need resolve+fetch.
    // `--force` mode targets `mal_id IS NOT NULL` rows that just need
    // a fresh episode fetch (UPSERT, idempotent).
    let mal_id_filter = if cli.force { "IS NOT NULL" } else { "IS NULL" };
    let mut sql = format!(
        "SELECT id, provider, external_id, shikimori_id, title, mal_id \
         FROM media_items \
         WHERE media_type = 'anime' AND mal_id {mal_id_filter}"
    );
    if cli.provider.is_some() {
        sql.push_str(" AND provider = $1");
    }
    sql.push_str(" ORDER BY created_at ASC");
    if let Some(n) = cli.limit {
        sql.push_str(&format!(" LIMIT {n}"));
    }

    let mut q = sqlx::query(&sql);
    if let Some(p) = &cli.provider {
        q = q.bind(p);
    }
    let rows = match q.fetch_all(pool).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "select failed");
            return summary;
        }
    };

    summary.total = rows.len() as i64;
    tracing::info!(
        total = summary.total,
        dry_run = cli.dry_run,
        force = cli.force,
        limit = ?cli.limit,
        provider = ?cli.provider,
        "candidates selected"
    );

    for (i, row) in rows.into_iter().enumerate() {
        let id: Uuid = match row.try_get("id") {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(error = %e, "row missing id, skipping");
                summary.failed += 1;
                continue;
            }
        };
        let provider: String = row.get("provider");
        let external_id: String = row.get("external_id");
        let shikimori_id: Option<i64> = row.get("shikimori_id");
        let title: String = row.get("title");
        let existing_mal_id: Option<i64> = row.get("mal_id");

        // --force short-circuit: skip resolve+UPDATE, go straight to fetch.
        if cli.force {
            let Some(mal_id) = existing_mal_id else {
                // Defensive: shouldn't happen because of the WHERE filter.
                tracing::warn!(%id, "force mode: row has NULL mal_id, skipping");
                summary.skipped += 1;
                continue;
            };
            tracing::info!(
                index = i + 1,
                total = summary.total,
                %provider,
                %external_id,
                mal_id,
                "force refetching episodes"
            );
            if !cli.dry_run {
                match episodes::fetch_and_store_mal(pool.clone(), mal, mal_id).await {
                    Ok(n) => {
                        summary.episodes_stored += n as i64;
                        summary.force_refetched += 1;
                        tracing::info!(mal_id, episodes = n, "episodes stored (force)");
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, mal_id, "force refetch failed");
                        summary.episodes_failed += 1;
                    }
                }
                tokio::time::sleep(BETWEEN_ITEMS_PAUSE).await;
            }
            continue;
        }

        let cand = BackfillCandidate {
            id,
            provider: provider.clone(),
            external_id: external_id.clone(),
            shikimori_id,
            title: title.clone(),
        };
        let strategy = choose_strategy(&cand);
        tracing::info!(
            index = i + 1,
            total = summary.total,
            %provider,
            %external_id,
            ?strategy,
            "processing"
        );

        let resolved: Option<i64> = match strategy {
            Strategy::MalIdFromExternal => try_external_id_as_mal(&cand),
            Strategy::ShikimoriLookup => {
                let shiki_id = cand.shikimori_id.expect("strategy implies Some");
                tokio::time::sleep(SHIKIMORI_PAUSE).await;
                match shikimori_lookup_mal(shikimori_http, shiki_id).await {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            shikimori_id = shiki_id,
                            "shikimori lookup failed"
                        );
                        summary.failed += 1;
                        continue;
                    }
                }
            }
            Strategy::JikanSearchByTitle => {
                tokio::time::sleep(JIKAN_SEARCH_PAUSE).await;
                match jikan_search_first(mal, &title).await {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::warn!(error = %e, %title, "jikan search failed");
                        summary.failed += 1;
                        continue;
                    }
                }
            }
            Strategy::Unresolvable => {
                tracing::warn!(%provider, %external_id, "unresolvable");
                summary.skipped += 1;
                continue;
            }
        };

        let Some(mal_id) = resolved else {
            tracing::warn!(%provider, %external_id, "no mal_id resolved");
            summary.skipped += 1;
            continue;
        };

        summary.resolved += 1;
        tracing::info!(%provider, %external_id, mal_id, "resolved mal_id");

        if cli.dry_run {
            continue;
        }

        if let Err(e) = sqlx::query("UPDATE media_items SET mal_id = $1 WHERE id = $2")
            .bind(mal_id)
            .bind(id)
            .execute(pool)
            .await
        {
            tracing::error!(error = %e, %id, "update mal_id failed");
            summary.failed += 1;
            continue;
        }

        match episodes::fetch_and_store_mal(pool.clone(), mal, mal_id).await {
            Ok(n) => {
                summary.episodes_stored += n as i64;
                tracing::info!(mal_id, episodes = n, "episodes stored");
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    mal_id,
                    "episode fetch failed (mal_id already updated; re-run is safe)"
                );
                summary.episodes_failed += 1;
            }
        }

        tokio::time::sleep(BETWEEN_ITEMS_PAUSE).await;
    }

    summary
}

async fn shikimori_lookup_mal(
    http: &reqwest::Client,
    shikimori_id: i64,
) -> Result<Option<i64>, Box<dyn std::error::Error + Send + Sync>> {
    let url = format!("https://shikimori.one/api/animes/{shikimori_id}");
    let resp = http.get(&url).send().await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !resp.status().is_success() {
        return Err(format!("status {}", resp.status()).into());
    }
    let body = resp.text().await?;
    Ok(mal_id_from_shikimori_anime_response(&body))
}

async fn jikan_search_first(
    mal: &MalService,
    title: &str,
) -> Result<Option<i64>, Box<dyn std::error::Error + Send + Sync>> {
    let results = mal.search(title).await?;
    // map_search/map_full always set mal_id for anime results.
    Ok(results.into_iter().next().and_then(|item| item.mal_id))
}

fn print_summary(s: &Summary, dry_run: bool) {
    eprintln!();
    eprintln!("==== Backfill summary ====");
    if dry_run {
        eprintln!("  (DRY RUN — no writes)");
    }
    eprintln!("  candidates:      {}", s.total);
    eprintln!("  resolved:        {}", s.resolved);
    eprintln!("  episodes stored: {}", s.episodes_stored);
    eprintln!("  episodes failed: {}", s.episodes_failed);
    eprintln!("  skipped:         {}", s.skipped);
    eprintln!("  failed:          {}", s.failed);
}
