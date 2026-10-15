//! Official-source cover images: a strict allowlist of CDN hosts plus their
//! required `Referer`, and the rewrite onto the `/src-image` proxy.
//!
//! Some publisher CDNs refuse hotlinks (webtoon-phinf.pstatic.net answers
//! 403 without a webtoons.com referer) and some answer
//! `application/octet-stream` (cdn-public.comici.jp), so the browser cannot
//! render the stored URL directly. Official hits therefore store their cover
//! as `/src-image?url=…` (the `/tmdb-image/` precedent) and the proxy fetches
//! it server-side with the right referer and a normalized content type.
//!
//! The allowlist doubles as the SSRF guard of the proxy: only these hosts are
//! ever fetched, and only over https.

/// `(host, referer)` pairs the image proxy may fetch.
pub const IMAGE_HOSTS: &[(&str, &str)] = &[
    ("webtoon-phinf.pstatic.net", "https://www.webtoons.com/"),
    ("image-comic.pstatic.net", "https://comic.naver.com/"),
    ("dn-img-page.kakao.com", "https://webtoon.kakao.com/"),
    ("cdn-public.comici.jp", "https://championcross.jp/"),
    (
        "image-repository-cdn.tappytoon.com",
        "https://www.tappytoon.com/",
    ),
    ("cdn.wuxiaworld.com", "https://lite.wuxiaworld.com/"),
    (
        "cdn-image.alphapolis.co.jp",
        "https://www.alphapolis.co.jp/",
    ),
    ("global-api.manga-up.com", "https://global.manga-up.com/"),
];

/// The referer required by `host`, if the host is allowed at all.
pub fn referer_for(host: &str) -> Option<&'static str> {
    IMAGE_HOSTS
        .iter()
        .find(|(allowed, _)| *allowed == host)
        .map(|(_, referer)| *referer)
}

/// The stored cover URL for an official hit: the `/src-image` proxy URL when
/// the host is allowlisted, otherwise the original URL untouched (a source
/// with a plain CDN keeps its direct link).
pub fn proxied(url: &str) -> String {
    let Ok(parsed) = url::Url::parse(url) else {
        return url.to_string();
    };
    let Some(host) = parsed.host_str() else {
        return url.to_string();
    };
    if parsed.scheme() != "https" || referer_for(host).is_none() {
        return url.to_string();
    }
    // The proxy path mirrors the original one (`/src-image/{host}{path}?{query}`)
    // so nothing needs percent-encoding: the host is the allowlist check and
    // the query rides along as-is.
    let mut out = format!("/src-image/{host}{}", parsed.path());
    if let Some(query) = parsed.query() {
        out.push('?');
        out.push_str(query);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlisted_hosts_are_proxied() {
        let url = "https://webtoon-phinf.pstatic.net/2025/1320.jpg?type=q90";
        assert_eq!(
            proxied(url),
            "/src-image/webtoon-phinf.pstatic.net/2025/1320.jpg?type=q90"
        );
        // No query string: no dangling `?`.
        assert_eq!(
            proxied("https://cdn.wuxiaworld.com/images/covers/cd.webp"),
            "/src-image/cdn.wuxiaworld.com/images/covers/cd.webp"
        );
    }

    #[test]
    fn foreign_hosts_keep_their_direct_url() {
        for url in [
            "https://example.com/cover.jpg",
            "http://webtoon-phinf.pstatic.net/cover.jpg", // http is never proxied
            "not a url",
        ] {
            assert_eq!(proxied(url), url, "{url}");
        }
    }

    #[test]
    fn referers_cover_every_allowlisted_host() {
        for (host, referer) in IMAGE_HOSTS {
            assert_eq!(referer_for(host), Some(*referer), "{host}");
            assert!(referer.starts_with("https://"), "{referer}");
        }
        assert_eq!(referer_for("evil.example"), None);
    }
}
