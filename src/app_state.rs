use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

use crate::metrics::MetricsHandle;
use crate::services::auth::AuthService;
use crate::services::email::EmailService;
use crate::services::external::google_books::GoogleBooksService;
use crate::services::external::igdb::IgdbService;
use crate::services::external::mal::MalService;
use crate::services::external::mangaupdates::MangaUpdatesService;
use crate::services::external::openlibrary::OpenLibraryService;
use crate::services::external::rawg::RawgService;
use crate::services::external::shikimori::ShikimoriService;
use crate::services::external::tmdb::TmdbService;
use crate::services::notifications::TelegramNotifier;
use crate::services::password_reset::PasswordResetService;
use crate::services::release_schedule::ReleaseScheduleService;
use crate::services::stats::StatsService;
use crate::services::tracking::TrackingService;
use reqwest::Client;

/// Read an environment variable, trimming it and falling back to `default`
/// when it is missing or cannot be parsed.
fn env_parsed<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<T>().ok())
        .unwrap_or(default)
}

/// Build the shared Postgres pool with explicit, env-tunable limits.
///
/// Limits are deliberately conservative: every replica multiplies connection
/// usage, and the background refresh acquires additional connections.
/// - `DATABASE_MAX_CONNECTIONS` (default 10)
/// - `DATABASE_ACQUIRE_TIMEOUT` in seconds (default 30)
pub async fn build_pool(database_url: &str) -> Result<PgPool, sqlx::Error> {
    let max_connections = env_parsed("DATABASE_MAX_CONNECTIONS", 10u32);
    let acquire_timeout_secs = env_parsed("DATABASE_ACQUIRE_TIMEOUT", 30u64);

    PgPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(Duration::from_secs(acquire_timeout_secs))
        .max_lifetime(Duration::from_secs(30 * 60))
        .idle_timeout(Duration::from_secs(10 * 60))
        .test_before_acquire(true)
        .connect(database_url)
        .await
}

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub http_client: Client,
    pub auth: AuthService,
    pub shikimori: ShikimoriService,
    pub mal: MalService,
    pub mangaupdates: MangaUpdatesService,
    pub tmdb: TmdbService,
    pub rawg: RawgService,
    pub igdb: IgdbService,
    pub google_books: GoogleBooksService,
    pub openlibrary: OpenLibraryService,
    pub tracking: TrackingService,
    pub release_schedule: ReleaseScheduleService,
    pub stats: StatsService,
    pub telegram: TelegramNotifier,
    pub email: EmailService,
    pub password_reset: PasswordResetService,
    pub metrics_handle: MetricsHandle,
}

impl AppState {
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        database_url: &str,
        tmdb_api_key: &str,
        rawg_api_key: &str,
        igdb_client_id: &str,
        igdb_client_secret: &str,
        telegram_bot_token: &str,
        resend_api_key: &str,
        email_from: &str,
        app_base_url: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let db = build_pool(database_url).await?;
        Self::from_pool(
            db,
            tmdb_api_key,
            rawg_api_key,
            igdb_client_id,
            igdb_client_secret,
            telegram_bot_token,
            resend_api_key,
            email_from,
            app_base_url,
        )
        .await
    }

    /// Build application state on top of an already-constructed pool.
    ///
    /// Migrations are applied here so every entry point (server, tests) gets a
    /// migrated schema.
    #[allow(clippy::too_many_arguments)]
    pub async fn from_pool(
        db: PgPool,
        tmdb_api_key: &str,
        rawg_api_key: &str,
        igdb_client_id: &str,
        igdb_client_secret: &str,
        telegram_bot_token: &str,
        resend_api_key: &str,
        email_from: &str,
        app_base_url: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        sqlx::migrate!("./migrations").run(&db).await?;
        let http_client = Client::new();
        let auth = AuthService::new(db.clone());
        let shikimori = ShikimoriService::new();
        let mal = MalService::new();
        let mangaupdates = MangaUpdatesService::new();
        let tmdb = TmdbService::new(tmdb_api_key.to_string());
        let rawg = RawgService::new(rawg_api_key.to_string());
        let igdb = IgdbService::new(igdb_client_id.to_string(), igdb_client_secret.to_string());
        let google_books = GoogleBooksService::new();
        let openlibrary = OpenLibraryService::new();
        let tracking = TrackingService::new(db.clone());
        let release_schedule = ReleaseScheduleService::new(db.clone());
        let stats = StatsService::new(db.clone());
        let telegram = TelegramNotifier::new(telegram_bot_token.to_string());
        let email = EmailService::new(resend_api_key, email_from, http_client.clone());
        let password_reset = PasswordResetService::new(
            db.clone(),
            auth.clone(),
            email.clone(),
            telegram.clone(),
            app_base_url.to_string(),
        );
        let metrics_handle = crate::metrics::init_metrics();
        Ok(Self {
            db,
            http_client,
            auth,
            shikimori,
            mal,
            mangaupdates,
            tmdb,
            rawg,
            igdb,
            google_books,
            openlibrary,
            tracking,
            release_schedule,
            stats,
            telegram,
            email,
            password_reset,
            metrics_handle,
        })
    }
}
