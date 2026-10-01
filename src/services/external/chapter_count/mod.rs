//! Raise-only external chapter-count sources (design Layer 2).
//!
//! A provider reports the *original-language* total chapter count for a media
//! item. Callers must only ever RAISE `media_items.chapters` from this value —
//! a transient parse failure returning a low number must never wipe progress.
//!
//! Async without `async_trait`: mirrors the `SearchFuture` pattern in
//! `services::search` (boxed `Pin<Future>`), already used in this codebase.

pub mod kakao;
pub mod mangaplus;
pub mod syosetu;

use std::future::Future;
use std::pin::Pin;

/// One external chapter-count reading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChapterCount {
    /// Total chapters as reported by the source (must be > 0 to be used).
    pub total: i32,
    /// Provenance written to `media_items.chapters_source`, e.g. `"auto:kakao"`.
    pub source: &'static str,
}

/// Boxed future returned by [`ChapterCountProvider::fetch`].
pub type ChapterCountFuture<'a> =
    Pin<Box<dyn Future<Output = anyhow::Result<Option<ChapterCount>>> + Send + 'a>>;

/// A raise-only source of the original-language chapter count.
pub trait ChapterCountProvider: Send + Sync {
    /// Stable name, e.g. `"kakao"` (matches the `auto:<name>` source suffix).
    fn name(&self) -> &'static str;

    /// Resolve a count for `external_id`. `Ok(None)` means "not found / not
    /// applicable"; `Err` means transient failure. Both must be treated as a
    /// no-op by callers.
    fn fetch<'a>(&'a self, external_id: &'a str) -> ChapterCountFuture<'a>;

    /// Media types this provider can enrich (e.g. `&["manhwa"]`).
    fn media_types(&self) -> &'static [&'static str];
}

/// Keep only readings that are safe to apply: non-zero, non-negative.
pub fn usable(count: Option<ChapterCount>) -> Option<ChapterCount> {
    count.filter(|c| c.total > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usable_rejects_none_zero_and_negative() {
        assert_eq!(usable(None), None);
        assert_eq!(
            usable(Some(ChapterCount {
                total: 0,
                source: "auto:kakao"
            })),
            None
        );
        assert_eq!(
            usable(Some(ChapterCount {
                total: -5,
                source: "auto:kakao"
            })),
            None
        );
        assert_eq!(
            usable(Some(ChapterCount {
                total: 157,
                source: "auto:kakao"
            })),
            Some(ChapterCount {
                total: 157,
                source: "auto:kakao"
            })
        );
    }
}

use std::sync::Arc;

/// Providers matching a media type, in priority order. Only raise-only sources
/// belong here; MangaUpdates stays the skeleton owner and is never a "source".
pub fn providers_for(media_type: &str) -> Vec<Arc<dyn ChapterCountProvider>> {
    let all: Vec<Arc<dyn ChapterCountProvider>> = vec![
        Arc::new(kakao::KakaoProvider::new()),
        Arc::new(mangaplus::MangaPlusProvider::new()),
        Arc::new(syosetu::SyosetuProvider::new()),
    ];
    all.into_iter()
        .filter(|p| p.media_types().contains(&media_type))
        .collect()
}

#[cfg(test)]
mod registry_tests {
    use super::*;

    #[test]
    fn providers_for_filters_by_media_type() {
        let manhwa = providers_for("manhwa");
        assert!(manhwa.iter().any(|p| p.name() == "kakao"));
        assert!(!manhwa.iter().any(|p| p.name() == "syosetu"));
        assert!(providers_for("novel").iter().any(|p| p.name() == "syosetu"));
        assert!(
            providers_for("manga")
                .iter()
                .any(|p| p.name() == "mangaplus")
        );
    }
}
