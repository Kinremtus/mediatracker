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

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use mediatracker::services::chapter_enrich::enrich_with_providers;
use mediatracker::services::external::chapter_count::{
    ChapterCount, ChapterCountFuture, ChapterCountProvider,
};
use mediatracker::services::official_resolve::{
    official_source_for_media_type, resolve_and_bind, OfficialSearch, ResolveOutcome,
};
use mediatracker::services::official_types::OfficialHit;
use mediatracker::services::source_ids::{get_source_id, set_source_id};

const SERIES_ID: i64 = 999_999_993; // distinct from source_id_mapping (…992)
const KAKAO_ID: &str = "64096846"; // real Kakao series id for Yongsa Party

/// Fixture-backed searcher: returns canned hits and records every call.
struct FixtureSearch {
    hits: Vec<OfficialHit>,
    calls: Arc<Mutex<Vec<(String, String)>>>,
}

impl FixtureSearch {
    fn new(hits: Vec<OfficialHit>) -> Self {
        Self {
            hits,
            calls: Arc::new(Mutex::new(Vec::new())),
        }
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
        let hits = self.hits.clone();
        let source = source.to_string();
        let query = query.to_string();
        Box::pin(async move {
            calls.lock().unwrap().push((source, query));
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
    sqlx::query("DELETE FROM series_chapters WHERE external_id = $1")
        .bind(&series)
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
async fn novel_is_skipped() {
    let ctx = setup().await;
    let series = SERIES_ID.to_string();
    let search = FixtureSearch::new(vec![hit(KAKAO_ID, "용사파티")]);
    let calls = search.calls();

    let outcome = resolve_and_bind(
        &ctx.pool,
        &search,
        "mangaupdates",
        &series,
        "novel",
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

    assert_eq!(applied, None, "157 is lower than 200, so nothing is applied");
    assert_eq!(
        chapter_count(&ctx.pool, &series).await,
        Some(200),
        "raise-only must never lower the stored count"
    );
}

#[test]
fn mapping_table() {
    assert_eq!(official_source_for_media_type("manhua"), Some("kuaikan"));
    assert_eq!(official_source_for_media_type("manhwa"), Some("kakao"));
    assert_eq!(official_source_for_media_type("manga"), Some("mangaplus"));
    assert_eq!(official_source_for_media_type("novel"), None);
    assert_eq!(official_source_for_media_type("comic"), None);
    assert_eq!(official_source_for_media_type("other-comics"), None);
    assert_eq!(official_source_for_media_type("anime"), None);
    assert_eq!(official_source_for_media_type(""), None);
}
