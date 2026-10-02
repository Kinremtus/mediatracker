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

/// manhua -> kuaikan, manhwa -> kakao, manga -> mangaplus; else None.
pub fn official_source_for_media_type(media_type: &str) -> Option<&'static str> {
    match media_type {
        "manhua" => Some("kuaikan"),
        "manhwa" => Some("kakao"),
        "manga" => Some("mangaplus"),
        _ => None,
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
                _ => Ok(Vec::new()),
            }
        })
    }
}

/// Resolve + persist. Returns the outcome on exact match, None otherwise.
pub async fn resolve_and_bind(
    db: &sqlx::PgPool,
    search: &dyn OfficialSearch,
    provider: &str,
    external_id: &str,
    media_type: &str,
    title: &str,
    candidates: &[String],
) -> Option<ResolveOutcome> {
    let source = official_source_for_media_type(media_type)?;

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

    let mut all: Vec<String> = Vec::with_capacity(candidates.len() + 1);
    all.push(title.to_string());
    for c in candidates {
        if !c.trim().is_empty() && !all.iter().any(|x| x == c) {
            all.push(c.clone());
        }
    }
    let query = official_title::pick_candidate(&all, source)?;

    let hits = match search.search(source, query).await {
        Ok(hits) => hits,
        Err(e) => {
            tracing::warn!(provider, external_id, source, error = %e, "official_resolve: search failed");
            return None;
        }
    };

    let hit = hits.into_iter().find(|h| {
        all.iter()
            .any(|c| official_title::titles_match(&h.title, c))
    })?;

    match crate::services::source_ids::set_source_id(db, provider, external_id, source, &hit.id)
        .await
    {
        Ok(()) => Some(ResolveOutcome {
            source: source.to_string(),
            source_id: hit.id,
        }),
        Err(e) => {
            tracing::warn!(provider, external_id, source, error = %e, "official_resolve: bind failed");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_media_types_to_sources() {
        assert_eq!(official_source_for_media_type("manhua"), Some("kuaikan"));
        assert_eq!(official_source_for_media_type("manhwa"), Some("kakao"));
        assert_eq!(official_source_for_media_type("manga"), Some("mangaplus"));
    }

    #[test]
    fn rejects_non_official_media_types() {
        assert_eq!(official_source_for_media_type("novel"), None);
        assert_eq!(official_source_for_media_type("comic"), None);
        assert_eq!(official_source_for_media_type("other-comics"), None);
        assert_eq!(official_source_for_media_type("anime"), None);
        assert_eq!(official_source_for_media_type(""), None);
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
    }
}
