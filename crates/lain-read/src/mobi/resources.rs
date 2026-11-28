//! Resource records (images, fonts, RESC) and the EPUB files made from
//! them. Resources are numbered from 1 in record order from the
//! book's first resource record; `recindex` (MOBI 7) and
//! `kindle:embed` (KF8) both use these numbers.

use std::collections::BTreeMap;

use super::epub::Item;
use super::{MAX_RECORD, be32};

/// Fonts are de-obfuscated over this many leading bytes.
const FONT_XOR_EXTENT: usize = 1040;

pub(crate) struct Resources<'a> {
    records: Vec<&'a [u8]>,
}

impl<'a> Resources<'a> {
    pub fn new(records: Vec<&'a [u8]>) -> Resources<'a> {
        Resources { records }
    }

    /// Resource `n` (1-based).
    pub fn get(&self, n: usize) -> Option<&'a [u8]> {
        self.records.get(n.checked_sub(1)?).copied()
    }

    /// Resource `n` if it is an image: (bytes, extension, media type).
    pub fn image(&self, n: usize) -> Option<(&'a [u8], &'static str, &'static str)> {
        let data = self.get(n)?;
        let (ext, mime) = image_type(data)?;
        Some((data, ext, mime))
    }

    /// The first record that starts with `magic`.
    pub fn find(&self, magic: &[u8]) -> Option<&'a [u8]> {
        self.records.iter().copied().find(|r| r.starts_with(magic))
    }
}

/// The image type by magic bytes.
pub(crate) fn image_type(data: &[u8]) -> Option<(&'static str, &'static str)> {
    if data.starts_with(&[0xff, 0xd8, 0xff]) {
        Some(("jpg", "image/jpeg"))
    } else if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(("png", "image/png"))
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        Some(("gif", "image/gif"))
    } else if data.starts_with(b"BM") && data.len() > 26 && be32(data, 14).is_some_and(|h| h.swap_bytes() >= 12) {
        Some(("bmp", "image/bmp"))
    } else if data.len() > 12 && data.starts_with(b"RIFF") && &data[8..12] == b"WEBP" {
        Some(("webp", "image/webp"))
    } else {
        None
    }
}

/// Decodes a FONT record: optional XOR obfuscation of the leading
/// bytes, then optional zlib. Returns (bytes, extension, media type).
pub(crate) fn decode_font(rec: &[u8]) -> Option<(Vec<u8>, &'static str, &'static str)> {
    if !rec.starts_with(b"FONT") {
        return None;
    }
    let size = be32(rec, 4)? as usize;
    let flags = be32(rec, 8)?;
    let start = be32(rec, 12)? as usize;
    let mut data = rec.get(start..)?.to_vec();
    if flags & 2 != 0 {
        let (xlen, xstart) = (be32(rec, 16)? as usize, be32(rec, 20)? as usize);
        let key = rec.get(xstart..xstart.checked_add(xlen)?)?;
        if key.is_empty() {
            return None;
        }
        let n = data.len().min(FONT_XOR_EXTENT);
        for (i, b) in data[..n].iter_mut().enumerate() {
            *b ^= key[i % key.len()];
        }
    }
    if flags & 1 != 0 {
        if size > MAX_RECORD {
            return None;
        }
        data = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&data, size).ok()?;
        if data.len() != size {
            return None;
        }
    }
    let (ext, mime) = match data.get(..4)? {
        b"\0\x01\0\0" | b"true" | b"ttcf" => ("ttf", "font/ttf"),
        b"OTTO" => ("otf", "font/otf"),
        b"wOFF" => ("woff", "font/woff"),
        b"wOF2" => ("woff2", "font/woff2"),
        _ => return None,
    };
    Some((data, ext, mime))
}

/// The EPUB path of image resource `n`.
pub(crate) fn image_path(n: usize, ext: &str) -> String {
    format!("images/image{n:05}.{ext}")
}

/// The files a conversion has referenced so far, by path.
#[derive(Default)]
pub(crate) struct Used {
    pub items: BTreeMap<String, Item>,
    /// Resources that are not usable, so they are not tried again.
    failed: std::collections::BTreeSet<usize>,
}

impl Used {
    /// The path of image resource `n`, adding the file on first use.
    pub fn image(&mut self, res: &Resources<'_>, n: usize) -> Option<String> {
        let (data, ext, mime) = res.image(n)?;
        let path = image_path(n, ext);
        self.items.entry(path.clone()).or_insert_with(|| Item::new(&path, mime, data.to_vec(), false));
        Some(path)
    }

    /// The path of resource `n` as an image or a font.
    pub fn resource(&mut self, res: &Resources<'_>, n: usize) -> Option<String> {
        if let Some(p) = self.image(res, n) {
            return Some(p);
        }
        if self.failed.contains(&n) {
            return None;
        }
        let Some((data, ext, mime)) = res.get(n).and_then(decode_font) else {
            self.failed.insert(n);
            return None;
        };
        let path = format!("fonts/font{n:05}.{ext}");
        self.items.entry(path.clone()).or_insert_with(|| Item::new(&path, mime, data, true));
        Some(path)
    }

    pub fn add(&mut self, item: Item) {
        self.items.insert(item.path.clone(), item);
    }

    pub fn into_items(self) -> Vec<Item> {
        self.items.into_values().collect()
    }
}

/// `to` (a path under OEBPS) relative to a file in directory `from_dir`
/// ("" for OEBPS itself).
pub(crate) fn relative(from_dir: &str, to: &str) -> String {
    if from_dir.is_empty() {
        return to.to_string();
    }
    match to.strip_prefix(from_dir).and_then(|r| r.strip_prefix('/')) {
        Some(rest) => rest.to_string(),
        None => format!("../{to}"),
    }
}

/// Parses a base-32 number (digits then A-V, as KF8 writes them).
pub(crate) fn base32(s: &str) -> Option<usize> {
    if s.is_empty() || s.len() > 12 {
        return None;
    }
    let mut v = 0usize;
    for c in s.chars() {
        let d = c.to_digit(32)? as usize;
        v = v.checked_mul(32)?.checked_add(d)?;
    }
    Some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base32_and_relative_paths() {
        assert_eq!(base32("0001"), Some(1));
        assert_eq!(base32("000V"), Some(31));
        assert_eq!(base32("0010"), Some(32));
        assert_eq!(base32("00g"), Some(16));
        assert_eq!(base32("00W"), None);
        assert_eq!(relative("styles", "images/a.png"), "../images/a.png");
        assert_eq!(relative("images", "images/a.png"), "a.png");
        assert_eq!(relative("", "images/a.png"), "images/a.png");
    }

    #[test]
    fn image_magic() {
        assert_eq!(image_type(&[0xff, 0xd8, 0xff, 0xe0]).unwrap().0, "jpg");
        assert_eq!(image_type(b"GIF89a....").unwrap().0, "gif");
        assert!(image_type(b"FLIS\0\0\0\x08").is_none());
        assert!(image_type(b"").is_none());
    }
}
