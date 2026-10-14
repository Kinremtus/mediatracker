//! Chapter-key resolution: `official_resolve::chapter_key`.
//!
//! The invariant (design :51-54): `series_chapters` rows live under the key of
//! their own source when the title has a meta-capable binding, otherwise under
//! the MangaUpdates identity key.

mod common;

use mediatracker::services::external::official_meta::SOURCES_WITH_META;
use mediatracker::services::official_resolve::{GLOBAL_SOURCE_ORDER, chapter_key};

async fn setup() -> common::TestContext {
    common::TestContext::new().await
}

async fn add_media_item(ctx: &common::TestContext, mu_id: &str, media_type: &str) {
    sqlx::query(
        "INSERT INTO media_items (provider, external_id, media_type, title) \
         VALUES ('mangaupdates', $1, $2, $3)",
    )
    .bind(mu_id)
    .bind(media_type)
    .bind(format!("Key fixture {mu_id}"))
    .execute(&ctx.pool)
    .await
    .expect("insert media_items row");
}

async fn bind(ctx: &common::TestContext, mu_id: &str, source: &str, source_id: &str) {
    sqlx::query(
        "INSERT INTO media_source_ids (provider, external_id, source, source_id) \
         VALUES ('mangaupdates', $1, $2, $3)",
    )
    .bind(mu_id)
    .bind(source)
    .bind(source_id)
    .execute(&ctx.pool)
    .await
    .expect("insert binding");
}

#[tokio::test]
async fn binding_bilibili_wins_over_mu_key() {
    let ctx = setup().await;
    add_media_item(&ctx, "900000001", "manhua").await;
    bind(&ctx, "900000001", "bilibili", "mc32437").await;

    let (provider, external_id) = chapter_key(&ctx.pool, "mangaupdates", "900000001").await;
    assert_eq!(provider, "bilibili");
    assert_eq!(external_id, "mc32437");
}

#[tokio::test]
async fn no_bindings_keeps_mu_key() {
    let ctx = setup().await;
    add_media_item(&ctx, "900000002", "manhwa").await;

    let (provider, external_id) = chapter_key(&ctx.pool, "mangaupdates", "900000002").await;
    assert_eq!(provider, "mangaupdates");
    assert_eq!(external_id, "900000002");
}

#[tokio::test]
async fn meta_less_source_is_filtered_out() {
    // `mh5` is a real binding with no fetch_source arm -> Ok(default), so the
    // key must not point at it; the meta-capable comicwalker wins instead.
    let ctx = setup().await;
    add_media_item(&ctx, "900000003", "manga").await;
    bind(&ctx, "900000003", "mh5", "mh-1").await;
    bind(&ctx, "900000003", "comicwalker", "cw-1").await;

    let (provider, external_id) = chapter_key(&ctx.pool, "mangaupdates", "900000003").await;
    assert_eq!(provider, "comicwalker");
    assert_eq!(external_id, "cw-1");
}

#[tokio::test]
async fn only_meta_less_binding_keeps_mu_key() {
    let ctx = setup().await;
    add_media_item(&ctx, "900000004", "manga").await;
    bind(&ctx, "900000004", "mh5", "mh-2").await;

    let (provider, external_id) = chapter_key(&ctx.pool, "mangaupdates", "900000004").await;
    assert_eq!(provider, "mangaupdates");
    assert_eq!(external_id, "900000004");
}

#[tokio::test]
async fn chain_order_prefers_mangaplus_for_manga() {
    // mangaplus gained its fetch arm after migration 028 and is first in the
    // manga chain, so it must win over comicwalker now.
    let ctx = setup().await;
    add_media_item(&ctx, "900000012", "manga").await;
    bind(&ctx, "900000012", "comicwalker", "cw-2").await;
    bind(&ctx, "900000012", "mangaplus", "mp-3").await;

    let (provider, external_id) = chapter_key(&ctx.pool, "mangaupdates", "900000012").await;
    assert_eq!(provider, "mangaplus");
    assert_eq!(external_id, "mp-3");
}

#[tokio::test]
async fn chain_order_prefers_kakao_over_naver_for_manhwa() {
    // kakao gained its fetch arm after migration 028 and is first in the
    // manhwa chain, so it must win over naver now.
    let ctx = setup().await;
    add_media_item(&ctx, "900000011", "manhwa").await;
    bind(&ctx, "900000011", "naver", "naver-2").await;
    bind(&ctx, "900000011", "kakao", "kakao-3").await;

    let (provider, external_id) = chapter_key(&ctx.pool, "mangaupdates", "900000011").await;
    assert_eq!(provider, "kakao");
    assert_eq!(external_id, "kakao-3");
}

#[tokio::test]
async fn chain_order_prefers_kuaikan_over_bilibili_for_manhua() {
    // kuaikan gained its fetch arm after migration 028 and is first in the
    // manhua chain, so it must win over bilibili now.
    let ctx = setup().await;
    add_media_item(&ctx, "900000010", "manhua").await;
    bind(&ctx, "900000010", "bilibili", "mc-1").await;
    bind(&ctx, "900000010", "kuaikan", "kk-1").await;

    let (provider, external_id) = chapter_key(&ctx.pool, "mangaupdates", "900000010").await;
    assert_eq!(provider, "kuaikan");
    assert_eq!(external_id, "kk-1");
}

#[tokio::test]
async fn non_mu_provider_passes_through() {
    let ctx = setup().await;

    let (provider, external_id) = chapter_key(&ctx.pool, "mangadex", "123").await;
    assert_eq!(provider, "mangadex");
    assert_eq!(external_id, "123");
}

#[tokio::test]
async fn without_media_item_row_global_fallback_order_decides() {
    // No media_items row (drawer preview) -> GLOBAL_SOURCE_ORDER. There
    // naver=2 comes before daum=3, so naver wins here too - the assertion only
    // pins that the fallback order is applied, not which chain.
    let ctx = setup().await;
    bind(&ctx, "900000006", "daum", "daum-10").await;
    bind(&ctx, "900000006", "naver", "naver-10").await;

    let (provider, external_id) = chapter_key(&ctx.pool, "mangaupdates", "900000006").await;
    assert_eq!(provider, "naver");
    assert_eq!(external_id, "naver-10");
}

#[tokio::test]
async fn chain_order_beats_global_order_for_novel() {
    // Discriminating case: novel chain starts ["syosetu", "kakuyomu", "qidian",
    // "ridibooks", "alphapolis", ...] but GLOBAL_SOURCE_ORDER ranks ridibooks(4)
    // before syosetu(8). Chain must win here, so this test fails if the
    // chain block is removed and the global order is applied unconditionally.
    let ctx = setup().await;
    add_media_item(&ctx, "900000008", "novel").await;
    bind(&ctx, "900000008", "ridibooks", "ridibooks-12").await;
    bind(&ctx, "900000008", "syosetu", "syosetu-12").await;

    let (provider, external_id) = chapter_key(&ctx.pool, "mangaupdates", "900000008").await;
    assert_eq!(provider, "syosetu");
    assert_eq!(external_id, "syosetu-12");
}

#[tokio::test]
async fn without_media_item_row_global_order_beats_chain_for_novel() {
    // Same two bindings, but no media_items row -> there is no chain at all and
    // GLOBAL_SOURCE_ORDER decides: ridibooks(4) before syosetu(8). Fails if the
    // chain is applied unconditionally regardless of media_type.
    let ctx = setup().await;
    bind(&ctx, "900000009", "ridibooks", "ridibooks-13").await;
    bind(&ctx, "900000009", "syosetu", "syosetu-13").await;

    let (provider, external_id) = chapter_key(&ctx.pool, "mangaupdates", "900000009").await;
    assert_eq!(provider, "ridibooks");
    assert_eq!(external_id, "ridibooks-13");
}

#[tokio::test]
async fn supported_binding_outside_chain_wins_over_mu_key() {
    // media_type 'manhua' chain knows only kuaikan/bilibili; a naver binding is
    // still meta-capable, so it must win after the chained sources.
    let ctx = setup().await;
    add_media_item(&ctx, "900000007", "manhua").await;
    bind(&ctx, "900000007", "naver", "naver-11").await;

    let (provider, external_id) = chapter_key(&ctx.pool, "mangaupdates", "900000007").await;
    assert_eq!(provider, "naver");
    assert_eq!(external_id, "naver-11");
}

#[test]
fn sources_with_meta_matches_fetch_source_arms() {
    // Hardcoded copy of the `fetch_source` arms in
    // src/services/external/official_meta.rs. Adding an arm there without
    // updating SOURCES_WITH_META fails this test on purpose.
    const FETCH_SOURCE_ARMS: &[&str] = &[
        "naver",
        "syosetu",
        "comicwalker",
        "kakuyomu",
        "alphapolis",
        "bilibili",
        "daum",
        "qidian",
        "ridibooks",
        "mangaup",
        "kuaikan",
        "kakao",
        "mangaplus",
        "tappytoon",
        "webtoons",
        "acqq",
        "comici",
        "naverwebtoon",
        "wuxiaworld",
    ];

    let mut arms: Vec<&str> = FETCH_SOURCE_ARMS.to_vec();
    let mut with_meta: Vec<&str> = SOURCES_WITH_META.to_vec();
    arms.sort_unstable();
    with_meta.sort_unstable();
    assert_eq!(with_meta, arms);

    // A source without a fetch arm must stay out (a binding to it keeps the
    // chapters on the MU key).
    assert!(!SOURCES_WITH_META.contains(&"mh5"), "mh5 has no fetch arm");
}

#[test]
fn global_source_order_has_the_same_set_as_supports_meta() {
    let mut global: Vec<&str> = GLOBAL_SOURCE_ORDER.to_vec();
    let mut with_meta: Vec<&str> = SOURCES_WITH_META.to_vec();
    global.sort_unstable();
    with_meta.sort_unstable();
    assert_eq!(global, with_meta);
}

#[test]
fn global_source_order_mirrors_migration_028_ranks() {
    // migrations/028_chapter_ownership.sql `supports_meta` global_rank order,
    // with kuaikan/kakao/mangaplus/tappytoon/webtoons/acqq/comici/naverwebtoon/
    // wuxiaworld appended (their fetch arms landed after that migration).
    assert_eq!(
        GLOBAL_SOURCE_ORDER,
        &[
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
            "wuxiaworld"
        ]
    );
}
