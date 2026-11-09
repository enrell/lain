//! Token primitives. HMAC-SHA256 comes from RustCrypto (`sha2`,
//! `hmac`) and randomness from `getrandom` (D-173). The JWT segments'
//! URL-safe unpadded base64 is a small in-tree function: no approved
//! crate exposes one (D-177).

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

fn mac(key: &[u8], msg: &[u8]) -> Hmac<Sha256> {
    let mut m = <Hmac<Sha256> as KeyInit>::new_from_slice(key).expect("HMAC takes keys of any length");
    m.update(msg);
    m
}

/// HMAC-SHA256.
pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    mac(key, msg).finalize().into_bytes().into()
}

/// Constant-time check of an HMAC-SHA256 tag.
pub fn hmac_sha256_verify(key: &[u8], msg: &[u8], tag: &[u8]) -> bool {
    mac(key, msg).verify_slice(tag).is_ok()
}

const URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// URL-safe base64 without padding (JWT segments).
pub fn b64_url(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        let chars = [(n >> 18) & 63, (n >> 12) & 63, (n >> 6) & 63, n & 63];
        for c in &chars[..=chunk.len()] {
            out.push(URL[*c as usize] as char);
        }
    }
    out
}

/// Strict decoding of [`b64_url`]: no padding, no other alphabet, and
/// unused trailing bits must be zero.
pub fn b64_url_decode(s: &str) -> Option<Vec<u8>> {
    let body = s.as_bytes();
    if body.len() % 4 == 1 {
        return None;
    }
    let mut lookup = [255u8; 256];
    for (i, c) in URL.iter().enumerate() {
        lookup[*c as usize] = i as u8;
    }
    let mut out = Vec::with_capacity(body.len() * 3 / 4);
    for chunk in body.chunks(4) {
        let mut n = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            let v = lookup[*c as usize];
            if v == 255 {
                return None;
            }
            n |= (v as u32) << (18 - 6 * i);
        }
        let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        let take = chunk.len() - 1;
        if take < 3 && bytes[take..].iter().any(|b| *b != 0) {
            return None;
        }
        out.extend_from_slice(&bytes[..take]);
    }
    Some(out)
}

/// Fills `buf` from the operating system's CSPRNG.
pub fn random_bytes(buf: &mut [u8]) -> std::io::Result<()> {
    getrandom::fill(buf).map_err(|e| std::io::Error::other(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn primitives_match_published_vectors() {
        let tag = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(hex(&tag), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
        assert!(hmac_sha256_verify(b"Jefe", b"what do ya want for nothing?", &tag));
        assert!(!hmac_sha256_verify(b"Jefe", b"what do ya want for nothing!", &tag));
    }

    #[test]
    fn base64_rfc4648() {
        for (raw, url) in [
            ("", ""),
            ("f", "Zg"),
            ("fo", "Zm8"),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg"),
            ("fooba", "Zm9vYmE"),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(b64_url(raw.as_bytes()), url);
            assert_eq!(b64_url_decode(url).unwrap(), raw.as_bytes());
        }
        assert_eq!(b64_url(&[0xfb, 0xff]), "-_8");
        assert!(b64_url_decode("Zg==").is_none());
        assert!(b64_url_decode("Zh").is_none(), "non-zero trailing bits are rejected");
        assert!(b64_url_decode("a").is_none());
        assert!(b64_url_decode("ab$d").is_none());
    }

    #[test]
    fn randomness_is_not_constant() {
        let (mut a, mut b) = ([0u8; 32], [0u8; 32]);
        random_bytes(&mut a).unwrap();
        random_bytes(&mut b).unwrap();
        assert_ne!(a, b);
    }
}
