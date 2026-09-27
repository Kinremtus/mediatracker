use axum::{
    Router,
    middleware::from_fn,
    middleware::from_fn_with_state,
    routing::{get, post},
};
use mediatracker::app_state::{AppState, build_pool};
use mediatracker::config::Config;
use mediatracker::metrics;
use mediatracker::middleware::auth_middleware;
use mediatracker::middleware::rate_limit::{RateLimiter, rate_limit_middleware};
use mediatracker::middleware::security_headers::security_headers_middleware;
use mediatracker::routes::{
    admin, auth, calendar, health_check, home, media, search, settings, stats, tmdb_episodes,
    tmdb_image, tracking,
};
use mediatracker::services::{cleanup, refresh_counts, release_schedule};
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tower_http::services::ServeDir;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load configuration
    let config = Config::from_env()?;
    info!("Starting MediaTracker on {}:{}", config.host, config.port);

    // Get API keys from environment
    let tmdb_api_key = std::env::var("TMDB_API_KEY").unwrap_or_default();
    let comic_vine_api_key = std::env::var("COMIC_VINE_API_KEY").unwrap_or_default();
    let rawg_api_key = std::env::var("RAWG_API_KEY").unwrap_or_default();
    let igdb_client_id = std::env::var("IGDB_CLIENT_ID").unwrap_or_default();
    let igdb_client_secret = std::env::var("IGDB_CLIENT_SECRET").unwrap_or_default();
    let telegram_bot_token = std::env::var("TELEGRAM_BOT_TOKEN").unwrap_or_default();
    let resend_api_key = std::env::var("RESEND_API_KEY").unwrap_or_default();
    let email_from = std::env::var("EMAIL_FROM").unwrap_or_default();
    let app_base_url = std::env::var("APP_BASE_URL").unwrap_or_default();

    // Initialize database and run migrations. The pool is built with explicit,
    // env-tunable limits so replicas do not exhaust Postgres connections.
    let db = build_pool(&config.database_url).await?;
    let state = AppState::from_pool(
        db,
        &tmdb_api_key,
        &rawg_api_key,
        &igdb_client_id,
        &igdb_client_secret,
        &telegram_bot_token,
        &resend_api_key,
        &email_from,
        &app_base_url,
        &comic_vine_api_key,
    )
    .await?;
    info!("Database connected and migrations applied");

    // Rate limiting only on brute-force-sensitive auth endpoints. Applied with
    // `route_layer` so it never affects browsing or HTMX polling.
    let rate_limit_per_min: u32 = std::env::var("RATE_LIMIT_PER_MIN")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(10);
    let rate_limiter = RateLimiter::new(rate_limit_per_min, Duration::from_secs(60));

    let auth_post_routes = Router::new()
        .route("/login", post(auth::post_login))
        .route("/register", post(auth::post_register))
        .route("/forgot-password", post(auth::post_forgot_password))
        .route("/reset-password", post(auth::post_reset_password))
        .route_layer(from_fn_with_state(rate_limiter, rate_limit_middleware));

    // Public routes
    let public_routes = Router::new()
        .route("/tmdb-image/{*path}", get(tmdb_image::get_tmdb_image))
        .route("/login", get(auth::get_login))
        .route("/register", get(auth::get_register))
        .route("/forgot-password", get(auth::get_forgot_password))
        .route("/reset-password", get(auth::get_reset_password))
        .merge(auth_post_routes);

    // Protected routes
    let protected_routes = Router::new()
        .route("/", get(home::get_home))
        .route("/logout", axum::routing::post(home::post_logout))
        .route("/search", get(search::get_search))
        .route(
            "/media/{provider}/{external_id}",
            get(media::get_media_detail),
        )
        .route(
            "/api/media/{provider}/{external_id}",
            get(media::get_media_drawer_content),
        )
        .route(
            "/api/anime/{provider}/{external_id}/episodes",
            get(media::get_episodes),
        )
        .route(
            "/api/anime/{provider}/{external_id}/episodes/{n}/watched",
            axum::routing::post(media::set_episode_watched),
        )
        .route(
            "/api/manga/{provider}/{external_id}/chapters",
            get(media::get_chapters),
        )
        .route(
            "/api/manga/{provider}/{external_id}/chapters/{n}/read",
            axum::routing::post(media::set_chapter_read),
        )
        .route(
            "/api/games/{provider}/{external_id}/additions",
            get(media::get_game_additions),
        )
        .route(
            "/api/search/suggestions",
            get(search::get_search_suggestions),
        )
        .route(
            "/api/tmdb/{external_id}/seasons",
            get(tmdb_episodes::get_tmdb_seasons),
        )
        .route(
            "/api/tmdb/{external_id}/seasons/{season_number}/watched",
            axum::routing::post(tmdb_episodes::post_tmdb_season_watched),
        )
        .route(
            "/api/tmdb/{external_id}/seasons/{season_number}/episodes",
            get(tmdb_episodes::get_tmdb_episodes),
        )
        .route(
            "/api/tmdb/{external_id}/seasons/{season_number}/episodes/{episode_number}/watched",
            axum::routing::post(tmdb_episodes::set_tmdb_episode_watched),
        )
        .route(
            "/tracking",
            get(tracking::get_tracking_list).post(tracking::post_add_to_tracking),
        )
        .route(
            "/tracking/{id}",
            axum::routing::post(tracking::post_update_tracking),
        )
        .route(
            "/tracking/{id}/delete",
            axum::routing::post(tracking::post_delete_tracking),
        )
        .route("/tracking/partial", get(tracking::htmx_tracking_partial))
        .route(
            "/tracking/{id}/htmx",
            axum::routing::post(tracking::htmx_update_tracking),
        )
        .route(
            "/tracking/{id}/progress-row",
            get(tracking::get_progress_row),
        )
        .route(
            "/tracking/{id}/htmx/delete",
            axum::routing::post(tracking::htmx_delete_tracking),
        )
        .route("/calendar", get(calendar::get_calendar))
        .route("/stats", get(stats::get_stats))
        .route("/settings", get(settings::get_settings))
        .route(
            "/settings/profile",
            axum::routing::post(settings::post_profile),
        )
        .route(
            "/settings/password",
            axum::routing::post(settings::post_password),
        )
        .route(
            "/settings/delete-account",
            axum::routing::post(settings::post_delete_account),
        )
        .route(
            "/settings/profile/htmx",
            axum::routing::post(settings::htmx_update_profile),
        )
        .route(
            "/settings/password/htmx",
            axum::routing::post(settings::htmx_update_password),
        )
        .route(
            "/settings/telegram/htmx",
            axum::routing::post(settings::htmx_save_telegram_chat_id),
        )
        .route(
            "/settings/telegram/test",
            axum::routing::post(settings::htmx_test_telegram),
        )
        .route("/admin", get(admin::get_admin_panel))
        .route(
            "/admin/refresh-details",
            axum::routing::post(admin::post_refresh_details),
        )
        .route(
            "/admin/enrich-chapters",
            axum::routing::post(admin::post_enrich_chapters),
        )
        .layer(from_fn_with_state(state.clone(), auth_middleware));

    // Metrics endpoint (without metrics middleware — recursive counting otherwise)
    let metrics_route = Router::new()
        .route("/metrics", get(metrics::metrics_handler))
        .with_state(state.metrics_handle.clone());

    // Background refresh + Telegram notifications. Only ONE process should run
    // this in production: the dedicated refresh Deployment sets
    // REFRESH_LOOP_ENABLED=true while the web Deployment sets it to false.
    // Defaults to enabled so docker-compose / local dev keep working untouched.
    let cancel = CancellationToken::new();
    let refresh_enabled = std::env::var("REFRESH_LOOP_ENABLED")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(true);
    if refresh_enabled {
        let refresh_cancel = cancel.clone();
        let bg_state = state.clone();
        tokio::spawn(async move {
            let ctx = refresh_counts::RefreshCtx {
                db: bg_state.db,
                shikimori: bg_state.shikimori,
                mal: bg_state.mal,
                mangaupdates: bg_state.mangaupdates,
                tmdb: bg_state.tmdb,
                rawg: bg_state.rawg,
                igdb: bg_state.igdb,
                google_books: bg_state.google_books,
                openlibrary: bg_state.openlibrary,
                mangadex: bg_state.mangadex,
                anilist: bg_state.anilist,
                comicvine: bg_state.comicvine,
            };
            refresh_counts::run_refresh_loop(ctx, refresh_cancel).await;
        });

        // Retention sweeps for unbounded tables (sessions, reset tokens,
        // notification log). Kept on the single background process so web
        // replicas never write on a timer.
        let cleanup_cancel = cancel.clone();
        let cleanup_db = state.db.clone();
        tokio::spawn(async move {
            cleanup::run_cleanup_loop(cleanup_db, cleanup_cancel).await;
        });
    } else {
        info!("REFRESH_LOOP_ENABLED=false: background refresh disabled in this process");
    }

    // In-process periodic release-schedule refresh with failure isolation.
    // Runs shortly after startup and then every `REFRESH_INTERVAL_SECS`
    // (default 1800; set to 0 to disable). Each pass takes a fresh advisory
    // lock so only one replica refreshes, and a panic in one pass is contained
    // so the loop keeps running.
    let refresh_interval_secs: u64 = std::env::var("REFRESH_INTERVAL_SECS")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(1800);

    if refresh_interval_secs > 0 {
        let schedule = state.release_schedule.clone();
        let shikimori = state.shikimori.clone();
        let telegram = state.telegram.clone();
        let db = state.db.clone();
        let release_cancel = cancel.clone();

        tokio::spawn(async move {
            tokio::select! {
                _ = release_cancel.cancelled() => return,
                _ = tokio::time::sleep(Duration::from_secs(5)) => {}
            }

            let mut interval = tokio::time::interval(Duration::from_secs(refresh_interval_secs));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            // Consume the immediate first tick so the loop below controls timing
            // (the first pass runs right away, then one per interval).
            interval.tick().await;

            loop {
                let schedule = schedule.clone();
                let shikimori = shikimori.clone();
                let telegram = telegram.clone();
                let db = db.clone();

                // Spawn each pass so a panic is contained and observable; the
                // advisory lock always releases its connection lock.
                let pass = tokio::spawn(async move {
                    release_schedule::with_refresh_lock(&db, move || async move {
                        release_schedule::refresh_release_schedule(
                            &schedule, &shikimori, &telegram,
                        )
                        .await;
                        Ok::<(), anyhow::Error>(())
                    })
                    .await
                })
                .await;

                match pass {
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => {
                        tracing::error!(error = %error, "release schedule refresh pass failed");
                    }
                    Err(join_error) => {
                        tracing::error!(error = %join_error, "release schedule refresh task panicked");
                    }
                }

                tokio::select! {
                    _ = release_cancel.cancelled() => {
                        info!("release schedule refresh: cancelled, shutting down");
                        break;
                    }
                    _ = interval.tick() => {}
                }
            }
        });
    }

    // All other routes with metrics recording
    let app = Router::new()
        .route("/health", get(health_check))
        .merge(public_routes)
        .merge(protected_routes)
        .nest_service("/static", ServeDir::new("static"))
        .layer(from_fn(metrics::metrics_middleware))
        .layer(from_fn(security_headers_middleware))
        .with_state(state);

    // Combine: /metrics is not wrapped by the middleware layer
    let app = Router::new().merge(metrics_route).merge(app);

    // Worker-only mode: no HTTP listener. Used by the dedicated refresh
    // Deployment so it never serves traffic and its lifecycle is not tied to a
    // port. Defaults to true so docker-compose / local dev are unaffected.
    let serve_http = std::env::var("SERVE_HTTP")
        .map(|v| !matches!(v.trim(), "false" | "0" | "no"))
        .unwrap_or(true);

    if !serve_http {
        info!("SERVE_HTTP=false: HTTP server disabled; running background worker only");
        wait_for_shutdown().await;
        cancel.cancel();
        return Ok(());
    }

    // Start server with graceful shutdown
    let listener =
        tokio::net::TcpListener::bind(format!("{}:{}", config.host, config.port)).await?;
    info!("Server listening on {}:{}", config.host, config.port);
    let cancel_on_shutdown = cancel.clone();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            wait_for_shutdown().await;
            cancel_on_shutdown.cancel();
        })
        .await?;

    Ok(())
}

/// Resolve when the process receives SIGTERM (k8s) or Ctrl-C.
async fn wait_for_shutdown() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut term = signal(SignalKind::terminate()).expect("install SIGTERM handler");
        tokio::select! {
            _ = term.recv() => {},
            _ = tokio::signal::ctrl_c() => {},
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await.ok();
}
