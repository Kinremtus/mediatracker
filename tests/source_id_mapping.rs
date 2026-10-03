mod common;

use std::sync::{Arc, Mutex};

use mediatracker::services::chapter_enrich::enrich_with_providers;
use mediatracker::services::external::chapter_count::{
    ChapterCount, ChapterCountFuture, ChapterCountProvider,
};
use mediatracker::services::source_ids::{
    delete_source_id, get_source_id, is_known_source, list_source_ids, set_source_id,
};

const SERIES_ID: i64 = 999_999_992; // MangaUpdates skeleton id
const KAKAO_ID: &str = "64096846"; // real Kakao series id for the Yongsa Party item

struct RecordingProvider {
    name: &'static str,
    seen: Arc<Mutex<Vec<String>>>,
}

impl ChapterCountProvider for RecordingProvider {
    fn name(&self) -> &'static str {
        self.name
    }

    fn fetch<'a>(&'a self, external_id: &'a str) -> ChapterCountFuture<'a> {
        let seen = self.seen.clone();
        let id = external_id.to_string();
        Box::pin(async move {
            seen.lock().unwrap().push(id);
            Ok(Some(ChapterCount {
                total: 157,
                source: "auto:kakao",
            }))
        })
    }

    fn media_types(&self) -> &'static [&'static str] {
        &["manhwa"]
    }
}

async fn setup() -> common::TestContext {
    let ctx = common::TestContext::new().await;
    let series = SERIES_ID.to_string();

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
         VALUES ('mangaupdates', $1, 'manhwa', 'Source Id Mapping Manga', 145)",
    )
    .bind(&series)
    .execute(&ctx.pool)
    .await
    .expect("insert media item");

    let nums: Vec<i32> = (1..=145).map(|c| c * 100).collect();
    sqlx::query(
        "INSERT INTO series_chapters (provider, external_id, chapter_number) \
         SELECT 'mangaupdates', $1, u.chapter_number FROM UNNEST($2::int[]) AS u(chapter_number) \
         ON CONFLICT DO NOTHING",
    )
    .bind(&series)
    .bind(&nums)
    .execute(&ctx.pool)
    .await
    .expect("insert skeleton");

    ctx
}

async fn chapters_state(pool: &sqlx::PgPool, series: &str) -> (Option<i32>, Option<String>) {
    sqlx::query_as::<_, (Option<i32>, Option<String>)>(
        "SELECT chapters, chapters_source FROM media_items \
         WHERE provider = 'mangaupdates' AND external_id = $1",
    )
    .bind(series)
    .fetch_one(pool)
    .await
    .expect("read chapters state")
}

fn recording(name: &'static str, seen: &Arc<Mutex<Vec<String>>>) -> Arc<dyn ChapterCountProvider> {
    Arc::new(RecordingProvider {
        name,
        seen: seen.clone(),
    })
}

#[tokio::test]
async fn bind_enrich_unbind_cycle() {
    let ctx = setup().await;
    let series = SERIES_ID.to_string();
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));

    // 2. No binding => the provider must never be queried, chapters stay 145.
    let applied = enrich_with_providers(
        &ctx.pool,
        "mangaupdates",
        &series,
        vec![recording("kakao", &seen)],
    )
    .await;
    assert_eq!(applied, None);
    assert!(
        seen.lock().unwrap().is_empty(),
        "provider must not be queried without a binding"
    );
    assert_eq!(chapters_state(&ctx.pool, &series).await.0, Some(145));

    // 3. Bind the Kakao id.
    set_source_id(&ctx.pool, "mangaupdates", &series, "kakao", KAKAO_ID)
        .await
        .expect("set binding");
    assert_eq!(
        get_source_id(&ctx.pool, "mangaupdates", &series, "kakao")
            .await
            .unwrap(),
        Some(KAKAO_ID.to_string())
    );

    // 4. Enrich uses the bound id, not the skeleton id.
    let applied = enrich_with_providers(
        &ctx.pool,
        "mangaupdates",
        &series,
        vec![recording("kakao", &seen)],
    )
    .await;
    assert_eq!(applied, Some(157));
    assert_eq!(seen.lock().unwrap().clone(), vec![KAKAO_ID.to_string()]);
    let (chapters, source) = chapters_state(&ctx.pool, &series).await;
    assert_eq!(chapters, Some(157));
    assert_eq!(source.as_deref(), Some("auto:kakao"));

    // 5. Unbind is idempotent and clears the list.
    assert!(
        delete_source_id(&ctx.pool, "mangaupdates", &series, "kakao")
            .await
            .unwrap()
    );
    assert!(
        list_source_ids(&ctx.pool, "mangaupdates", &series)
            .await
            .unwrap()
            .is_empty()
    );

    // 6. Re-enrich skips (no new query) and never lowers the count.
    let applied = enrich_with_providers(
        &ctx.pool,
        "mangaupdates",
        &series,
        vec![recording("kakao", &seen)],
    )
    .await;
    assert_eq!(applied, None);
    assert_eq!(seen.lock().unwrap().len(), 1, "no new query after unbind");
    assert_eq!(chapters_state(&ctx.pool, &series).await.0, Some(157));
}

#[test]
fn mangaupdates_is_not_a_bindable_source() {
    assert!(!is_known_source("mangaupdates"));
}

#[test]
fn new_official_sources_are_bindable_and_narou_is_not() {
    for source in [
        "comicwalker",
        "kakuyomu",
        "alphapolis",
        "bilibili",
        "daum",
        "qidian",
        "ridibooks",
        "mangaup",
    ] {
        assert!(
            is_known_source(source),
            "{source} must be a known bindable source"
        );
    }
    // Naro reuses `syosetu`; `narou` itself must never be bindable.
    assert!(!is_known_source("narou"));
}

#[tokio::test]
async fn enrich_extends_skeleton_to_reported_count() {
    let ctx = setup().await;
    let series = SERIES_ID.to_string();
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));

    set_source_id(&ctx.pool, "mangaupdates", &series, "kakao", KAKAO_ID)
        .await
        .expect("set binding");

    let applied = enrich_with_providers(
        &ctx.pool,
        "mangaupdates",
        &series,
        vec![recording("kakao", &seen)],
    )
    .await;
    assert_eq!(applied, Some(157));

    // Skeleton must have grown from 145 to 157 so chapters 146..157 are tickable.
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM series_chapters \
         WHERE provider = 'mangaupdates' AND external_id = $1",
    )
    .bind(&series)
    .fetch_one(&ctx.pool)
    .await
    .expect("count skeleton");
    assert_eq!(count, 157, "enrichment must extend the skeleton");

    let last: Option<i32> = sqlx::query_scalar(
        "SELECT MAX(chapter_number) FROM series_chapters \
         WHERE provider = 'mangaupdates' AND external_id = $1",
    )
    .bind(&series)
    .fetch_one(&ctx.pool)
    .await
    .expect("max chapter");
    assert_eq!(last, Some(15700), "last row is chapter 157 at x100 scale");
}
