//! MangaPlus (official app API) client: device registration, chapter count and
//! the full browse catalog.
//!
//! The MANGA Plus app talks protobuf over HTTPS and authenticates with a
//! `device_secret` obtained from `PUT /api/register`. The response bodies are
//! decoded with the vendored [`crate::services::protobuf_wire`] parser so no
//! `prost`/`protoc` dependency is pulled in.
//!
//! This module is intentionally not registered in `external/mod.rs` yet
//! (task 2.5 wires it in).

use crate::services::official_types::OfficialHit;
use crate::services::protobuf_wire::{WireValue, field_values, message_field};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// API host used by every endpoint below.
pub const API_HOST: &str = "jumpg-api.tokyo-cdn.com";

/// Salt baked into the official Android app. `security_key` is
/// `md5(device_token + APP_SECRET_SALT)`.
pub const APP_SECRET_SALT: &str = "4Kin9vGg";

/// Android OkHttp User-Agent the app sends. Arbitrary UAs get a 400 from the
/// image CDN; the JSON/protobuf API is more lenient but we mirror the app.
pub const UA: &str = "okhttp/4.12.0";

/// `app_ver` query param. Pinned to the version we reverse-engineered.
pub const APP_VER: &str = "237";

/// `os_ver` query param (`Build.VERSION.SDK_INT`).
pub const OS_VER: &str = "35";

/// How long a catalog snapshot stays fresh before a refetch.
const CATALOG_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Hard cap on a single catalog response body (4 MiB). The real response is
/// ~450 KiB; the cap defends against a misbehaving upstream streaming
/// gigabytes through `Bytes::collect`.
const MAX_CATALOG_BYTES: u64 = 4 * 1024 * 1024;

/// Cached catalog snapshot plus the instant it was fetched.
type CatalogCache = Mutex<Option<(Instant, Arc<Vec<OfficialHit>>)>>;

/// In-process catalog cache. Refetched after [`CATALOG_TTL`] or on first use
/// after a restart (cheap, so no on-disk persistence).
static CATALOG: OnceLock<CatalogCache> = OnceLock::new();

/// Thin async client for the MangaPlus app API.
pub struct MangaPlusClient {
    http: reqwest::Client,
    device_token: String,
}

impl MangaPlusClient {
    /// Build a client with a freshly derived device token.
    pub fn new() -> Self {
        // DEVIATION FROM PLAN: the plan prose says the reqwest client should
        // carry `cookie_store(true)`. We deliberately omit it here: `/register`
        // and `/title_detailV3` need no cookies, and only the image reader
        // (`manga_viewer_v3` -> jumpg-assets3) depends on the `plus_vw_token`
        // cookie. Omitting it avoids enabling the reqwest `cookies` feature.
        let seed = format!(
            "{}:{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
            std::process::id()
        );
        let device_token = md5_hex(seed.as_bytes());
        Self {
            http: crate::services::external::http_client(),
            device_token,
        }
    }

    /// `PUT /api/register` -> `RegistrationData.device_secret`.
    ///
    /// `device_token` is an opaque per-install key; the server treats it as a
    /// key, not something validated against device attestation.
    pub async fn register(&self, device_token: &str) -> anyhow::Result<String> {
        // reqwest 0.13 gates `.query()` behind the optional `query` feature,
        // which this crate does not enable, so the query string is built with
        // the `url` crate (same pattern as chapter_count/mangaplus.rs).
        let sk = security_key(device_token);
        let mut url = url::Url::parse(&format!("https://{API_HOST}/api/register"))?;
        url.query_pairs_mut()
            .append_pair("os", "android")
            .append_pair("os_ver", OS_VER)
            .append_pair("app_ver", APP_VER)
            .append_pair("device_token", device_token)
            .append_pair("security_key", sk.as_str());
        let resp = self
            .http
            .put(url)
            .header(reqwest::header::USER_AGENT, UA)
            .send()
            .await?
            .error_for_status()?;
        let bytes = resp.bytes().await?;
        parse_device_secret(&bytes)
            .ok_or_else(|| anyhow::anyhow!("mangaplus register: no device_secret"))
    }

    /// `GET /api/title_detailV3` -> total chapter count.
    ///
    /// Returns `Ok(None)` when the payload is missing/undecodable so the
    /// caller can keep the existing count (raise-only safe).
    pub async fn title_chapter_count(
        &self,
        secret: &str,
        title_id: &str,
    ) -> anyhow::Result<Option<i32>> {
        let mut url = url::Url::parse(&format!("https://{API_HOST}/api/title_detailV3"))?;
        url.query_pairs_mut()
            .append_pair("os", "android")
            .append_pair("os_ver", OS_VER)
            .append_pair("app_ver", APP_VER)
            .append_pair("title_id", title_id)
            .append_pair("secret", secret)
            .append_pair("lang", "eng")
            .append_pair("clang", "eng")
            .append_pair("country_code", "US");
        let resp = self
            .http
            .get(url)
            .header(reqwest::header::USER_AGENT, UA)
            .send()
            .await?
            .error_for_status()?;
        let bytes = resp.bytes().await?;
        Ok(parse_chapter_count(&bytes))
    }

    /// Full browse catalog, merged across the `serializing` and `completed`
    /// buckets and cached in-process for [`CATALOG_TTL`].
    pub async fn catalog(&self) -> anyhow::Result<Arc<Vec<OfficialHit>>> {
        let cell = CATALOG.get_or_init(|| Mutex::new(None));
        {
            let guard = cell.lock().expect("mangaplus catalog mutex poisoned");
            if let Some((at, hits)) = guard.as_ref()
                && at.elapsed() < CATALOG_TTL
            {
                return Ok(hits.clone());
            }
        }

        let secret = self.register(&self.device_token).await?;
        let mut merged: Vec<OfficialHit> = Vec::new();

        // DEVIATION FROM PLAN PROSE: the plan names `/api/title_list/search`
        // as the catalog endpoint, but per the proto comment `search`
        // populates `contents` (field 5), while only `/title_list/all_v3`
        // populates `all_titles_group` (field 3) - which is exactly what
        // `parse_catalog` and the fixtures use. So `all_v3` is required.
        for bucket in ["serializing", "completed"] {
            let mut url = url::Url::parse(&format!("https://{API_HOST}/api/title_list/all_v3"))?;
            url.query_pairs_mut()
                .append_pair("type", bucket)
                .append_pair("lang", "eng")
                .append_pair("clang", "eng")
                .append_pair("os", "android")
                .append_pair("os_ver", OS_VER)
                .append_pair("app_ver", APP_VER)
                .append_pair("secret", secret.as_str());
            let resp = self
                .http
                .get(url)
                .header(reqwest::header::USER_AGENT, UA)
                .send()
                .await?
                .error_for_status()?;

            if let Some(len) = resp.content_length()
                && len > MAX_CATALOG_BYTES
            {
                anyhow::bail!(
                    "mangaplus catalog bucket {bucket} too large: {len} bytes (cap {MAX_CATALOG_BYTES})"
                );
            }
            let bytes = resp.bytes().await?;
            if bytes.len() as u64 > MAX_CATALOG_BYTES {
                anyhow::bail!(
                    "mangaplus catalog bucket {bucket} too large: {} bytes (cap {MAX_CATALOG_BYTES})",
                    bytes.len()
                );
            }
            merged.extend(parse_catalog(&bytes));
        }

        // Dedup by id: the two buckets occasionally overlap (a title that was
        // completed and later resumed).
        let mut seen = std::collections::HashSet::new();
        merged.retain(|hit| seen.insert(hit.id.clone()));

        let hits = Arc::new(merged);
        {
            let mut guard = cell.lock().expect("mangaplus catalog mutex poisoned");
            *guard = Some((Instant::now(), hits.clone()));
        }
        Ok(hits)
    }
}

impl Default for MangaPlusClient {
    fn default() -> Self {
        Self::new()
    }
}

/// `md5(device_token + APP_SECRET_SALT)`, lowercase hex.
pub fn security_key(device_token: &str) -> String {
    md5_hex(format!("{device_token}{APP_SECRET_SALT}").as_bytes())
}

/// Decode `Response.success` (1) -> `RegistrationData` (2) -> `device_secret` (1).
pub fn parse_device_secret(body: &[u8]) -> Option<String> {
    let success = message_field(body, 1)?;
    let registration = message_field(success, 2)?;
    let raw = message_field(registration, 1)?;
    let secret = String::from_utf8_lossy(raw).trim().to_string();
    if secret.is_empty() {
        None
    } else {
        Some(secret)
    }
}

/// Decode the chapter count from `title_detailV3`.
///
/// Prefers `chapter_list_v2` (field 38); falls back to the legacy
/// `chapter_list_group` (field 28) summing first/mid/last lists. Returns
/// `None` when neither is present so the caller keeps the current count.
pub fn parse_chapter_count(body: &[u8]) -> Option<i32> {
    let success = message_field(body, 1)?;
    let view = message_field(success, 8)?;

    let v2 = field_values(view, 38);
    let v2_count = v2
        .iter()
        .filter(|value| matches!(value, WireValue::Bytes(_)))
        .count();
    if v2_count > 0 {
        return Some(v2_count as i32);
    }

    let group = message_field(view, 28)?;
    let legacy_count: usize = [2u32, 3, 4]
        .iter()
        .map(|&field| {
            field_values(group, field)
                .iter()
                .filter(|value| matches!(value, WireValue::Bytes(_)))
                .count()
        })
        .sum();
    if legacy_count > 0 {
        Some(legacy_count as i32)
    } else {
        None
    }
}

/// Decode `Response.success` (1) -> `SearchView` (35) -> `all_titles_group` (3)
/// -> each group's `titles` (2) -> `{title_id (1), name (2)}`.
pub fn parse_catalog(body: &[u8]) -> Vec<OfficialHit> {
    let mut hits = Vec::new();
    let Some(success) = message_field(body, 1) else {
        return hits;
    };
    let Some(search_view) = message_field(success, 35) else {
        return hits;
    };

    for group in field_values(search_view, 3) {
        let WireValue::Bytes(group) = group else {
            continue;
        };
        for title in field_values(group, 2) {
            let WireValue::Bytes(title) = title else {
                continue;
            };
            let id = field_values(title, 1)
                .into_iter()
                .find_map(|value| match value {
                    WireValue::Varint(number) => Some(number.to_string()),
                    _ => None,
                });
            let Some(id) = id else {
                continue;
            };
            let name = message_field(title, 2)
                .map(|raw| String::from_utf8_lossy(raw).trim().to_string())
                .unwrap_or_default();
            hits.push(OfficialHit { id, title: name });
        }
    }
    hits
}

/// Minimal MD5 (RFC 1321) so no new dependency is added. Returns lowercase hex.
///
/// Vendored verbatim from `chapter_count/mangaplus.rs`: `md-5` is only a
/// transitive entry in `Cargo.lock`, so it cannot be used without promoting it
/// to a direct dependency.
pub(crate) fn md5_hex(input: &[u8]) -> String {
    // Per-round left-rotation amounts (RFC 1321, section 3.4).
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];

    // K[i] = floor(2^32 * abs(sin(i + 1))), for i in 0..64 (RFC 1321, section 3.4).
    const K: [u32; 64] = [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613,
        0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193,
        0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d,
        0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
        0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122,
        0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa,
        0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244,
        0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
        0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
        0xeb86d391,
    ];

    let mut a0: u32 = 0x67452301;
    let mut b0: u32 = 0xefcdab89;
    let mut c0: u32 = 0x98badcfe;
    let mut d0: u32 = 0x10325476;

    // Pre-processing: append 0x80, pad with zeros to 56 mod 64, then the
    // 64-bit little-endian message length in bits.
    let mut msg = input.to_vec();
    let bit_len = (input.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_le_bytes());

    for chunk in msg.as_chunks::<64>().0 {
        let mut m = [0u32; 16];
        for (i, word) in m.iter_mut().enumerate() {
            let off = i * 4;
            *word =
                u32::from_le_bytes([chunk[off], chunk[off + 1], chunk[off + 2], chunk[off + 3]]);
        }

        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);

        for i in 0..64 {
            let (f, g) = match i {
                0..=15 => ((b & c) | (!b & d), i),
                16..=31 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let tmp = d;
            d = c;
            c = b;
            let sum = a.wrapping_add(f).wrapping_add(K[i]).wrapping_add(m[g]);
            b = b.wrapping_add(sum.rotate_left(S[i]));
            a = tmp;
        }

        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }

    // Digest bytes are the four state words serialized little-endian.
    let mut out = String::with_capacity(32);
    for word in [a0, b0, c0, d0] {
        for byte in word.to_le_bytes() {
            out.push_str(&format!("{byte:02x}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const REGISTER_FIXTURE: &[u8] =
        include_bytes!("../../../tests/fixtures/mangaplus_register.bin");
    const DETAIL_FIXTURE: &[u8] =
        include_bytes!("../../../tests/fixtures/mangaplus_title_detail.bin");
    const SEARCH_FIXTURE: &[u8] =
        include_bytes!("../../../tests/fixtures/mangaplus_search_view.bin");

    #[test]
    fn md5_hex_matches_rfc_vectors() {
        assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(md5_hex(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
    }

    #[test]
    fn security_key_matches_official_derivation() {
        // md5("abc" + "4Kin9vGg")
        assert_eq!(security_key("abc"), "72f176b1b24f0dcb9307f307490004fd");
    }

    #[test]
    fn parse_device_secret_reads_register_fixture() {
        assert_eq!(
            parse_device_secret(REGISTER_FIXTURE),
            Some("0123456789abcdef0123456789abcdef".to_string())
        );
    }

    #[test]
    fn parse_device_secret_is_safe_on_truncated_and_empty_buffers() {
        assert_eq!(parse_device_secret(&REGISTER_FIXTURE[..3]), None);
        assert_eq!(parse_device_secret(&[]), None);
    }

    #[test]
    fn parse_chapter_count_reads_detail_fixture() {
        assert_eq!(parse_chapter_count(DETAIL_FIXTURE), Some(5));
    }

    #[test]
    fn parse_chapter_count_is_safe_on_truncated_and_empty_buffers() {
        assert_eq!(parse_chapter_count(&DETAIL_FIXTURE[..3]), None);
        assert_eq!(parse_chapter_count(&[]), None);
    }

    #[test]
    fn parse_catalog_reads_search_fixture() {
        let hits = parse_catalog(SEARCH_FIXTURE);
        assert_eq!(hits.len(), 2);
        let ids: Vec<&str> = hits.iter().map(|hit| hit.id.as_str()).collect();
        assert_eq!(ids, ["100020", "100838"]);
        let titles: Vec<&str> = hits.iter().map(|hit| hit.title.as_str()).collect();
        assert_eq!(titles, ["One Piece", "My Hero Academia"]);
    }

    #[test]
    fn parse_catalog_is_safe_on_truncated_and_empty_buffers() {
        assert!(parse_catalog(&SEARCH_FIXTURE[..3]).is_empty());
        assert!(parse_catalog(&[]).is_empty());
    }
}
