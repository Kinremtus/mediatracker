//! Behavioural tests for the game additions cache:
//! upsert is idempotent, reads are ordered (released NULLS LAST, then name),
//! and unknown providers are a no-op (no network, no rows).

mod common;

use mediatracker::services::additions::{fetch_and_store, get_additions, upsert_additions};
use mediatracker::services::external::AdditionRow;
use mediatracker::services::external::igdb::IgdbService;
use mediatracker::services::external::rawg::RawgService;

const PROVIDER: &str = "rawg";
const GAME_ID: &str = "555000111";

fn row(id: &str, name: &str, kind: &str, released: Option<(i32, u32, u32)>) -> AdditionRow {
    AdditionRow {
        addition_external_id: id.to_string(),
        name: name.to_string(),
        kind: kind.to_string(),
        released: released.and_then(|(y, m, d)| chrono::NaiveDate::from_ymd_opt(y, m, d)),
    }
}

async fn setup() -> common::TestContext {
    let ctx = common::TestContext::new().await;
    sqlx::query("DELETE FROM game_additions WHERE provider = $1 AND external_id = $2")
        .bind(PROVIDER)
        .bind(GAME_ID)
        .execute(&ctx.pool)
        .await
        .expect("cleanup game_additions");
    ctx
}

#[tokio::test]
async fn upsert_is_idempotent_and_refreshes_fields() {
    let ctx = setup().await;

    let first = vec![
        row("1", "Blood and Wine", "addition", Some((2016, 5, 31))),
        row("2", "Hearts of Stone", "addition", None),
    ];
    let n = upsert_additions(&ctx.pool, PROVIDER, GAME_ID, &first)
        .await
        .expect("first upsert");
    assert_eq!(n, 2);

    let second = vec![
        row(
            "1",
            "Blood and Wine (GOTY)",
            "addition",
            Some((2016, 5, 31)),
        ),
        row("2", "Hearts of Stone", "addition", None),
    ];
    upsert_additions(&ctx.pool, PROVIDER, GAME_ID, &second)
        .await
        .expect("second upsert");

    let stored = get_additions(&ctx.pool, PROVIDER, GAME_ID)
        .await
        .expect("read");
    assert_eq!(stored.len(), 2, "repeated upsert must not duplicate rows");
    assert!(
        stored
            .iter()
            .any(|r| r.addition_external_id == "1" && r.name == "Blood and Wine (GOTY)")
    );
}

#[tokio::test]
async fn get_additions_orders_dated_first_then_name() {
    let ctx = setup().await;

    let rows = vec![
        row("1", "Zzz", "addition", None),
        row("2", "Aaa", "addition", None),
        row("3", "Later", "addition", Some((2020, 1, 1))),
        row("4", "Earlier", "addition", Some((2010, 1, 1))),
    ];
    upsert_additions(&ctx.pool, PROVIDER, GAME_ID, &rows)
        .await
        .expect("upsert");

    let stored = get_additions(&ctx.pool, PROVIDER, GAME_ID)
        .await
        .expect("read");
    let names: Vec<&str> = stored.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["Earlier", "Later", "Aaa", "Zzz"],
        "dated ASC first, undated last ordered by name"
    );
}

#[tokio::test]
async fn unknown_provider_is_a_noop() {
    let ctx = setup().await;
    let rawg = RawgService::new(String::new());
    let igdb = IgdbService::new(String::new(), String::new());

    let n = fetch_and_store(&ctx.pool, "unknown", GAME_ID, &rawg, &igdb)
        .await
        .expect("unknown provider must not error");
    assert_eq!(n, 0);

    let stored = get_additions(&ctx.pool, "unknown", GAME_ID)
        .await
        .expect("read");
    assert!(stored.is_empty());
}

#[tokio::test]
async fn get_additions_empty_for_uncached_game() {
    let ctx = setup().await;
    let stored = get_additions(&ctx.pool, PROVIDER, GAME_ID)
        .await
        .expect("read");
    assert!(stored.is_empty());
}
