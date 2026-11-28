//! JPEG (DCTDecode): a baseline/extended/progressive Huffman decoder and a baseline encoder.
//!
//! The decoder keeps one 8-bit plane per component at its own sampling resolution. Sequential scans are
//! transformed block by block as they are decoded; progressive scans accumulate coefficients that are transformed
//! once at the end. Upsampling and colour conversion then run row by row into the interleaved output.
//!
//! Corrupt or truncated entropy data degrades like libjpeg does: the damaged restart interval is left at zero
//! coefficients (mid gray) and decoding resumes at the next RST marker.

use super::CodecError;

const MAX_DIM: u32 = 65535;
const MAX_PIXELS: u64 = 1 << 28;
/// Block visits allowed over all scans, as a multiple of the frame's block count: legitimate progressive files
/// use around ten passes per component, so this only stops files built to spin on empty scans.
const WORK_FACTOR: u64 = 64;

/// The colour model of [`Decoded::data`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorKind {
    Gray,
    Rgb,
    Cmyk,
}

impl ColorKind {
    pub fn channels(self) -> usize {
        match self {
            ColorKind::Gray => 1,
            ColorKind::Rgb => 3,
            ColorKind::Cmyk => 4,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Decoded {
    pub width: u32,
    pub height: u32,
    pub kind: ColorKind,
    /// Interleaved 8-bit samples, 1/3/4 per pixel.
    pub data: Vec<u8>,
    /// True when an Adobe APP14 marker was present (Adobe CMYK/YCCK JPEGs store inverted CMYK; the caller decides
    /// about inversion).
    pub adobe: bool,
}

fn err(msg: impl Into<String>) -> CodecError {
    CodecError(format!("jpeg: {}", msg.into()))
}

/// Zigzag position to natural (row-major) coefficient index.
#[rustfmt::skip]
const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21,
    28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61,
    54, 47, 55, 62, 63,
];

// Annex K tables, used by the encoder and as the decoder's fallback when a file omits its Huffman tables
// (Motion-JPEG frames do).
#[rustfmt::skip]
const STD_LUMA_QT: [u8; 64] = [
    16, 11, 10, 16, 24, 40, 51, 61, 12, 12, 14, 19, 26, 58, 60, 55, 14, 13, 16, 24, 40, 57, 69, 56, 14, 17, 22, 29,
    51, 87, 80, 62, 18, 22, 37, 56, 68, 109, 103, 77, 24, 35, 55, 64, 81, 104, 113, 92, 49, 64, 78, 87, 103, 121,
    120, 101, 72, 92, 95, 98, 112, 100, 103, 99,
];
#[rustfmt::skip]
const STD_CHROMA_QT: [u8; 64] = [
    17, 18, 24, 47, 99, 99, 99, 99, 18, 21, 26, 66, 99, 99, 99, 99, 24, 26, 56, 99, 99, 99, 99, 99, 47, 66, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99,
];
const STD_DC_LUMA_BITS: [u8; 16] = [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0];
const STD_DC_CHROMA_BITS: [u8; 16] = [0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0];
const STD_DC_VALUES: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
const STD_AC_LUMA_BITS: [u8; 16] = [0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 0x7d];
#[rustfmt::skip]
const STD_AC_LUMA_VALUES: [u8; 162] = [
    0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07, 0x22, 0x71,
    0x14, 0x32, 0x81, 0x91, 0xa1, 0x08, 0x23, 0x42, 0xb1, 0xc1, 0x15, 0x52, 0xd1, 0xf0, 0x24, 0x33, 0x62, 0x72,
    0x82, 0x09, 0x0a, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x34, 0x35, 0x36, 0x37,
    0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59,
    0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x83,
    0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3,
    0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3,
    0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, 0xe1, 0xe2,
    0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa,
];
const STD_AC_CHROMA_BITS: [u8; 16] = [0, 2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 0x77];
#[rustfmt::skip]
const STD_AC_CHROMA_VALUES: [u8; 162] = [
    0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71, 0x13, 0x22,
    0x32, 0x81, 0x08, 0x14, 0x42, 0x91, 0xa1, 0xb1, 0xc1, 0x09, 0x23, 0x33, 0x52, 0xf0, 0x15, 0x62, 0x72, 0xd1,
    0x0a, 0x16, 0x24, 0x34, 0xe1, 0x25, 0xf1, 0x17, 0x18, 0x19, 0x1a, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x35, 0x36,
    0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58,
    0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a,
    0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a,
    0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba,
    0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda,
    0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa,
];

// ---------------------------------------------------------------------------------------------------------------
// Decoder
// ---------------------------------------------------------------------------------------------------------------

/// Decodes a JPEG into interleaved 8-bit samples.
pub fn decode(data: &[u8]) -> Result<Decoded, CodecError> {
    let mut dec = Decoder::new(data);
    dec.parse(false)?;
    dec.finish()
}

/// Header only: (width, height, components) without decoding scans.
pub fn info(data: &[u8]) -> Result<(u32, u32, u8), CodecError> {
    let mut dec = Decoder::new(data);
    dec.parse(true)?;
    let f = dec.frame.as_ref().ok_or_else(|| err("no frame header"))?;
    Ok((f.width as u32, f.height as u32, f.comps.len() as u8))
}

/// Canonical Huffman decoding table with a 9-bit lookahead.
#[derive(Clone)]
struct Huffman {
    /// `length << 8 | symbol` for codes of up to 9 bits, indexed by the next 9 bits; 0 means "longer code".
    lookup: Box<[u16; 512]>,
    /// AC fast path: when a code and its magnitude bits fit in the 9-bit lookahead, the decoded coefficient as
    /// `value << 16 | run << 8 | total length`; 0 otherwise.
    fast_ac: Box<[i32; 512]>,
    /// Largest code of each length (index 1..=16), -1 when there is none.
    maxcode: [i32; 17],
    /// Offset from a code of each length to its symbol index.
    valptr: [i32; 17],
    values: Vec<u8>,
}

impl Huffman {
    fn new(counts: &[u8; 16], symbols: &[u8]) -> Result<Huffman, CodecError> {
        let total: usize = counts.iter().map(|&c| usize::from(c)).sum();
        if total > 256 || total > symbols.len() {
            return Err(err("bad Huffman table"));
        }
        let mut t = Huffman {
            lookup: Box::new([0; 512]),
            fast_ac: Box::new([0; 512]),
            maxcode: [-1; 17],
            valptr: [0; 17],
            values: symbols[..total].to_vec(),
        };
        let mut code: i32 = 0;
        let mut k: i32 = 0;
        for len in 1..=16usize {
            let n = i32::from(counts[len - 1]);
            t.valptr[len] = k - code;
            for _ in 0..n {
                if code >= 1 << len {
                    return Err(err("overfull Huffman table"));
                }
                if len <= 9 {
                    let shift = 9 - len;
                    let first = (code as usize) << shift;
                    let entry = ((len as u16) << 8) | u16::from(t.values[k as usize]);
                    for slot in &mut t.lookup[first..first + (1 << shift)] {
                        *slot = entry;
                    }
                }
                code += 1;
                k += 1;
            }
            if n > 0 {
                t.maxcode[len] = code - 1;
            }
            code <<= 1;
        }
        for (look, e) in t.lookup.iter().enumerate() {
            let (len, rs) = (u32::from(e >> 8), (e & 0xff) as u32);
            let (run, s) = (rs >> 4, rs & 15);
            if len == 0 || s == 0 || len + s > 9 {
                continue;
            }
            let bits = ((look as u32) >> (9 - len - s)) & ((1 << s) - 1);
            let v = bits as i32;
            let v = if v < 1 << (s - 1) { v - (1 << s) + 1 } else { v };
            t.fast_ac[look] = (v << 16) | ((run as i32) << 8) | (len + s) as i32;
        }
        Ok(t)
    }
}

/// Entropy-coded segment reader: bits MSB first in a 64-bit accumulator, byte stuffing removed. At a marker or
/// the end of the data it feeds zero bits and counts them, so a decoder can tell real data from padding.
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    acc: u64,
    nbits: u32,
    stopped: bool,
    pad_bits: u64,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8], pos: usize) -> Self {
        Reader { data, pos, acc: 0, nbits: 0, stopped: false, pad_bits: 0 }
    }

    fn restart_at(&mut self, pos: usize) {
        *self = Reader::new(self.data, pos);
    }

    fn refill(&mut self) {
        // Fast path: eight bytes without 0xff need no unstuffing.
        if !self.stopped
            && self.nbits <= 56
            && let Some(chunk) = self.data.get(self.pos..self.pos + 8)
        {
            let word = u64::from_be_bytes(chunk.try_into().unwrap_or([0; 8]));
            let inv = !word;
            if inv.wrapping_sub(0x0101_0101_0101_0101) & !inv & 0x8080_8080_8080_8080 == 0 {
                let take = (64 - self.nbits) / 8;
                let bits = take * 8;
                self.acc |= (word >> (64 - bits)) << (64 - self.nbits - bits);
                self.nbits += bits;
                self.pos += take as usize;
                return;
            }
        }
        while self.nbits <= 56 {
            let mut byte = 0u8;
            if !self.stopped {
                match self.data.get(self.pos) {
                    None => self.stopped = true,
                    Some(&0xff) => {
                        let mut p = self.pos + 1;
                        while self.data.get(p) == Some(&0xff) {
                            p += 1;
                        }
                        if self.data.get(p) == Some(&0) {
                            byte = 0xff;
                            self.pos = p + 1;
                        } else {
                            // A marker: leave `pos` on it.
                            self.stopped = true;
                        }
                    }
                    Some(&b) => {
                        byte = b;
                        self.pos += 1;
                    }
                }
            }
            if self.stopped {
                self.pad_bits += 8;
            }
            self.acc |= u64::from(byte) << (56 - self.nbits);
            self.nbits += 8;
        }
    }

    /// True once decoding has consumed padding past the end of the segment.
    fn overrun(&self) -> bool {
        self.stopped && u64::from(self.nbits) < self.pad_bits
    }

    #[inline]
    fn bits(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        if self.nbits < n {
            self.refill();
        }
        let v = (self.acc >> (64 - n)) as u32;
        self.acc <<= n;
        self.nbits -= n;
        v
    }

    #[inline]
    fn bit(&mut self) -> bool {
        self.bits(1) != 0
    }

    /// Reads an `s`-bit magnitude and sign-extends it (F.2.2.1).
    #[inline]
    fn receive_extend(&mut self, s: u32) -> i32 {
        if s == 0 {
            return 0;
        }
        let v = self.bits(s) as i32;
        if v < 1 << (s - 1) { v - (1 << s) + 1 } else { v }
    }

    #[inline]
    fn decode(&mut self, t: &Huffman) -> Option<u8> {
        if self.nbits < 16 {
            self.refill();
        }
        let e = t.lookup[(self.acc >> 55) as usize];
        if e != 0 {
            let len = u32::from(e >> 8);
            self.acc <<= len;
            self.nbits -= len;
            return Some(e as u8);
        }
        let code16 = (self.acc >> 48) as i32;
        for len in 10..=16 {
            let code = code16 >> (16 - len);
            if code <= t.maxcode[len] {
                self.acc <<= len;
                self.nbits -= len as u32;
                let idx = code + t.valptr[len];
                return usize::try_from(idx).ok().and_then(|i| t.values.get(i)).copied();
            }
        }
        None
    }
}

struct Component {
    id: u8,
    h: usize,
    v: usize,
    tq: usize,
    /// Blocks per row and column in the plane, padded to whole MCUs.
    bw: usize,
    bh: usize,
    /// Size of the component's real samples.
    cw: usize,
    ch: usize,
    /// Samples, `bw * 8` per row; allocated when the first scan starts.
    plane: Vec<u8>,
    /// Progressive only: all coefficients, 64 per block in natural order.
    coefs: Vec<i16>,
    /// Progressive only: the dequantisation table latched at the component's first scan.
    qt: Option<[f32; 64]>,
    dc_pred: i32,
}

struct Frame {
    width: usize,
    height: usize,
    progressive: bool,
    comps: Vec<Component>,
    hmax: usize,
    vmax: usize,
    mcux: usize,
    mcuy: usize,
    allocated: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScanKind {
    Sequential,
    DcFirst,
    DcRefine,
    AcFirst,
    AcRefine,
}

struct Scan {
    /// (frame component index, DC table, AC table)
    comps: Vec<(usize, usize, usize)>,
    ss: usize,
    se: usize,
    ah: u32,
    al: u32,
}

struct Decoder<'a> {
    data: &'a [u8],
    qt: [Option<[u16; 64]>; 4],
    dc: [Option<Huffman>; 4],
    ac: [Option<Huffman>; 4],
    restart: usize,
    adobe: Option<u8>,
    frame: Option<Frame>,
    scans: usize,
    work: u64,
}

/// Position of the 0xff that introduces the next marker at or after `pos`, skipping stray bytes.
fn next_marker(data: &[u8], mut pos: usize) -> Option<usize> {
    while pos + 1 < data.len() {
        if data[pos] == 0xff {
            let m = data[pos + 1];
            if m != 0 && m != 0xff {
                return Some(pos);
            }
        }
        pos += 1;
    }
    None
}

fn be16(data: &[u8], pos: usize) -> Option<usize> {
    Some(usize::from(*data.get(pos)?) << 8 | usize::from(*data.get(pos + 1)?))
}

/// Finds the DNL marker that supplies the height when SOF says 0. FF DC cannot occur inside entropy-coded data,
/// so a byte search is reliable.
fn find_dnl(data: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 5 < data.len() {
        if data[i] == 0xff && data[i + 1] == 0xdc && data[i + 2] == 0 && data[i + 3] == 4 {
            let n = be16(data, i + 4)?;
            return (n > 0).then_some(n);
        }
        i += 1;
    }
    None
}

impl<'a> Decoder<'a> {
    fn new(data: &'a [u8]) -> Self {
        Decoder {
            data,
            qt: [None; 4],
            dc: [None, None, None, None],
            ac: [None, None, None, None],
            restart: 0,
            adobe: None,
            frame: None,
            scans: 0,
            work: 0,
        }
    }

    fn parse(&mut self, headers_only: bool) -> Result<(), CodecError> {
        let data = self.data;
        // Some producers put junk before SOI; look for it near the start.
        let soi = (0..data.len().min(1024).saturating_sub(1))
            .find(|&i| data[i] == 0xff && data[i + 1] == 0xd8)
            .ok_or_else(|| err("missing SOI marker"))?;
        let mut pos = soi + 2;
        while let Some(m) = next_marker(data, pos) {
            let marker = data[m + 1];
            pos = m + 2;
            match marker {
                0xd8 | 0x01 | 0xd0..=0xd7 => continue,
                0xd9 => break,
                _ => {}
            }
            let Some(len) = be16(data, pos).filter(|&l| l >= 2) else { break };
            let seg_end = pos + len;
            let seg = &data[(pos + 2).min(data.len())..seg_end.min(data.len())];
            let truncated = seg_end > data.len();
            let result = match marker {
                0xc0..=0xc2 if truncated => Err(err("truncated frame header")),
                0xc0..=0xc2 => self.parse_sof(seg, marker == 0xc2, seg_end),
                0xc3 | 0xc5..=0xc7 => return Err(err("lossless and hierarchical JPEG are not supported")),
                0xc9..=0xcb | 0xcd..=0xcf => return Err(err("arithmetic-coded JPEG is not supported")),
                0xc4 => self.parse_dht(seg),
                0xdb => self.parse_dqt(seg),
                0xdd => {
                    self.restart = be16(seg, 0).unwrap_or(0);
                    Ok(())
                }
                0xee => {
                    if seg.len() >= 12 && &seg[..5] == b"Adobe" {
                        self.adobe = Some(seg[11]);
                    }
                    Ok(())
                }
                0xda if headers_only => return Ok(()),
                0xda => match self.start_scan(seg) {
                    Ok(Some(scan)) => {
                        pos = self.decode_scan(&scan, seg_end.min(data.len()));
                        self.scans += 1;
                        continue;
                    }
                    Ok(None) => Ok(()),
                    Err(e) => Err(e),
                },
                _ => Ok(()),
            };
            if let Err(e) = result {
                // After image data exists a broken header only ends decoding early.
                if self.scans > 0 {
                    break;
                }
                return Err(e);
            }
            if headers_only && self.frame.is_some() {
                return Ok(());
            }
            pos = seg_end;
        }
        Ok(())
    }

    fn parse_sof(&mut self, seg: &[u8], progressive: bool, after: usize) -> Result<(), CodecError> {
        if self.frame.is_some() {
            return Ok(()); // only the first frame counts
        }
        if seg.len() < 6 {
            return Err(err("short frame header"));
        }
        if seg[0] != 8 {
            return Err(err(format!("{}-bit precision is not supported", seg[0])));
        }
        let mut height = be16(seg, 1).unwrap_or(0);
        let width = be16(seg, 3).unwrap_or(0);
        let nf = usize::from(seg[5]);
        if width == 0 {
            return Err(err("zero width"));
        }
        if height == 0 {
            height = find_dnl(self.data, after).ok_or_else(|| err("zero height and no DNL marker"))?;
        }
        if width as u64 * height as u64 > MAX_PIXELS {
            return Err(err(format!("image too large ({width}x{height})")));
        }
        if !matches!(nf, 1 | 3 | 4) {
            return Err(err(format!("{nf} components are not supported")));
        }
        if seg.len() < 6 + 3 * nf {
            return Err(err("short frame header"));
        }
        let mut comps = Vec::with_capacity(nf);
        for i in 0..nf {
            let c = &seg[6 + 3 * i..9 + 3 * i];
            let (h, v, tq) = (usize::from(c[1] >> 4), usize::from(c[1] & 15), usize::from(c[2]));
            if !(1..=4).contains(&h) || !(1..=4).contains(&v) || tq > 3 {
                return Err(err("bad component parameters"));
            }
            comps.push(Component {
                id: c[0],
                h,
                v,
                tq,
                bw: 0,
                bh: 0,
                cw: 0,
                ch: 0,
                plane: Vec::new(),
                coefs: Vec::new(),
                qt: None,
                dc_pred: 0,
            });
        }
        let hmax = comps.iter().map(|c| c.h).max().unwrap_or(1);
        let vmax = comps.iter().map(|c| c.v).max().unwrap_or(1);
        let mcux = width.div_ceil(8 * hmax);
        let mcuy = height.div_ceil(8 * vmax);
        for c in &mut comps {
            c.bw = mcux * c.h;
            c.bh = mcuy * c.v;
            c.cw = (width * c.h).div_ceil(hmax);
            c.ch = (height * c.v).div_ceil(vmax);
        }
        self.frame = Some(Frame { width, height, progressive, comps, hmax, vmax, mcux, mcuy, allocated: false });
        Ok(())
    }

    fn parse_dht(&mut self, seg: &[u8]) -> Result<(), CodecError> {
        let mut p = 0;
        while p < seg.len() {
            let (tc, th) = (seg[p] >> 4, usize::from(seg[p] & 15));
            let counts: &[u8; 16] =
                seg.get(p + 1..p + 17).and_then(|s| s.try_into().ok()).ok_or_else(|| err("short DHT"))?;
            let total: usize = counts.iter().map(|&c| usize::from(c)).sum();
            let symbols = seg.get(p + 17..p + 17 + total).ok_or_else(|| err("short DHT"))?;
            if tc > 1 || th > 3 {
                return Err(err("bad DHT table id"));
            }
            let table = Huffman::new(counts, symbols)?;
            if tc == 0 {
                self.dc[th] = Some(table);
            } else {
                self.ac[th] = Some(table);
            }
            p += 17 + total;
        }
        Ok(())
    }

    fn parse_dqt(&mut self, seg: &[u8]) -> Result<(), CodecError> {
        let mut p = 0;
        while p < seg.len() {
            let (pq, tq) = (seg[p] >> 4, usize::from(seg[p] & 15));
            if pq > 1 || tq > 3 {
                return Err(err("bad DQT table id"));
            }
            let size = if pq == 0 { 64 } else { 128 };
            let raw = seg.get(p + 1..p + 1 + size).ok_or_else(|| err("short DQT"))?;
            let mut table = [0u16; 64];
            for (k, &z) in ZIGZAG.iter().enumerate() {
                table[z] = if pq == 0 { u16::from(raw[k]) } else { u16::from_be_bytes([raw[2 * k], raw[2 * k + 1]]) };
            }
            self.qt[tq] = Some(table);
            p += 1 + size;
        }
        Ok(())
    }

    /// Parses SOS and prepares the frame for it. `None` skips a scan whose parameters make no sense.
    fn start_scan(&mut self, seg: &[u8]) -> Result<Option<Scan>, CodecError> {
        let frame = self.frame.as_mut().ok_or_else(|| err("scan before frame header"))?;
        let ns = usize::from(*seg.first().ok_or_else(|| err("short SOS"))?);
        if ns == 0 || ns > 4 || seg.len() < 4 + 2 * ns {
            return Err(err("bad SOS"));
        }
        let mut comps = Vec::with_capacity(ns);
        for i in 0..ns {
            let (id, tables) = (seg[1 + 2 * i], seg[2 + 2 * i]);
            let Some(ci) = frame.comps.iter().position(|c| c.id == id) else { return Ok(None) };
            let (td, ta) = (usize::from(tables >> 4), usize::from(tables & 15));
            if td > 3 || ta > 3 || comps.iter().any(|&(c, _, _)| c == ci) {
                return Ok(None);
            }
            comps.push((ci, td, ta));
        }
        let p = 1 + 2 * ns;
        let (ss, se, ah, al) =
            (usize::from(seg[p]), usize::from(seg[p + 1]), u32::from(seg[p + 2] >> 4), u32::from(seg[p + 2] & 15));
        if frame.progressive {
            let dc = ss == 0;
            let valid = if dc { se == 0 } else { ss <= se && se <= 63 && ns == 1 };
            if !valid || al > 13 || ah > 13 {
                return Ok(None);
            }
        }
        // Quantisation tables must exist by now; Huffman tables fall back to the standard ones.
        for &(ci, _, _) in &comps {
            let c = &mut frame.comps[ci];
            let q = self.qt[c.tq].ok_or_else(|| err("missing quantization table"))?;
            if frame.progressive && c.qt.is_none() {
                c.qt = Some(scaled_qt(&q));
            }
        }
        if !frame.allocated {
            for c in &mut frame.comps {
                if frame.progressive {
                    c.coefs = vec![0; c.bw * c.bh * 64];
                } else {
                    c.plane = vec![128; c.bw * c.bh * 64];
                }
            }
            frame.allocated = true;
        }
        for (_, td, ta) in &comps {
            if self.dc[*td].is_none() {
                let (bits, vals) = if *td % 2 == 0 {
                    (&STD_DC_LUMA_BITS, &STD_DC_VALUES)
                } else {
                    (&STD_DC_CHROMA_BITS, &STD_DC_VALUES)
                };
                self.dc[*td] = Some(Huffman::new(bits, vals)?);
            }
            if self.ac[*ta].is_none() {
                let (bits, vals) = if *ta % 2 == 0 {
                    (&STD_AC_LUMA_BITS, &STD_AC_LUMA_VALUES[..])
                } else {
                    (&STD_AC_CHROMA_BITS, &STD_AC_CHROMA_VALUES[..])
                };
                self.ac[*ta] = Some(Huffman::new(bits, vals)?);
            }
        }
        Ok(Some(Scan { comps, ss, se, ah, al }))
    }

    /// Decodes one scan's entropy-coded data starting at `start`; returns where marker parsing resumes.
    fn decode_scan(&mut self, scan: &Scan, start: usize) -> usize {
        let data = self.data;
        let Some(frame) = self.frame.as_mut() else { return start };
        let kind = if !frame.progressive {
            ScanKind::Sequential
        } else {
            match (scan.ss == 0, scan.ah == 0) {
                (true, true) => ScanKind::DcFirst,
                (true, false) => ScanKind::DcRefine,
                (false, true) => ScanKind::AcFirst,
                (false, false) => ScanKind::AcRefine,
            }
        };
        let total_blocks: u64 = frame.comps.iter().map(|c| (c.bw * c.bh) as u64).sum();
        let budget = total_blocks.saturating_mul(WORK_FACTOR);

        // Sequential scans need the dequantisation tables now.
        let mut qts = Vec::with_capacity(scan.comps.len());
        for &(ci, _, _) in &scan.comps {
            qts.push(self.qt[frame.comps[ci].tq].map(|q| scaled_qt(&q)).unwrap_or([0.0; 64]));
        }

        let single = scan.comps.len() == 1;
        let (mcus_x, mcus_y) = if single {
            let c = &frame.comps[scan.comps[0].0];
            (c.cw.div_ceil(8), c.ch.div_ceil(8))
        } else {
            (frame.mcux, frame.mcuy)
        };
        let total = mcus_x * mcus_y;
        let blocks_per_mcu: u64 = if single {
            1
        } else {
            scan.comps.iter().map(|&(ci, _, _)| (frame.comps[ci].h * frame.comps[ci].v) as u64).sum()
        };

        let mut reader = Reader::new(data, start);
        for c in &mut frame.comps {
            c.dc_pred = 0;
        }
        let mut eobrun = 0u32;
        let mut block = [0f32; 64];
        let interval = self.restart;
        let mut next_rst = if interval > 0 { interval } else { usize::MAX };
        let mut expected_rst = 0u8;
        let mut dead = false;
        let mut mcu = 0usize;
        while mcu < total {
            if mcu == next_rst {
                match find_rst(data, reader.pos) {
                    Some((n, after)) => {
                        // A later RST than expected means intervals were lost: skip them to stay aligned.
                        let skipped = usize::from(n.wrapping_sub(expected_rst) & 7);
                        expected_rst = (n + 1) & 7;
                        reader.restart_at(after);
                        for c in &mut frame.comps {
                            c.dc_pred = 0;
                        }
                        eobrun = 0;
                        dead = false;
                        mcu += skipped * interval;
                        next_rst = mcu + interval;
                        if mcu >= total {
                            break;
                        }
                    }
                    None => break,
                }
            }
            if self.work > budget {
                break;
            }
            self.work += blocks_per_mcu;
            if !dead {
                let (mx, my) = (mcu % mcus_x, mcu / mcus_x);
                let mut ok = true;
                'mcu: for (si, &(ci, td, ta)) in scan.comps.iter().enumerate() {
                    let (Some(dc), Some(ac)) = (self.dc[td].as_ref(), self.ac[ta].as_ref()) else {
                        ok = false;
                        break;
                    };
                    let c = &mut frame.comps[ci];
                    let (nh, nv) = if single { (1, 1) } else { (c.h, c.v) };
                    for v in 0..nv {
                        for h in 0..nh {
                            let (bx, by) = if single { (mx, my) } else { (mx * c.h + h, my * c.v + v) };
                            let r = match kind {
                                ScanKind::Sequential => {
                                    let r = decode_block(&mut reader, dc, ac, &mut c.dc_pred, &mut block, &qts[si]);
                                    if let Ok(last) = r {
                                        let stride = c.bw * 8;
                                        let off = by * 8 * stride + bx * 8;
                                        if let Some(dst) = c.plane.get_mut(off..) {
                                            if last == 0 {
                                                fill_dc(block[0], dst, stride)
                                            } else {
                                                idct(&block, dst, stride)
                                            }
                                        }
                                    }
                                    r.map(|_| ())
                                }
                                _ => {
                                    let off = (by * c.bw + bx) * 64;
                                    match c.coefs.get_mut(off..off + 64) {
                                        Some(coef) => match kind {
                                            ScanKind::DcFirst => {
                                                dc_first(&mut reader, dc, &mut c.dc_pred, coef, scan.al)
                                            }
                                            ScanKind::DcRefine => {
                                                if reader.bit() {
                                                    coef[0] |= 1 << scan.al;
                                                }
                                                Ok(())
                                            }
                                            ScanKind::AcFirst => ac_first(&mut reader, ac, coef, scan, &mut eobrun),
                                            _ => ac_refine(&mut reader, ac, coef, scan, &mut eobrun),
                                        },
                                        None => Err(()),
                                    }
                                }
                            };
                            if r.is_err() {
                                ok = false;
                                break 'mcu;
                            }
                        }
                    }
                }
                if !ok || reader.overrun() {
                    dead = true;
                }
            }
            mcu += 1;
        }
        // Resume marker parsing where the entropy data stops.
        let resume = reader.pos.max(start);
        next_marker(data, resume).unwrap_or(data.len())
    }

    fn finish(mut self) -> Result<Decoded, CodecError> {
        let frame = self.frame.as_mut().ok_or_else(|| err("no frame header"))?;
        if self.scans == 0 || !frame.allocated {
            return Err(err("no image data"));
        }
        if frame.progressive {
            for c in &mut frame.comps {
                c.plane = vec![128; c.bw * c.bh * 64];
                let Some(q) = c.qt else { continue };
                let stride = c.bw * 8;
                let mut block = [0f32; 64];
                for by in 0..c.ch.div_ceil(8) {
                    for bx in 0..c.cw.div_ceil(8) {
                        let off = (by * c.bw + bx) * 64;
                        let coef = &c.coefs[off..off + 64];
                        let dc_only = coef[1..].iter().all(|&v| v == 0);
                        let dst = &mut c.plane[by * 8 * stride + bx * 8..];
                        if dc_only {
                            fill_dc(f32::from(coef[0]) * q[0], dst, stride);
                        } else {
                            for i in 0..64 {
                                block[i] = f32::from(coef[i]) * q[i];
                            }
                            idct(&block, dst, stride);
                        }
                    }
                }
                c.coefs = Vec::new();
            }
        }
        let nc = frame.comps.len();
        let (kind, transform) = match nc {
            1 => (ColorKind::Gray, Transform::None),
            3 => {
                let rgb_ids = frame.comps[0].id == b'R' && frame.comps[1].id == b'G' && frame.comps[2].id == b'B';
                match self.adobe {
                    Some(0) => (ColorKind::Rgb, Transform::None),
                    Some(_) => (ColorKind::Rgb, Transform::YCbCr),
                    None if rgb_ids => (ColorKind::Rgb, Transform::None),
                    None => (ColorKind::Rgb, Transform::YCbCr),
                }
            }
            _ => match self.adobe {
                None | Some(0) => (ColorKind::Cmyk, Transform::None),
                Some(_) => (ColorKind::Cmyk, Transform::Ycck),
            },
        };
        let data = convert(frame, transform);
        Ok(Decoded { width: frame.width as u32, height: frame.height as u32, kind, data, adobe: self.adobe.is_some() })
    }
}

/// The RST marker that ends the current restart interval, searched from `from`: (n, position after it). None when
/// another marker or the end of the data comes first.
fn find_rst(data: &[u8], from: usize) -> Option<(u8, usize)> {
    let mut i = from;
    while i + 1 < data.len() {
        if data[i] == 0xff {
            match data[i + 1] {
                0 | 0xff => {}
                m @ 0xd0..=0xd7 => return Some((m - 0xd0, i + 2)),
                _ => return None,
            }
        }
        i += 1;
    }
    None
}

/// AAN scale factors: the float IDCT below expects coefficients pre-multiplied by these.
const AAN: [f32; 8] = [1.0, 1.387_039_8, 1.306_563, 1.175_875_6, 1.0, 0.785_694_96, 0.541_196_1, 0.275_899_38];

/// Dequantisation table with the AAN factors and the final 1/8 folded in, natural order.
fn scaled_qt(q: &[u16; 64]) -> [f32; 64] {
    let mut s = [0f32; 64];
    for r in 0..8 {
        for c in 0..8 {
            s[r * 8 + c] = f32::from(q[r * 8 + c]) * AAN[r] * AAN[c] / 8.0;
        }
    }
    s
}

/// Decodes one sequential block into dequantised coefficients; returns the last zigzag index written.
#[inline]
fn decode_block(
    r: &mut Reader,
    dc: &Huffman,
    ac: &Huffman,
    pred: &mut i32,
    block: &mut [f32; 64],
    q: &[f32; 64],
) -> Result<usize, ()> {
    *block = [0.0; 64];
    let t = u32::from(r.decode(dc).ok_or(())?);
    if t > 16 {
        return Err(());
    }
    *pred = pred.wrapping_add(r.receive_extend(t));
    block[0] = *pred as f32 * q[0];
    let mut k = 1;
    let mut last = 0;
    while k < 64 {
        if r.nbits < 16 {
            r.refill();
        }
        let e = ac.fast_ac[(r.acc >> 55) as usize];
        if e != 0 {
            let len = (e & 0xff) as u32;
            r.acc <<= len;
            r.nbits -= len;
            k += ((e >> 8) & 0xff) as usize;
            if k > 63 {
                return Err(());
            }
            let z = ZIGZAG[k];
            block[z] = (e >> 16) as f32 * q[z];
            last = k;
            k += 1;
            continue;
        }
        let rs = r.decode(ac).ok_or(())?;
        let (run, s) = (usize::from(rs >> 4), u32::from(rs & 15));
        if s == 0 {
            if run != 15 {
                break;
            }
            k += 16;
            continue;
        }
        k += run;
        if k > 63 {
            return Err(());
        }
        let z = ZIGZAG[k];
        block[z] = r.receive_extend(s) as f32 * q[z];
        last = k;
        k += 1;
    }
    Ok(last)
}

fn dc_first(r: &mut Reader, dc: &Huffman, pred: &mut i32, coef: &mut [i16], al: u32) -> Result<(), ()> {
    let t = u32::from(r.decode(dc).ok_or(())?);
    if t > 16 {
        return Err(());
    }
    *pred = pred.wrapping_add(r.receive_extend(t));
    coef[0] = (*pred << al) as i16;
    Ok(())
}

fn ac_first(r: &mut Reader, ac: &Huffman, coef: &mut [i16], scan: &Scan, eobrun: &mut u32) -> Result<(), ()> {
    if *eobrun > 0 {
        *eobrun -= 1;
        return Ok(());
    }
    let mut k = scan.ss;
    while k <= scan.se {
        let rs = r.decode(ac).ok_or(())?;
        let (run, s) = (u32::from(rs >> 4), u32::from(rs & 15));
        if s == 0 {
            if run < 15 {
                *eobrun = (1 << run) - 1 + r.bits(run);
                break;
            }
            k += 16;
            continue;
        }
        k += run as usize;
        if k > scan.se {
            return Err(());
        }
        coef[ZIGZAG[k]] = (r.receive_extend(s) << scan.al) as i16;
        k += 1;
    }
    Ok(())
}

/// Successive approximation of AC coefficients (G.1.2.3), following libjpeg's decode_mcu_AC_refine.
fn ac_refine(r: &mut Reader, ac: &Huffman, coef: &mut [i16], scan: &Scan, eobrun: &mut u32) -> Result<(), ()> {
    let p1: i16 = 1 << scan.al;
    let m1: i16 = -1 << scan.al;
    let refine = |r: &mut Reader, c: &mut i16| {
        if r.bit() && (*c & p1) == 0 {
            *c = if *c >= 0 { c.wrapping_add(p1) } else { c.wrapping_add(m1) };
        }
    };
    let mut k = scan.ss;
    if *eobrun == 0 {
        while k <= scan.se {
            let rs = r.decode(ac).ok_or(())?;
            let (mut run, s) = (u32::from(rs >> 4), rs & 15);
            let mut value = 0;
            if s != 0 {
                value = if r.bit() { p1 } else { m1 };
            } else if run != 15 {
                *eobrun = (1 << run) + r.bits(run);
                break;
            }
            // Refine nonzero coefficients on the way to the run-th zero one.
            while k <= scan.se {
                let c = &mut coef[ZIGZAG[k]];
                if *c != 0 {
                    refine(r, c);
                } else {
                    if run == 0 {
                        break;
                    }
                    run -= 1;
                }
                k += 1;
            }
            if s != 0 && k <= scan.se {
                coef[ZIGZAG[k]] = value;
            }
            k += 1;
        }
    }
    if *eobrun > 0 {
        while k <= scan.se {
            let c = &mut coef[ZIGZAG[k]];
            if *c != 0 {
                refine(r, c);
            }
            k += 1;
        }
        *eobrun -= 1;
    }
    Ok(())
}

#[inline]
fn to_u8(v: f32) -> u8 {
    (v + 128.5).clamp(0.0, 255.0) as u8
}

fn fill_dc(dc: f32, dst: &mut [u8], stride: usize) {
    let v = to_u8(dc);
    for row in dst.chunks_mut(stride).take(8) {
        if let Some(px) = row.get_mut(..8) {
            px.fill(v);
        }
    }
}

/// One AAN 1-D IDCT pass over the eight columns of `inp` (row-major), written so each step is the same across
/// the eight lanes and vectorises.
#[inline(always)]
fn idct_pass(inp: &[f32; 64], out: &mut [f32; 64]) {
    for c in 0..8 {
        let (x0, x1, x2, x3) = (inp[c], inp[8 + c], inp[16 + c], inp[24 + c]);
        let (x4, x5, x6, x7) = (inp[32 + c], inp[40 + c], inp[48 + c], inp[56 + c]);
        // Even part.
        let tmp10 = x0 + x4;
        let tmp11 = x0 - x4;
        let tmp13 = x2 + x6;
        let tmp12 = (x2 - x6) * std::f32::consts::SQRT_2 - tmp13;
        let t0 = tmp10 + tmp13;
        let t3 = tmp10 - tmp13;
        let t1 = tmp11 + tmp12;
        let t2 = tmp11 - tmp12;
        // Odd part.
        let z13 = x5 + x3;
        let z10 = x5 - x3;
        let z11 = x1 + x7;
        let z12 = x1 - x7;
        let t7 = z11 + z13;
        let t11 = (z11 - z13) * std::f32::consts::SQRT_2;
        let z5 = (z10 + z12) * 1.847_759;
        let t10 = 1.082_392_2 * z12 - z5;
        let t12 = -2.613_126 * z10 + z5;
        let t6 = t12 - t7;
        let t5 = t11 - t6;
        let t4 = t10 + t5;
        out[c] = t0 + t7;
        out[56 + c] = t0 - t7;
        out[8 + c] = t1 + t6;
        out[48 + c] = t1 - t6;
        out[16 + c] = t2 + t5;
        out[40 + c] = t2 - t5;
        out[32 + c] = t3 + t4;
        out[24 + c] = t3 - t4;
    }
}

/// Inverse DCT of dequantised, AAN-scaled coefficients into an 8x8 block of `dst`.
fn idct(block: &[f32; 64], dst: &mut [u8], stride: usize) {
    let mut tmp = [0f32; 64];
    idct_pass(block, &mut tmp);
    let mut t = [0f32; 64];
    for r in 0..8 {
        for c in 0..8 {
            t[c * 8 + r] = tmp[r * 8 + c];
        }
    }
    idct_pass(&t, &mut tmp);
    // tmp[k * 8 + r] is the sample at row r, column k.
    for (r, row) in dst.chunks_mut(stride).take(8).enumerate() {
        if let Some(px) = row.get_mut(..8) {
            for (k, p) in px.iter_mut().enumerate() {
                *p = to_u8(tmp[k * 8 + r]);
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Transform {
    None,
    YCbCr,
    Ycck,
}

/// How a component reaches full resolution.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Upsample {
    Full,
    H2V2,
    H2V1,
    H1V2,
    Nearest,
}

/// Output row `y` of component `c` at full resolution: the plane row itself for full-size components, otherwise
/// upsampled into `out` (at least `2 * cw` long).
fn upsample_row<'a>(
    c: &'a Component,
    mode: Upsample,
    f: &Frame,
    y: usize,
    out: &'a mut [u8],
    colsum: &mut [u16],
    xmap: &[usize],
) -> &'a [u8] {
    let stride = c.bw * 8;
    let row = |i: usize| -> &'a [u8] {
        let i = i.min(c.ch - 1);
        &c.plane[i * stride..i * stride + c.cw]
    };
    let cw = c.cw;
    match mode {
        Upsample::Full => return row(y),
        Upsample::H2V1 => fancy_h2(row(y), out),
        Upsample::H1V2 => {
            let cy = y / 2;
            let (far, bias) = if y.is_multiple_of(2) { (cy.saturating_sub(1), 1) } else { (cy + 1, 2) };
            for ((o, &n), &fr) in out.iter_mut().zip(row(cy)).zip(row(far)) {
                *o = ((u16::from(n) * 3 + u16::from(fr) + bias) >> 2) as u8;
            }
        }
        Upsample::H2V2 => {
            let cy = y / 2;
            let far = if y.is_multiple_of(2) { cy.saturating_sub(1) } else { cy + 1 };
            let cs = &mut colsum[..cw];
            for ((c, &n), &fr) in cs.iter_mut().zip(row(cy)).zip(row(far)) {
                *c = u16::from(n) * 3 + u16::from(fr);
            }
            let cs = &colsum[..cw];
            out[0] = ((cs[0] * 4 + 8) >> 4) as u8;
            if cw == 1 {
                out[1] = ((cs[0] * 4 + 7) >> 4) as u8;
            } else {
                out[1] = ((cs[0] * 3 + cs[1] + 7) >> 4) as u8;
                for (o, win) in out[2..2 * cw - 2].as_chunks_mut::<2>().0.iter_mut().zip(cs.windows(3)) {
                    let mid = win[1] * 3;
                    *o = [((mid + win[0] + 8) >> 4) as u8, ((mid + win[2] + 7) >> 4) as u8];
                }
                out[2 * cw - 2] = ((cs[cw - 1] * 3 + cs[cw - 2] + 8) >> 4) as u8;
                out[2 * cw - 1] = ((cs[cw - 1] * 4 + 7) >> 4) as u8;
            }
        }
        Upsample::Nearest => {
            let src = row(y * c.v / f.vmax);
            for (o, &x) in out.iter_mut().zip(xmap) {
                *o = src[x];
            }
        }
    }
    out
}

/// libjpeg's h2v1 "fancy" (triangle) horizontal doubling.
fn fancy_h2(src: &[u8], out: &mut [u8]) {
    let n = src.len();
    if n == 1 {
        out[0] = src[0];
        out[1] = src[0];
        return;
    }
    let s = |i: usize| u16::from(src[i]);
    out[0] = src[0];
    out[1] = ((s(0) * 3 + s(1) + 2) >> 2) as u8;
    for (o, win) in out[2..2 * n - 2].as_chunks_mut::<2>().0.iter_mut().zip(src.windows(3)) {
        let mid = u16::from(win[1]) * 3;
        *o = [((mid + u16::from(win[0]) + 1) >> 2) as u8, ((mid + u16::from(win[2]) + 2) >> 2) as u8];
    }
    out[2 * n - 2] = ((s(n - 1) * 3 + s(n - 2) + 1) >> 2) as u8;
    out[2 * n - 1] = src[n - 1];
}

// YCbCr to RGB in 16-bit fixed point, as libjpeg's jdcolor.c.
const FIX_1_402: i32 = 91881;
const FIX_1_772: i32 = 116_130;
const FIX_0_714: i32 = 46802;
const FIX_0_344: i32 = 22554;

#[inline]
fn ycc_to_rgb(y: u8, cb: u8, cr: u8) -> [u8; 3] {
    let (y, cb, cr) = (i32::from(y), i32::from(cb) - 128, i32::from(cr) - 128);
    let r = y + ((FIX_1_402 * cr + 32768) >> 16);
    let g = y + ((-FIX_0_344 * cb - FIX_0_714 * cr + 32768) >> 16);
    let b = y + ((FIX_1_772 * cb + 32768) >> 16);
    [r.clamp(0, 255) as u8, g.clamp(0, 255) as u8, b.clamp(0, 255) as u8]
}

/// Upsamples every component and converts to the output colour model, one row at a time.
fn convert(f: &Frame, transform: Transform) -> Vec<u8> {
    let (w, h, nc) = (f.width, f.height, f.comps.len());
    let mut out = vec![0u8; w * h * nc];
    let modes: Vec<Upsample> = f
        .comps
        .iter()
        .map(|c| match (f.hmax == c.h, f.vmax == c.v, f.hmax == 2 * c.h, f.vmax == 2 * c.v) {
            (true, true, _, _) => Upsample::Full,
            (_, _, true, true) => Upsample::H2V2,
            (_, true, true, _) => Upsample::H2V1,
            (true, _, _, true) => Upsample::H1V2,
            _ => Upsample::Nearest,
        })
        .collect();
    let xmaps: Vec<Vec<usize>> = f
        .comps
        .iter()
        .zip(&modes)
        .map(|(c, m)| {
            if *m == Upsample::Nearest {
                (0..w).map(|x| (x * c.h / f.hmax).min(c.cw - 1)).collect()
            } else {
                Vec::new()
            }
        })
        .collect();
    let row_len = w.max(2 * f.comps.iter().map(|c| c.cw).max().unwrap_or(1)) + 2;
    let mut bufs: Vec<Vec<u8>> = vec![vec![0u8; row_len]; nc];
    let mut colsum = vec![0u16; row_len];
    for (y, line) in out.chunks_exact_mut(w * nc).enumerate() {
        let mut rows: [&[u8]; 4] = [&[]; 4];
        for (i, buf) in bufs.iter_mut().enumerate() {
            rows[i] = &upsample_row(&f.comps[i], modes[i], f, y, buf, &mut colsum, &xmaps[i])[..w];
        }
        match (nc, transform) {
            (1, _) => line.copy_from_slice(rows[0]),
            (3, Transform::None) => {
                for (((px, &r), &g), &b) in
                    line.as_chunks_mut::<3>().0.iter_mut().zip(rows[0]).zip(rows[1]).zip(rows[2])
                {
                    *px = [r, g, b];
                }
            }
            (3, _) => {
                for (((px, &yy), &cb), &cr) in
                    line.as_chunks_mut::<3>().0.iter_mut().zip(rows[0]).zip(rows[1]).zip(rows[2])
                {
                    *px = ycc_to_rgb(yy, cb, cr);
                }
            }
            (_, Transform::Ycck) => {
                let pixels = line.as_chunks_mut::<4>().0.iter_mut().zip(rows[0]).zip(rows[1]).zip(rows[2]).zip(rows[3]);
                for ((((px, &yy), &cb), &cr), &k) in pixels {
                    let [r, g, b] = ycc_to_rgb(yy, cb, cr);
                    *px = [255 - r, 255 - g, 255 - b, k];
                }
            }
            _ => {
                let pixels = line.as_chunks_mut::<4>().0.iter_mut().zip(rows[0]).zip(rows[1]).zip(rows[2]).zip(rows[3]);
                for ((((px, &c), &m), &yy), &k) in pixels {
                    *px = [c, m, yy, k];
                }
            }
        }
    }
    let _ = h;
    out
}

// ---------------------------------------------------------------------------------------------------------------
// Encoder
// ---------------------------------------------------------------------------------------------------------------

/// Baseline encoder: `components` 1 (gray) or 3 (RGB, written as YCbCr 4:2:0 with JFIF APP0); quality 1..=100
/// scales the Annex K tables like libjpeg.
pub fn encode(pixels: &[u8], width: u32, height: u32, components: u8, quality: u8) -> Result<Vec<u8>, CodecError> {
    encode_with_restart(pixels, width, height, components, quality, 0)
}

/// [`encode`] with a restart interval in MCUs (0 for none).
fn encode_with_restart(
    pixels: &[u8],
    width: u32,
    height: u32,
    components: u8,
    quality: u8,
    restart: u16,
) -> Result<Vec<u8>, CodecError> {
    if width == 0 || height == 0 || width > MAX_DIM || height > MAX_DIM {
        return Err(err(format!("bad dimensions {width}x{height}")));
    }
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(err(format!("image too large ({width}x{height})")));
    }
    if components != 1 && components != 3 {
        return Err(err("only 1 or 3 components can be encoded"));
    }
    if !(1..=100).contains(&quality) {
        return Err(err("quality must be 1..=100"));
    }
    let (w, h, nc) = (width as usize, height as usize, usize::from(components));
    if pixels.len() != w * h * nc {
        return Err(err(format!("expected {} bytes of pixels, got {}", w * h * nc, pixels.len())));
    }
    let q = u32::from(quality);
    let scale = if q < 50 { 5000 / q } else { 200 - 2 * q };
    let make_qt = |base: &[u8; 64]| -> [u8; 64] {
        std::array::from_fn(|i| ((u32::from(base[i]) * scale + 50) / 100).clamp(1, 255) as u8)
    };
    let luma_qt = make_qt(&STD_LUMA_QT);
    let chroma_qt = make_qt(&STD_CHROMA_QT);

    let mut out = Vec::with_capacity(w * h / 4 + 1024);
    out.extend_from_slice(&[0xff, 0xd8]);
    // JFIF APP0: version 1.01, aspect ratio 1:1, no thumbnail.
    out.extend_from_slice(&[0xff, 0xe0, 0, 16, b'J', b'F', b'I', b'F', 0, 1, 1, 0, 0, 1, 0, 1, 0, 0]);
    let tables: &[(u8, &[u8; 64])] = if nc == 1 { &[(0, &luma_qt)] } else { &[(0, &luma_qt), (1, &chroma_qt)] };
    for &(id, qt) in tables {
        out.extend_from_slice(&[0xff, 0xdb, 0, 67, id]);
        out.extend(ZIGZAG.iter().map(|&z| qt[z]));
    }
    // SOF0
    let sof_len = 8 + 3 * nc as u16;
    out.extend_from_slice(&[0xff, 0xc0]);
    out.extend_from_slice(&sof_len.to_be_bytes());
    out.push(8);
    out.extend_from_slice(&(height as u16).to_be_bytes());
    out.extend_from_slice(&(width as u16).to_be_bytes());
    out.push(nc as u8);
    if nc == 1 {
        out.extend_from_slice(&[1, 0x11, 0]);
    } else {
        out.extend_from_slice(&[1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
    }
    let huff: &[(u8, &[u8; 16], &[u8])] = &[
        (0x00, &STD_DC_LUMA_BITS, &STD_DC_VALUES),
        (0x10, &STD_AC_LUMA_BITS, &STD_AC_LUMA_VALUES),
        (0x01, &STD_DC_CHROMA_BITS, &STD_DC_VALUES),
        (0x11, &STD_AC_CHROMA_BITS, &STD_AC_CHROMA_VALUES),
    ];
    for &(id, bits, vals) in &huff[..if nc == 1 { 2 } else { 4 }] {
        let len = 3 + 16 + vals.len() as u16;
        out.extend_from_slice(&[0xff, 0xc4]);
        out.extend_from_slice(&len.to_be_bytes());
        out.push(id);
        out.extend_from_slice(bits);
        out.extend_from_slice(vals);
    }
    if restart > 0 {
        out.extend_from_slice(&[0xff, 0xdd, 0, 4]);
        out.extend_from_slice(&restart.to_be_bytes());
    }
    // SOS
    if nc == 1 {
        out.extend_from_slice(&[0xff, 0xda, 0, 8, 1, 1, 0x00, 0, 63, 0]);
    } else {
        out.extend_from_slice(&[0xff, 0xda, 0, 12, 3, 1, 0x00, 2, 0x11, 3, 0x11, 0, 63, 0]);
    }

    let luma =
        (HuffCodes::new(&STD_DC_LUMA_BITS, &STD_DC_VALUES), HuffCodes::new(&STD_AC_LUMA_BITS, &STD_AC_LUMA_VALUES));
    let chroma = (
        HuffCodes::new(&STD_DC_CHROMA_BITS, &STD_DC_VALUES),
        HuffCodes::new(&STD_AC_CHROMA_BITS, &STD_AC_CHROMA_VALUES),
    );
    let luma_div: [f32; 64] = std::array::from_fn(|i| f32::from(luma_qt[i]));
    let chroma_div: [f32; 64] = std::array::from_fn(|i| f32::from(chroma_qt[i]));
    let mut bw = BitWriter { out, acc: 0, n: 0 };
    let mcu_size = if nc == 1 { 8 } else { 16 };
    let (mcux, mcuy) = (w.div_ceil(mcu_size), h.div_ceil(mcu_size));
    let mut preds = [0i32; 3];
    let mut rst = 0u8;
    let mut blk = [0f32; 64];
    let px = |x: usize, y: usize| -> usize { (y.min(h - 1) * w + x.min(w - 1)) * nc };
    for my in 0..mcuy {
        for mx in 0..mcux {
            let index = my * mcux + mx;
            if restart > 0 && index > 0 && index % usize::from(restart) == 0 {
                bw.flush();
                bw.out.extend_from_slice(&[0xff, 0xd0 + rst]);
                rst = (rst + 1) & 7;
                preds = [0; 3];
            }
            if nc == 1 {
                for y in 0..8 {
                    for x in 0..8 {
                        blk[y * 8 + x] = f32::from(pixels[px(mx * 8 + x, my * 8 + y)]) - 128.0;
                    }
                }
                encode_block(&mut bw, &blk, &luma_div, &luma, &mut preds[0]);
                continue;
            }
            // 16x16 RGB to YCbCr; chroma averaged over 2x2.
            let mut ys = [0f32; 256];
            let mut cb = [0f32; 64];
            let mut cr = [0f32; 64];
            for y in 0..16 {
                for x in 0..16 {
                    let p = px(mx * 16 + x, my * 16 + y);
                    let (r, g, b) = (f32::from(pixels[p]), f32::from(pixels[p + 1]), f32::from(pixels[p + 2]));
                    ys[y * 16 + x] = 0.299 * r + 0.587 * g + 0.114 * b - 128.0;
                    let ci = (y / 2) * 8 + x / 2;
                    cb[ci] += (-0.168_736 * r - 0.331_264 * g + 0.5 * b) * 0.25;
                    cr[ci] += (0.5 * r - 0.418_688 * g - 0.081_312 * b) * 0.25;
                }
            }
            for (by, bx) in [(0, 0), (0, 8), (8, 0), (8, 8)] {
                for y in 0..8 {
                    for x in 0..8 {
                        blk[y * 8 + x] = ys[(by + y) * 16 + bx + x];
                    }
                }
                encode_block(&mut bw, &blk, &luma_div, &luma, &mut preds[0]);
            }
            encode_block(&mut bw, &cb, &chroma_div, &chroma, &mut preds[1]);
            encode_block(&mut bw, &cr, &chroma_div, &chroma, &mut preds[2]);
        }
    }
    bw.flush();
    let mut out = bw.out;
    out.extend_from_slice(&[0xff, 0xd9]);
    Ok(out)
}

/// Per-symbol code words for encoding (Annex C).
struct HuffCodes {
    code: [u16; 256],
    size: [u8; 256],
}

impl HuffCodes {
    fn new(bits: &[u8; 16], vals: &[u8]) -> HuffCodes {
        let mut t = HuffCodes { code: [0; 256], size: [0; 256] };
        let mut code = 0u16;
        let mut k = 0;
        for len in 1..=16u8 {
            for _ in 0..bits[usize::from(len) - 1] {
                t.code[usize::from(vals[k])] = code;
                t.size[usize::from(vals[k])] = len;
                code += 1;
                k += 1;
            }
            code <<= 1;
        }
        t
    }
}

struct BitWriter {
    out: Vec<u8>,
    acc: u32,
    n: u32,
}

impl BitWriter {
    #[inline]
    fn put(&mut self, code: u32, len: u32) {
        self.acc = (self.acc << len) | (code & ((1 << len) - 1));
        self.n += len;
        while self.n >= 8 {
            let b = (self.acc >> (self.n - 8)) as u8;
            self.out.push(b);
            if b == 0xff {
                self.out.push(0);
            }
            self.n -= 8;
        }
    }

    /// Pads the last byte with 1 bits.
    fn flush(&mut self) {
        if self.n > 0 {
            let pad = 8 - self.n;
            self.put((1 << pad) - 1, pad);
        }
    }
}

/// Orthonormal 8-point DCT basis, `C[u][x]`.
fn dct_basis() -> &'static [[f32; 8]; 8] {
    static BASIS: std::sync::OnceLock<[[f32; 8]; 8]> = std::sync::OnceLock::new();
    BASIS.get_or_init(|| {
        std::array::from_fn(|u| {
            std::array::from_fn(|x| {
                let cu = if u == 0 { std::f64::consts::FRAC_1_SQRT_2 } else { 1.0 };
                (cu / 2.0 * ((2 * x + 1) as f64 * u as f64 * std::f64::consts::PI / 16.0).cos()) as f32
            })
        })
    })
}

/// Forward DCT, quantisation and Huffman coding of one level-shifted block.
fn encode_block(bw: &mut BitWriter, blk: &[f32; 64], div: &[f32; 64], codes: &(HuffCodes, HuffCodes), pred: &mut i32) {
    let c = dct_basis();
    let mut tmp = [0f32; 64];
    // Rows: tmp[y][u] = sum_x C[u][x] f[y][x]
    for (row, out) in blk.as_chunks::<8>().0.iter().zip(tmp.as_chunks_mut::<8>().0) {
        for (o, cu) in out.iter_mut().zip(c) {
            *o = cu.iter().zip(row).map(|(a, b)| a * b).sum();
        }
    }
    // Columns: F[v][u] = sum_y C[v][y] tmp[y][u], then quantise.
    let mut q = [0i32; 64];
    for (v, cv) in c.iter().enumerate() {
        for u in 0..8 {
            let s: f32 = cv.iter().enumerate().map(|(y, a)| a * tmp[y * 8 + u]).sum();
            let i = v * 8 + u;
            // Baseline Huffman tables code AC magnitudes up to 1023 and DC differences up to 2047.
            q[i] = ((s / div[i]).round() as i32).clamp(-1023, 1023);
        }
    }
    let (dc, ac) = codes;
    let diff = q[0] - *pred;
    *pred = q[0];
    let (cat, extra) = magnitude(diff);
    bw.put(u32::from(dc.code[cat as usize]), u32::from(dc.size[cat as usize]));
    bw.put(extra, cat);
    let mut run = 0;
    for &z in &ZIGZAG[1..] {
        let v = q[z];
        if v == 0 {
            run += 1;
            continue;
        }
        while run > 15 {
            bw.put(u32::from(ac.code[0xf0]), u32::from(ac.size[0xf0]));
            run -= 16;
        }
        let (cat, extra) = magnitude(v);
        let sym = (run << 4) as usize | cat as usize;
        bw.put(u32::from(ac.code[sym]), u32::from(ac.size[sym]));
        bw.put(extra, cat);
        run = 0;
    }
    if run > 0 {
        bw.put(u32::from(ac.code[0]), u32::from(ac.size[0]));
    }
}

/// Category (bit count) and the extra bits of a coefficient (F.1.2.1).
fn magnitude(v: i32) -> (u32, u32) {
    if v == 0 {
        return (0, 0);
    }
    let a = v.unsigned_abs();
    let cat = 32 - a.leading_zeros();
    let extra = if v < 0 { (v - 1) as u32 & ((1 << cat) - 1) } else { a };
    (cat, extra)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// Smooth gradients, a few hard edges and a little texture: close to photographic content.
    fn test_image(w: usize, h: usize, nc: usize) -> Vec<u8> {
        let mut v = Vec::with_capacity(w * h * nc);
        for y in 0..h {
            for x in 0..w {
                let (fx, fy) = (x as f32 / w.max(1) as f32, y as f32 / h.max(1) as f32);
                let ring = (((fx - 0.5).powi(2) + (fy - 0.4).powi(2)).sqrt() * 18.0).sin();
                let edge = if (x / 37 + y / 29) % 2 == 0 { 30.0 } else { -30.0 };
                for c in 0..nc {
                    let base = 128.0 + 70.0 * ((fx * 3.0 + c as f32).sin() * (fy * 2.0 - c as f32 * 0.7).cos());
                    let val = base + 25.0 * ring + edge * (c as f32 - 1.0).abs().min(1.0);
                    v.push(val.clamp(0.0, 255.0) as u8);
                }
            }
        }
        v
    }

    fn psnr(a: &[u8], b: &[u8]) -> f64 {
        assert_eq!(a.len(), b.len());
        let mse: f64 =
            a.iter().zip(b).map(|(&x, &y)| (f64::from(x) - f64::from(y)).powi(2)).sum::<f64>() / a.len() as f64;
        if mse == 0.0 { 99.0 } else { 10.0 * (255.0f64 * 255.0 / mse).log10() }
    }

    fn mae(a: &[u8], b: &[u8]) -> f64 {
        assert_eq!(a.len(), b.len());
        a.iter().zip(b).map(|(&x, &y)| (f64::from(x) - f64::from(y)).abs()).sum::<f64>() / a.len() as f64
    }

    #[test]
    fn round_trip_gray_and_rgb() {
        for &(w, h) in &[(1usize, 1usize), (7, 5), (8, 8), (16, 16), (17, 33), (100, 75), (333, 251)] {
            for &nc in &[1usize, 3] {
                for &(quality, min_psnr) in &[(50u8, 24.0), (90, 30.0), (100, 36.0)] {
                    let img = test_image(w, h, nc);
                    let jpg = encode(&img, w as u32, h as u32, nc as u8, quality).unwrap();
                    let d = decode(&jpg).unwrap();
                    assert_eq!((d.width, d.height), (w as u32, h as u32));
                    assert_eq!(d.kind, if nc == 1 { ColorKind::Gray } else { ColorKind::Rgb });
                    assert!(!d.adobe);
                    let p = psnr(&img, &d.data);
                    // Chroma subsampling of tiny images costs more than quantisation does.
                    let floor = if nc == 3 && w < 16 { min_psnr - 6.0 } else { min_psnr };
                    assert!(p >= floor, "{w}x{h}x{nc} q{quality}: PSNR {p:.1} < {floor}");
                    assert_eq!(info(&jpg).unwrap(), (w as u32, h as u32, nc as u8));
                }
            }
        }
    }

    #[test]
    fn restart_intervals_decode_identically() {
        let (w, h) = (130, 70);
        for nc in [1u8, 3] {
            let img = test_image(w, h, usize::from(nc));
            let plain = decode(&encode(&img, w as u32, h as u32, nc, 85).unwrap()).unwrap();
            for ri in [1u16, 3, 7] {
                let jpg = encode_with_restart(&img, w as u32, h as u32, nc, 85, ri).unwrap();
                assert_eq!(decode(&jpg).unwrap().data, plain.data, "nc {nc} interval {ri}");
            }
        }
    }

    /// Byte ranges of each restart interval's entropy data in a file written by our encoder.
    fn intervals(jpg: &[u8]) -> Vec<(usize, usize)> {
        let sos = (0..jpg.len() - 1).find(|&i| jpg[i] == 0xff && jpg[i + 1] == 0xda).unwrap();
        let mut start = sos + 2 + be16(jpg, sos + 2).unwrap();
        let mut out = Vec::new();
        let mut i = start;
        while i + 1 < jpg.len() {
            if jpg[i] == 0xff && jpg[i + 1] != 0 {
                out.push((start, i));
                start = i + 2;
                i += 2;
                continue;
            }
            i += 1;
        }
        out
    }

    #[test]
    fn corrupt_interval_resyncs() {
        let (w, h) = (160, 64);
        let img = test_image(w, h, 3);
        // One MCU row (10 MCUs of 16x16) per interval.
        let jpg = encode_with_restart(&img, w as u32, h as u32, 3, 85, 10).unwrap();
        let clean = decode(&jpg).unwrap();
        let iv = intervals(&jpg);
        assert_eq!(iv.len(), 4);
        for garbage in [0x00u8, 0x55, 0xff] {
            let mut bad = jpg.clone();
            let (s, e) = iv[1];
            // Garble the entropy data without forging markers (a forged marker legitimately ends the scan).
            for i in s + 2..e - 2 {
                if bad[i] != 0xff && bad[i - 1] != 0xff {
                    let v = if garbage == 0xff { 0x00 } else { bad[i] ^ garbage };
                    bad[i] = if v == 0xff { 0xfe } else { v };
                }
            }
            if garbage == 0xff {
                // An interval cut short: drop most of its data.
                bad.drain(s + 4..e);
            }
            let d = decode(&bad).unwrap();
            // Fancy upsampling blends chroma across the interval boundary, so leave out one row on each side.
            let line = w * 3;
            assert_eq!(&d.data[..15 * line], &clean.data[..15 * line], "interval 0 must be untouched");
            assert_eq!(
                &d.data[33 * line..],
                &clean.data[33 * line..],
                "garbage {garbage:#x}: intervals after the damage must resync"
            );
        }
        // A missing RST marker: the following intervals are found by number.
        let mut bad = jpg.clone();
        let (_, e) = iv[0];
        bad.drain(e..e + 2);
        let d = decode(&bad).unwrap();
        assert_eq!(d.data.len(), clean.data.len());
    }

    /// Removes every DHT segment.
    fn strip_marker(jpg: &[u8], marker: u8) -> Vec<u8> {
        let mut out = jpg[..2].to_vec();
        let mut pos = 2;
        while pos + 4 <= jpg.len() && jpg[pos] == 0xff {
            let m = jpg[pos + 1];
            if m == 0xda {
                out.extend_from_slice(&jpg[pos..]);
                break;
            }
            let len = be16(jpg, pos + 2).unwrap();
            if m != marker {
                out.extend_from_slice(&jpg[pos..pos + 2 + len]);
            }
            pos += 2 + len;
        }
        out
    }

    #[test]
    fn missing_tables() {
        let img = test_image(48, 40, 3);
        let jpg = encode(&img, 48, 40, 3, 80).unwrap();
        // Our encoder uses the Annex K Huffman tables, the decoder's fallback, so dropping them changes nothing.
        let no_dht = strip_marker(&jpg, 0xc4);
        assert!(no_dht.len() < jpg.len());
        assert_eq!(decode(&no_dht).unwrap().data, decode(&jpg).unwrap().data);
        assert!(decode(&strip_marker(&jpg, 0xdb)).is_err(), "no quantization tables");
    }

    #[test]
    fn sixteen_bit_quantization_tables() {
        let img = test_image(40, 30, 3);
        let jpg = encode(&img, 40, 30, 3, 85).unwrap();
        // Rewrite each 8-bit DQT segment as a 16-bit one.
        let mut wide = Vec::new();
        let mut i = 0;
        while i < jpg.len() {
            if jpg[i] == 0xff && jpg.get(i + 1) == Some(&0xdb) {
                let id = jpg[i + 4];
                wide.extend_from_slice(&[0xff, 0xdb, 0, 131, 0x10 | id]);
                for &q in &jpg[i + 5..i + 69] {
                    wide.extend_from_slice(&[0, q]);
                }
                i += 69;
            } else {
                wide.push(jpg[i]);
                i += 1;
            }
        }
        assert_eq!(wide.len(), jpg.len() + 128);
        assert_eq!(decode(&wide).unwrap().data, decode(&jpg).unwrap().data);
    }

    fn lcg(seed: &mut u32) -> u32 {
        *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        *seed >> 8
    }

    #[test]
    fn hostile_inputs_never_panic() {
        let img = test_image(70, 45, 3);
        let jpg = encode_with_restart(&img, 70, 45, 3, 75, 4).unwrap();
        let gray = encode(&test_image(33, 20, 1), 33, 20, 1, 75).unwrap();
        for file in [&jpg, &gray] {
            for cut in 0..file.len() {
                let r = decode(&file[..cut]);
                if let Ok(d) = &r {
                    assert_eq!(d.data.len(), (d.width * d.height) as usize * d.kind.channels());
                }
                let _ = info(&file[..cut]);
            }
        }
        let mut seed = 7;
        for _ in 0..3000 {
            let mut bad = jpg.clone();
            for _ in 0..1 + lcg(&mut seed) % 8 {
                let i = lcg(&mut seed) as usize % bad.len();
                bad[i] = lcg(&mut seed) as u8;
            }
            if let Ok(d) = decode(&bad) {
                assert_eq!(d.data.len(), (d.width * d.height) as usize * d.kind.channels());
            }
        }
        for len in [0usize, 1, 2, 3, 10, 100, 1000] {
            let mut junk: Vec<u8> = (0..len).map(|_| lcg(&mut seed) as u8).collect();
            assert!(decode(&junk).is_err());
            junk.splice(0..0, [0xff, 0xd8]);
            let _ = decode(&junk);
        }
        // Absurd and degenerate frame headers.
        let sof = |w: u16, h: u16, nf: u8, prec: u8, sampling: u8| {
            let mut v = vec![0xff, 0xd8, 0xff, 0xc0, 0, 8 + 3 * nf, prec];
            v.extend_from_slice(&h.to_be_bytes());
            v.extend_from_slice(&w.to_be_bytes());
            v.push(nf);
            for i in 0..nf {
                v.extend_from_slice(&[i + 1, sampling, 0]);
            }
            v.extend_from_slice(&[0xff, 0xd9]);
            v
        };
        assert!(decode(&sof(65535, 65535, 3, 8, 0x11)).is_err());
        assert!(info(&sof(65535, 65535, 3, 8, 0x11)).is_err());
        assert!(decode(&sof(0, 10, 1, 8, 0x11)).is_err());
        assert!(decode(&sof(10, 0, 1, 8, 0x11)).is_err());
        assert!(decode(&sof(10, 10, 1, 12, 0x11)).is_err());
        assert!(decode(&sof(10, 10, 2, 8, 0x11)).is_err());
        assert!(decode(&sof(10, 10, 1, 8, 0x51)).is_err());
        assert!(decode(&sof(10, 10, 3, 8, 0x11)).is_err(), "no scan");
        assert_eq!(info(&sof(4000, 3000, 3, 8, 0x22)).unwrap(), (4000, 3000, 3));
        let mut arith = sof(10, 10, 1, 8, 0x11);
        arith[3] = 0xc9;
        assert!(decode(&arith).unwrap_err().0.contains("arithmetic"));
    }

    #[test]
    fn dnl_supplies_missing_height() {
        let img = test_image(40, 24, 1);
        let jpg = encode(&img, 40, 24, 1, 90).unwrap();
        let sof = (0..jpg.len()).find(|&i| jpg[i] == 0xff && jpg[i + 1] == 0xc0).unwrap();
        let mut dnl = jpg.clone();
        dnl[sof + 5] = 0;
        dnl[sof + 6] = 0;
        let eoi = dnl.len() - 2;
        dnl.splice(eoi..eoi, [0xff, 0xdc, 0, 4, 0, 24]);
        assert_eq!(decode(&dnl).unwrap().data, decode(&jpg).unwrap().data);
        assert_eq!(info(&dnl).unwrap(), (40, 24, 1));
    }

    #[test]
    fn encoder_rejects_bad_input() {
        assert!(encode(&[0; 3], 1, 1, 3, 0).is_err());
        assert!(encode(&[0; 3], 1, 1, 3, 101).is_err());
        assert!(encode(&[0; 2], 1, 1, 2, 50).is_err());
        assert!(encode(&[0; 4], 1, 1, 3, 50).is_err());
        assert!(encode(&[], 0, 1, 1, 50).is_err());
        assert!(encode(&[], 65536, 1, 1, 50).is_err());
        assert!(encode(&[0; 4], 2, 2, 1, 50).is_ok());
    }

    // ----- cross-checks against installed tools (skipped when they are missing) -----

    fn have(tool: &str) -> bool {
        Command::new(tool).arg("-version").output().is_ok()
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> TempDir {
            let p = std::env::temp_dir().join(format!("pdf-codecs-jpeg-{}-{name}", std::process::id()));
            std::fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
        fn path(&self, f: &str) -> PathBuf {
            self.0.join(f)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn run(cmd: &mut Command) -> bool {
        cmd.output().map(|o| o.status.success()).unwrap_or(false)
    }

    fn write_pnm(path: &Path, img: &[u8], w: usize, h: usize, nc: usize) {
        let mut v = format!("{}\n{w} {h}\n255\n", if nc == 1 { "P5" } else { "P6" }).into_bytes();
        v.extend_from_slice(img);
        std::fs::write(path, v).unwrap();
    }

    /// ffmpeg's own decode of `jpg` to packed rgb24 (or gray).
    fn ffmpeg_decode(dir: &TempDir, jpg: &Path, pix_fmt: &str) -> Option<Vec<u8>> {
        let out = dir.path("ref.raw");
        let ok = run(Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-i"])
            .arg(jpg)
            .args(["-f", "rawvideo", "-pix_fmt", pix_fmt])
            .arg(&out));
        if ok { std::fs::read(out).ok() } else { None }
    }

    /// libjpeg's djpeg, which upsamples chroma the same way, as a tight reference.
    fn djpeg_decode(dir: &TempDir, jpg: &Path) -> Option<Vec<u8>> {
        let out = dir.path("ref.pnm");
        if !run(Command::new("djpeg").arg("-pnm").arg("-outfile").arg(&out).arg(jpg)) {
            return None;
        }
        let pnm = std::fs::read(out).ok()?;
        // Header: magic, width, height, maxval, each followed by one whitespace byte.
        let mut fields = 0;
        let mut i = 0;
        while fields < 4 && i < pnm.len() {
            if pnm[i].is_ascii_whitespace() && i > 0 && !pnm[i - 1].is_ascii_whitespace() {
                fields += 1;
            }
            i += 1;
        }
        Some(pnm[i..].to_vec())
    }

    /// Compares our decode with djpeg (tight, when installed) and ffmpeg (loose: swscale interpolates chroma
    /// differently, which matters on small, colourful images).
    fn check_reference(dir: &TempDir, jpg: &Path, ffmpeg_max: f64, label: &str) {
        let data = std::fs::read(jpg).unwrap();
        let d = decode(&data).unwrap_or_else(|e| panic!("{label}: {e}"));
        if let Some(reference) = djpeg_decode(dir, jpg) {
            let m = mae(&d.data, &reference);
            eprintln!("{label}: {}x{} {:?} MAE vs djpeg {m:.3}", d.width, d.height, d.kind);
            assert!(m <= 0.6, "{label}: MAE vs djpeg {m:.3}");
        }
        if d.width >= 64 && have("ffmpeg") {
            let fmt = if d.kind == ColorKind::Gray { "gray" } else { "rgb24" };
            let reference = ffmpeg_decode(dir, jpg, fmt).unwrap_or_else(|| panic!("{label}: ffmpeg could not decode"));
            let m = mae(&d.data, &reference);
            eprintln!("{label}: {}x{} {:?} MAE vs ffmpeg {m:.3}", d.width, d.height, d.kind);
            assert!(m <= ffmpeg_max, "{label}: MAE vs ffmpeg {m:.3} > {ffmpeg_max}");
        }
    }

    #[test]
    fn matches_ffmpeg_for_its_own_files() {
        if !have("ffmpeg") {
            return;
        }
        let dir = TempDir::new("ffmpeg");
        for (pix, max) in [("yuvj420p", 3.0), ("yuvj422p", 2.5), ("yuvj444p", 1.0), ("gray", 0.6)] {
            for size in ["333x251", "16x16", "35x9"] {
                let jpg = dir.path(&format!("{pix}-{size}.jpg"));
                let ok = run(Command::new("ffmpeg")
                    .args(["-v", "error", "-y", "-f", "lavfi", "-i", &format!("testsrc=size={size}"), "-frames:v", "1"])
                    .args(["-q:v", "3", "-pix_fmt", pix])
                    .arg(&jpg));
                if !ok {
                    continue;
                }
                check_reference(&dir, &jpg, max, &format!("ffmpeg {pix} {size}"));
            }
        }
    }

    #[test]
    fn our_files_decode_in_ffmpeg() {
        if !have("ffmpeg") {
            return;
        }
        let dir = TempDir::new("ours");
        for (nc, w, h) in [(3usize, 333usize, 251usize), (1, 101, 77)] {
            let img = test_image(w, h, nc);
            let jpg = encode(&img, w as u32, h as u32, nc as u8, 90).unwrap();
            let path = dir.path("ours.jpg");
            std::fs::write(&path, &jpg).unwrap();
            let reference = ffmpeg_decode(&dir, &path, if nc == 1 { "gray" } else { "rgb24" }).expect("ffmpeg decode");
            let ours = decode(&jpg).unwrap().data;
            let m = mae(&ours, &reference);
            eprintln!("our encoder, nc {nc}: MAE against ffmpeg {m:.3}");
            assert!(m < 3.0);
        }
    }

    #[test]
    fn libjpeg_variants() {
        if !have("cjpeg") {
            return;
        }
        let dir = TempDir::new("cjpeg");
        let (w, h) = (257usize, 131usize);
        let ppm = dir.path("in.ppm");
        write_pnm(&ppm, &test_image(w, h, 3), w, h, 3);
        let pgm = dir.path("in.pgm");
        write_pnm(&pgm, &test_image(w, h, 1), w, h, 1);
        let cases: &[(&str, &[&str], &Path, f64)] = &[
            ("progressive 420", &["-progressive", "-quality", "90"], &ppm, 3.0),
            ("progressive 444", &["-progressive", "-sample", "1x1", "-quality", "95"], &ppm, 1.0),
            ("progressive gray", &["-progressive", "-quality", "90"], &pgm, 0.6),
            ("restart rows", &["-restart", "1", "-quality", "85"], &ppm, 3.0),
            ("restart blocks", &["-restart", "5B", "-sample", "2x1", "-quality", "85"], &ppm, 3.0),
            ("sampling 1x2", &["-sample", "1x2", "-quality", "90"], &ppm, 3.0),
            ("sampling 4x1", &["-sample", "4x1", "-quality", "90"], &ppm, 4.0),
            ("optimized", &["-optimize", "-quality", "75"], &ppm, 3.0),
            ("rgb", &["-rgb", "-quality", "90"], &ppm, 1.0),
        ];
        for (label, args, input, max) in cases {
            let jpg = dir.path("out.jpg");
            if !run(Command::new("cjpeg").args(*args).arg("-outfile").arg(&jpg).arg(input)) {
                continue;
            }
            check_reference(&dir, &jpg, *max, label);
        }
        // Arithmetic coding is refused cleanly.
        let jpg = dir.path("arith.jpg");
        if run(Command::new("cjpeg").args(["-arithmetic"]).arg("-outfile").arg(&jpg).arg(&ppm)) {
            assert!(decode(&std::fs::read(&jpg).unwrap()).is_err());
        }
    }

    #[test]
    fn imagemagick_progressive_and_cmyk() {
        if !have("magick") {
            return;
        }
        let dir = TempDir::new("magick");
        let (w, h) = (199usize, 121usize);
        let img = test_image(w, h, 3);
        let ppm = dir.path("in.ppm");
        write_pnm(&ppm, &img, w, h, 3);
        let prog = dir.path("prog.jpg");
        if run(Command::new("magick").arg(&ppm).args(["-interlace", "JPEG", "-quality", "88"]).arg(&prog)) {
            check_reference(&dir, &prog, 3.0, "magick progressive");
            let d = decode(&std::fs::read(&prog).unwrap()).unwrap();
            assert!(psnr(&d.data, &img) > 28.0);
        }
        let cmyk = dir.path("cmyk.jpg");
        let raw = dir.path("cmyk.raw");
        if run(Command::new("magick").arg(&ppm).args(["-colorspace", "CMYK", "-quality", "95"]).arg(&cmyk))
            && run(Command::new("magick").arg(&cmyk).args(["-depth", "8"]).arg(format!("cmyk:{}", raw.display())))
        {
            let d = decode(&std::fs::read(&cmyk).unwrap()).unwrap();
            assert_eq!(d.kind, ColorKind::Cmyk);
            assert!(d.adobe);
            let reference = std::fs::read(&raw).unwrap();
            // Adobe CMYK is stored inverted; ImageMagick hands back true CMYK.
            let inverted: Vec<u8> = d.data.iter().map(|&v| 255 - v).collect();
            let m = mae(&inverted, &reference);
            eprintln!("magick Adobe CMYK: MAE {m:.3}");
            assert!(m < 1.5, "CMYK MAE {m:.3}");
        }
    }

    /// CPU time of this thread in seconds (Linux), so the measurement ignores time spent waiting for a core on a
    /// busy machine; wall time elsewhere.
    fn thread_time() -> f64 {
        std::fs::read_to_string("/proc/thread-self/schedstat")
            .ok()
            .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
            .map(|ns| ns / 1e9)
            .unwrap_or_else(|| {
                static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
                START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64()
            })
    }

    /// Best-of-five CPU time of decoding `jpg`, in milliseconds.
    fn time_decode(jpg: &[u8]) -> f64 {
        let mut best = f64::MAX;
        for _ in 0..5 {
            let t = thread_time();
            let d = decode(jpg).unwrap();
            best = best.min(thread_time() - t);
            assert!(!d.data.is_empty());
        }
        best * 1000.0
    }

    /// `cargo test --release -- --ignored --nocapture jpeg::tests::timing` prints decode times for a large image.
    #[test]
    #[ignore]
    fn timing() {
        let (w, h) = (3000usize, 4000usize);
        let img = test_image(w, h, 3);
        let jpg = encode(&img, w as u32, h as u32, 3, 90).unwrap();
        eprintln!("decode {w}x{h} 4:2:0 q90 ({} KiB): {:.1} ms", jpg.len() / 1024, time_decode(&jpg));
        // Photographic noise makes the entropy decoder do several times more work.
        let mut seed = 1u32;
        let noisy: Vec<u8> = img.iter().map(|&v| v.saturating_add((lcg(&mut seed) % 48) as u8)).collect();
        let jpg = encode(&noisy, w as u32, h as u32, 3, 90).unwrap();
        eprintln!("decode {w}x{h} 4:2:0 q90 noisy ({} KiB): {:.1} ms", jpg.len() / 1024, time_decode(&jpg));
        if have("cjpeg") {
            let dir = TempDir::new("timing");
            let ppm = dir.path("big.ppm");
            write_pnm(&ppm, &noisy, w, h, 3);
            let prog = dir.path("big-prog.jpg");
            if run(Command::new("cjpeg").args(["-progressive", "-quality", "90"]).arg("-outfile").arg(&prog).arg(&ppm))
            {
                let data = std::fs::read(&prog).unwrap();
                eprintln!(
                    "decode {w}x{h} progressive q90 noisy ({} KiB): {:.1} ms",
                    data.len() / 1024,
                    time_decode(&data)
                );
            }
        }
    }
}
