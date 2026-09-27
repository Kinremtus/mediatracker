//! Single dispatcher for the eight external media providers.
//!
//! Before this module every call site (`routes/media.rs` twice,
//! `routes/admin.rs::fetch_details_for_provider`,
//! `services/refresh_counts.rs`) repeated the same provider -> service
//! `match`, so adding a provider meant wiring it in four places. `Provider`
//! owns the per-provider behaviour (fetch, politeness delay, concurrency)
//! and `ProviderClients` is a borrowed bundle, so both `AppState` and the
//! refresh worker can build one without cloning the whole state.

use std::time::Duration;

use crate::app_state::AppState;
use crate::models::media_item::CreateMediaItem;
use crate::services::external::anilist::AniListService;
use crate::services::external::comicvine::ComicVineService;
use crate::services::external::google_books::GoogleBooksService;
use crate::services::external::hardcover::HardcoverService;
use crate::services::external::igdb::IgdbService;
use crate::services::external::mal::MalService;
use crate::services::external::mangadex::MangaDexService;
use crate::services::external::mangaupdates::MangaUpdatesService;
use crate::services::external::openlibrary::OpenLibraryService;
use crate::services::external::rawg::RawgService;
use crate::services::external::shikimori::ShikimoriService;
use crate::services::external::tmdb::TmdbService;

/// Borrowed set of provider clients. `AppState` owns all eleven; the
/// refresh worker keeps its own copies in `RefreshCtx`.
pub struct ProviderClients<'a> {
    pub shikimori: &'a ShikimoriService,
    pub mal: &'a MalService,
    pub mangaupdates: &'a MangaUpdatesService,
    pub tmdb: &'a TmdbService,
    pub rawg: &'a RawgService,
    pub igdb: &'a IgdbService,
    pub google_books: &'a GoogleBooksService,
    pub openlibrary: &'a OpenLibraryService,
    pub mangadex: &'a MangaDexService,
    pub anilist: &'a AniListService,
    pub comicvine: &'a ComicVineService,
    pub hardcover: &'a HardcoverService,
}

impl<'a> ProviderClients<'a> {
    /// Borrow every provider client out of the shared application state.
    pub fn from_state(state: &'a AppState) -> Self {
        Self {
            shikimori: &state.shikimori,
            mal: &state.mal,
            mangaupdates: &state.mangaupdates,
            tmdb: &state.tmdb,
            rawg: &state.rawg,
            igdb: &state.igdb,
            google_books: &state.google_books,
            openlibrary: &state.openlibrary,
            mangadex: &state.mangadex,
            anilist: &state.anilist,
            comicvine: &state.comicvine,
            hardcover: &state.hardcover,
        }
    }
}

/// Media types served by the manga-family chain (MAL/Shikimori split
/// their detail endpoint by family).
fn is_manga_family(media_type: &str) -> bool {
    matches!(
        media_type,
        "manga" | "manhwa" | "manhua" | "novel" | "other-comics" | "comic"
    )
}

/// One external provider, resolved from its `media_items.provider` name.
#[derive(Clone)]
pub enum Provider {
    Shikimori(ShikimoriService),
    Mal(MalService),
    MangaUpdates(MangaUpdatesService),
    Tmdb(TmdbService),
    Rawg(RawgService),
    Igdb(IgdbService),
    GoogleBooks(GoogleBooksService),
    OpenLibrary(OpenLibraryService),
    MangaDex(MangaDexService),
    AniList(AniListService),
    ComicVine(ComicVineService),
    Hardcover(HardcoverService),
}

impl Provider {
    /// Map a provider name to an instance. Returns `None` for unknown
    /// names so each caller picks its own error handling.
    pub fn from_name(clients: &ProviderClients<'_>, name: &str) -> Option<Self> {
        match name {
            "shikimori" => Some(Self::Shikimori(clients.shikimori.clone())),
            "mal" => Some(Self::Mal(clients.mal.clone())),
            "mangaupdates" => Some(Self::MangaUpdates(clients.mangaupdates.clone())),
            "tmdb" => Some(Self::Tmdb(clients.tmdb.clone())),
            "rawg" => Some(Self::Rawg(clients.rawg.clone())),
            "igdb" => Some(Self::Igdb(clients.igdb.clone())),
            "google_books" => Some(Self::GoogleBooks(clients.google_books.clone())),
            "openlibrary" => Some(Self::OpenLibrary(clients.openlibrary.clone())),
            "mangadex" => Some(Self::MangaDex(clients.mangadex.clone())),
            "anilist" => Some(Self::AniList(clients.anilist.clone())),
            "comicvine" => Some(Self::ComicVine(clients.comicvine.clone())),
            "hardcover" => Some(Self::Hardcover(clients.hardcover.clone())),
            _ => None,
        }
    }

    /// Canonical provider name (matches `media_items.provider`).
    pub fn name(&self) -> &'static str {
        match self {
            Self::Shikimori(_) => "shikimori",
            Self::Mal(_) => "mal",
            Self::MangaUpdates(_) => "mangaupdates",
            Self::Tmdb(_) => "tmdb",
            Self::Rawg(_) => "rawg",
            Self::Igdb(_) => "igdb",
            Self::GoogleBooks(_) => "google_books",
            Self::OpenLibrary(_) => "openlibrary",
            Self::MangaDex(_) => "mangadex",
            Self::AniList(_) => "anilist",
            Self::ComicVine(_) => "comicvine",
            Self::Hardcover(_) => "hardcover",
        }
    }

    /// Fetch full details. `media_type` is only meaningful for TMDB; the
    /// other providers ignore it. TMDB call sites pass either the raw
    /// media type (routes) or their own normalised value (refresh worker).
    pub async fn fetch(
        &self,
        external_id: &str,
        media_type: &str,
    ) -> Result<CreateMediaItem, anyhow::Error> {
        match self {
            Self::Mal(s) if is_manga_family(media_type) => s.get_manga_details(external_id).await,
            Self::Mal(s) => s.get_details(external_id).await,
            Self::Shikimori(s) if is_manga_family(media_type) => {
                s.get_manga_details(external_id).await
            }
            Self::Shikimori(s) => s.get_details(external_id).await,
            Self::MangaUpdates(s) => s.get_details(external_id).await,
            Self::Tmdb(s) => s.get_details(external_id, media_type).await,
            Self::Rawg(s) => s.get_details(external_id).await,
            Self::Igdb(s) => s.get_details(external_id).await,
            Self::GoogleBooks(s) => s.get_details(external_id).await,
            Self::OpenLibrary(s) => s.get_details(external_id).await,
            Self::MangaDex(s) => s.get_details(external_id).await,
            Self::AniList(s) => s.get_details(external_id).await,
            Self::ComicVine(s) => s.get_details(external_id).await,
            Self::Hardcover(s) => s.get_details(external_id).await,
        }
    }

    /// Per-request politeness delay for the bulk refresh worker.
    pub fn delay(&self) -> Duration {
        match self {
            Self::Mal(_) => Duration::from_millis(350),
            Self::ComicVine(_) => Duration::from_millis(1000),
            Self::Hardcover(_) => Duration::from_millis(1000),
            Self::AniList(_) => Duration::from_millis(350),
            _ => Duration::from_millis(200),
        }
    }

    /// How many parallel requests this provider tolerates.
    pub fn concurrency(&self) -> usize {
        match self {
            Self::Mal(_) => 2,
            Self::ComicVine(_) | Self::AniList(_) | Self::Hardcover(_) => 1,
            _ => 3,
        }
    }
}
