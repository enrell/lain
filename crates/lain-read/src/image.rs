//! Image headers without decoding pixels: the format and dimensions of
//! PNG, JPEG, GIF and WebP, read from the first bytes of a file. Page
//! indexes report them so clients lay out before loading, and avatar
//! uploads are identified by them, never by a client's Content-Type.

use crate::zip::crc32;

/// How many leading bytes a header probe needs at most (a JPEG frame
/// header can sit behind large EXIF or ICC segments).
pub const HEAD_BYTES: usize = 1 << 20;

/// The image format of `b` and its width and height, for the formats
/// Go's `image.DecodeConfig` reads (PNG, JPEG baseline/extended/
/// progressive, GIF). WebP is [`webp_size`].
pub fn decode_config(b: &[u8]) -> Option<(&'static str, u32, u32)> {
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        return png(b).map(|(w, h)| ("png", w, h));
    }
    if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        return gif(b).map(|(w, h)| ("gif", w, h));
    }
    if b.starts_with(&[0xff, 0xd8]) {
        return jpeg(b).map(|(w, h)| ("jpeg", w, h));
    }
    None
}

fn be16(b: &[u8], i: usize) -> u32 {
    u32::from(u16::from_be_bytes([b[i], b[i + 1]]))
}

fn be32(b: &[u8], i: usize) -> u32 {
    u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

fn le16(b: &[u8], i: usize) -> u32 {
    u32::from(u16::from_le_bytes([b[i], b[i + 1]]))
}

fn png(b: &[u8]) -> Option<(u32, u32)> {
    // The IHDR chunk comes first: length 13, type, payload, CRC.
    if b.len() < 33 || be32(b, 8) != 13 || &b[12..16] != b"IHDR" {
        return None;
    }
    if crc32(&b[12..29]) != be32(b, 29) {
        return None;
    }
    let (w, h, depth, color) = (be32(b, 16), be32(b, 20), b[24], b[25]);
    let ok = match color {
        0 => matches!(depth, 1 | 2 | 4 | 8 | 16),
        3 => matches!(depth, 1 | 2 | 4 | 8),
        2 | 4 | 6 => matches!(depth, 8 | 16),
        _ => false,
    };
    (ok && w <= i32::MAX as u32 && h <= i32::MAX as u32).then_some((w, h))
}

fn gif(b: &[u8]) -> Option<(u32, u32)> {
    if b.len() < 13 {
        return None;
    }
    let flags = b[10];
    if flags & 0x80 != 0 {
        // The global color table must be present.
        let table = 3 * (1usize << ((flags & 7) + 1));
        if b.len() < 13 + table {
            return None;
        }
    }
    Some((le16(b, 6), le16(b, 8)))
}

fn jpeg(b: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2;
    loop {
        // Markers may be padded with any number of 0xff fill bytes.
        while i < b.len() && b[i] != 0xff {
            i += 1;
        }
        while i < b.len() && b[i] == 0xff {
            i += 1;
        }
        let marker = *b.get(i)?;
        i += 1;
        match marker {
            0xd8 | 0x01 | 0xd0..=0xd7 => continue,
            0xd9 | 0xda => return None,
            _ => {}
        }
        if i + 2 > b.len() {
            return None;
        }
        let len = be16(b, i) as usize;
        if len < 2 {
            return None;
        }
        match marker {
            0xc0..=0xc2 => {
                if i + 7 > b.len() || len < 8 {
                    return None;
                }
                return Some((be16(b, i + 5), be16(b, i + 3)));
            }
            // Lossless, hierarchical and arithmetic frames: unsupported.
            0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf => return None,
            _ => i += len,
        }
    }
}

/// WebP's canvas size (lossy, lossless and extended), 0×0 if `b` is
/// not one.
pub fn webp_size(b: &[u8]) -> (u32, u32) {
    if b.len() < 30 || &b[0..4] != b"RIFF" || &b[8..12] != b"WEBP" {
        return (0, 0);
    }
    match &b[12..16] {
        b"VP8 " if b[23] == 0x9d && b[24] == 0x01 && b[25] == 0x2a => (le16(b, 26) & 0x3fff, le16(b, 28) & 0x3fff),
        b"VP8L" if b[20] == 0x2f => {
            let v = u32::from_le_bytes([b[21], b[22], b[23], b[24]]);
            ((v & 0x3fff) + 1, ((v >> 14) & 0x3fff) + 1)
        }
        b"VP8X" => {
            let w = u32::from(b[24]) | u32::from(b[25]) << 8 | u32::from(b[26]) << 16;
            let h = u32::from(b[27]) | u32::from(b[28]) << 8 | u32::from(b[29]) << 16;
            (w + 1, h + 1)
        }
        _ => (0, 0),
    }
}

/// A page's dimensions from its leading bytes; 0×0 when the header is
/// not readable (clients then measure after load).
pub fn dimensions(head: &[u8], mime: &str) -> (u32, u32) {
    if mime == "image/webp" {
        return webp_size(head);
    }
    decode_config(head).map(|(_, w, h)| (w, h)).unwrap_or((0, 0))
}

/// Identifies an uploaded picture by content: its mime type when it is
/// a PNG, JPEG, GIF or WebP whose sides are 1 to `max_side` pixels.
pub fn sniff(b: &[u8], max_side: u32) -> Option<&'static str> {
    let fits = |w: u32, h: u32| w > 0 && h > 0 && w <= max_side && h <= max_side;
    if b.len() >= 30 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        let (w, h) = webp_size(b);
        return fits(w, h).then_some("image/webp");
    }
    let (format, w, h) = decode_config(b)?;
    fits(w, h).then_some(match format {
        "png" => "image/png",
        "jpeg" => "image/jpeg",
        _ => "image/gif",
    })
}

/// A valid grayscale PNG of the given size (tests and fixtures).
pub fn test_png(w: u32, h: u32) -> Vec<u8> {
    fn chunk(out: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut c = kind.to_vec();
        c.extend_from_slice(data);
        out.extend_from_slice(&c);
        out.extend_from_slice(&crc32(&c).to_be_bytes());
    }
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 0, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    // Rows of filter byte 0 + one gray byte per pixel. Huge test
    // canvases only need a valid header, so the data is capped.
    let rows = h.min(64) as usize;
    let raw = vec![0u8; rows * (w.min(4096) as usize + 1)];
    chunk(&mut out, b"IDAT", &miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6));
    chunk(&mut out, b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn webp(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut b = b"RIFF\0\0\0\0WEBP".to_vec();
        b.extend_from_slice(kind);
        b.extend_from_slice(&[0; 4]);
        b.extend_from_slice(body);
        b.resize(40, 0);
        b
    }

    #[test]
    fn png_gif_jpeg_headers() {
        assert_eq!(decode_config(&test_png(16, 24)), Some(("png", 16, 24)));
        let mut bad = test_png(16, 24);
        bad[17] ^= 1; // the IHDR CRC no longer matches
        assert_eq!(decode_config(&bad), None);
        let gif = b"GIF89a\x0a\x00\x05\x00\x00\x00\x00;".to_vec();
        assert_eq!(decode_config(&gif), Some(("gif", 10, 5)));
        assert_eq!(decode_config(b"GIF89a\x0a\x00\x05\x00\x80\x00\x00"), None, "missing color table");
        // SOI, an APP0 segment, padding, then a baseline frame header.
        let mut jpg = vec![0xff, 0xd8, 0xff, 0xe0, 0x00, 0x04, 0, 0, 0xff, 0xff];
        jpg.extend_from_slice(&[0xc0, 0x00, 0x11, 0x08, 0x00, 0x30, 0x00, 0x40, 0x03]);
        assert_eq!(decode_config(&jpg), Some(("jpeg", 64, 48)));
        let mut lossless = jpg.clone();
        lossless[10] = 0xc3;
        assert_eq!(decode_config(&lossless), None);
        assert_eq!(decode_config(&jpg[..12]), None);
        assert_eq!(decode_config(b"<svg onload=alert(1)>"), None);
    }

    #[test]
    fn webp_sizes() {
        // VP8 lossy: frame tag, start code, 14-bit sizes.
        let mut body = vec![0u8; 3];
        body.extend_from_slice(&[0x9d, 0x01, 0x2a]);
        body.extend_from_slice(&(640u16).to_le_bytes());
        body.extend_from_slice(&(480u16).to_le_bytes());
        assert_eq!(webp_size(&webp(b"VP8 ", &body)), (640, 480));
        // VP8L lossless: signature 0x2f, then (w-1) | (h-1) << 14.
        let v: u32 = 99 | (49 << 14);
        let mut body = vec![0x2f];
        body.extend_from_slice(&v.to_le_bytes());
        assert_eq!(webp_size(&webp(b"VP8L", &body)), (100, 50));
        // VP8X extended: 24-bit (w-1), (h-1) after 4 flag bytes.
        let body = [0, 0, 0, 0, 0xff, 0x0f, 0, 0x1f, 0, 0];
        assert_eq!(webp_size(&webp(b"VP8X", &body)), (4096, 32));
        assert_eq!(webp_size(b"RIFF"), (0, 0));
    }

    #[test]
    fn sniffing_avatars() {
        assert_eq!(sniff(&test_png(64, 64), 4096), Some("image/png"));
        assert_eq!(sniff(&test_png(5000, 1), 4096), None);
        assert_eq!(sniff(b"<svg onload=alert(1)>", 4096), None);
        let body = [0, 0, 0, 0, 0xff, 0x2f, 0, 0x1f, 0, 0];
        assert_eq!(sniff(&webp(b"VP8X", &body), 4096), None, "12288 px wide");
        assert_eq!(dimensions(&test_png(3, 4), "image/jpeg"), (3, 4), "content wins over the name");
    }
}
