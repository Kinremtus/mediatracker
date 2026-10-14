//! MangaPlus (official app API) client: device registration, the full chapter
//! list / chapter count and the browse catalog.
//!
//! The MANGA Plus app talks protobuf over HTTPS and authenticates with a
//! `device_secret` obtained from `PUT /api/register`. The response bodies are
//! decoded with the vendored [`crate::services::protobuf_wire`] parser so no
//! `prost`/`protoc` dependency is pulled in.

use crate::services::external::chapter_title::{self, EpisodeRow, ShortTitle, main_count};
use crate::services::external::official_meta::{self, OfficialMeta};
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

/// Hard cap on a single catalog response body. The real response is
/// ~450 KiB; the cap defends against a misbehaving upstream streaming
/// gigabytes through `Bytes::collect`.
const MAX_CATALOG_BYTES: u64 = official_meta::MAX_DETAIL_BYTES;

/// Cached catalog snapshot plus the instant it was fetched.
type CatalogCache = Mutex<Option<(Instant, Arc<Vec<OfficialHit>>)>>;

/// In-process catalog cache. Refetched after [`CATALOG_TTL`] or on first use
/// after a restart (cheap, so no on-disk persistence).
static CATALOG: OnceLock<CatalogCache> = OnceLock::new();

/// Process-wide device token: one per process, not one per request. Derived
/// from the clock + pid the first time it is needed.
static DEVICE_TOKEN: OnceLock<String> = OnceLock::new();

/// Cached `(registered_at, device_token, device_secret)`. `PUT /api/register`
/// issues a fresh registration on every call, so a drawer open, the 6 h refresh
/// and the count provider would otherwise hammer the upstream register endpoint.
static REGISTRATION: OnceLock<Mutex<Option<(Instant, String, String)>>> = OnceLock::new();

/// How long a cached registration is reused before re-registering.
const REGISTRATION_TTL: Duration = Duration::from_secs(6 * 60 * 60);

/// The process-wide device token, derived once from the clock + pid.
pub fn device_token() -> &'static str {
    DEVICE_TOKEN.get_or_init(|| {
        let seed = format!(
            "{}:{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
            std::process::id()
        );
        md5_hex(seed.as_bytes())
    })
}

/// A cached `device_secret`, registering on first use or after
/// [`REGISTRATION_TTL`]. Shared by `fetch_meta` and the chapter-count provider.
pub async fn cached_secret(client: &reqwest::Client) -> anyhow::Result<String> {
    let cell = REGISTRATION.get_or_init(|| Mutex::new(None));
    {
        let guard = cell.lock().expect("mangaplus registration mutex poisoned");
        if let Some((at, _token, secret)) = guard.as_ref()
            && at.elapsed() < REGISTRATION_TTL
        {
            return Ok(secret.clone());
        }
    }
    let token = device_token().to_string();
    let secret = register_with(client, &token).await?;
    *cell.lock().expect("mangaplus registration mutex poisoned") =
        Some((Instant::now(), token, secret.clone()));
    Ok(secret)
}

/// `PUT /api/register` -> `RegistrationData.device_secret`.
///
/// The URL carries the device token and the derived security key, so every
/// error is stripped of its URL before it can reach a log.
pub async fn register_with(client: &reqwest::Client, device_token: &str) -> anyhow::Result<String> {
    let sk = security_key(device_token);
    let mut url = url::Url::parse(&format!("https://{API_HOST}/api/register"))?;
    url.query_pairs_mut()
        .append_pair("os", "android")
        .append_pair("os_ver", OS_VER)
        .append_pair("app_ver", APP_VER)
        .append_pair("device_token", device_token)
        .append_pair("security_key", sk.as_str());
    let resp = client
        .put(url)
        .header(reqwest::header::USER_AGENT, UA)
        .send()
        .await
        .map_err(reqwest::Error::without_url)?
        .error_for_status()
        .map_err(reqwest::Error::without_url)?;
    let bytes = resp.bytes().await.map_err(reqwest::Error::without_url)?;
    parse_device_secret(&bytes)
        .ok_or_else(|| anyhow::anyhow!("mangaplus register: no device_secret"))
}

/// `GET /api/title_detailV3` raw protobuf body, size-guarded. The URL carries
/// the secret, so errors are stripped of it (see [`register_with`]).
pub async fn title_detail_bytes(
    client: &reqwest::Client,
    secret: &str,
    title_id: &str,
) -> anyhow::Result<Vec<u8>> {
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
    let resp = client
        .get(url)
        .header(reqwest::header::USER_AGENT, UA)
        .send()
        .await
        .map_err(reqwest::Error::without_url)?
        .error_for_status()
        .map_err(reqwest::Error::without_url)?;
    if let Some(len) = resp.content_length()
        && len > official_meta::MAX_DETAIL_BYTES
    {
        anyhow::bail!("mangaplus detail too large: {len} bytes");
    }
    let bytes = resp.bytes().await.map_err(reqwest::Error::without_url)?;
    if bytes.len() as u64 > official_meta::MAX_DETAIL_BYTES {
        anyhow::bail!("mangaplus detail too large: {} bytes", bytes.len());
    }
    Ok(bytes.to_vec())
}

/// Thin async client for the MangaPlus app API (catalog search).
pub struct MangaPlusClient {
    http: reqwest::Client,
}

impl MangaPlusClient {
    /// Build a client around the shared outbound HTTP client.
    pub fn new() -> Self {
        Self {
            http: crate::services::external::http_client(),
        }
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

        let secret = cached_secret(&self.http).await?;
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

/// Number of MAIN chapters from `title_detailV3`.
///
/// Counts `chapter_list_v2` (field 38) rows whose `name` is `#NNN` — bonus
/// rows (`ex`) never inflate the count. Falls back to the legacy
/// `chapter_list_group` (field 28) summing first/mid/last lists when v2 is
/// absent. Returns `None` when neither is present so the caller keeps the
/// current count.
pub fn parse_chapter_count(body: &[u8]) -> Option<i32> {
    let success = message_field(body, 1)?;
    let view = message_field(success, 8)?;

    let rows = v2_rows(view);
    if !rows.is_empty() {
        return Some(main_count(&chapter_title::rows_to_chapters(rows)) as i32);
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

/// First string value of `field` in `msg`, trimmed.
fn string_field(msg: &[u8], field: u32) -> Option<String> {
    field_values(msg, field)
        .into_iter()
        .find_map(|value| match value {
            WireValue::Bytes(bytes) => Some(String::from_utf8_lossy(bytes).trim().to_string()),
            _ => None,
        })
}

/// First integer value of `field` in `msg` (a varint, or a numeric string as
/// protojson encodes int64).
fn int_field(msg: &[u8], field: u32) -> Option<i64> {
    field_values(msg, field)
        .into_iter()
        .find_map(|value| match value {
            WireValue::Varint(number) => i64::try_from(number).ok(),
            WireValue::Bytes(bytes) => std::str::from_utf8(bytes).ok()?.trim().parse().ok(),
            _ => None,
        })
}

/// `chapter_list_v2` (field 38) -> oldest-first rows. Each row's `name` (3) is
/// `#NNN` for a numbered chapter, anything else (`ex`) is a bonus row whose
/// `subTitle` (4) becomes its title; `startTimeStamp` (6) is unix seconds.
/// The list is oldest-first; reverse when a descending list slips through (the
/// proto carries a `chapters_descending` flag we do not read). Shared by
/// [`parse_detail`] and [`parse_chapter_count`] so the count can never disagree
/// with the chapter list.
fn v2_rows(view: &[u8]) -> Vec<EpisodeRow> {
    let mut rows: Vec<EpisodeRow> = field_values(view, 38)
        .into_iter()
        .filter_map(|value| {
            let WireValue::Bytes(chapter) = value else {
                return None;
            };
            let kind = chapter_title::classify_mangaplus_name(string_field(chapter, 3).as_deref());
            let release_date = int_field(chapter, 6)
                .and_then(|secs| chrono::DateTime::from_timestamp(secs, 0))
                .map(|dt| dt.date_naive());
            // Bonus rows carry their human label (`Bonus Chapter 1`); numbered
            // rows have no title of their own.
            let title = matches!(kind, ShortTitle::Unnumbered)
                .then(|| string_field(chapter, 4))
                .flatten();
            Some(EpisodeRow {
                title,
                release_date,
                kind,
            })
        })
        .collect();
    if is_descending(&rows) {
        rows.reverse();
    }
    rows
}

/// Decode `title_detailV3` into chapter metadata.
///
/// `chapter_list_v2` (field 38) holds the complete list: each row's `name` (3)
/// is `#NNN` for a numbered chapter, anything else (`ex`) is a bonus row whose
/// `subTitle` (4) becomes its title. `startTimeStamp` (6) is unix seconds.
/// `count` is the main count — bonuses never inflate it. An undecodable
/// payload degrades to an empty meta so the resolve chain just moves on.
pub fn parse_detail(body: &[u8]) -> OfficialMeta {
    let empty = || OfficialMeta {
        source: "mangaplus".to_string(),
        ..OfficialMeta::default()
    };
    let Some(success) = message_field(body, 1) else {
        return empty();
    };
    let Some(view) = message_field(success, 8) else {
        return empty();
    };
    let chapters = chapter_title::rows_to_chapters(v2_rows(view));
    let main = main_count(&chapters);
    if !chapters.is_empty() {
        tracing::debug!(
            source = "mangaplus",
            rows = chapters.len(),
            main,
            "chapter list parsed"
        );
    }
    OfficialMeta {
        source: "mangaplus".to_string(),
        count: (main > 0).then_some(main as i32),
        chapters,
        update_schedule: None,
        next_update_at: None,
    }
}

/// True when the first main number is ahead of the last one (descending list).
fn is_descending(rows: &[EpisodeRow]) -> bool {
    let number = |row: &EpisodeRow| match row.kind {
        ShortTitle::Base(n) => Some(n),
        _ => None,
    };
    match (
        rows.iter().find_map(number),
        rows.iter().rev().find_map(number),
    ) {
        (Some(first), Some(last)) => first > last,
        _ => false,
    }
}

/// Fetch the full chapter list for one MangaPlus title id.
///
/// The app API needs a registration handshake, so this builds its own client
/// (unlike the other `fetch_meta` arms, which reuse the shared HTTP client).
pub async fn fetch_meta(title_id: &str) -> anyhow::Result<OfficialMeta> {
    if !official_meta::is_digit_id(title_id) {
        return Ok(OfficialMeta {
            source: "mangaplus".to_string(),
            ..OfficialMeta::default()
        });
    }
    let client = crate::services::external::http_client();
    let secret = cached_secret(&client).await?;
    let body = title_detail_bytes(&client, &secret, title_id).await?;
    Ok(parse_detail(&body))
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
            hits.push(OfficialHit::new(id, name));
        }
    }
    hits
}

/// Minimal MD5 (RFC 1321) so no new dependency is added. Returns lowercase hex.
///
/// Vendored so no new dependency is added: `md-5` is only a transitive entry
/// in `Cargo.lock`, so it cannot be used without promoting it to a direct
/// dependency. The chapter-count provider reuses this function instead of
/// carrying its own copy.
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
    const BONUS_FIXTURE: &[u8] =
        include_bytes!("../../../tests/fixtures/mangaplus_detail_bonus.bin");
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
        // Real Tokyo Ghoul payload: 143 `#NNN` rows, no bonuses.
        assert_eq!(parse_chapter_count(DETAIL_FIXTURE), Some(143));
        // The bonus fixture has two numbered rows and one `ex` row.
        assert_eq!(parse_chapter_count(BONUS_FIXTURE), Some(2));
    }

    #[test]
    fn parse_detail_reads_the_real_fixture() {
        let meta = parse_detail(DETAIL_FIXTURE);
        assert_eq!(meta.source, "mangaplus");
        assert_eq!(meta.chapters.len(), 143);
        assert_eq!(meta.count, Some(143));
        assert!(meta.chapters.iter().all(|c| !c.is_bonus));
        assert_eq!(meta.chapters[0].number_x100, 100);
        assert_eq!(
            meta.chapters[0].release_date,
            official_meta::parse_date("2019-01-20")
        );
        assert_eq!(meta.chapters[142].number_x100, 14300);
    }

    #[test]
    fn parse_detail_classifies_bonus_rows() {
        let meta = parse_detail(BONUS_FIXTURE);
        assert_eq!(meta.chapters.len(), 3);
        // Main count excludes the `ex` row.
        assert_eq!(meta.count, Some(2));
        assert_eq!(meta.chapters[0].number_x100, 100);
        assert_eq!(meta.chapters[1].number_x100, 200);
        let bonus = &meta.chapters[2];
        assert!(bonus.is_bonus);
        assert_eq!(bonus.number_x100, 0);
        assert_eq!(bonus.title.as_deref(), Some("Bonus Chapter 1"));
    }

    #[test]
    fn parse_detail_is_safe_on_truncated_and_empty_buffers() {
        assert_eq!(parse_detail(&DETAIL_FIXTURE[..3]).chapters.len(), 0);
        assert_eq!(parse_detail(&[]).chapters.len(), 0);
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
