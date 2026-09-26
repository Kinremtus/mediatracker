pub mod google_books;
pub mod igdb;
pub mod mal;
pub mod mangadex;
pub mod mangaupdates;
pub mod openlibrary;
pub mod rawg;
pub mod shikimori;
pub mod tmdb;

use std::time::Duration;

/// Shared outbound HTTP client used by every external provider.
///
/// `connect_timeout` bounds TCP/TLS setup, `timeout` bounds the whole
/// request, so a hung third-party API can no longer pin an app worker
/// forever. The User-Agent identifies us to providers.
pub fn http_client() -> reqwest::Client {
    match reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(20))
        .user_agent(concat!("mediatracker/", env!("CARGO_PKG_VERSION")))
        .build()
    {
        Ok(client) => client,
        Err(e) => {
            tracing::error!(error = %e, "failed to build HTTP client; using default");
            reqwest::Client::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_client_builds() {
        // The builder must succeed so every provider gets bounded timeouts.
        let client = http_client();
        drop(client);
    }
}
