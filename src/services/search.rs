use crate::app_state::AppState;
use crate::models::media_item::CreateMediaItem;

fn extend(
    acc: &mut Vec<CreateMediaItem>,
    result: Result<Vec<CreateMediaItem>, anyhow::Error>,
    provider: &'static str,
) {
    match result {
        Ok(items) => acc.extend(items),
        Err(e) => tracing::warn!(provider, error = %e, "external search failed"),
    }
}

/// Аниме: Shikimori + MyAnimeList (Jikan).
/// MangaUpdates не каталогизирует аниме — только комиксы/новеллы.
pub async fn anime(state: &AppState, query: &str) -> Vec<CreateMediaItem> {
    let (shiki, mal) = tokio::join!(
        state.shikimori.search(query),
        state.mal.search(query),
    );
    let mut out = Vec::new();
    extend(&mut out, shiki, "shikimori");
    extend(&mut out, mal, "mal");
    out
}

pub async fn manga(state: &AppState, query: &str) -> Vec<CreateMediaItem> {
    let mut out = Vec::new();
    extend(
        &mut out,
        state.mangaupdates.search_by_type(query, &["Manga"]).await,
        "mangaupdates",
    );
    out
}

pub async fn manhwa(state: &AppState, query: &str) -> Vec<CreateMediaItem> {
    let mut out = Vec::new();
    extend(
        &mut out,
        state.mangaupdates.search_by_type(query, &["Manhwa"]).await,
        "mangaupdates",
    );
    out
}

pub async fn manhua(state: &AppState, query: &str) -> Vec<CreateMediaItem> {
    let mut out = Vec::new();
    extend(
        &mut out,
        state.mangaupdates.search_by_type(query, &["Manhua"]).await,
        "mangaupdates",
    );
    out
}

pub async fn novel(state: &AppState, query: &str) -> Vec<CreateMediaItem> {
    let mut out = Vec::new();
    extend(
        &mut out,
        state.mangaupdates.search_by_type(query, &["Novel"]).await,
        "mangaupdates",
    );
    out
}

pub async fn other_comics(state: &AppState, query: &str) -> Vec<CreateMediaItem> {
    let mut out = Vec::new();
    extend(
        &mut out,
        state
            .mangaupdates
            .search_by_type(
                query,
                &[
                    "OEL",
                    "Doujinshi",
                    "Filipino",
                    "Indonesian",
                    "Thai",
                    "Vietnamese",
                    "Malaysian",
                ],
            )
            .await,
        "mangaupdates",
    );
    out
}

/// Фильмы / сериалы / дорамы / мультики → TMDB. IMDb — позже.
pub async fn movie(state: &AppState, query: &str) -> Vec<CreateMediaItem> {
    let mut out = Vec::new();
    if state.tmdb.api_key.is_empty() {
        tracing::warn!("TMDB_API_KEY not set, movie search skipped");
        return out;
    }
    extend(
        &mut out,
        state.tmdb.search_movies(query, None).await,
        "tmdb",
    );
    out
}

pub async fn series(state: &AppState, query: &str) -> Vec<CreateMediaItem> {
    let mut out = Vec::new();
    if state.tmdb.api_key.is_empty() {
        tracing::warn!("TMDB_API_KEY not set, series search skipped");
        return out;
    }
    extend(&mut out, state.tmdb.search_tv(query, None).await, "tmdb");
    out
}

pub async fn dramas(state: &AppState, query: &str) -> Vec<CreateMediaItem> {
    let mut out = Vec::new();
    if state.tmdb.api_key.is_empty() {
        return out;
    }
    extend(
        &mut out,
        state.tmdb.search_tv(query, Some(18)).await,
        "tmdb",
    );
    out
}

pub async fn cartoons(state: &AppState, query: &str) -> Vec<CreateMediaItem> {
    let mut out = Vec::new();
    if state.tmdb.api_key.is_empty() {
        return out;
    }
    extend(
        &mut out,
        state.tmdb.search_tv(query, Some(16)).await,
        "tmdb",
    );
    out
}

pub async fn animated_movies(state: &AppState, query: &str) -> Vec<CreateMediaItem> {
    let mut out = Vec::new();
    if state.tmdb.api_key.is_empty() {
        return out;
    }
    extend(
        &mut out,
        state.tmdb.search_movies(query, Some(16)).await,
        "tmdb",
    );
    out
}

/// Игры → RAWG.
pub async fn game(state: &AppState, query: &str) -> Vec<CreateMediaItem> {
    let mut out = Vec::new();
    if state.rawg.api_key.is_empty() {
        tracing::warn!("RAWG_API_KEY not set, game search skipped");
        return out;
    }
    extend(&mut out, state.rawg.search(query).await, "rawg");
    out
}

/// Книги → Google Books.
pub async fn book(state: &AppState, query: &str) -> Vec<CreateMediaItem> {
    let mut out = Vec::new();
    extend(
        &mut out,
        state.google_books.search(query).await,
        "google_books",
    );
    out
}

/// Без выбранного типа — срез по всем категориям (по одному запросу на группу провайдеров).
pub async fn all_types(state: &AppState, query: &str) -> Vec<CreateMediaItem> {
    let (anime_r, manga_r, movie_r, game_r, book_r) = tokio::join!(
        anime(state, query),
        state.mangaupdates.search(query),
        movie(state, query),
        game(state, query),
        book(state, query),
    );

    let mut out = anime_r;
    extend(&mut out, manga_r, "mangaupdates");
    out.extend(movie_r);
    out.extend(game_r);
    out.extend(book_r);
    out
}

pub async fn by_media_type(state: &AppState, query: &str, search_type: &str) -> Vec<CreateMediaItem> {
    match search_type {
        "anime" => anime(state, query).await,
        "manga" => manga(state, query).await,
        "manhwa" => manhwa(state, query).await,
        "manhua" => manhua(state, query).await,
        "novel" => novel(state, query).await,
        "other-comics" => other_comics(state, query).await,
        "movie" => movie(state, query).await,
        "series" => series(state, query).await,
        "dramas" => dramas(state, query).await,
        "cartoons" => cartoons(state, query).await,
        "animated-movies" => animated_movies(state, query).await,
        "game" => game(state, query).await,
        "book" => book(state, query).await,
        _ => all_types(state, query).await,
    }
}
