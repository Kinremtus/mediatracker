use std::collections::HashSet;

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

fn normalize_title(title: &str) -> String {
    title.to_lowercase().trim().to_string()
}

/// Приоритет провайдера для типа медиа (0 =canonical, 10 = остальные).
fn provider_priority(media_type: &str, provider: &str) -> u8 {
    match media_type {
        "anime" => match provider {
            "mal" => 0,
            "shikimori" => 1,
            _ => 10,
        },
        "manga" | "manhwa" | "manhua" | "novel" | "other-comics" => match provider {
            "mangaupdates" => 0,
            _ => 10,
        },
        "game" => match provider {
            "rawg" => 0,
            _ => 10,
        },
        "book" => match provider {
            "google_books" => 0,
            _ => 10,
        },
        "movie" | "series" | "dramas" | "cartoons" | "animated-movies" => match provider {
            "tmdb" => 0,
            _ => 10,
        },
        _ => 10,
    }
}

/// Дедупликация по (comparison_key, media_type).
/// Сортирует по приоритету провайдера для типа, оставляет первый уникальный.
fn deduplicate_by_title(items: Vec<CreateMediaItem>) -> Vec<CreateMediaItem> {
    let mut sorted = items;
    sorted.sort_by_key(|item| provider_priority(&item.media_type, &item.provider));

    let mut seen = HashSet::new();
    sorted
        .into_iter()
        .filter(|item| {
            let key = item
                .comparison_key
                .as_deref()
                .unwrap_or(&item.title);
            let normalized = normalize_title(key);
            seen.insert((normalized, item.media_type.clone()))
        })
        .collect()
}

/// Аниме: Shikimori + MyAnimeList (Jikan).
/// MangaUpdates не каталогизирует аниме — только комиксы/новеллы.
pub async fn anime(state: &AppState, query: &str) -> Vec<CreateMediaItem> {
    let (shiki_res, mal_res) = tokio::join!(
        state.shikimori.search(query),
        state.mal.search(query),
    );

    let mut shiki_items = match shiki_res {
        Ok(items) => items,
        Err(e) => {
            tracing::warn!(provider = "shikimori", error = %e, "external search failed");
            Vec::new()
        }
    };

    let mal_items = match mal_res {
        Ok(items) => items,
        Err(e) => {
            tracing::warn!(provider = "mal", error = %e, "external search failed");
            Vec::new()
        }
    };

    let shiki_mal_ids: HashSet<i64> = shiki_items
        .iter()
        .filter_map(|item| item.mal_id)
        .collect();

    let filtered_count = mal_items
        .iter()
        .filter(|item| item.mal_id.map_or(false, |id| shiki_mal_ids.contains(&id)))
        .count();

    if filtered_count > 0 {
        tracing::info!(filtered = filtered_count, "deduplicated MAL results already present in Shikimori");
    }

    for item in mal_items.into_iter() {
        if item.mal_id.map_or(true, |id| !shiki_mal_ids.contains(&id)) {
            shiki_items.push(item);
        }
    }

    deduplicate_by_title(shiki_items)
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
    deduplicate_by_title(out)
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
