//! MangaPlus (JP) chapter-count source.
//!
//! Thin adapter over [`crate::services::external::official_mangaplus`]: it
//! reuses the shared registration cache and the `title_detailV3` decode, so the
//! count path never registers upstream on its own. Raise-only: any failure or
//! parse gap returns `Ok(None)` so the count is never lowered.

use super::{ChapterCount, ChapterCountFuture, ChapterCountProvider};
use crate::services::external::official_mangaplus::{
    cached_secret, parse_chapter_count, title_detail_bytes,
};

pub struct MangaPlusProvider;

impl MangaPlusProvider {
    pub fn new() -> Self {
        Self
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
            let client = crate::services::external::http_client();
            let secret = match cached_secret(&client).await {
                Ok(secret) => secret,
                Err(e) => {
                    tracing::debug!(error = %e, "mangaplus register failed");
                    return Ok(None);
                }
            };
            let body = match title_detail_bytes(&client, &secret, external_id).await {
                Ok(body) => body,
                Err(e) => {
                    tracing::debug!(error = %e, "mangaplus detail failed");
                    return Ok(None);
                }
            };
            Ok(parse_chapter_count(&body).map(|total| ChapterCount {
                total,
                source: "auto:mangaplus",
            }))
        })
    }

    fn media_types(&self) -> &'static [&'static str] {
        &["manga"]
    }
}
