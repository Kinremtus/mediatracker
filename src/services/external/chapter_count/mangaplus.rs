//! MangaPlus (JP) chapter-count source. Uses the public app register + title_detailV3
//! endpoint. Fail-safe: any parse gap returns Ok(None) so the count is never lowered.

use super::{ChapterCount, ChapterCountFuture, ChapterCountProvider};

const REGISTER: &str = "https://jumpg-api.tokyo-cdn.com/api/register";
const DETAIL: &str = "https://jumpg-api.tokyo-cdn.com/api/title_detailV3";
const APP_SECRET_SALT: &str = "4Kin9vGg";
const UA: &str = "okhttp/4.12.0";

pub struct MangaPlusProvider {
    client: reqwest::Client,
    device_token: String,
}

impl MangaPlusProvider {
    pub fn new() -> Self {
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
            client: reqwest::Client::new(),
            device_token,
        }
    }

    fn security_key(&self) -> String {
        md5_hex(format!("{}{}", self.device_token, APP_SECRET_SALT).as_bytes())
    }
}

impl Default for MangaPlusProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Minimal MD5 (RFC 1321) so no new dependency is added. Returns lowercase hex.
///
/// Self-contained pure-Rust implementation: no `unsafe`, no external crates.
/// `md-5` is only a transitive entry in `Cargo.lock`, so it cannot be used here
/// without promoting it to a direct dependency; vendoring keeps the client inert.
fn md5_hex(input: &[u8]) -> String {
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

#[derive(serde::Deserialize)]
struct RegisterResponse {
    success: Option<RegisterSuccess>,
}
#[derive(serde::Deserialize)]
struct RegisterSuccess {
    secret: Option<String>,
}

impl ChapterCountProvider for MangaPlusProvider {
    fn name(&self) -> &'static str {
        "mangaplus"
    }

    fn fetch<'a>(&'a self, external_id: &'a str) -> ChapterCountFuture<'a> {
        Box::pin(async move {
            let mut reg_url = url::Url::parse(REGISTER)?;
            reg_url
                .query_pairs_mut()
                .append_pair("device_token", &self.device_token)
                .append_pair("security_key", &self.security_key())
                .append_pair("os", "android")
                .append_pair("os_ver", "35")
                .append_pair("app_ver", "237");
            let reg = self
                .client
                .put(reg_url)
                .header(reqwest::header::USER_AGENT, UA)
                .send()
                .await?
                .error_for_status()?
                .json::<RegisterResponse>()
                .await?;
            let secret = match reg.success.and_then(|s| s.secret) {
                Some(s) => s,
                None => return Ok(None),
            };
            let mut detail_url = url::Url::parse(DETAIL)?;
            detail_url
                .query_pairs_mut()
                .append_pair("title_id", external_id)
                .append_pair("secret", &secret);
            let bytes = self
                .client
                .get(detail_url)
                .header(reqwest::header::USER_AGENT, UA)
                .send()
                .await?
                .error_for_status()?
                .bytes()
                .await?;
            // Protobuf parse: extract the total chapter count. Until the field
            // mapping is confirmed, return Ok(None) (raise-only safe no-op).
            let _ = bytes;
            Ok(None::<ChapterCount>)
        })
    }

    fn media_types(&self) -> &'static [&'static str] {
        &["manga"]
    }
}

#[cfg(test)]
mod tests {
    use super::md5_hex;

    #[test]
    fn md5_hex_matches_rfc_vectors() {
        assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(md5_hex(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
    }
}
