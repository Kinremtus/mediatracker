//! Official sources as first-class search providers for the local catalog.
//!
//! `/search` fans out over every official source of the selected media type
//! (the same chains the resolve walk uses, but ALL of them — not first-match),
//! in parallel, script-gated by the query: a Latin query never reaches the
//! Hangul-only catalogs and vice versa. The hits become `CreateMediaItem`s
//! that the catalog stores and the grid renders; adding one binds it directly
//! to its source (no MangaUpdates intermediary).
//!
//! The provider chains (`services::search`) are the fallback: they are only
//! consulted when the official sources of the type found nothing.

use crate::app_state::AppState;
use crate::models::media_item::CreateMediaItem;
use crate::services::official_resolve;
use crate::services::official_title;
use crate::services::official_types::OfficialHit;

/// The sources of `media_type` whose search script accepts the query (the
/// same gate the resolve walk applies: `pick_candidate`).
pub fn gated_sources(media_type: &str, query: &str) -> Vec<&'static str> {
    let candidates = [query.to_string()];
    official_resolve::official_source_for_media_type(media_type)
        .iter()
        .copied()
        .filter(|source| official_title::pick_candidate(&candidates, source).is_some())
        .collect()
}

/// Search every gated official source of the media type in parallel and map
/// the hits onto `CreateMediaItem`s. A failed source only drops its own hits;
/// an empty result means "no official match" (the caller then falls back to
/// the provider chains).
pub async fn search(state: &AppState, media_type: &str, query: &str) -> Vec<CreateMediaItem> {
    let sources = gated_sources(media_type, query);
    let mut set = tokio::task::JoinSet::new();
    for source in sources {
        let client = state.http_client.clone();
        let query = query.to_string();
        set.spawn(async move {
            let hits = official_resolve::search_source(&client, source, &query).await;
            (source, hits)
        });
    }
    let mut items = Vec::new();
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok((source, Ok(hits))) => {
                items.extend(
                    hits.into_iter()
                        .map(|hit| hit_to_item(source, media_type, hit)),
                );
            }
            Ok((source, Err(e))) => {
                tracing::warn!(source, error = %e, "official search: source failed");
            }
            Err(e) => tracing::warn!(error = %e, "official search: task failed"),
        }
    }
    items
}

/// One official hit as a search result: the id/title plus every rich field
/// the source's search answer exposed.
fn hit_to_item(source: &str, media_type: &str, hit: OfficialHit) -> CreateMediaItem {
    let extra = hit.extra;
    CreateMediaItem {
        provider: source.to_string(),
        external_id: hit.id,
        media_type: media_type.to_string(),
        title: hit.title,
        // Some publisher CDNs refuse hotlinks: the cover goes through the
        // `/src-image` proxy (see `services::source_images`).
        poster_url: extra
            .poster_url
            .map(|url| crate::services::source_images::proxied(&url)),
        description: extra.description,
        status: extra.status,
        genres: extra.genres,
        year: extra.year,
        score: extra.score,
        chapters: extra.chapters,
        authors: extra.author.into_iter().collect(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gated_sources_respect_scripts_and_chains() {
        // A Latin query: Hangul-only and Han-only catalogs are skipped.
        let latin = gated_sources("manhwa", "Lore Olympus");
        assert_eq!(latin, vec!["ridibooks", "tappytoon", "webtoons", "toomics"]);
        assert!(!latin.contains(&"kakao"));
        assert!(!latin.contains(&"naverwebtoon"));
        assert!(!latin.contains(&"naver"));
        assert!(!latin.contains(&"bomtoon"));

        // A Hangul query: the KR catalogs answer, the English ones don't.
        let hangul = gated_sources("manhwa", "여신강림");
        assert_eq!(
            hangul,
            vec!["kakao", "naverwebtoon", "naver", "ridibooks", "bomtoon"]
        );

        // A JP query reaches every JP-first catalog of the chain.
        let jp = gated_sources("manga", "僕の心のヤバイやつ");
        assert_eq!(
            jp,
            vec![
                "mangaplus",
                "comicwalker",
                "mangaup",
                "comici",
                "alphapolis",
                "sundaywebry",
                "yanmaga"
            ]
        );

        // Unknown types have no sources.
        assert!(gated_sources("anime", "Naruto").is_empty());
        assert!(gated_sources("", "anything").is_empty());
    }

    #[test]
    fn hit_to_item_carries_source_type_and_rich_fields() {
        let hit = OfficialHit::with_extra(
            "1320",
            "Lore Olympus",
            crate::services::official_types::HitExtra {
                poster_url: Some("https://example.test/1320.jpg".to_string()),
                description: Some("a retelling".to_string()),
                author: Some("Rachel Smythe".to_string()),
                status: Some("ongoing".to_string()),
                genres: vec!["Fantasy".to_string()],
                year: Some(2018),
                score: Some(8.9),
                chapters: Some(261),
            },
        );
        let item = hit_to_item("webtoons", "other-comics", hit);
        assert_eq!(item.provider, "webtoons");
        assert_eq!(item.external_id, "1320");
        assert_eq!(item.media_type, "other-comics");
        assert_eq!(item.title, "Lore Olympus");
        assert_eq!(
            item.poster_url.as_deref(),
            Some("https://example.test/1320.jpg")
        );
        assert_eq!(item.description.as_deref(), Some("a retelling"));
        assert_eq!(item.authors, vec!["Rachel Smythe".to_string()]);
        assert_eq!(item.status.as_deref(), Some("ongoing"));
        assert_eq!(item.genres, vec!["Fantasy".to_string()]);
        assert_eq!(item.year, Some(2018));
        assert_eq!(item.score, Some(8.9));
        assert_eq!(item.chapters, Some(261));
    }
}
