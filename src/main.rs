use mediatracker::app_state::AppState;
use mediatracker::config::Config;
use mediatracker::middleware::auth_middleware;
use mediatracker::routes::{auth, home, media, search, stats, tracking};
use axum::{middleware::from_fn_with_state, routing::get, Router};
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

    // Initialize database and run migrations
    let state = AppState::new(&config.database_url, &tmdb_api_key, &rawg_api_key).await?;
    info!("Database connected and migrations applied");

    // Public routes
    let public_routes = Router::new()
        .route("/login", get(auth::get_login).post(auth::post_login))
        .route("/register", get(auth::get_register).post(auth::post_register));

    // Protected routes
    let protected_routes = Router::new()
        .route("/", get(home::get_home))
        .route("/logout", axum::routing::post(home::post_logout))
        .route("/search", get(search::get_search))
        .route("/media/{provider}/{external_id}", get(media::get_media_detail))
        .route("/tracking", get(tracking::get_tracking_list).post(tracking::post_add_to_tracking))
        .route("/tracking/{id}", axum::routing::post(tracking::post_update_tracking).delete(tracking::post_delete_tracking))
        .route("/stats", get(stats::get_stats))
        .layer(from_fn_with_state(state.clone(), auth_middleware));

    // Combine routes
    let app = Router::new()
        .route("/health", get(health_check))
        .merge(public_routes)
        .merge(protected_routes)
        .nest_service("/static", ServeDir::new("static"))
        .with_state(state);

    // Start server
    let listener = tokio::net::TcpListener::bind(format!("{}:{}", config.host, config.port)).await?;
    info!("Server listening on {}:{}", config.host, config.port);
    axum::serve(listener, app).await?;

    Ok(())
}

async fn health_check() -> &'static str {
    "ok"
}
