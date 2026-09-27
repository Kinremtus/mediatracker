//! P2-O: the shared provider dispatcher must map every known provider
//! name to the right instance and refuse unknown names explicitly.

mod common;

use mediatracker::services::external::dispatch::{Provider, ProviderClients};

#[tokio::test]
async fn provider_dispatcher_maps_names_and_rejects_unknown() {
    let ctx = common::TestContext::new().await;
    let clients = ProviderClients::from_state(&ctx.state);

    for name in [
        "shikimori",
        "mal",
        "mangaupdates",
        "tmdb",
        "rawg",
        "igdb",
        "google_books",
        "openlibrary",
        "mangadex",
        "anilist",
        "comicvine",
    ] {
        let provider = Provider::from_name(&clients, name)
            .unwrap_or_else(|| panic!("provider {name} was not mapped"));
        assert_eq!(provider.name(), name);
    }

    assert!(
        Provider::from_name(&clients, "unknown_provider").is_none(),
        "unknown provider names must not resolve"
    );
}
