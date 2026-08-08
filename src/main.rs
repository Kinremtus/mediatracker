use mediatracker::app_state::AppState;
use mediatracker::config::Config;
use mediatracker::metrics;
use mediatracker::middleware::auth_middleware;
use mediatracker::routes::{admin, auth, calendar, home, media, search, settings, stats, tmdb_episodes, tmdb_image, tracking};
use mediatracker::services::refresh_counts;
use axum::{middleware::from_fn, middleware::from_fn_with_state, routing::get, Router, Json};
use serde_json::json;
use tokio_util::sync::CancellationToken;
use tower_http::services::ServeDir;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load configuration
    let config = Config::from_env()?;
    info!("Starting MediaTracker on {}:{}", config.host, config.port);

    // Get API keys from environment
    let tmdb_api_key = std::env::var("TMDB_API_KEY").unwrap_or_default();
    let rawg_api_key = std::env::var("RAWG_API_KEY").unwrap_or_default();
    let igdb_client_id = std::env::var("IGDB_CLIENT_ID").unwrap_or_default();
    let igdb_client_secret = std::env::var("IGDB_CLIENT_SECRET").unwrap_or_default();
    let telegram_bot_token = std::env::var("TELEGRAM_BOT_TOKEN").unwrap_or_default();
    let resend_api_key = std::env::var("RESEND_API_KEY").unwrap_or_default();
    let email_from = std::env::var("EMAIL_FROM").unwrap_or_default();
    let app_base_url = std::env::var("APP_BASE_URL").unwrap_or_default();

    // Initialize database and run migrations
    let state = AppState::new(
        &config.database_url,
        &tmdb_api_key,
        &rawg_api_key,
        &igdb_client_id,
        &igdb_client_secret,
        &telegram_bot_token,
        &resend_api_key,
        &email_from,
        &app_base_url,
    ).await?;
    info!("Database connected and migrations applied");

    // Public routes
    let public_routes = Router::new()
        .route("/tmdb-image/{*path}", get(tmdb_image::get_tmdb_image))
        .route("/login", get(auth::get_login).post(auth::post_login))
        .route("/register", get(auth::get_register).post(auth::post_register))
        .route("/forgot-password", get(auth::get_forgot_password).post(auth::post_forgot_password))
        .route("/reset-password", get(auth::get_reset_password).post(auth::post_reset_password));

    // Protected routes
    let protected_routes = Router::new()
        .route("/", get(home::get_home))
        .route("/logout", axum::routing::post(home::post_logout))
        .route("/search", get(search::get_search))
        .route("/media/{provider}/{external_id}", get(media::get_media_detail))
        .route("/api/media/{provider}/{external_id}", get(media::get_media_drawer_content))
        .route("/api/anime/{provider}/{external_id}/episodes", get(media::get_episodes))
        .route("/api/anime/{provider}/{external_id}/episodes/{n}/watched", axum::routing::post(media::set_episode_watched))
        .route("/api/manga/{provider}/{external_id}/chapters", get(media::get_chapters))
        .route("/api/manga/{provider}/{external_id}/chapters/{n}/read", axum::routing::post(media::set_chapter_read))
        .route("/api/search/suggestions", get(search::get_search_suggestions))
        .route("/api/tmdb/{external_id}/seasons", get(tmdb_episodes::get_tmdb_seasons))
        .route("/api/tmdb/{external_id}/seasons/{season_number}/watched", axum::routing::post(tmdb_episodes::post_tmdb_season_watched))
        .route("/api/tmdb/{external_id}/seasons/{season_number}/episodes", get(tmdb_episodes::get_tmdb_episodes))
        .route("/api/tmdb/{external_id}/seasons/{season_number}/episodes/{episode_number}/watched", axum::routing::post(tmdb_episodes::set_tmdb_episode_watched))
        .route("/tracking", get(tracking::get_tracking_list).post(tracking::post_add_to_tracking))
        .route("/tracking/{id}", axum::routing::post(tracking::post_update_tracking))
        .route("/tracking/{id}/delete", axum::routing::post(tracking::post_delete_tracking))
        .route("/tracking/partial", get(tracking::htmx_tracking_partial))
        .route("/tracking/{id}/htmx", axum::routing::post(tracking::htmx_update_tracking))
        .route("/tracking/{id}/htmx/delete", axum::routing::post(tracking::htmx_delete_tracking))
        .route("/calendar", get(calendar::get_calendar))
        .route("/stats", get(stats::get_stats))
        .route("/settings", get(settings::get_settings))
        .route("/settings/profile", axum::routing::post(settings::post_profile))
        .route("/settings/password", axum::routing::post(settings::post_password))
        .route("/settings/delete-account", axum::routing::post(settings::post_delete_account))
        .route("/settings/profile/htmx", axum::routing::post(settings::htmx_update_profile))
        .route("/settings/password/htmx", axum::routing::post(settings::htmx_update_password))
        .route("/settings/telegram/htmx", axum::routing::post(settings::htmx_save_telegram_chat_id))
        .route("/settings/telegram/test", axum::routing::post(settings::htmx_test_telegram))
        .route("/admin", get(admin::get_admin_panel))
        .route("/admin/refresh-details", axum::routing::post(admin::post_refresh_details))
        .route("/admin/enrich-chapters", axum::routing::post(admin::post_enrich_chapters))
        .layer(from_fn_with_state(state.clone(), auth_middleware));

    // Metrics endpoint (without metrics middleware — recursive counting otherwise)
    let metrics_route = Router::new()
        .route("/metrics", get(metrics::metrics_handler))
        .with_state(state.metrics_handle.clone());

    // Start background refresh loop
    let cancel = CancellationToken::new();
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
        };
        refresh_counts::run_refresh_loop(ctx, refresh_cancel).await;
    });

    // All other routes with metrics recording
    let app = Router::new()
        .route("/health", get(health_check))
        .merge(public_routes)
        .merge(protected_routes)
        .nest_service("/static", ServeDir::new("static"))
        .layer(from_fn(metrics::metrics_middleware))
        .with_state(state);

    // Combine: /metrics is not wrapped by the middleware layer
    let app = Router::new()
        .merge(metrics_route)
        .merge(app);

    // Start server with graceful shutdown
    let listener = tokio::net::TcpListener::bind(format!("{}:{}", config.host, config.port)).await?;
    info!("Server listening on {}:{}", config.host, config.port);
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            #[cfg(unix)]
            {
                use tokio::signal::unix::{signal, SignalKind};
                let mut term = signal(SignalKind::terminate()).expect("install SIGTERM handler");
                tokio::select! {
                    _ = term.recv() => {},
                    _ = tokio::signal::ctrl_c() => {},
                }
            }
            #[cfg(not(unix))]
            tokio::signal::ctrl_c().await.ok();

            cancel.cancel();
        })
        .await?;

    Ok(())
}

async fn health_check() -> Json<serde_json::Value> {
    Json(json!({"status": "ok"}))
}
