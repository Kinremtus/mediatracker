use sqlx::PgPool;

use crate::services::auth::AuthService;
use crate::services::external::mangaupdates::MangaUpdatesService;
use crate::services::external::rawg::RawgService;
use crate::services::external::shikimori::ShikimoriService;
use crate::services::external::tmdb::TmdbService;
use crate::services::stats::StatsService;
use crate::services::tracking::TrackingService;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub auth: AuthService,
    pub shikimori: ShikimoriService,
    pub mangaupdates: MangaUpdatesService,
    pub tmdb: TmdbService,
    pub rawg: RawgService,
    pub tracking: TrackingService,
    pub stats: StatsService,
}

impl AppState {
    pub async fn new(
        database_url: &str,
        tmdb_api_key: &str,
        rawg_api_key: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let db = PgPool::connect(database_url).await?;
        sqlx::migrate!("./migrations").run(&db).await?;
        let auth = AuthService::new(db.clone());
        let shikimori = ShikimoriService::new();
        let mangaupdates = MangaUpdatesService::new();
        let tmdb = TmdbService::new(tmdb_api_key.to_string());
        let rawg = RawgService::new(rawg_api_key.to_string());
        let tracking = TrackingService::new(db.clone());
        let stats = StatsService::new(db.clone());
        Ok(Self {
            db,
            auth,
            shikimori,
            mangaupdates,
            tmdb,
            rawg,
            tracking,
            stats,
        })
    }
}
