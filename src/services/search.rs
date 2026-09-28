use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;

use crate::app_state::AppState;
use crate::models::media_item::CreateMediaItem;

/// Ленивое будущее поиска у одного провайдера. Создание future дешёвое —
/// сетевой запрос уходит только при первом `.await`.
type SearchFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<CreateMediaItem>, anyhow::Error>> + Send + 'a>>;

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
            "anilist" => 2,
            _ => 10,
        },
        "manga" | "manhwa" | "manhua" | "novel" | "other-comics" => match provider {
            "mangaupdates" => 0,
            "mangadex" => 1,
            "mal" => 2,
            "shikimori" => 3,
            "anilist" => 4,
            _ => 10,
        },
        "comic" => match provider {
            "comicvine" => 0,
            _ => 10,
        },
        "game" => match provider {
            "rawg" => 0,
            "igdb" => 1,
            _ => 10,
        },
        "book" => match provider {
            "hardcover" => 0,
            "google_books" => 1,
            "openlibrary" => 2,
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
            let key = item.comparison_key.as_deref().unwrap_or(&item.title);
            let normalized = normalize_title(key);
            seen.insert((normalized, item.media_type.clone()))
        })
        .collect()
}

/// Ленивая цепочка: futures опрашиваются по одному, поэтому следующий
/// провайдер не запрашивается, пока предыдущий не вернул Err или пусто.
/// Первый непустой `Ok` побеждает; пусто, если все провалились/пусты.
async fn first_non_empty(attempts: Vec<(&'static str, SearchFuture<'_>)>) -> Vec<CreateMediaItem> {
    for (provider, fut) in attempts {
        match fut.await {
            Ok(items) if !items.is_empty() => return items,
            Ok(_) => tracing::debug!(provider, "search empty, trying next provider"),
            Err(e) => tracing::warn!(provider, error = %e, "search failed, trying next provider"),
        }
    }
    Vec::new()
}

/// Переписать media_type на запрошенный, сохранив provider/external_id.
fn normalize_media_type(items: Vec<CreateMediaItem>, requested: &str) -> Vec<CreateMediaItem> {
    let mut items = items;
    for item in &mut items {
        item.media_type = requested.to_string();
    }
    items
}

/// Manga-family: MangaUpdates -> MangaDex -> Jikan(MAL) -> Shikimori -> AniList.
/// Ленивая цепочка: первый провайдер с непустым результатом побеждает,
/// остальные не запрашиваются.
async fn manga_family_search(
    state: &AppState,
    query: &str,
    mu_types: &[&str],
    requested: &str,
) -> Vec<CreateMediaItem> {
    let attempts: Vec<(&'static str, SearchFuture<'_>)> = vec![
        (
            "mangaupdates",
            Box::pin(state.mangaupdates.search_by_type(query, mu_types)),
        ),
        ("mangadex", Box::pin(state.mangadex.search(query))),
        ("mal", Box::pin(state.mal.search_manga(query))),
        ("shikimori", Box::pin(state.shikimori.search_manga(query))),
        ("anilist", Box::pin(state.anilist.search(query, requested))),
    ];
    let winner = first_non_empty(attempts).await;
    deduplicate_by_title(normalize_media_type(winner, requested))
}

/// Аниме: Shikimori + MyAnimeList (Jikan).
/// MangaUpdates не каталогизирует аниме — только комиксы/новеллы.
pub async fn anime(state: &AppState, query: &str) -> Vec<CreateMediaItem> {
    let (shiki_res, mal_res) =
        tokio::join!(state.shikimori.search(query), state.mal.search(query),);

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

    let shiki_mal_ids: HashSet<i64> = shiki_items.iter().filter_map(|item| item.mal_id).collect();

    let filtered_count = mal_items
        .iter()
        .filter(|item| item.mal_id.is_some_and(|id| shiki_mal_ids.contains(&id)))
        .count();

    if filtered_count > 0 {
        tracing::info!(
            filtered = filtered_count,
            "deduplicated MAL results already present in Shikimori"
        );
    }

    for item in mal_items.into_iter() {
        if item.mal_id.is_none_or(|id| !shiki_mal_ids.contains(&id)) {
            shiki_items.push(item);
        }
    }

    let merged = deduplicate_by_title(shiki_items);
    if !merged.is_empty() {
        return merged;
    }

    // Lazy last-resort fallback: only queried when Shikimori + MAL both came
    // back empty. Mirrors the manga-family `first_non_empty` laziness.
    match state.anilist.search(query, "anime").await {
        Ok(items) => deduplicate_by_title(normalize_media_type(items, "anime")),
        Err(e) => {
            tracing::warn!(provider = "anilist", error = %e, "anime fallback failed");
            Vec::new()
        }
    }
}

pub async fn manga(state: &AppState, query: &str) -> Vec<CreateMediaItem> {
    manga_family_search(state, query, &["Manga"], "manga").await
}

pub async fn manhwa(state: &AppState, query: &str) -> Vec<CreateMediaItem> {
    manga_family_search(state, query, &["Manhwa"], "manhwa").await
}

pub async fn manhua(state: &AppState, query: &str) -> Vec<CreateMediaItem> {
    manga_family_search(state, query, &["Manhua"], "manhua").await
}

pub async fn novel(state: &AppState, query: &str) -> Vec<CreateMediaItem> {
    manga_family_search(state, query, &["Novel"], "novel").await
}

pub async fn other_comics(state: &AppState, query: &str) -> Vec<CreateMediaItem> {
    manga_family_search(
        state,
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
        "other-comics",
    )
    .await
}

/// Комиксы (западные) → ComicVine. Без ключа warn + пусто.
pub async fn comic(state: &AppState, query: &str) -> Vec<CreateMediaItem> {
    if !state.comicvine.is_configured() {
        tracing::warn!("COMIC_VINE_API_KEY not set, comic search skipped");
        return Vec::new();
    }
    let mut out = Vec::new();
    extend(&mut out, state.comicvine.search(query).await, "comicvine");
    deduplicate_by_title(normalize_media_type(out, "comic"))
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
        tracing::warn!("TMDB_API_KEY not set, dramas search skipped");
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
        tracing::warn!("TMDB_API_KEY not set, cartoons search skipped");
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
        tracing::warn!("TMDB_API_KEY not set, animated movies search skipped");
        return out;
    }
    extend(
        &mut out,
        state.tmdb.search_movies(query, Some(16)).await,
        "tmdb",
    );
    out
}

/// Игры → RAWG + IGDB.
pub async fn game(state: &AppState, query: &str) -> Vec<CreateMediaItem> {
    let mut has_rawg = false;
    let mut has_igdb = false;

    if !state.rawg.api_key.is_empty() {
        has_rawg = true;
    }
    if state.igdb.is_configured() {
        has_igdb = true;
    }

    if !has_rawg && !has_igdb {
        tracing::warn!("No game providers configured (RAWG_API_KEY / IGDB_CLIENT_ID+SECRET)");
        return Vec::new();
    }

    let (rawg_res, igdb_res) = tokio::join!(
        async {
            if has_rawg {
                state.rawg.search(query).await
            } else {
                Ok(Vec::new())
            }
        },
        async {
            if has_igdb {
                state.igdb.search(query).await
            } else {
                Ok(Vec::new())
            }
        },
    );

    let mut items = Vec::new();
    extend(&mut items, rawg_res, "rawg");
    extend(&mut items, igdb_res, "igdb");
    deduplicate_by_title(items)
}

/// Книги → Hardcover + Google Books + Open Library.
pub async fn book(state: &AppState, query: &str) -> Vec<CreateMediaItem> {
    let (hc_res, gb_res, ol_res) = tokio::join!(
        state.hardcover.search(query),
        state.google_books.search(query),
        state.openlibrary.search(query),
    );

    let mut items = Vec::new();
    extend(&mut items, hc_res, "hardcover");
    extend(&mut items, gb_res, "google_books");
    extend(&mut items, ol_res, "openlibrary");
    deduplicate_by_title(items)
}

/// Без выбранного типа — срез по всем категориям (по одному запросу на группу провайдеров).
pub async fn all_types(state: &AppState, query: &str) -> Vec<CreateMediaItem> {
    let (anime_r, manga_r, movie_r, series_r, game_r, book_r, comic_r) = tokio::join!(
        anime(state, query),
        state.mangaupdates.search(query),
        movie(state, query),
        series(state, query),
        game(state, query),
        book(state, query),
        comic(state, query),
    );

    let mut out = anime_r;
    extend(&mut out, manga_r, "mangaupdates");
    out.extend(movie_r);
    out.extend(series_r);
    out.extend(game_r);
    out.extend(book_r);
    out.extend(comic_r);
    deduplicate_by_title(out)
}

pub async fn by_media_type(
    state: &AppState,
    query: &str,
    search_type: &str,
) -> Vec<CreateMediaItem> {
    match search_type {
        "anime" => anime(state, query).await,
        "manga" => manga(state, query).await,
        "manhwa" => manhwa(state, query).await,
        "manhua" => manhua(state, query).await,
        "novel" => novel(state, query).await,
        "other-comics" => other_comics(state, query).await,
        "comic" => comic(state, query).await,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn item(provider: &str, media_type: &str, key: &str) -> CreateMediaItem {
        CreateMediaItem {
            title: key.to_string(),
            provider: provider.to_string(),
            comparison_key: Some(key.to_string()),
            media_type: media_type.to_string(),
            ..Default::default()
        }
    }

    fn boxed<'a>(
        fut: impl std::future::Future<Output = Result<Vec<CreateMediaItem>, anyhow::Error>> + Send + 'a,
    ) -> SearchFuture<'a> {
        Box::pin(fut)
    }

    #[tokio::test]
    async fn first_non_empty_returns_first_non_empty() {
        let attempts = vec![
            ("a", boxed(async { Ok(vec![item("a", "manga", "one")]) })),
            ("b", boxed(async { Ok(vec![item("b", "manga", "two")]) })),
        ];
        let out = first_non_empty(attempts).await;
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].title, "one");
    }

    #[tokio::test]
    async fn first_non_empty_skips_empty_then_returns() {
        let attempts = vec![
            ("a", boxed(async { Ok(Vec::new()) })),
            ("b", boxed(async { Err(anyhow::anyhow!("boom")) })),
            ("c", boxed(async { Ok(vec![item("c", "manga", "three")]) })),
        ];
        let out = first_non_empty(attempts).await;
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].title, "three");
    }

    #[tokio::test]
    async fn first_non_empty_all_empty_or_err() {
        let attempts = vec![
            ("a", boxed(async { Ok(Vec::new()) })),
            ("b", boxed(async { Err(anyhow::anyhow!("boom")) })),
        ];
        assert!(first_non_empty(attempts).await.is_empty());
    }

    #[tokio::test]
    async fn first_non_empty_does_not_poll_providers_after_winner() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let polled = Arc::new(AtomicUsize::new(0));
        let polled_later = Arc::clone(&polled);
        let attempts = vec![
            ("a", boxed(async { Ok(vec![item("a", "manga", "one")]) })),
            (
                "b",
                boxed(async move {
                    polled_later.fetch_add(1, Ordering::SeqCst);
                    Ok(vec![item("b", "manga", "two")])
                }),
            ),
        ];
        let out = first_non_empty(attempts).await;
        assert_eq!(out[0].title, "one");
        assert_eq!(
            polled.load(Ordering::SeqCst),
            0,
            "later providers must stay unpolled once a winner is found"
        );
    }

    #[test]
    fn normalize_media_type_overrides() {
        let items = vec![item("mangadex", "manga", "one")];
        let out = normalize_media_type(items, "manhwa");
        assert_eq!(out[0].media_type, "manhwa");
        assert_eq!(out[0].provider, "mangadex");
        assert_eq!(out[0].external_id, "");
    }

    #[test]
    fn deduplicate_keeps_higher_priority() {
        let items = vec![
            item("mangadex", "manga", "same"),
            item("mangaupdates", "manga", "same"),
        ];
        let out = deduplicate_by_title(items);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].provider, "mangaupdates");
    }

    #[test]
    fn deduplicate_prefers_hardcover() {
        let items = vec![
            item("google_books", "book", "same"),
            item("hardcover", "book", "same"),
        ];
        let out = deduplicate_by_title(items);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].provider, "hardcover");
    }

    #[test]
    fn provider_priority_anime_order() {
        assert_eq!(provider_priority("anime", "mal"), 0);
        assert_eq!(provider_priority("anime", "shikimori"), 1);
        assert_eq!(provider_priority("anime", "anilist"), 2);
        assert_eq!(provider_priority("anime", "tmdb"), 10);
    }

    #[test]
    fn provider_priority_manga_anilist_is_last() {
        assert_eq!(provider_priority("manga", "anilist"), 4);
    }

    #[test]
    fn deduplicate_prefers_shikimori_over_anilist_for_anime() {
        let items = vec![
            item("anilist", "anime", "same"),
            item("shikimori", "anime", "same"),
        ];
        let out = deduplicate_by_title(items);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].provider, "shikimori");
    }
}
