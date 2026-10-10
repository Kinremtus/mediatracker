//! Auto-resolve an official source for a media item and bind it on an exact
//! normalized-title match. The walk issues one search per source until the
//! first match; a meta-capable binding short-circuits before any network I/O,
//! while a non-meta binding still lets the cached sources be tried.

use std::future::Future;
use std::pin::Pin;

use crate::services::external::official_meta::SOURCES_WITH_META;
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

/// Global fallback order used when the title has no `media_items` row yet
/// (drawer preview, chapters written before the item was added).
///
/// The first ten entries mirror the `supports_meta` global ranks of
/// `migrations/028_chapter_ownership.sql` (bilibili=1 ... qidian=10); kuaikan
/// gained its fetch arm after that migration, so it is appended last. The set
/// must match `SOURCES_WITH_META` (enforced by `tests/chapter_key.rs`).
pub const GLOBAL_SOURCE_ORDER: &[&str] = &[
    "bilibili",
    "naver",
    "daum",
    "ridibooks",
    "comicwalker",
    "mangaup",
    "alphapolis",
    "syosetu",
    "kakuyomu",
    "qidian",
    "kuaikan",
];

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
                    crate::services::external::official_comicwalker::search(&self.client, query)
                        .await
                }
                "kakuyomu" => {
                    crate::services::external::official_kakuyomu::search(&self.client, query).await
                }
                "alphapolis" => {
                    crate::services::external::official_alphapolis::search(&self.client, query)
                        .await
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

/// Sources whose search is backed by a module-local cache + cooldown
/// (`official_bilibili::search_cached`). Only these may be tried after a
/// non-meta match is already in hand: the walk must not spend extra live
/// requests on quota-limited or slow endpoints. Deliberately just bilibili —
/// it is the only cached meta source, so `manhwa`/`manga` never upgrade a
/// non-meta match (there is no cheap meta source to try).
const CACHED_SEARCH_SOURCES: &[&str] = &["bilibili"];

/// Resolve + persist across the ordered source list.
///
/// A meta-capable binding (a source whose chapter detail can actually be
/// fetched, [`SOURCES_WITH_META`]) wins; a match from a source without chapter
/// detail (kuaikan/kakao/mangaplus) is bound and returned only when no
/// meta-capable source can be found. Zero network I/O when a meta-capable
/// binding already exists; a non-meta binding alone still allows the walk to
/// try the cached sources (bilibili) so the chapters can move under a
/// meta-capable key. One search per source until the first match.
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

    // Existing bindings, in chain order. A meta-capable binding short-circuits
    // before any network I/O; a non-meta one is kept as the fallback while the
    // walk below still tries to upgrade to a meta-capable source.
    let mut bound_sources: Vec<&str> = Vec::new();
    let mut fallback: Option<ResolveOutcome> = None;
    for &source in sources {
        match crate::services::source_ids::get_source_id(db, provider, external_id, source).await {
            Ok(Some(id)) => {
                if SOURCES_WITH_META.contains(&source) {
                    return Some(ResolveOutcome {
                        source: source.to_string(),
                        source_id: id,
                    });
                }
                if fallback.is_none() {
                    fallback = Some(ResolveOutcome {
                        source: source.to_string(),
                        source_id: id,
                    });
                }
                bound_sources.push(source);
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

    // Where the chapter rows live right now, BEFORE any binding change: a
    // fresh meta-capable binding must carry them over (rekey) or they would be
    // orphaned under the old key.
    let old_key = chapter_key(db, provider, external_id).await;

    // Once a non-meta match is in hand, only cached sources may be searched: a
    // pre-existing non-meta binding also starts the walk in that mode.
    let mut only_cached = fallback.is_some();
    let mut fallback_from_search: Option<ResolveOutcome> = None;

    // Walk the sources in priority order; a failed source only skips itself.
    for &source in sources {
        if bound_sources.contains(&source) {
            continue;
        }
        if only_cached && !CACHED_SEARCH_SOURCES.contains(&source) {
            continue;
        }
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
            .find(|h| {
                all.iter()
                    .any(|c| official_title::titles_match(&h.title, c))
            })
        else {
            continue;
        };
        match crate::services::source_ids::set_source_id(db, provider, external_id, source, &hit.id)
            .await
        {
            Ok(()) => {
                let outcome = ResolveOutcome {
                    source: source.to_string(),
                    source_id: hit.id,
                };
                if SOURCES_WITH_META.contains(&source) {
                    // The winner: move the rows under the new key so the
                    // official chapters are not orphaned. A rekey failure only
                    // defers them — the next enrich re-seeds the key.
                    if let Err(e) = rekey_chapters(
                        db,
                        &old_key.0,
                        &old_key.1,
                        &outcome.source,
                        &outcome.source_id,
                    )
                    .await
                    {
                        tracing::warn!(provider, external_id, source, error = %e,
                            "official_resolve: chapter rekey failed");
                    }
                    return Some(outcome);
                }
                if fallback_from_search.is_none() {
                    fallback_from_search = Some(outcome);
                }
                only_cached = true;
            }
            Err(e) => {
                tracing::warn!(provider, external_id, source, error = %e, "official_resolve: bind failed");
                continue;
            }
        }
    }
    fallback_from_search.or(fallback)
}

/// Carry chapter rows + read progress from one key to another after a fresh
/// meta-capable binding. Target rows win: an old row whose number already
/// exists at the target is dropped (the target copy is authoritative). Used to
/// happen once per title, at bind time, so plain per-row statements are fine.
///
/// Refuses to move when another MU identity already owns the target key
/// (two aliases resolving to the same official id): merging their catalogs
/// would delete rows. The binding is still recorded by the caller.
async fn rekey_chapters(
    db: &sqlx::PgPool,
    from_provider: &str,
    from_id: &str,
    to_provider: &str,
    to_id: &str,
) -> anyhow::Result<()> {
    if from_provider == to_provider && from_id == to_id {
        return Ok(());
    }
    let owned_elsewhere: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM media_source_ids \
         WHERE source = $1 AND source_id = $2 \
           AND NOT (provider = $3 AND external_id = $4))",
    )
    .bind(to_provider)
    .bind(to_id)
    .bind(from_provider)
    .bind(from_id)
    .fetch_one(db)
    .await?;
    if owned_elsewhere {
        tracing::warn!(
            from_provider,
            from_id,
            to_provider,
            to_id,
            "official_resolve: target key belongs to another title, rekey skipped"
        );
        return Ok(());
    }
    let mut tx = db.begin().await?;
    sqlx::query(
        r#"
        UPDATE series_chapters sc
        SET provider = $3, external_id = $4
        WHERE sc.provider = $1 AND sc.external_id = $2
          AND NOT EXISTS (
              SELECT 1 FROM series_chapters t
              WHERE t.provider = $3 AND t.external_id = $4
                AND t.chapter_number = sc.chapter_number
          )
        "#,
    )
    .bind(from_provider)
    .bind(from_id)
    .bind(to_provider)
    .bind(to_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM series_chapters WHERE provider = $1 AND external_id = $2")
        .bind(from_provider)
        .bind(from_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        r#"
        UPDATE user_chapter_progress p
        SET provider = $3, external_id = $4
        WHERE p.provider = $1 AND p.external_id = $2
          AND NOT EXISTS (
              SELECT 1 FROM user_chapter_progress t
              WHERE t.user_id = p.user_id
                AND t.provider = $3 AND t.external_id = $4
                AND t.chapter_number = p.chapter_number
          )
        "#,
    )
    .bind(from_provider)
    .bind(from_id)
    .bind(to_provider)
    .bind(to_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM user_chapter_progress WHERE provider = $1 AND external_id = $2")
        .bind(from_provider)
        .bind(from_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Key `(provider, external_id)` under which the `series_chapters` rows of a
/// title live (design :51-54): the source's own key when it has a meta-capable
/// binding, otherwise the MangaUpdates identity key (MU skeleton chapters).
///
/// - Input is the MU key; any other provider is returned unchanged, so a second
///   resolution pass is a no-op (idempotence).
/// - Candidate bindings are filtered through [`SOURCES_WITH_META`], then ranked
///   by `official_source_for_media_type(media_type)` of the `media_items` row.
///   A supported binding outside that chain still wins (after every chained
///   source), ranked by [`GLOBAL_SOURCE_ORDER`] — same rule as migration 028.
/// - No `media_items` row (drawer preview) -> [`GLOBAL_SOURCE_ORDER`].
/// - No meta-capable binding -> the MU key. That is the normal fallback for a MU
///   title, hence `debug` and not `warn`.
pub async fn chapter_key(db: &sqlx::PgPool, provider: &str, external_id: &str) -> (String, String) {
    let mu_key = (provider.to_string(), external_id.to_string());
    if provider != "mangaupdates" {
        return mu_key;
    }

    let bindings = match crate::services::source_ids::list_source_ids(db, provider, external_id)
        .await
    {
        Ok(bindings) => bindings,
        Err(e) => {
            tracing::warn!(provider, external_id, error = %e, "official_resolve: binding lookup failed, keeping MU key");
            return mu_key;
        }
    };

    // A failed lookup is a DB problem, not an expected state: log it and fall
    // back to the global order (same outcome as a missing row).
    let media_type = match sqlx::query_scalar::<_, String>(
        "SELECT media_type FROM media_items WHERE provider = $1 AND external_id = $2",
    )
    .bind(provider)
    .bind(external_id)
    .fetch_optional(db)
    .await
    {
        Ok(media_type) => media_type,
        Err(e) => {
            tracing::warn!(provider, external_id, error = %e, "official_resolve: media_items lookup failed, using global order");
            None
        }
    };

    // Priority: the media_type chain first, then any other meta-capable source
    // by global rank. Chain entries without a fetch_source arm (kakao,
    // mangaplus) are dropped here - otherwise the key would point at a
    // source whose chapters are written under another one.
    let mut order: Vec<&str> = media_type
        .as_deref()
        .map(official_source_for_media_type)
        .unwrap_or(&[])
        .iter()
        .copied()
        .filter(|s| SOURCES_WITH_META.contains(s))
        .collect();
    for &source in GLOBAL_SOURCE_ORDER {
        if !order.contains(&source) {
            order.push(source);
        }
    }

    for source in order {
        if let Some(binding) = bindings.iter().find(|b| b.source == source) {
            return (binding.source.clone(), binding.source_id.clone());
        }
    }

    tracing::debug!(
        provider,
        external_id,
        "official_resolve: no meta-capable binding, chapters stay under the MU key"
    );
    mu_key
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
        assert!(hit_matches_media_type(
            "alphapolis",
            "novel/381661058",
            "novel"
        ));
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
