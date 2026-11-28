//! PNG encoder for 8-bit images: adaptive per-row filters and zlib through miniz_oxide.

use super::CodecError;

/// Largest width or height accepted, and the largest pixel count.
const MAX_DIM: u32 = 65535;
const MAX_PIXELS: u64 = 1 << 28;
/// IDAT payloads are split so no chunk length gets near the 2^31 limit.
const IDAT_CHUNK: usize = 1 << 20;

/// The sample layout of the pixels handed to [`encode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PngColor {
    Gray,
    GrayAlpha,
    Rgb,
    Rgba,
}

impl PngColor {
    /// Samples per pixel.
    pub fn channels(self) -> usize {
        match self {
            PngColor::Gray => 1,
            PngColor::GrayAlpha => 2,
            PngColor::Rgb => 3,
            PngColor::Rgba => 4,
        }
    }

    fn color_type(self) -> u8 {
        match self {
            PngColor::Gray => 0,
            PngColor::GrayAlpha => 4,
            PngColor::Rgb => 2,
            PngColor::Rgba => 6,
        }
    }
}

/// Encodes 8-bit samples, rows packed without padding. `level` 0..=10 (miniz levels); each row gets the filter
/// (None/Sub/Up/Average/Paeth) whose output has the smallest sum of absolute values, the usual libpng heuristic.
pub fn encode(pixels: &[u8], width: u32, height: u32, color: PngColor, level: u8) -> Result<Vec<u8>, CodecError> {
    if width == 0 || height == 0 || width > MAX_DIM || height > MAX_DIM {
        return Err(CodecError(format!("png: bad dimensions {width}x{height}")));
    }
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(CodecError(format!("png: image too large ({width}x{height})")));
    }
    let bpp = color.channels();
    let row_len = width as usize * bpp;
    let expected = row_len * height as usize;
    if pixels.len() != expected {
        return Err(CodecError(format!("png: expected {expected} bytes of pixels, got {}", pixels.len())));
    }

    let raw = filter_rows(pixels, row_len, bpp);
    let compressed = miniz_oxide::deflate::compress_to_vec_zlib(&raw, level.min(10));

    let mut out = Vec::with_capacity(compressed.len() + 64);
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n']);
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&height.to_be_bytes());
    ihdr[8] = 8; // bit depth
    ihdr[9] = color.color_type();
    // compression 0, filter method 0, no interlace: already zero.
    write_chunk(&mut out, b"IHDR", &ihdr);
    if compressed.is_empty() {
        write_chunk(&mut out, b"IDAT", &[]);
    }
    for part in compressed.chunks(IDAT_CHUNK) {
        write_chunk(&mut out, b"IDAT", part);
    }
    write_chunk(&mut out, b"IEND", &[]);
    Ok(out)
}

/// Prefixes every row with its filter type byte and the filtered samples.
fn filter_rows(pixels: &[u8], row_len: usize, bpp: usize) -> Vec<u8> {
    let rows = pixels.len() / row_len.max(1);
    let mut raw = Vec::with_capacity(rows * (row_len + 1));
    let zero = vec![0u8; row_len];
    let mut candidates: [Vec<u8>; 5] = std::array::from_fn(|_| vec![0u8; row_len]);
    for (y, cur) in pixels.chunks_exact(row_len).enumerate() {
        let prev = if y == 0 { &zero[..] } else { &pixels[(y - 1) * row_len..y * row_len] };
        let mut best = 0usize;
        let mut best_score = u64::MAX;
        for (kind, cand) in candidates.iter_mut().enumerate() {
            let score = apply_filter(kind, cur, prev, bpp, cand);
            if score < best_score {
                best_score = score;
                best = kind;
            }
        }
        raw.push(best as u8);
        raw.extend_from_slice(&candidates[best]);
    }
    raw
}

/// Writes filter `kind` of `cur` into `out` and returns the sum of the outputs read as signed bytes.
fn apply_filter(kind: usize, cur: &[u8], prev: &[u8], bpp: usize, out: &mut [u8]) -> u64 {
    let mut score = 0u64;
    for i in 0..cur.len() {
        let x = cur[i];
        let a = if i >= bpp { cur[i - bpp] } else { 0 };
        let b = prev[i];
        let c = if i >= bpp { prev[i - bpp] } else { 0 };
        let v = match kind {
            0 => x,
            1 => x.wrapping_sub(a),
            2 => x.wrapping_sub(b),
            3 => x.wrapping_sub(((u16::from(a) + u16::from(b)) / 2) as u8),
            _ => x.wrapping_sub(paeth(a, b, c)),
        };
        out[i] = v;
        score += u64::from((v as i8).unsigned_abs());
    }
    score
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = i16::from(a) + i16::from(b) - i16::from(c);
    let pa = (p - i16::from(a)).abs();
    let pb = (p - i16::from(b)).abs();
    let pc = (p - i16::from(c)).abs();
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

fn write_chunk(out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

const CRC_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut n = 0;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        table[n] = c;
        n += 1;
    }
    table
};

/// CRC-32 as PNG chunks use it (ISO 3309, reflected, initial and final xor 0xffffffff).
pub fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xffff_ffffu32;
    for &b in data {
        c = CRC_TABLE[((c ^ u32::from(b)) & 0xff) as usize] ^ (c >> 8);
    }
    c ^ 0xffff_ffff
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Parsed {
        width: u32,
        height: u32,
        color_type: u8,
        idat: Vec<u8>,
    }

    /// Walks the chunks, checking each CRC, and collects IHDR fields and the IDAT stream.
    fn parse(png: &[u8]) -> Parsed {
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n']);
        let mut pos = 8;
        let mut parsed = Parsed { width: 0, height: 0, color_type: 255, idat: Vec::new() };
        let mut seen_end = false;
        let mut first = true;
        while pos < png.len() {
            let len = u32::from_be_bytes(png[pos..pos + 4].try_into().unwrap()) as usize;
            let kind = &png[pos + 4..pos + 8];
            let body = &png[pos + 8..pos + 8 + len];
            let crc = u32::from_be_bytes(png[pos + 8 + len..pos + 12 + len].try_into().unwrap());
            assert_eq!(crc, crc32(&png[pos + 4..pos + 8 + len]), "bad CRC on {:?}", kind);
            if first {
                assert_eq!(kind, b"IHDR");
                first = false;
            }
            match kind {
                b"IHDR" => {
                    assert_eq!(len, 13);
                    parsed.width = u32::from_be_bytes(body[0..4].try_into().unwrap());
                    parsed.height = u32::from_be_bytes(body[4..8].try_into().unwrap());
                    assert_eq!(body[8], 8);
                    parsed.color_type = body[9];
                    assert_eq!(&body[10..13], &[0, 0, 0]);
                }
                b"IDAT" => parsed.idat.extend_from_slice(body),
                b"IEND" => {
                    assert_eq!(len, 0);
                    seen_end = true;
                }
                other => panic!("unexpected chunk {other:?}"),
            }
            pos += 12 + len;
        }
        assert!(seen_end);
        assert_eq!(pos, png.len());
        parsed
    }

    fn unfilter(raw: &[u8], width: usize, height: usize, bpp: usize) -> Vec<u8> {
        let row_len = width * bpp;
        assert_eq!(raw.len(), height * (row_len + 1));
        let mut out = vec![0u8; row_len * height];
        for y in 0..height {
            let line = &raw[y * (row_len + 1)..(y + 1) * (row_len + 1)];
            let kind = line[0];
            for i in 0..row_len {
                let a = if i >= bpp { out[y * row_len + i - bpp] } else { 0 };
                let b = if y > 0 { out[(y - 1) * row_len + i] } else { 0 };
                let c = if y > 0 && i >= bpp { out[(y - 1) * row_len + i - bpp] } else { 0 };
                let x = line[1 + i];
                out[y * row_len + i] = match kind {
                    0 => x,
                    1 => x.wrapping_add(a),
                    2 => x.wrapping_add(b),
                    3 => x.wrapping_add(((u16::from(a) + u16::from(b)) / 2) as u8),
                    4 => x.wrapping_add(paeth(a, b, c)),
                    k => panic!("bad filter {k}"),
                };
            }
        }
        out
    }

    fn sample_image(width: usize, height: usize, bpp: usize) -> Vec<u8> {
        let mut seed = 0x1234_5678u32;
        let mut v = Vec::with_capacity(width * height * bpp);
        for y in 0..height {
            for x in 0..width {
                for c in 0..bpp {
                    seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                    // Gradients mixed with noise so every filter type wins somewhere.
                    let smooth = (x * 3 + y * 5 + c * 40) as u32;
                    let noise = if (x / 7 + y / 5) % 3 == 0 { seed >> 24 } else { 0 };
                    v.push((smooth + noise) as u8);
                }
            }
        }
        v
    }

    #[test]
    fn crc_known_value() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(crc32(b"IEND"), 0xae42_6082);
    }

    #[test]
    fn round_trip_all_color_types() {
        let cases = [(PngColor::Gray, 0u8), (PngColor::GrayAlpha, 4), (PngColor::Rgb, 2), (PngColor::Rgba, 6)];
        for (color, ct) in cases {
            for &(w, h) in &[(1usize, 1usize), (3, 7), (64, 33), (257, 19)] {
                for level in [0u8, 1, 6, 10] {
                    let bpp = color.channels();
                    let img = sample_image(w, h, bpp);
                    let png = encode(&img, w as u32, h as u32, color, level).unwrap();
                    let parsed = parse(&png);
                    assert_eq!((parsed.width, parsed.height, parsed.color_type), (w as u32, h as u32, ct));
                    let raw = miniz_oxide::inflate::decompress_to_vec_zlib(&parsed.idat).unwrap();
                    assert_eq!(unfilter(&raw, w, h, bpp), img, "{color:?} {w}x{h} level {level}");
                }
            }
        }
    }

    #[test]
    fn adaptive_filters_are_used() {
        let img = sample_image(200, 50, 3);
        let png = encode(&img, 200, 50, PngColor::Rgb, 6).unwrap();
        let raw = miniz_oxide::inflate::decompress_to_vec_zlib(&parse(&png).idat).unwrap();
        let mut kinds = [false; 5];
        for y in 0..50 {
            kinds[raw[y * 601] as usize] = true;
        }
        assert!(kinds.iter().filter(|k| **k).count() >= 2, "only one filter type used: {kinds:?}");
    }

    #[test]
    fn rejects_bad_input() {
        assert!(encode(&[0; 3], 1, 1, PngColor::Rgba, 6).is_err());
        assert!(encode(&[], 0, 1, PngColor::Gray, 6).is_err());
        assert!(encode(&[0; 4], 2, 2, PngColor::Gray, 6).is_ok());
        assert!(encode(&[0; 5], 2, 2, PngColor::Gray, 6).is_err());
        assert!(encode(&[], 70000, 1, PngColor::Gray, 6).is_err());
        assert!(encode(&[], 65535, 65535, PngColor::Gray, 6).is_err());
    }
}
