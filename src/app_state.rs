use sqlx::PgPool;

use reqwest::Client;
use crate::metrics::MetricsHandle;
use crate::services::auth::AuthService;
use crate::services::external::google_books::GoogleBooksService;
use crate::services::external::igdb::IgdbService;
use crate::services::external::mal::MalService;
use crate::services::external::mangaupdates::MangaUpdatesService;
use crate::services::external::openlibrary::OpenLibraryService;
use crate::services::external::rawg::RawgService;
use crate::services::external::shikimori::ShikimoriService;
use crate::services::external::tmdb::TmdbService;
use crate::services::notifications::TelegramNotifier;
use crate::services::release_schedule::ReleaseScheduleService;
use crate::services::stats::StatsService;
use crate::services::tracking::TrackingService;

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
    pub metrics_handle: MetricsHandle,
}

impl AppState {
    pub async fn new(
        database_url: &str,
        tmdb_api_key: &str,
        rawg_api_key: &str,
        igdb_client_id: &str,
        igdb_client_secret: &str,
        telegram_bot_token: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let db = PgPool::connect(database_url).await?;
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
            metrics_handle,
        })
    }
}
