//! MangaPlus (JP) chapter-count source.
//!
//! Thin adapter over [`crate::services::external::official_mangaplus`]: it owns
//! the per-provider device token and delegates registration plus the
//! `title_detailV3` decode to [`MangaPlusClient`]. Raise-only: any failure or
//! parse gap returns `Ok(None)` so the count is never lowered.

use super::{ChapterCount, ChapterCountFuture, ChapterCountProvider};
use crate::services::external::official_mangaplus::{MangaPlusClient, md5_hex};

pub struct MangaPlusProvider {
    client: MangaPlusClient,
    device_token: String,
}

impl MangaPlusProvider {
    pub fn new() -> Self {
        let seed = format!(
            "{}:{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
            std::process::id()
        );
        Self {
            client: MangaPlusClient::new(),
            device_token: md5_hex(seed.as_bytes()),
        }
    }
}

impl Default for MangaPlusProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl ChapterCountProvider for MangaPlusProvider {
    fn name(&self) -> &'static str {
        "mangaplus"
    }

    fn fetch<'a>(&'a self, external_id: &'a str) -> ChapterCountFuture<'a> {
        Box::pin(async move {
            let secret = match self.client.register(&self.device_token).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::debug!(error = %e, "mangaplus register failed");
                    return Ok(None);
                }
            };
            match self.client.title_chapter_count(&secret, external_id).await {
                Ok(Some(total)) => Ok(Some(ChapterCount {
                    total,
                    source: "auto:mangaplus",
                })),
                Ok(None) => Ok(None),
                Err(e) => {
                    tracing::debug!(error = %e, "mangaplus detail failed");
                    Ok(None)
                }
            }
        })
    }

    fn media_types(&self) -> &'static [&'static str] {
        &["manga"]
    }
}

#[cfg(test)]
mod tests {
    use super::md5_hex;

    #[test]
    fn md5_hex_matches_rfc_vectors() {
        assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(md5_hex(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
    }
}
