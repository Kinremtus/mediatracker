//! Task 4.1: offline verification of the official-source resolver.
//!
//! The resolver is exercised with a fixture-injected [`OfficialSearch`], so no
//! network I/O ever happens. A call counter records every `(source, query)` the
//! resolver asks for, which lets us prove the "existing binding skips network"
//! and "novel is skipped" paths are genuinely zero-request.
//!
//! Mirrors the style of `tests/source_id_mapping.rs` (testcontainers harness via
//! `tests/common/mod.rs`, per-test `setup()` that cleans + seeds its own rows).

mod common;

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use mediatracker::services::chapter_enrich::enrich_with_providers;
use mediatracker::services::external::chapter_count::{
    ChapterCount, ChapterCountFuture, ChapterCountProvider,
};
use mediatracker::services::official_resolve::{
    OfficialSearch, ResolveOutcome, official_source_for_media_type, resolve_and_bind,
};
use mediatracker::services::official_types::OfficialHit;
use mediatracker::services::source_ids::{get_source_id, set_source_id};

const SERIES_ID: i64 = 999_999_993; // distinct from source_id_mapping (…992)
const KAKAO_ID: &str = "64096846"; // real Kakao series id for Yongsa Party
/// Target id used by the rekey tests: after a meta-capable bind the chapter
/// rows move under this `(bilibili, id)` key.
const BILIBILI_ID: &str = "990001";

/// Fixture-backed searcher: canned hits per source, optional forced errors, and
/// a recorder of every `(source, query)` call.
struct FixtureSearch {
    hits_for: HashMap<String, Vec<OfficialHit>>,
    fail_sources: HashSet<String>,
    calls: Arc<Mutex<Vec<(String, String)>>>,
}

impl FixtureSearch {
    /// Same hits for every source (default for the single-source tests).
    fn new(hits: Vec<OfficialHit>) -> Self {
        let mut hits_for = HashMap::new();
        for src in ["kakao", "naver", "kuaikan", "mangaplus"] {
            hits_for.insert(src.to_string(), hits.clone());
        }
        Self {
            hits_for,
            fail_sources: HashSet::new(),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Explicit per-source hits (empty vector = "no match from this source").
    fn with_sources(pairs: &[(&str, Vec<OfficialHit>)]) -> Self {
        Self {
            hits_for: pairs
                .iter()
                .map(|(s, h)| (s.to_string(), h.clone()))
                .collect(),
            fail_sources: HashSet::new(),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Force `search(source, _)` to return `Err`.
    fn failing(mut self, source: &str) -> Self {
        self.fail_sources.insert(source.to_string());
        self
    }

    /// Shared handle to the recorded `(source, query)` calls.
    fn calls(&self) -> Arc<Mutex<Vec<(String, String)>>> {
        self.calls.clone()
    }
}

impl OfficialSearch for FixtureSearch {
    fn search<'a>(
        &'a self,
        source: &'a str,
        query: &'a str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<OfficialHit>>> + Send + 'a>> {
        let calls = self.calls.clone();
        let source_owned = source.to_string();
        let query_owned = query.to_string();
        let hits = self.hits_for.get(source).cloned().unwrap_or_default();
        let fail = self.fail_sources.contains(source);
        Box::pin(async move {
            calls.lock().unwrap().push((source_owned, query_owned));
            if fail {
                anyhow::bail!("fixture: forced search failure");
            }
            Ok(hits)
        })
    }
}

/// Raise-only chapter provider mock (mirrors `tests/source_id_mapping.rs`).
struct RecordingProvider {
    name: &'static str,
    total: i32,
    seen: Arc<Mutex<Vec<String>>>,
}

impl ChapterCountProvider for RecordingProvider {
    fn name(&self) -> &'static str {
        self.name
    }

    fn fetch<'a>(&'a self, external_id: &'a str) -> ChapterCountFuture<'a> {
        let seen = self.seen.clone();
        let id = external_id.to_string();
        let total = self.total;
        Box::pin(async move {
            seen.lock().unwrap().push(id);
            Ok(Some(ChapterCount {
                total,
                source: "auto:kakao",
            }))
        })
    }

    fn media_types(&self) -> &'static [&'static str] {
        &["manhwa"]
    }
}

/// Fresh DB + a seeded MangaUpdates skeleton media item.
async fn setup() -> common::TestContext {
    let ctx = common::TestContext::new().await;
    let series = SERIES_ID.to_string();

    // Clean first so the insert below can stay ON CONFLICT-free.
    sqlx::query("DELETE FROM media_source_ids WHERE external_id = $1")
        .bind(&series)
        .execute(&ctx.pool)
        .await
        .expect("delete stale bindings");
    sqlx::query("DELETE FROM series_chapters WHERE external_id IN ($1, $2)")
        .bind(&series)
        .bind(BILIBILI_ID)
        .execute(&ctx.pool)
        .await
        .expect("delete stale chapters");
    sqlx::query("DELETE FROM media_items WHERE provider = 'mangaupdates' AND external_id = $1")
        .bind(&series)
        .execute(&ctx.pool)
        .await
        .expect("delete stale media item");

    sqlx::query(
        "INSERT INTO media_items (provider, external_id, media_type, title, chapters) \
         VALUES ('mangaupdates', $1, 'manhwa', 'Yongsa Party', 145)",
    )
    .bind(&series)
    .execute(&ctx.pool)
    .await
    .expect("insert media item");

    ctx
}

async fn chapter_count(pool: &sqlx::PgPool, series: &str) -> Option<i32> {
    sqlx::query_scalar(
        "SELECT chapters FROM media_items \
         WHERE provider = 'mangaupdates' AND external_id = $1",
    )
    .bind(series)
    .fetch_one(pool)
    .await
    .expect("read chapters")
}

fn hit(id: &str, title: &str) -> OfficialHit {
    OfficialHit {
        id: id.to_string(),
        title: title.to_string(),
    }
}

#[tokio::test]
async fn exact_match_binds_kakao() {
    let ctx = setup().await;
    let series = SERIES_ID.to_string();
    let search = FixtureSearch::new(vec![hit(KAKAO_ID, "용사파티")]);
    let calls = search.calls();

    let outcome = resolve_and_bind(
        &ctx.pool,
        &search,
        "mangaupdates",
        &series,
        "manhwa",
        "용사파티",
        &[],
    )
    .await;

    assert_eq!(
        outcome,
        Some(ResolveOutcome {
            source: "kakao".to_string(),
            source_id: KAKAO_ID.to_string(),
        })
    );
    assert_eq!(
        get_source_id(&ctx.pool, "mangaupdates", &series, "kakao")
            .await
            .unwrap(),
        Some(KAKAO_ID.to_string())
    );

    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 1, "exactly one search request");
    assert_eq!(calls[0].0, "kakao");
}

#[tokio::test]
async fn non_exact_hit_does_not_bind() {
    let ctx = setup().await;
    let series = SERIES_ID.to_string();
    let search = FixtureSearch::new(vec![hit(KAKAO_ID, "용사파티")]);

    let outcome = resolve_and_bind(
        &ctx.pool,
        &search,
        "mangaupdates",
        &series,
        "manhwa",
        "용사파티 X",
        &[],
    )
    .await;

    assert_eq!(outcome, None);
    assert_eq!(
        get_source_id(&ctx.pool, "mangaupdates", &series, "kakao")
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn existing_binding_skips_network() {
    let ctx = setup().await;
    let series = SERIES_ID.to_string();

    set_source_id(&ctx.pool, "mangaupdates", &series, "kakao", KAKAO_ID)
        .await
        .expect("pre-bind kakao");

    let search = FixtureSearch::new(vec![hit(KAKAO_ID, "용사파티")]);
    let calls = search.calls();

    let outcome = resolve_and_bind(
        &ctx.pool,
        &search,
        "mangaupdates",
        &series,
        "manhwa",
        "용사파티",
        &[],
    )
    .await;

    assert_eq!(
        outcome,
        Some(ResolveOutcome {
            source: "kakao".to_string(),
            source_id: KAKAO_ID.to_string(),
        })
    );
    assert!(
        calls.lock().unwrap().is_empty(),
        "existing binding must short-circuit before any network I/O"
    );
}

#[tokio::test]
async fn unsupported_media_type_is_skipped() {
    let ctx = setup().await;
    let series = SERIES_ID.to_string();
    let search = FixtureSearch::new(vec![hit(KAKAO_ID, "용사파티")]);
    let calls = search.calls();

    let outcome = resolve_and_bind(
        &ctx.pool,
        &search,
        "mangaupdates",
        &series,
        "comic",
        "용사파티",
        &[],
    )
    .await;

    assert_eq!(outcome, None);
    assert!(
        calls.lock().unwrap().is_empty(),
        "unsupported media_type must never hit the network"
    );
}

#[tokio::test]
async fn script_pick_uses_hangul_query() {
    let ctx = setup().await;
    let series = SERIES_ID.to_string();
    let search = FixtureSearch::new(vec![hit(KAKAO_ID, "원피스")]);
    let calls = search.calls();

    let outcome = resolve_and_bind(
        &ctx.pool,
        &search,
        "mangaupdates",
        &series,
        "manhwa",
        "One Piece",
        &["원피스".to_string()],
    )
    .await;

    assert_eq!(
        outcome,
        Some(ResolveOutcome {
            source: "kakao".to_string(),
            source_id: KAKAO_ID.to_string(),
        })
    );

    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].1, "원피스", "kakao must be queried with Hangul");
}

#[tokio::test]
async fn manhua_maps_to_kuaikan() {
    let ctx = setup().await;
    let series = SERIES_ID.to_string();
    let search = FixtureSearch::new(vec![hit("123", "我的英雄学院")]);

    let outcome = resolve_and_bind(
        &ctx.pool,
        &search,
        "mangaupdates",
        &series,
        "manhua",
        "我的英雄学院",
        &[],
    )
    .await;

    assert_eq!(
        outcome,
        Some(ResolveOutcome {
            source: "kuaikan".to_string(),
            source_id: "123".to_string(),
        })
    );
    assert_eq!(
        get_source_id(&ctx.pool, "mangaupdates", &series, "kuaikan")
            .await
            .unwrap(),
        Some("123".to_string())
    );
}

#[tokio::test]
async fn manga_maps_to_mangaplus() {
    let ctx = setup().await;
    let series = SERIES_ID.to_string();
    let search = FixtureSearch::new(vec![hit("100020", "One Piece")]);

    let outcome = resolve_and_bind(
        &ctx.pool,
        &search,
        "mangaupdates",
        &series,
        "manga",
        "One Piece",
        &[],
    )
    .await;

    assert_eq!(
        outcome,
        Some(ResolveOutcome {
            source: "mangaplus".to_string(),
            source_id: "100020".to_string(),
        })
    );
    assert_eq!(
        get_source_id(&ctx.pool, "mangaupdates", &series, "mangaplus")
            .await
            .unwrap(),
        Some("100020".to_string())
    );
}

#[tokio::test]
async fn raise_only_keeps_higher_count() {
    let ctx = setup().await;
    let series = SERIES_ID.to_string();

    // A higher, known-good count already exists (e.g. a manual/MU value).
    sqlx::query(
        "UPDATE media_items SET chapters = 200 \
         WHERE provider = 'mangaupdates' AND external_id = $1",
    )
    .bind(&series)
    .execute(&ctx.pool)
    .await
    .expect("seed chapters = 200");

    set_source_id(&ctx.pool, "mangaupdates", &series, "kakao", KAKAO_ID)
        .await
        .expect("bind kakao");

    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let provider: Arc<dyn ChapterCountProvider> = Arc::new(RecordingProvider {
        name: "kakao",
        total: 157,
        seen: seen.clone(),
    });

    let applied = enrich_with_providers(&ctx.pool, "mangaupdates", &series, vec![provider]).await;

    assert_eq!(
        applied, None,
        "157 is lower than 200, so nothing is applied"
    );
    assert_eq!(
        chapter_count(&ctx.pool, &series).await,
        Some(200),
        "raise-only must never lower the stored count"
    );
}

#[test]
fn mapping_table() {
    assert_eq!(
        official_source_for_media_type("manhua"),
        &["kuaikan", "bilibili"][..]
    );
    assert_eq!(
        official_source_for_media_type("manhwa"),
        &["kakao", "naver", "ridibooks", "tappytoon", "webtoons"][..]
    );
    assert_eq!(
        official_source_for_media_type("manga"),
        &["mangaplus", "comicwalker", "mangaup", "alphapolis"][..]
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
            "naver"
        ][..]
    );
    assert!(official_source_for_media_type("comic").is_empty());
    assert!(official_source_for_media_type("other-comics").is_empty());
    assert!(official_source_for_media_type("anime").is_empty());
    assert!(official_source_for_media_type("").is_empty());
}

#[tokio::test]
async fn manhwa_falls_back_to_naver() {
    let ctx = setup().await;
    let series = SERIES_ID.to_string();
    let search = FixtureSearch::with_sources(&[
        ("kakao", vec![]),
        ("naver", vec![hit("10319749", "왕의 힘으로 회귀한다")]),
    ]);
    let calls = search.calls();

    let outcome = resolve_and_bind(
        &ctx.pool,
        &search,
        "mangaupdates",
        &series,
        "manhwa",
        "왕의 힘으로 회귀한다",
        &[],
    )
    .await;

    assert_eq!(
        outcome,
        Some(ResolveOutcome {
            source: "naver".to_string(),
            source_id: "10319749".to_string(),
        })
    );
    assert_eq!(
        get_source_id(&ctx.pool, "mangaupdates", &series, "naver")
            .await
            .unwrap(),
        Some("10319749".to_string())
    );
    assert_eq!(
        get_source_id(&ctx.pool, "mangaupdates", &series, "kakao")
            .await
            .unwrap(),
        None,
        "kakao returned no hit, so it must not be bound"
    );

    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 2, "kakao first, then naver");
    assert_eq!(calls[0].0, "kakao");
    assert_eq!(calls[1].0, "naver");
}

#[tokio::test]
async fn kakao_error_falls_through_to_naver() {
    let ctx = setup().await;
    let series = SERIES_ID.to_string();
    let search = FixtureSearch::with_sources(&[
        ("kakao", vec![]),
        ("naver", vec![hit("10319749", "왕의 힘으로 회귀한다")]),
    ])
    .failing("kakao");
    let calls = search.calls();

    let outcome = resolve_and_bind(
        &ctx.pool,
        &search,
        "mangaupdates",
        &series,
        "manhwa",
        "왕의 힘으로 회귀한다",
        &[],
    )
    .await;

    assert_eq!(
        outcome,
        Some(ResolveOutcome {
            source: "naver".to_string(),
            source_id: "10319749".to_string(),
        })
    );
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 2, "a failed kakao must not block naver");
    assert_eq!(calls[1].0, "naver");
}

#[tokio::test]
async fn existing_naver_binding_short_circuits() {
    let ctx = setup().await;
    let series = SERIES_ID.to_string();

    set_source_id(&ctx.pool, "mangaupdates", &series, "naver", "10319749")
        .await
        .expect("pre-bind naver");

    let search =
        FixtureSearch::with_sources(&[("naver", vec![hit("10319749", "왕의 힘으로 회귀한다")])]);
    let calls = search.calls();

    let outcome = resolve_and_bind(
        &ctx.pool,
        &search,
        "mangaupdates",
        &series,
        "manhwa",
        "왕의 힘으로 회귀한다",
        &[],
    )
    .await;

    assert_eq!(
        outcome,
        Some(ResolveOutcome {
            source: "naver".to_string(),
            source_id: "10319749".to_string(),
        })
    );
    assert!(
        calls.lock().unwrap().is_empty(),
        "an existing binding for any source must skip ALL network I/O"
    );
}

#[tokio::test]
async fn no_match_anywhere_binds_nothing() {
    let ctx = setup().await;
    let series = SERIES_ID.to_string();
    let search = FixtureSearch::with_sources(&[("kakao", vec![]), ("naver", vec![])]);
    let calls = search.calls();

    let outcome = resolve_and_bind(
        &ctx.pool,
        &search,
        "mangaupdates",
        &series,
        "manhwa",
        "존재하지 않는 제목",
        &[],
    )
    .await;

    assert_eq!(outcome, None);
    assert_eq!(
        calls.lock().unwrap().len(),
        3,
        "the three Hangul-capable manhwa sources were tried once (tappytoon and webtoons are English-only)"
    );
    assert_eq!(
        get_source_id(&ctx.pool, "mangaupdates", &series, "naver")
            .await
            .unwrap(),
        None
    );
}

/// kuaikan is meta-capable now: a pre-bound kuaikan binding is the winner and
/// the walk short-circuits without any search call.
#[tokio::test]
async fn bound_kuaikan_short_circuits_without_network() {
    let ctx = setup().await;
    let series = SERIES_ID.to_string();

    set_source_id(&ctx.pool, "mangaupdates", &series, "kuaikan", "22928")
        .await
        .expect("pre-bind kuaikan");

    let search = FixtureSearch::with_sources(&[
        ("kuaikan", vec![hit("22928", "妖神记")]),
        ("bilibili", vec![hit(BILIBILI_ID, "妖神记")]),
    ]);
    let calls = search.calls();

    let outcome = resolve_and_bind(
        &ctx.pool,
        &search,
        "mangaupdates",
        &series,
        "manhua",
        "妖神记",
        &[],
    )
    .await;

    assert_eq!(
        outcome,
        Some(ResolveOutcome {
            source: "kuaikan".to_string(),
            source_id: "22928".to_string(),
        }),
        "the bound meta-capable kuaikan is the winner"
    );
    assert_eq!(
        get_source_id(&ctx.pool, "mangaupdates", &series, "kuaikan")
            .await
            .unwrap(),
        Some("22928".to_string())
    );
    assert_eq!(
        get_source_id(&ctx.pool, "mangaupdates", &series, "bilibili")
            .await
            .unwrap(),
        None,
        "bilibili is never bound"
    );

    let calls = calls.lock().unwrap();
    assert!(calls.is_empty(), "a bound source needs no network call");
}

/// A fresh kuaikan match is a meta-capable match: it binds, rekeys and ends
/// the walk - bilibili is not searched at all.
#[tokio::test]
async fn fresh_kuaikan_match_wins_and_stops_the_walk() {
    let ctx = setup().await;
    let series = SERIES_ID.to_string();
    let search = FixtureSearch::with_sources(&[
        ("kuaikan", vec![hit("22928", "妖神记")]),
        ("bilibili", vec![hit(BILIBILI_ID, "妖神记")]),
    ]);
    let calls = search.calls();

    let outcome = resolve_and_bind(
        &ctx.pool,
        &search,
        "mangaupdates",
        &series,
        "manhua",
        "妖神记",
        &[],
    )
    .await;

    assert_eq!(
        outcome,
        Some(ResolveOutcome {
            source: "kuaikan".to_string(),
            source_id: "22928".to_string(),
        })
    );
    assert_eq!(
        get_source_id(&ctx.pool, "mangaupdates", &series, "kuaikan")
            .await
            .unwrap(),
        Some("22928".to_string())
    );
    assert_eq!(
        get_source_id(&ctx.pool, "mangaupdates", &series, "bilibili")
            .await
            .unwrap(),
        None
    );

    let calls = calls.lock().unwrap();
    let order: Vec<&str> = calls.iter().map(|c| c.0.as_str()).collect();
    assert_eq!(order, vec!["kuaikan"], "the walk stops at the meta match");
}

/// kuaikan is tried first; when it has no match the walk still reaches the
/// cached bilibili and binds it.
#[tokio::test]
async fn bilibili_is_tried_when_kuaikan_has_no_match() {
    let ctx = setup().await;
    let series = SERIES_ID.to_string();
    let search = FixtureSearch::with_sources(&[
        ("kuaikan", vec![]),
        ("bilibili", vec![hit(BILIBILI_ID, "妖神记")]),
    ]);
    let calls = search.calls();

    let outcome = resolve_and_bind(
        &ctx.pool,
        &search,
        "mangaupdates",
        &series,
        "manhua",
        "妖神记",
        &[],
    )
    .await;

    assert_eq!(
        outcome,
        Some(ResolveOutcome {
            source: "bilibili".to_string(),
            source_id: BILIBILI_ID.to_string(),
        })
    );
    let calls = calls.lock().unwrap();
    let order: Vec<&str> = calls.iter().map(|c| c.0.as_str()).collect();
    assert_eq!(order, vec!["kuaikan", "bilibili"]);
}

/// A web-novel hit (`novel/`-prefixed id) binds only a `novel` item: the kind
/// gate in `hit_matches_media_type` keeps the prefix from binding comics.
#[tokio::test]
async fn novel_hit_binds_only_a_novel_item() {
    let ctx = setup().await;
    let series = SERIES_ID.to_string();
    let search =
        FixtureSearch::with_sources(&[("naver", vec![hit("novel/3400123", "전지적 독자 시점")])]);

    let outcome = resolve_and_bind(
        &ctx.pool,
        &search,
        "mangaupdates",
        &series,
        "novel",
        "전지적 독자 시점",
        &[],
    )
    .await;

    assert_eq!(
        outcome,
        Some(ResolveOutcome {
            source: "naver".to_string(),
            source_id: "novel/3400123".to_string(),
        })
    );
    assert_eq!(
        get_source_id(&ctx.pool, "mangaupdates", &series, "naver")
            .await
            .unwrap(),
        Some("novel/3400123".to_string())
    );
}

/// A fresh meta-capable bind moves the MU-key chapter rows and read progress
/// under the official key. Target rows win over old duplicates.
#[tokio::test]
async fn rekey_moves_chapters_and_progress_to_the_bound_key() {
    let ctx = setup().await;
    let series = SERIES_ID.to_string();

    sqlx::query(
        "INSERT INTO series_chapters (provider, external_id, chapter_number, title_en) \
         VALUES ('mangaupdates', $1, 100, 'MU 100'), \
                ('mangaupdates', $1, 200, 'MU 200'), \
                ('bilibili', $2, 100, 'Official 100')",
    )
    .bind(&series)
    .bind(BILIBILI_ID)
    .execute(&ctx.pool)
    .await
    .expect("seed chapters");

    let username = "rekey-fixture-user";
    sqlx::query("DELETE FROM users WHERE username = $1")
        .bind(username)
        .execute(&ctx.pool)
        .await
        .expect("clean fixture user");
    let user_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO users (username, email, password_hash, role) \
         VALUES ($1, 'rekey@example.test', 'fakehash', 'user') RETURNING id",
    )
    .bind(username)
    .fetch_one(&ctx.pool)
    .await
    .expect("create fixture user");
    sqlx::query(
        "INSERT INTO user_chapter_progress \
             (user_id, provider, external_id, chapter_number, read, read_at) \
         VALUES ($1, 'mangaupdates', $2, 100, TRUE, NOW())",
    )
    .bind(user_id)
    .bind(&series)
    .execute(&ctx.pool)
    .await
    .expect("seed progress");

    let search = FixtureSearch::with_sources(&[
        ("kuaikan", vec![]),
        ("bilibili", vec![hit(BILIBILI_ID, "妖神记")]),
    ]);
    let outcome = resolve_and_bind(
        &ctx.pool,
        &search,
        "mangaupdates",
        &series,
        "manhua",
        "妖神记",
        &[],
    )
    .await;
    assert_eq!(outcome.map(|o| o.source), Some("bilibili".to_string()));

    let rows: Vec<(String, i32, Option<String>)> = sqlx::query_as(
        "SELECT provider, chapter_number, title_en FROM series_chapters \
         WHERE external_id IN ($1, $2) ORDER BY chapter_number",
    )
    .bind(&series)
    .bind(BILIBILI_ID)
    .fetch_all(&ctx.pool)
    .await
    .expect("read chapters after rekey");

    assert_eq!(
        rows,
        vec![
            (
                "bilibili".to_string(),
                100,
                Some("Official 100".to_string())
            ),
            ("bilibili".to_string(), 200, Some("MU 200".to_string())),
        ],
        "rows moved under (bilibili, {BILIBILI_ID}); the target copy of 100 wins"
    );

    let progress: Vec<(String, i32)> = sqlx::query_as(
        "SELECT provider, chapter_number FROM user_chapter_progress \
         WHERE user_id = $1 ORDER BY chapter_number",
    )
    .bind(user_id)
    .fetch_all(&ctx.pool)
    .await
    .expect("read progress after rekey");
    assert_eq!(
        progress,
        vec![("bilibili".to_string(), 100)],
        "read progress follows the rows"
    );
}

/// Two MU aliases resolving to the same official id must not merge catalogs:
/// the rekey is refused when another identity already owns the target key.
#[tokio::test]
async fn rekey_refused_when_another_title_owns_the_target_key() {
    let ctx = setup().await;
    let series = SERIES_ID.to_string();
    let alias = "999999991";

    sqlx::query("DELETE FROM media_source_ids WHERE external_id = $1")
        .bind(alias)
        .execute(&ctx.pool)
        .await
        .expect("clean alias binding");
    sqlx::query(
        "INSERT INTO series_chapters (provider, external_id, chapter_number, title_en) \
         VALUES ('mangaupdates', $1, 100, 'Alias 100')",
    )
    .bind(&series)
    .execute(&ctx.pool)
    .await
    .expect("seed chapters");
    // Another MU identity is already bound to the same official id.
    sqlx::query(
        "INSERT INTO media_source_ids (provider, external_id, source, source_id) \
         VALUES ('mangaupdates', $1, 'bilibili', $2)",
    )
    .bind(alias)
    .bind(BILIBILI_ID)
    .execute(&ctx.pool)
    .await
    .expect("seed alias binding");

    let search = FixtureSearch::with_sources(&[
        ("kuaikan", vec![]),
        ("bilibili", vec![hit(BILIBILI_ID, "妖神记")]),
    ]);
    let outcome = resolve_and_bind(
        &ctx.pool,
        &search,
        "mangaupdates",
        &series,
        "manhua",
        "妖神记",
        &[],
    )
    .await;
    assert_eq!(
        outcome.map(|o| o.source),
        Some("bilibili".to_string()),
        "the binding itself is still recorded"
    );

    let rows: Vec<(String, i32)> = sqlx::query_as(
        "SELECT provider, chapter_number FROM series_chapters \
         WHERE external_id = $1 ORDER BY chapter_number",
    )
    .bind(&series)
    .fetch_all(&ctx.pool)
    .await
    .expect("read rows after refused rekey");
    assert_eq!(
        rows,
        vec![("mangaupdates".to_string(), 100)],
        "rows stay under the MU key (no cross-title merge)"
    );
}
