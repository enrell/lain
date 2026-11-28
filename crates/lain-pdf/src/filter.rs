//! Stream filters (§7.4): the general ones decoded here, the image ones
//! (`DCTDecode`, `CCITTFaxDecode`, `JBIG2Decode`) through [`crate::codec`].
//! Every decoder is capped by [`Limits::max_decoded`] and returns what it
//! decoded when the data is cut short or corrupt, as viewers do.

use miniz_oxide::inflate::stream::{InflateState, inflate};
use miniz_oxide::{DataFormat, MZError, MZFlush, MZStatus};

use crate::codec;
use crate::error::{PdfError, Result};
use crate::object::{Dict, Object};

/// The largest decoded stream.
pub const MAX_DECODED: usize = 512 << 20;

/// What decoding produced beyond the bytes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Decoded {
    pub data: Vec<u8>,
    /// Set when the last filter was `DCTDecode` or `JPXDecode`: the
    /// picture's own size and component count, and whether a JPEG had an
    /// Adobe marker.
    pub jpeg: Option<JpegInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JpegInfo {
    pub width: u32,
    pub height: u32,
    pub components: u8,
    pub adobe: bool,
}

/// One filter of a chain and its parameters.
#[derive(Debug, Clone)]
pub struct Filter {
    pub name: String,
    pub params: Option<Dict>,
}

/// Expands abbreviated inline-image filter names.
pub fn full_name(name: &str) -> &str {
    match name {
        "AHx" => "ASCIIHexDecode",
        "A85" => "ASCII85Decode",
        "LZW" => "LZWDecode",
        "Fl" => "FlateDecode",
        "RL" => "RunLengthDecode",
        "CCF" => "CCITTFaxDecode",
        "DCT" => "DCTDecode",
        other => other,
    }
}

/// Applies `filters` in order. `jbig2_globals` is the decoded globals
/// stream of a JBIG2 filter, when it has one.
pub fn decode(data: &[u8], filters: &[Filter], jbig2_globals: Option<&[u8]>) -> Result<Decoded> {
    let mut cur: Vec<u8> = Vec::new();
    let mut first = true;
    let mut jpeg = None;
    for f in filters {
        let input: &[u8] = if first { data } else { &cur };
        let p = f.params.as_ref();
        let out = match full_name(&f.name) {
            "FlateDecode" => predict(inflate_lenient(input)?, p)?,
            "LZWDecode" => {
                let early = p.and_then(|d| d.get("EarlyChange")).and_then(Object::as_i64).unwrap_or(1) != 0;
                predict(lzw(input, early)?, p)?
            }
            "ASCIIHexDecode" => ascii_hex(input),
            "ASCII85Decode" => ascii85(input),
            "RunLengthDecode" => run_length(input)?,
            "Crypt" => input.to_vec(),
            "DCTDecode" => {
                let d = codec::jpeg::decode(input).map_err(|e| PdfError::invalid(format!("jpeg: {e}")))?;
                let components = match d.kind {
                    codec::jpeg::ColorKind::Gray => 1,
                    codec::jpeg::ColorKind::Rgb => 3,
                    codec::jpeg::ColorKind::Cmyk => 4,
                };
                jpeg = Some(JpegInfo { width: d.width, height: d.height, components, adobe: d.adobe });
                d.data
            }
            "CCITTFaxDecode" => {
                let cp = ccitt_params(p);
                codec::ccitt::decode(input, &cp).map_err(|e| PdfError::invalid(format!("ccitt: {e}")))?
            }
            "JBIG2Decode" => {
                let bm =
                    codec::jbig2::decode(input, jbig2_globals).map_err(|e| PdfError::invalid(format!("jbig2: {e}")))?;
                // JBIG2 has 1 = black; PDF image samples have 0 = black.
                let mut v = bm.data;
                for b in &mut v {
                    *b = !*b;
                }
                v
            }
            "JPXDecode" => {
                let d = codec::jpx::decode(input).map_err(|e| PdfError::invalid(format!("jpx: {e}")))?;
                // An alpha channel is dropped here; the image's own /SMask
                // (or SMaskInData) is what PDF composites with.
                let n = usize::from(d.components.max(1));
                let (data, components) = if d.has_alpha && n > 1 {
                    (d.data.chunks(n).flat_map(|px| px[..n - 1].iter().copied()).collect::<Vec<u8>>(), d.components - 1)
                } else {
                    (d.data, d.components)
                };
                jpeg = Some(JpegInfo { width: d.width, height: d.height, components, adobe: false });
                data
            }
            other => return Err(PdfError::unsupported(format!("filter {other}"))),
        };
        if out.len() > MAX_DECODED {
            return Err(PdfError::too_large("decoded stream"));
        }
        cur = out;
        first = false;
    }
    if first {
        cur = data.to_vec();
    }
    Ok(Decoded { data: cur, jpeg })
}

fn ccitt_params(p: Option<&Dict>) -> codec::ccitt::CcittParams {
    let mut c = codec::ccitt::CcittParams::default();
    let Some(d) = p else { return c };
    let int = |k: &str| d.get(k).and_then(Object::as_i64);
    let flag = |k: &str| d.get(k).and_then(Object::as_bool);
    if let Some(k) = int("K") {
        c.k = k.clamp(-1, 1_000_000) as i32;
    }
    if let Some(v) = int("Columns").filter(|v| *v > 0 && *v <= 1 << 20) {
        c.columns = v as u32;
    }
    if let Some(v) = int("Rows").filter(|v| *v >= 0 && *v <= 1 << 20) {
        c.rows = v as u32;
    }
    if let Some(v) = flag("EndOfLine") {
        c.end_of_line = v;
    }
    if let Some(v) = flag("EncodedByteAlign") {
        c.encoded_byte_align = v;
    }
    if let Some(v) = flag("EndOfBlock") {
        c.end_of_block = v;
    }
    if let Some(v) = flag("BlackIs1") {
        c.black_is_1 = v;
    }
    if let Some(v) = int("DamagedRowsBeforeError").filter(|v| *v >= 0) {
        c.damaged_rows_before_error = v.min(u32::MAX as i64) as u32;
    }
    c
}

/// zlib, or raw DEFLATE when the header is bad; corrupt data yields the
/// bytes decoded before the damage.
pub fn inflate_lenient(input: &[u8]) -> Result<Vec<u8>> {
    let zlib = input.len() >= 2 && (u16::from(input[0]) << 8 | u16::from(input[1])) % 31 == 0 && input[0] & 0x0f == 8;
    let format = if zlib { DataFormat::ZLibIgnoreChecksum } else { DataFormat::Raw };
    let out = inflate_with(input, format)?;
    if out.is_empty() && zlib {
        // A zlib-looking header on raw data.
        return inflate_with(input, DataFormat::Raw);
    }
    Ok(out)
}

fn inflate_with(input: &[u8], format: DataFormat) -> Result<Vec<u8>> {
    let mut state = InflateState::new_boxed(format);
    let mut out: Vec<u8> = Vec::with_capacity((input.len() * 4).clamp(1024, 64 << 20));
    let mut buf = vec![0u8; 64 << 10];
    let mut pos = 0usize;
    loop {
        let r = inflate(&mut state, &input[pos..], &mut buf, MZFlush::None);
        pos += r.bytes_consumed;
        out.extend_from_slice(&buf[..r.bytes_written]);
        if out.len() > MAX_DECODED {
            return Err(PdfError::too_large("inflated stream"));
        }
        match r.status {
            Ok(MZStatus::StreamEnd) => break,
            Ok(_) => {
                if r.bytes_consumed == 0 && r.bytes_written == 0 {
                    // Input exhausted mid-stream: keep what came out.
                    break;
                }
            }
            Err(MZError::Buf) if r.bytes_written == 0 => break,
            Err(_) => break,
        }
    }
    Ok(out)
}

/// PNG (10–15) and TIFF (2) predictors (§7.4.4.4).
pub fn predict(data: Vec<u8>, p: Option<&Dict>) -> Result<Vec<u8>> {
    let Some(p) = p else { return Ok(data) };
    let int = |k: &str, d: i64| p.get(k).and_then(Object::as_i64).unwrap_or(d);
    let predictor = int("Predictor", 1);
    if predictor <= 1 {
        return Ok(data);
    }
    let colors = int("Colors", 1).clamp(1, 32) as usize;
    let bpc = int("BitsPerComponent", 8);
    let bpc = if matches!(bpc, 1 | 2 | 4 | 8 | 16) { bpc as usize } else { 8 };
    let columns = int("Columns", 1).clamp(1, 1 << 20) as usize;
    let bpp = (colors * bpc).div_ceil(8).max(1);
    let row = (colors * bpc * columns).div_ceil(8);
    if predictor == 2 {
        return Ok(tiff_predict(data, colors, bpc, row));
    }
    // PNG: each row has a filter-type byte.
    let mut out = Vec::with_capacity(data.len());
    let mut prev = vec![0u8; row];
    let mut cur = vec![0u8; row];
    for chunk in data.chunks(row + 1) {
        if chunk.len() < 2 {
            break;
        }
        let kind = chunk[0];
        let src = &chunk[1..];
        cur[..src.len()].copy_from_slice(src);
        cur[src.len()..].fill(0);
        for i in 0..row {
            let a = if i >= bpp { cur[i - bpp] } else { 0 };
            let b = prev[i];
            let c = if i >= bpp { prev[i - bpp] } else { 0 };
            let x = cur[i];
            cur[i] = match kind {
                1 => x.wrapping_add(a),
                2 => x.wrapping_add(b),
                3 => x.wrapping_add(((u16::from(a) + u16::from(b)) / 2) as u8),
                4 => x.wrapping_add(paeth(a, b, c)),
                _ => x,
            };
        }
        out.extend_from_slice(&cur[..src.len().min(row)]);
        std::mem::swap(&mut prev, &mut cur);
    }
    Ok(out)
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = i16::from(a) + i16::from(b) - i16::from(c);
    let (pa, pb, pc) = ((p - i16::from(a)).abs(), (p - i16::from(b)).abs(), (p - i16::from(c)).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

fn tiff_predict(mut data: Vec<u8>, colors: usize, bpc: usize, row: usize) -> Vec<u8> {
    if row == 0 {
        return data;
    }
    for line in data.chunks_mut(row) {
        match bpc {
            8 => {
                for i in colors..line.len() {
                    line[i] = line[i].wrapping_add(line[i - colors]);
                }
            }
            16 => {
                let step = colors * 2;
                let mut i = step;
                while i + 1 < line.len() {
                    let prev = u16::from_be_bytes([line[i - step], line[i - step + 1]]);
                    let v = u16::from_be_bytes([line[i], line[i + 1]]).wrapping_add(prev);
                    line[i..i + 2].copy_from_slice(&v.to_be_bytes());
                    i += 2;
                }
            }
            _ => {
                // Sub-byte samples: unpack, add, repack.
                let n = line.len() * 8 / bpc;
                let mask = (1u16 << bpc) - 1;
                let get = |l: &[u8], i: usize| -> u16 {
                    let bit = i * bpc;
                    (u16::from(l[bit / 8]) >> (8 - bpc - bit % 8)) & mask
                };
                let mut vals: Vec<u16> = (0..n).map(|i| get(line, i)).collect();
                for i in colors..n {
                    vals[i] = (vals[i] + vals[i - colors]) & mask;
                }
                line.fill(0);
                for (i, v) in vals.iter().enumerate() {
                    let bit = i * bpc;
                    line[bit / 8] |= (*v as u8) << (8 - bpc - bit % 8);
                }
            }
        }
    }
    data
}

/// LZW (§7.4.4.2) with the PDF `EarlyChange` rule.
pub fn lzw(input: &[u8], early: bool) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() * 3);
    // Table entries as (prefix index, last byte, length); strings are
    // rebuilt backwards into `scratch`.
    let mut prefix: Vec<u16> = Vec::with_capacity(4096);
    let mut last: Vec<u8> = Vec::with_capacity(4096);
    let mut len: Vec<u16> = Vec::with_capacity(4096);
    let reset = |prefix: &mut Vec<u16>, last: &mut Vec<u8>, len: &mut Vec<u16>| {
        prefix.clear();
        last.clear();
        len.clear();
        for i in 0..256u16 {
            prefix.push(u16::MAX);
            last.push(i as u8);
            len.push(1);
        }
        // 256 clear, 257 end.
        for _ in 0..2 {
            prefix.push(u16::MAX);
            last.push(0);
            len.push(0);
        }
    };
    reset(&mut prefix, &mut last, &mut len);
    let mut width = 9u32;
    let mut bits: u32 = 0;
    let mut nbits = 0u32;
    let mut prev: Option<u16> = None;
    let mut scratch: Vec<u8> = Vec::with_capacity(4096);
    let mut iter = input.iter();
    let emit = |code: u16, prefix: &[u16], last: &[u8], out: &mut Vec<u8>, scratch: &mut Vec<u8>| -> Option<u8> {
        scratch.clear();
        let mut c = code;
        let mut guard = 0;
        loop {
            scratch.push(*last.get(c as usize)?);
            let p = *prefix.get(c as usize)?;
            if p == u16::MAX {
                break;
            }
            c = p;
            guard += 1;
            if guard > 4096 {
                return None;
            }
        }
        let first = *scratch.last()?;
        out.extend(scratch.iter().rev());
        Some(first)
    };
    loop {
        while nbits < width {
            let Some(&b) = iter.next() else { return Ok(out) };
            bits = (bits << 8) | u32::from(b);
            nbits += 8;
        }
        let code = ((bits >> (nbits - width)) & ((1 << width) - 1)) as u16;
        nbits -= width;
        bits &= (1u32 << nbits).wrapping_sub(1);
        match code {
            256 => {
                reset(&mut prefix, &mut last, &mut len);
                width = 9;
                prev = None;
                continue;
            }
            257 => return Ok(out),
            _ => {}
        }
        let next = prefix.len() as u16;
        match prev {
            None => {
                if code >= 256 {
                    return Ok(out);
                }
                out.push(code as u8);
            }
            Some(p) => {
                let first = if code < next {
                    let Some(f) = emit(code, &prefix, &last, &mut out, &mut scratch) else { return Ok(out) };
                    f
                } else if code == next {
                    // KwKwK: the new string is prev + prev's first byte.
                    let Some(f) = emit(p, &prefix, &last, &mut out, &mut scratch) else { return Ok(out) };
                    out.push(f);
                    f
                } else {
                    return Ok(out);
                };
                if prefix.len() < 4096 {
                    prefix.push(p);
                    last.push(first);
                    len.push(len[p as usize].saturating_add(1));
                }
            }
        }
        prev = Some(code);
        if out.len() > MAX_DECODED {
            return Err(PdfError::too_large("LZW stream"));
        }
        let size = prefix.len() as u32 + u32::from(early);
        width = if size >= 2048 {
            12
        } else if size >= 1024 {
            11
        } else if size >= 512 {
            10
        } else {
            9
        };
    }
}

pub fn ascii_hex(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len() / 2);
    let mut hi: Option<u8> = None;
    for &c in input {
        if c == b'>' {
            break;
        }
        let v = match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => continue,
        };
        match hi.take() {
            Some(h) => out.push(h << 4 | v),
            None => hi = Some(v),
        }
    }
    if let Some(h) = hi {
        out.push(h << 4);
    }
    out
}

pub fn ascii85(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len() * 4 / 5);
    let mut group = [0u32; 5];
    let mut n = 0usize;
    let mut i = 0;
    // A leading `<~` is tolerated.
    if input.starts_with(b"<~") {
        i = 2;
    }
    while i < input.len() {
        let c = input[i];
        i += 1;
        if c == b'~' {
            break;
        }
        if c == b'z' && n == 0 {
            out.extend_from_slice(&[0, 0, 0, 0]);
            continue;
        }
        if !(b'!'..=b'u').contains(&c) {
            continue;
        }
        group[n] = u32::from(c - b'!');
        n += 1;
        if n == 5 {
            let v = group.iter().fold(0u64, |acc, &d| acc * 85 + u64::from(d));
            out.extend_from_slice(&(v as u32).to_be_bytes());
            n = 0;
        }
    }
    if n > 1 {
        for g in group.iter_mut().skip(n) {
            *g = 84;
        }
        let v = group.iter().fold(0u64, |acc, &d| acc * 85 + u64::from(d));
        out.extend_from_slice(&(v as u32).to_be_bytes()[..n - 1]);
    }
    out
}

pub fn run_length(input: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() * 2);
    let mut i = 0;
    while i < input.len() {
        let l = input[i];
        i += 1;
        match l {
            128 => break,
            0..=127 => {
                let n = usize::from(l) + 1;
                let end = (i + n).min(input.len());
                out.extend_from_slice(&input[i..end]);
                i = end;
            }
            _ => {
                let Some(&b) = input.get(i) else { break };
                i += 1;
                out.resize(out.len() + 257 - usize::from(l), b);
            }
        }
        if out.len() > MAX_DECODED {
            return Err(PdfError::too_large("run-length stream"));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(name: &str) -> Filter {
        Filter { name: name.into(), params: None }
    }

    #[test]
    fn flate_and_png_predictor() {
        let rows: Vec<u8> = vec![2, 1, 2, 3, 2, 1, 1, 1];
        let z = miniz_oxide::deflate::compress_to_vec_zlib(&rows, 6);
        let mut p = Dict::new();
        p.insert("Predictor", Object::Int(12));
        p.insert("Columns", Object::Int(3));
        let out = decode(&z, &[Filter { name: "FlateDecode".into(), params: Some(p) }], None).unwrap();
        assert_eq!(out.data, vec![1, 2, 3, 2, 3, 4]);
        // A truncated stream gives the bytes before the cut.
        let big: Vec<u8> = (0..20000u32).map(|i| (i % 251) as u8).collect();
        let z = miniz_oxide::deflate::compress_to_vec_zlib(&big, 6);
        let part = inflate_lenient(&z[..z.len() / 2]).unwrap();
        assert!(!part.is_empty() && big.starts_with(&part));
        // Raw deflate without a zlib header.
        let raw = miniz_oxide::deflate::compress_to_vec(&big, 6);
        assert_eq!(inflate_lenient(&raw).unwrap(), big);
    }

    #[test]
    fn ascii_filters() {
        assert_eq!(ascii_hex(b"48 65 6C 6c 6>"), b"Hell`");
        assert_eq!(ascii85(b"87cURD]i,\"Ebo80~>"), b"Hello World!");
        assert_eq!(ascii85(b"<~z~>"), [0, 0, 0, 0]);
        assert_eq!(run_length(&[2, b'a', b'b', b'c', 254, b'x', 128]).unwrap(), b"abcxxx");
    }

    #[test]
    fn lzw_spec_example() {
        // §7.4.4.2 example: 45 45 45 45 45 65 45 45 45 66.
        let enc = [0x80, 0x0B, 0x60, 0x50, 0x22, 0x0C, 0x0C, 0x85, 0x01];
        assert_eq!(lzw(&enc, true).unwrap(), [45, 45, 45, 45, 45, 65, 45, 45, 45, 66]);
        assert!(decode(&enc, &[f("LZW")], None).is_ok());
        let _ = lzw(&[0xff; 64], true);
    }

    #[test]
    fn tiff_predictor() {
        let mut p = Dict::new();
        p.insert("Predictor", Object::Int(2));
        p.insert("Columns", Object::Int(3));
        assert_eq!(predict(vec![1, 1, 1, 5, 0, 0], Some(&p)).unwrap(), vec![1, 2, 3, 5, 5, 5]);
    }
}
