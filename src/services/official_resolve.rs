//! Auto-resolve an official source for a media item and bind it on an exact
//! normalized-title match. At most one search request per call; an existing
//! binding short-circuits before any network I/O.

use std::future::Future;
use std::pin::Pin;

use crate::services::official_title;
use crate::services::official_types::OfficialHit;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveOutcome {
    pub source: String,
    pub source_id: String,
}

/// Ordered official sources for a media type, highest priority first.
/// All wired sources are auto-bind; order encodes priority (quota-limited or
/// expensive sources last). `narou` is intentionally absent: it reuses `syosetu`.
pub fn official_source_for_media_type(media_type: &str) -> &'static [&'static str] {
    match media_type {
        // kuaikan is existing/cheap -> first; bilibili is quota-limited -> last.
        "manhua" => &["kuaikan", "bilibili"],
        // kakao/naver existing; daum is the new bindable source; ridibooks last.
        "manhwa" => &["kakao", "naver", "daum", "ridibooks"],
        // mangaplus existing; comicwalker/mangaup new; alphapolis last.
        "manga" => &["mangaplus", "comicwalker", "mangaup", "alphapolis"],
        // syosetu existing; kakuyomu new JP; qidian CN; ridibooks/alphapolis last.
        "novel" => &["syosetu", "kakuyomu", "qidian", "ridibooks", "alphapolis"],
        _ => &[],
    }
}

/// Network abstraction so tests can inject a fixture-backed searcher.
pub trait OfficialSearch: Send + Sync {
    fn search<'a>(
        &'a self,
        source: &'a str,
        query: &'a str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<OfficialHit>>> + Send + 'a>>;
}

pub struct HttpOfficialSearch {
    client: reqwest::Client,
    mangaplus: crate::services::external::official_mangaplus::MangaPlusClient,
}

impl HttpOfficialSearch {
    pub fn new() -> Self {
        Self {
            client: crate::services::external::http_client(),
            mangaplus: crate::services::external::official_mangaplus::MangaPlusClient::new(),
        }
    }
}

impl Default for HttpOfficialSearch {
    fn default() -> Self {
        Self::new()
    }
}

impl OfficialSearch for HttpOfficialSearch {
    fn search<'a>(
        &'a self,
        source: &'a str,
        query: &'a str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<OfficialHit>>> + Send + 'a>> {
        Box::pin(async move {
            match source {
                "kuaikan" => {
                    crate::services::external::official_kuaikan::search(&self.client, query).await
                }
                "kakao" => {
                    crate::services::external::official_kakao::search(&self.client, query).await
                }
                "naver" => {
                    crate::services::external::official_naver::search(&self.client, query).await
                }
                "mangaplus" => {
                    let catalog = self.mangaplus.catalog().await?;
                    let mut hits: Vec<OfficialHit> = catalog
                        .iter()
                        .filter(|h| official_title::titles_match(&h.title, query))
                        .cloned()
                        .collect();
                    if hits.is_empty() {
                        let q = official_title::normalize_title(query);
                        if !q.is_empty() {
                            hits = catalog
                                .iter()
                                .filter(|h| official_title::normalize_title(&h.title).contains(&q))
                                .cloned()
                                .collect();
                        }
                    }
                    Ok(hits)
                }
                "comicwalker" => {
                    crate::services::external::official_comicwalker::search(&self.client, query).await
                }
                "kakuyomu" => {
                    crate::services::external::official_kakuyomu::search(&self.client, query).await
                }
                "alphapolis" => {
                    crate::services::external::official_alphapolis::search(&self.client, query).await
                }
                // Bilibili has a tiny guest quota: use the cached/cooldown path.
                "bilibili" => {
                    crate::services::external::official_bilibili::search_cached(&self.client, query)
                        .await
                }
                "daum" => {
                    crate::services::external::official_daum::search(&self.client, query).await
                }
                "qidian" => {
                    crate::services::external::official_qidian::search(&self.client, query).await
                }
                "ridibooks" => {
                    crate::services::external::official_ridibooks::search(&self.client, query).await
                }
                "mangaup" => {
                    crate::services::external::official_mangaup::search(&self.client, query).await
                }
                _ => Ok(Vec::new()),
            }
        })
    }
}

/// Alphapolis search returns both novel and manga hits in one list, tagged by an
/// id prefix (`novel/...` vs `manga/official/...`). Keep only ids that match the
/// item's `media_type`; every other source (and unknown media type) is unaffected
/// where it matters. Pure + total: never panics.
fn hit_matches_media_type(source: &str, id: &str, media_type: &str) -> bool {
    if source != "alphapolis" {
        return true;
    }
    match media_type {
        "novel" => id.starts_with("novel/"),
        "manga" => id.starts_with("manga/official/"),
        _ => false,
    }
}

/// Resolve + persist across the ordered source list. Returns the outcome on the
/// first exact normalized-title match, None otherwise. Zero network I/O when any
/// source already has a binding; one search per source until the first match.
pub async fn resolve_and_bind(
    db: &sqlx::PgPool,
    search: &dyn OfficialSearch,
    provider: &str,
    external_id: &str,
    media_type: &str,
    title: &str,
    candidates: &[String],
) -> Option<ResolveOutcome> {
    let sources = official_source_for_media_type(media_type);
    if sources.is_empty() {
        return None;
    }

    // Short-circuit before any network I/O: the first source (list order =
    // priority) that already has a binding wins.
    for &source in sources {
        match crate::services::source_ids::get_source_id(db, provider, external_id, source).await {
            Ok(Some(id)) => {
                return Some(ResolveOutcome {
                    source: source.to_string(),
                    source_id: id,
                });
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(provider, external_id, source, error = %e, "official_resolve: binding lookup failed");
                return None;
            }
        }
    }

    // Candidate list: title first, then associated titles, deduped in order.
    let mut all: Vec<String> = Vec::with_capacity(candidates.len() + 1);
    all.push(title.to_string());
    for c in candidates {
        if !c.trim().is_empty() && !all.iter().any(|x| x == c) {
            all.push(c.clone());
        }
    }

    // Walk the sources in priority order; a failed source only skips itself.
    for &source in sources {
        let Some(query) = official_title::pick_candidate(&all, source) else {
            continue;
        };
        let hits = match search.search(source, query).await {
            Ok(hits) => hits,
            Err(e) => {
                tracing::warn!(provider, external_id, source, error = %e, "official_resolve: search failed");
                continue;
            }
        };
        let Some(hit) = hits
            .into_iter()
            .filter(|h| hit_matches_media_type(source, &h.id, media_type))
            .find(|h| all.iter().any(|c| official_title::titles_match(&h.title, c)))
        else {
            continue;
        };
        match crate::services::source_ids::set_source_id(db, provider, external_id, source, &hit.id)
            .await
        {
            Ok(()) => {
                return Some(ResolveOutcome {
                    source: source.to_string(),
                    source_id: hit.id,
                });
            }
            Err(e) => {
                tracing::warn!(provider, external_id, source, error = %e, "official_resolve: bind failed");
                continue;
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_media_types_to_sources() {
        // bilibili is last in `manhua` (guest quota), others by priority.
        assert_eq!(
            official_source_for_media_type("manhua"),
            &["kuaikan", "bilibili"][..]
        );
        assert_eq!(
            official_source_for_media_type("manhwa"),
            &["kakao", "naver", "daum", "ridibooks"][..]
        );
        assert_eq!(
            official_source_for_media_type("manga"),
            &["mangaplus", "comicwalker", "mangaup", "alphapolis"][..]
        );
        assert_eq!(
            official_source_for_media_type("novel"),
            &["syosetu", "kakuyomu", "qidian", "ridibooks", "alphapolis"][..]
        );
    }

    #[test]
    fn rejects_non_official_media_types() {
        assert!(official_source_for_media_type("comic").is_empty());
        assert!(official_source_for_media_type("other-comics").is_empty());
        assert!(official_source_for_media_type("anime").is_empty());
        assert!(official_source_for_media_type("").is_empty());
    }

    #[test]
    fn alphapolis_hits_are_filtered_by_media_type_prefix() {
        // novel item keeps `novel/...` and drops manga hits.
        assert!(hit_matches_media_type("alphapolis", "novel/381661058", "novel"));
        assert!(!hit_matches_media_type(
            "alphapolis",
            "manga/official/407000825",
            "novel"
        ));
        // manga item keeps `manga/official/...` and drops novel hits.
        assert!(hit_matches_media_type(
            "alphapolis",
            "manga/official/407000825",
            "manga"
        ));
        assert!(!hit_matches_media_type(
            "alphapolis",
            "novel/381661058",
            "manga"
        ));
        // Unknown media type drops everything (never panics).
        assert!(!hit_matches_media_type("alphapolis", "novel/1", "other"));
        // Non-alphapolis sources are untouched.
        assert!(hit_matches_media_type("kuaikan", "12345", "manhua"));
    }

    #[test]
    fn titles_match_exact_and_reject_prefixes() {
        assert!(official_title::titles_match("One Piece", "one-piece"));
        assert!(!official_title::titles_match(
            "One Piece",
            "One Piece Party"
        ));
    }

    #[test]
    fn pick_candidate_selects_provider_script() {
        let candidates = ["One Piece".to_string(), "원피스".to_string()];
        assert_eq!(
            official_title::pick_candidate(&candidates, "kakao"),
            Some("원피스")
        );
        assert_eq!(
            official_title::pick_candidate(&candidates, "naver"),
            Some("원피스")
        );
    }
}
