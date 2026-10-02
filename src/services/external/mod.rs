pub mod anilist;
pub mod chapter_count;
pub mod comicvine;
pub mod google_books;
pub mod hardcover;
pub mod igdb;
pub mod mal;
pub mod mangadex;
pub mod mangaupdates;
pub mod official_kakao;
pub mod official_kuaikan;
pub mod official_mangaplus;
pub mod openlibrary;
pub mod rawg;
pub mod shikimori;
pub mod tmdb;

pub mod dispatch;

use anyhow::Error as AnyhowError;
use std::time::Duration;

/// Shared outbound HTTP client used by every external provider.
///
/// `connect_timeout` bounds TCP/TLS setup, `timeout` bounds the whole
/// request, so a hung third-party API can no longer pin an app worker
/// forever. The User-Agent identifies us to providers.
pub fn http_client() -> reqwest::Client {
    match reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(20))
        .user_agent(concat!("mediatracker/", env!("CARGO_PKG_VERSION")))
        .build()
    {
        Ok(client) => client,
        Err(e) => {
            tracing::error!(error = %e, "failed to build HTTP client; using default");
            reqwest::Client::new()
        }
    }
}

/// Marker error: the provider explicitly reported the entity as missing
/// (HTTP 404, or a structured "not found" payload). It carries the provider
/// message so logs stay useful, but the card layer only needs to know the
/// *class* of failure to pick the right fallback badge text.
#[derive(Debug, thiserror::Error)]
#[error("not found: {0}")]
pub struct NotFoundError(pub String);

/// True when an `anyhow` chain was produced by [`NotFoundError`].
///
/// Searches the whole error chain, so the marker is still detected when the
/// provider error was later wrapped with `.context(...)`.
pub fn is_not_found(e: &AnyhowError) -> bool {
    e.chain()
        .any(|c| c.downcast_ref::<NotFoundError>().is_some())
}

/// One DLC / expansion ("addition") for a game, normalized across providers.
///
/// `kind` is one of `"dlc"`, `"expansion"` or `"addition"` (RAWG has a single
/// undifferentiated additions list; IGDB distinguishes dlcs vs expansions).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdditionRow {
    pub addition_external_id: String,
    pub name: String,
    pub kind: String,
    pub released: Option<chrono::NaiveDate>,
}

impl AdditionRow {
    /// Human-readable label for the UI badge.
    pub fn kind_label(&self) -> &'static str {
        match self.kind.as_str() {
            "dlc" => "DLC",
            "expansion" => "Расширение",
            _ => "Дополнение",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_client_builds() {
        // The builder must succeed so every provider gets bounded timeouts.
        let client = http_client();
        drop(client);
    }

    #[test]
    fn addition_row_kind_labels() {
        let make = |kind: &str| AdditionRow {
            addition_external_id: "1".to_string(),
            name: "X".to_string(),
            kind: kind.to_string(),
            released: None,
        };
        assert_eq!(make("dlc").kind_label(), "DLC");
        assert_eq!(make("expansion").kind_label(), "Расширение");
        assert_eq!(make("addition").kind_label(), "Дополнение");
        assert_eq!(make("weird").kind_label(), "Дополнение");
    }

    #[test]
    fn is_not_found_detects_marker_error() {
        let err: anyhow::Error = NotFoundError("Shikimori 42".to_string()).into();
        assert!(is_not_found(&err));
    }

    #[test]
    fn is_not_found_rejects_other_errors() {
        let err = anyhow::anyhow!("connection refused");
        assert!(!is_not_found(&err));
        let wrapped = err.context("while fetching details");
        assert!(!is_not_found(&wrapped));
    }

    #[test]
    fn is_not_found_detects_marker_through_context() {
        let err: anyhow::Error = NotFoundError("Shikimori 42".to_string()).into();
        let wrapped = err.context("while fetching details");
        assert!(is_not_found(&wrapped));
    }
}
