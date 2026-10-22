//! Search catalog contract: store/lookup roundtrip, freshness gating,
//! upserts and the refresh worklist.
//!
//! Each test runs against its own throwaway Postgres container
//! (`common::TestContext`), so the fixtures below never collide with the
//! other integration suites.

mod common;

use mediatracker::models::media_item::CreateMediaItem;
use mediatracker::services::search_catalog;

fn item(provider: &str, id: &str, title: &str, media_type: &str) -> CreateMediaItem {
    CreateMediaItem {
        provider: provider.to_string(),
        external_id: id.to_string(),
        media_type: media_type.to_string(),
        title: title.to_string(),
        poster_url: Some(format!("https://example.test/{id}.jpg")),
        year: Some(2020),
        score: Some(8.5),
        description: Some(format!("about {title}")),
        ..Default::default()
    }
}

#[tokio::test]
async fn store_then_lookup_roundtrip_keeps_payload_and_official_order() {
    let ctx = common::TestContext::new().await;
    let items = vec![
        item("mangaupdates", "111", "Lore Olympus", "other-comics"),
        item("webtoons", "1320", "Lore Olympus", "other-comics"),
    ];
    search_catalog::store(&ctx.pool, "other-comics", "Lore Olympus", &items)
        .await
        .expect("store");

    let got = search_catalog::lookup(&ctx.pool, "other-comics", "Lore Olympus")
        .await
        .expect("lookup")
        .expect("a fresh query answers from the catalog");
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].provider, "webtoons", "official hits come first");
    assert_eq!(got[1].provider, "mangaupdates");
    // Payload fields survive the JSONB roundtrip.
    assert_eq!(
        got[0].poster_url.as_deref(),
        Some("https://example.test/1320.jpg")
    );
    assert_eq!(got[0].description.as_deref(), Some("about Lore Olympus"));
    assert_eq!(got[0].year, Some(2020));
    assert_eq!(got[0].score, Some(8.5));
}

#[tokio::test]
async fn lookup_matches_by_normalized_query_and_scope() {
    let ctx = common::TestContext::new().await;
    let items = vec![item("webtoons", "1320", "Lore Olympus", "other-comics")];
    search_catalog::store(&ctx.pool, "other-comics", "Lore Olympus", &items)
        .await
        .expect("store");

    // The same query — after normalization — is served from the catalog.
    for query in [
        "Lore Olympus",
        "lore olympus",
        "LORE OLYMPUS",
        "lore  olympus!",
    ] {
        let got = search_catalog::lookup(&ctx.pool, "other-comics", query)
            .await
            .expect("lookup")
            .unwrap_or_else(|| panic!("{query} must answer from the catalog"));
        assert_eq!(got.len(), 1, "{query}");
    }
    // A different query is NOT fresh: it goes live to discover its own hits
    // (serving "lore" from the "Lore Olympus" row would freeze the catalog and
    // never surface new titles for broader queries).
    assert!(
        search_catalog::lookup(&ctx.pool, "other-comics", "lore")
            .await
            .expect("lookup")
            .is_none()
    );
    // Another media type is not leaked.
    assert!(
        search_catalog::lookup(&ctx.pool, "manhwa", "Lore Olympus")
            .await
            .expect("lookup")
            .is_none()
    );
    // A punctuation-only query answers empty without a live round-trip.
    let empty = search_catalog::lookup(&ctx.pool, "other-comics", "!!!")
        .await
        .expect("lookup")
        .expect("a punctuation-only query answers empty without a live round-trip");
    assert!(empty.is_empty());
}

#[tokio::test]
async fn lookup_returns_the_whole_live_answer_not_only_title_matches() {
    let ctx = common::TestContext::new().await;
    // A live provider search returns fuzzy matches: most titles do NOT contain
    // the whole query string (the old `title LIKE` lookup showed 1 of 24).
    let items = vec![
        item(
            "mangaupdates",
            "1",
            "Tsuki ga Michibiku Isekai Douchuu",
            "manga",
        ),
        item("mangaupdates", "2", "Jouou ga Michibiku Koi", "manga"),
        item("mangaupdates", "3", "Muchuu Douchuu", "manga"),
    ];
    search_catalog::store(
        &ctx.pool,
        "manga",
        "Tsuki ga Michibiku Isekai Douchuu",
        &items,
    )
    .await
    .expect("store");

    let got = search_catalog::lookup(&ctx.pool, "manga", "Tsuki ga Michibiku Isekai Douchuu")
        .await
        .expect("lookup")
        .expect("a fresh query answers from the catalog");
    assert_eq!(got.len(), 3, "the stored answer set is replayed in full");
}

#[tokio::test]
async fn overlapping_queries_keep_their_own_answers() {
    let ctx = common::TestContext::new().await;
    let a = item("mangaupdates", "1", "Lore Olympus", "other-comics");
    let b = item("mangaupdates", "2", "Olympus Heights", "other-comics");
    let c = item("mangaupdates", "3", "Lore Legends", "other-comics");

    search_catalog::store(&ctx.pool, "other-comics", "Lore Olympus", &[a, b.clone()])
        .await
        .expect("store Q1");
    // A second live search re-upserts `b`; with only `last_query` that row
    // would leave Q1's answer.
    search_catalog::store(&ctx.pool, "other-comics", "Olympus Heights", &[b, c])
        .await
        .expect("store Q2");

    let q1 = search_catalog::lookup(&ctx.pool, "other-comics", "Lore Olympus")
        .await
        .expect("lookup Q1")
        .expect("fresh");
    assert_eq!(q1.len(), 2, "Q1 keeps its own answer");
    assert!(q1.iter().any(|i| i.title == "Lore Olympus"));
    assert!(q1.iter().any(|i| i.title == "Olympus Heights"));

    let q2 = search_catalog::lookup(&ctx.pool, "other-comics", "Olympus Heights")
        .await
        .expect("lookup Q2")
        .expect("fresh");
    assert_eq!(q2.len(), 2);
    assert!(
        !q2.iter().any(|i| i.title == "Lore Olympus"),
        "Q2 must not inherit Q1-only rows"
    );
}

#[tokio::test]
async fn stale_query_goes_live_and_empty_answers_are_not_recorded() {
    let ctx = common::TestContext::new().await;
    let items = vec![item("webtoons", "1320", "Lore Olympus", "other-comics")];
    search_catalog::store(&ctx.pool, "other-comics", "Lore Olympus", &items)
        .await
        .expect("store");

    // Past the TTL the lookup declines, so the caller runs the live search.
    sqlx::query("UPDATE search_queries SET fetched_at = NOW() - INTERVAL '25 hours'")
        .execute(&ctx.pool)
        .await
        .expect("backdate");
    assert!(
        search_catalog::lookup(&ctx.pool, "other-comics", "Lore Olympus")
            .await
            .expect("lookup")
            .is_none(),
        "a stale query must not be served from the catalog"
    );

    // An empty live answer is not recorded: a transient all-fail must not
    // hide the title for a whole TTL window.
    search_catalog::store(&ctx.pool, "manhwa", "Nothing Found", &[])
        .await
        .expect("store empty");
    assert!(
        search_catalog::lookup(&ctx.pool, "manhwa", "Nothing Found")
            .await
            .expect("lookup")
            .is_none()
    );
}

#[tokio::test]
async fn store_upserts_and_refreshes_the_query() {
    let ctx = common::TestContext::new().await;
    let first = vec![item("webtoons", "1320", "Lore Olympus", "other-comics")];
    search_catalog::store(&ctx.pool, "other-comics", "Lore Olympus", &first)
        .await
        .expect("first store");

    sqlx::query("UPDATE search_queries SET fetched_at = NOW() - INTERVAL '25 hours'")
        .execute(&ctx.pool)
        .await
        .expect("backdate");

    let mut second = item("webtoons", "1320", "Lore Olympus", "other-comics");
    second.poster_url = Some("https://example.test/new.jpg".to_string());
    search_catalog::store(&ctx.pool, "other-comics", "Lore Olympus", &[second])
        .await
        .expect("second store");

    let got = search_catalog::lookup(&ctx.pool, "other-comics", "Lore Olympus")
        .await
        .expect("lookup")
        .expect("re-stored query is fresh again");
    assert_eq!(
        got.len(),
        1,
        "the same (provider, id) is upserted, not duplicated"
    );
    assert_eq!(
        got[0].poster_url.as_deref(),
        Some("https://example.test/new.jpg")
    );
}

#[tokio::test]
async fn apply_refresh_upserts_and_always_touches() {
    let ctx = common::TestContext::new().await;
    let original = vec![item("webtoons", "1320", "Lore Olympus", "other-comics")];
    search_catalog::store(&ctx.pool, "other-comics", "Lore Olympus", &original)
        .await
        .expect("store");

    // Stale it, then refresh with fresh data: upsert + query fresh again.
    sqlx::query("UPDATE search_queries SET fetched_at = NOW() - INTERVAL '25 hours'")
        .execute(&ctx.pool)
        .await
        .expect("backdate");
    let mut fresh = item("webtoons", "1320", "Lore Olympus", "other-comics");
    fresh.poster_url = Some("https://example.test/refreshed.jpg".to_string());
    search_catalog::apply_refresh(&ctx.pool, "other-comics", "Lore Olympus", &[fresh])
        .await
        .expect("apply refresh");
    let got = search_catalog::lookup(&ctx.pool, "other-comics", "Lore Olympus")
        .await
        .expect("lookup")
        .expect("the refreshed query answers from the catalog again");
    assert_eq!(
        got[0].poster_url.as_deref(),
        Some("https://example.test/refreshed.jpg")
    );

    // Stale again, refresh with an EMPTY answer: the data is kept (the catalog
    // never deletes) and the query is still bumped, so a query whose sources
    // went silent cannot monopolise the oldest refresh slot.
    sqlx::query("UPDATE search_queries SET fetched_at = NOW() - INTERVAL '25 hours'")
        .execute(&ctx.pool)
        .await
        .expect("backdate");
    search_catalog::apply_refresh(&ctx.pool, "other-comics", "Lore Olympus", &[])
        .await
        .expect("apply empty refresh");
    let got = search_catalog::lookup(&ctx.pool, "other-comics", "Lore Olympus")
        .await
        .expect("lookup")
        .expect("the touched query is fresh");
    assert_eq!(got.len(), 1, "the catalog keeps its last known data");
    assert_eq!(
        got[0].poster_url.as_deref(),
        Some("https://example.test/refreshed.jpg")
    );
}

#[tokio::test]
async fn find_item_returns_the_stored_payload() {
    let ctx = common::TestContext::new().await;
    let items = vec![item("webtoons", "1320", "Lore Olympus", "other-comics")];
    search_catalog::store(&ctx.pool, "other-comics", "Lore Olympus", &items)
        .await
        .expect("store");

    let found = search_catalog::find_item(&ctx.pool, "webtoons", "1320")
        .await
        .expect("find")
        .expect("row exists");
    assert_eq!(found.title, "Lore Olympus");
    assert_eq!(found.media_type, "other-comics");
    assert_eq!(
        found.poster_url.as_deref(),
        Some("https://example.test/1320.jpg")
    );
    assert!(
        search_catalog::find_item(&ctx.pool, "webtoons", "9999")
            .await
            .expect("find")
            .is_none()
    );
}

#[tokio::test]
async fn stale_queries_are_oldest_first_and_bounded() {
    let ctx = common::TestContext::new().await;
    for (query, age_hours) in [("one", 30), ("two", 50), ("three", 40), ("fresh", 1)] {
        search_catalog::store(
            &ctx.pool,
            "manhwa",
            query,
            &[item("webtoons", query, query, "manhwa")],
        )
        .await
        .expect("store");
        sqlx::query("UPDATE search_queries SET fetched_at = NOW() - make_interval(hours => $1) WHERE query_norm = $2")
            .bind(age_hours)
            .bind(query)
            .execute(&ctx.pool)
            .await
            .expect("backdate");
    }

    let stale = search_catalog::stale_queries(&ctx.pool, 2)
        .await
        .expect("stale list");
    assert_eq!(stale.len(), 2, "the limit is honoured");
    assert_eq!(stale[0].query_text, "two", "oldest first");
    assert_eq!(stale[1].query_text, "three");
    assert_eq!(stale[0].media_type, "manhwa");

    // Touching a refreshed query removes it from the worklist.
    search_catalog::touch_query(&ctx.pool, "manhwa", "two")
        .await
        .expect("touch");
    let stale = search_catalog::stale_queries(&ctx.pool, 10)
        .await
        .expect("stale list");
    assert_eq!(
        stale
            .iter()
            .map(|q| q.query_text.as_str())
            .collect::<Vec<_>>(),
        vec!["three", "one"]
    );
}
