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

/// One official hit as a search result. Rich fields (poster, description,
/// author, status, genres, year) land here as the sources expose them.
fn hit_to_item(source: &str, media_type: &str, hit: OfficialHit) -> CreateMediaItem {
    CreateMediaItem {
        provider: source.to_string(),
        external_id: hit.id,
        media_type: media_type.to_string(),
        title: hit.title,
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
        assert_eq!(latin, vec!["ridibooks", "tappytoon", "webtoons"]);
        assert!(!latin.contains(&"kakao"));
        assert!(!latin.contains(&"naverwebtoon"));
        assert!(!latin.contains(&"naver"));

        // A Hangul query: the KR catalogs answer, the English ones don't.
        let hangul = gated_sources("manhwa", "여신강림");
        assert_eq!(hangul, vec!["kakao", "naverwebtoon", "naver", "ridibooks"]);

        // A JP query reaches every JP-first catalog of the chain.
        let jp = gated_sources("manga", "僕の心のヤバイやつ");
        assert_eq!(
            jp,
            vec![
                "mangaplus",
                "comicwalker",
                "mangaup",
                "comici",
                "alphapolis"
            ]
        );

        // Unknown types have no sources.
        assert!(gated_sources("anime", "Naruto").is_empty());
        assert!(gated_sources("", "anything").is_empty());
    }

    #[test]
    fn hit_to_item_carries_source_and_type() {
        let hit = OfficialHit {
            id: "1320".to_string(),
            title: "Lore Olympus".to_string(),
        };
        let item = hit_to_item("webtoons", "other-comics", hit);
        assert_eq!(item.provider, "webtoons");
        assert_eq!(item.external_id, "1320");
        assert_eq!(item.media_type, "other-comics");
        assert_eq!(item.title, "Lore Olympus");
    }
}
