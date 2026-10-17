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
        // kuaikan is existing/cheap -> first; bilibili is quota-limited; the
        // Tencent catalogue (acqq) last.
        "manhua" => &["kuaikan", "bilibili", "acqq", "manmanapp"],
        // kakao/naver existing; daum was dropped: the service merged into
        // Kakao and its gateway API is token-gated (no guest search), so it can
        // never bind; naverwebtoon (the free comic.naver.com originals) comes
        // before the paid `naver` store: almost every Naver original is on
        // both, and the free platform is where readers tick episodes;
        // ridibooks and the English-only platforms last.
        "manhwa" => &[
            "kakao",
            "naverwebtoon",
            "naver",
            "ridibooks",
            "tappytoon",
            "webtoons",
            "bomtoon",
            "toomics",
        ],
        // mangaplus existing; comicwalker/mangaup new; comici covers the
        // comici-platform publisher sites (Champion Cross, Takecomic, PASH!
        // neo, Young Champion Web, Big Comics); alphapolis last.
        "manga" => &[
            "mangaplus",
            "comicwalker",
            "mangaup",
            "comici",
            "alphapolis",
            "sundaywebry",
            "yanmaga",
        ],
        // syosetu existing; kakuyomu new JP; qidian CN; ridibooks/alphapolis
        // KR/JP; kakao/naver added for KR web novels; wuxiaworld last (the
        // licensed English translations).
        "novel" => &[
            "syosetu",
            "kakuyomu",
            "qidian",
            "ridibooks",
            "alphapolis",
            "kakao",
            "naver",
            "wuxiaworld",
        ],
        // Comics and OEL entries (MangaUpdates `OEL`, MangaDex originals): the
        // licensed English platforms are the official sources there.
        "comic" | "other-comics" => &["webtoons", "tappytoon"],
        _ => &[],
    }
}

/// Global fallback order used when the title has no `media_items` row yet
/// (drawer preview, chapters written before the item was added).
///
/// The first ten entries mirror the `supports_meta` global ranks of
/// `migrations/028_chapter_ownership.sql` (bilibili=1 ... qidian=10); kuaikan,
/// kakao, mangaplus, tappytoon, webtoons, acqq, comici, naverwebtoon and
/// wuxiaworld gained their fetch arms after that migration, so they are
/// appended last (in that order). The set must match `SOURCES_WITH_META`
/// (enforced by `tests/chapter_key.rs`).
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
    "kakao",
    "mangaplus",
    "tappytoon",
    "webtoons",
    "acqq",
    "comici",
    "naverwebtoon",
    "wuxiaworld",
    "bomtoon",
    "toomics",
    "manmanapp",
    "sundaywebry",
    "yanmaga",
];

/// Network abstraction so tests can inject a fixture-backed searcher.
pub trait OfficialSearch: Send + Sync {
    fn search<'a>(
        &'a self,
        source: &'a str,
        query: &'a str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<OfficialHit>>> + Send + 'a>>;
}

/// Search one official source over HTTP (the dispatch behind
/// [`HttpOfficialSearch`]). A free function so callers that need owned,
/// `'static` futures — the search-catalog fan-out spawns one task per source —
/// can call it without borrowing a searcher.
pub async fn search_source(
    client: &reqwest::Client,
    source: &str,
    query: &str,
) -> anyhow::Result<Vec<OfficialHit>> {
    match source {
        "kuaikan" => crate::services::external::official_kuaikan::search(client, query).await,
        "kakao" => crate::services::external::official_kakao::search_cached(client, query).await,
        "naver" => crate::services::external::official_naver::search(client, query).await,
        "tappytoon" => crate::services::external::official_tappytoon::search(client, query).await,
        "webtoons" => crate::services::external::official_webtoons::search(client, query).await,
        "acqq" => crate::services::external::official_acqq::search(client, query).await,
        "comici" => crate::services::external::official_comici::search(client, query).await,
        "naverwebtoon" => {
            crate::services::external::official_naverwebtoon::search(client, query).await
        }
        "wuxiaworld" => crate::services::external::official_wuxiaworld::search(client, query).await,
        "bomtoon" => crate::services::external::official_bomtoon::search(client, query).await,
        "toomics" => crate::services::external::official_toomics::search(client, query).await,
        "manmanapp" => crate::services::external::official_manmanapp::search(client, query).await,
        "sundaywebry" => {
            crate::services::external::official_sundaywebry::search(client, query).await
        }
        "yanmaga" => crate::services::external::official_yanmaga::search(client, query).await,
        "mangaplus" => {
            // The catalog lives in a process-wide cache, so a per-call
            // client is cheap.
            let mangaplus = crate::services::external::official_mangaplus::MangaPlusClient::new();
            let catalog = mangaplus.catalog().await?;
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
            crate::services::external::official_comicwalker::search(client, query).await
        }
        "kakuyomu" => crate::services::external::official_kakuyomu::search(client, query).await,
        "alphapolis" => crate::services::external::official_alphapolis::search(client, query).await,
        // Bilibili has a tiny guest quota: use the cached/cooldown path.
        "bilibili" => {
            crate::services::external::official_bilibili::search_cached(client, query).await
        }
        "daum" => crate::services::external::official_daum::search(client, query).await,
        "qidian" => crate::services::external::official_qidian::search(client, query).await,
        "ridibooks" => crate::services::external::official_ridibooks::search(client, query).await,
        "mangaup" => crate::services::external::official_mangaup::search(client, query).await,
        _ => Ok(Vec::new()),
    }
}

/// HTTP-backed [`OfficialSearch`] for the resolve walk and the admin flows.
pub struct HttpOfficialSearch {
    client: reqwest::Client,
}

impl HttpOfficialSearch {
    pub fn new() -> Self {
        Self {
            client: crate::services::external::http_client(),
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
        Box::pin(async move { search_source(&self.client, source, query).await })
    }
}

/// Alphapolis tags novel hits `novel/...` and manga hits `manga/official/...`;
/// naver and kakao tag web-novel hits `novel/{id}` while comic ids stay bare
/// (existing bindings keep working). Keep only ids that match the item's
/// `media_type`; every other source is unaffected. Pure + total: never panics.
fn hit_matches_media_type(source: &str, id: &str, media_type: &str) -> bool {
    match source {
        "alphapolis" => match media_type {
            "novel" => id.starts_with("novel/"),
            "manga" => id.starts_with("manga/official/"),
            _ => false,
        },
        // Naver and Kakao encode the kind in the hit id (`novel/{id}`); comic
        // ids stay bare so existing bindings keep working.
        "naver" | "kakao" => match media_type {
            "novel" => id.starts_with("novel/"),
            _ => !id.starts_with("novel/"),
        },
        _ => true,
    }
}

/// Resolve + persist across the ordered source list.
///
/// Every source in every chain is meta-capable ([`SOURCES_WITH_META`]), so the
/// rule is simply "first match wins": an existing binding short-circuits without
/// network I/O, otherwise the sources are searched in priority order and the
/// first matching hit is bound, rekeyed and returned. One search per source
/// until the first match; a failed source only skips itself.
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

    // An existing binding short-circuits before any network I/O. A binding can
    // predate its source gaining a fetch arm (kuaikan/kakao/mangaplus): chapter
    // rows written while the source was non-meta still sit under the MU identity
    // key, so move them (and read progress) under the resolved key or they stay
    // stranded. A no-op for the usual case.
    for &source in sources {
        match crate::services::source_ids::get_source_id(db, provider, external_id, source).await {
            Ok(Some(id)) => {
                if let Err(e) = rekey_chapters(db, provider, external_id, source, &id).await {
                    tracing::warn!(provider, external_id, source, error = %e,
                        "official_resolve: bound-key rekey failed");
                }
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

    // Where the chapter rows live right now, BEFORE any binding change: a
    // fresh meta-capable binding must carry them over (rekey) or they would be
    // orphaned under the old key.
    let old_key = chapter_key(db, provider, external_id).await;

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
                // The winner: move the rows under the new key so the official
                // chapters are not orphaned. A rekey failure only defers them —
                // the next enrich re-seeds the key.
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
            Err(e) => {
                tracing::warn!(provider, external_id, source, error = %e, "official_resolve: bind failed");
                continue;
            }
        }
    }
    None
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
    // by global rank. Chain entries without a fetch_source arm are dropped
    // here - otherwise the key would point at a source whose chapters are
    // written under another one.
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
        // bilibili before the guest-quota-free acqq/manmanapp additions.
        assert_eq!(
            official_source_for_media_type("manhua"),
            &["kuaikan", "bilibili", "acqq", "manmanapp"][..]
        );
        assert_eq!(
            official_source_for_media_type("manhwa"),
            &[
                "kakao",
                "naverwebtoon",
                "naver",
                "ridibooks",
                "tappytoon",
                "webtoons",
                "bomtoon",
                "toomics"
            ][..]
        );
        assert_eq!(
            official_source_for_media_type("manga"),
            &[
                "mangaplus",
                "comicwalker",
                "mangaup",
                "comici",
                "alphapolis",
                "sundaywebry",
                "yanmaga"
            ][..]
        );
        assert_eq!(
            official_source_for_media_type("novel"),
            &[
                "syosetu",
                "kakuyomu",
                "qidian",
                "ridibooks",
                "alphapolis",
                "kakao",
                "naver",
                "wuxiaworld"
            ][..]
        );
    }

    #[test]
    fn rejects_non_official_media_types() {
        assert_eq!(
            official_source_for_media_type("comic"),
            &["webtoons", "tappytoon"][..]
        );
        assert_eq!(
            official_source_for_media_type("other-comics"),
            &["webtoons", "tappytoon"][..]
        );
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
    fn naver_and_kakao_hits_are_filtered_by_the_novel_prefix() {
        // A novel item keeps `novel/...` and drops bare comic ids.
        assert!(hit_matches_media_type("naver", "novel/3400123", "novel"));
        assert!(!hit_matches_media_type("naver", "6448122", "novel"));
        assert!(hit_matches_media_type("kakao", "novel/47050646", "novel"));
        assert!(!hit_matches_media_type("kakao", "64096846", "novel"));
        // Every other media type keeps bare ids and drops the novel ones.
        assert!(hit_matches_media_type("naver", "6448122", "manhwa"));
        assert!(!hit_matches_media_type("naver", "novel/3400123", "manhwa"));
        assert!(hit_matches_media_type("kakao", "64096846", "manhwa"));
        assert!(!hit_matches_media_type("kakao", "novel/47050646", "manhwa"));
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
