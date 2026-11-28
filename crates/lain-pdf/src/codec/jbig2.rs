//! JBIG2 (ITU-T T.88) decoding for the PDF JBIG2Decode filter.
//!
//! PDF stores the "embedded" organisation: bare segments without a file header, with shared dictionaries in a
//! separate JBIG2Globals stream that is read first. Only page 1 (the first page information segment) is
//! rendered. Every region type the standard defines is decoded: generic (MQ or MMR), generic refinement, text
//! (symbol instances, arithmetic or Huffman, with refinement), and halftone over a pattern dictionary.
//!
//! Hostile input: every loop is bounded by the data, by the image caps or by a work budget, and the arithmetic
//! decoder reports when it has run past its data so loops over decoded counts stop instead of spinning on
//! synthetic bytes. A segment that fails to decode is skipped; the page keeps what was drawn.

use super::CodecError;
use std::collections::HashMap;
use std::rc::Rc;

const MAX_DIM: u32 = 65535;
const MAX_PIXELS: u64 = 1 << 28;
/// Pixels decoded or drawn per call, a few seconds of work: real pages need a few times their own size.
const WORK_LIMIT: u64 = 1 << 31;
/// Symbols in one dictionary or text region.
const MAX_SYMBOLS: u64 = 1 << 20;
/// Bytes past the end the arithmetic decoder may invent before a region stops; real streams need two or three.
const MQ_OVERRUN: u32 = 32;

fn err(msg: impl Into<String>) -> CodecError {
    CodecError(format!("jbig2: {}", msg.into()))
}

/// A bilevel image: packed rows, MSB first, 1 = black (JBIG2's own convention).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    /// Bytes per row.
    pub stride: u32,
    /// Packed rows MSB first, 1 = black (JBIG2's own convention).
    pub data: Vec<u8>,
}

impl Bitmap {
    fn new(width: u32, height: u32) -> Result<Bitmap, CodecError> {
        Bitmap::filled(width, height, 0)
    }

    fn filled(width: u32, height: u32, value: u8) -> Result<Bitmap, CodecError> {
        if width > MAX_DIM || height > MAX_DIM || u64::from(width) * u64::from(height) > MAX_PIXELS {
            return Err(err(format!("bitmap too large ({width}x{height})")));
        }
        let stride = width.div_ceil(8);
        let fill = if value != 0 { 0xff } else { 0 };
        Ok(Bitmap { width, height, stride, data: vec![fill; stride as usize * height as usize] })
    }

    /// Pixel at (x, y); 0 outside the bitmap, as the templates require.
    #[inline]
    fn get(&self, x: i64, y: i64) -> u32 {
        if x < 0 || y < 0 || x >= i64::from(self.width) || y >= i64::from(self.height) {
            return 0;
        }
        let i = y as usize * self.stride as usize + (x as usize >> 3);
        self.data.get(i).map_or(0, |b| u32::from(b >> (7 - (x & 7))) & 1)
    }

    #[inline]
    fn put(&mut self, x: i64, y: i64, v: u32) {
        if x < 0 || y < 0 || x >= i64::from(self.width) || y >= i64::from(self.height) {
            return;
        }
        let i = y as usize * self.stride as usize + (x as usize >> 3);
        let mask = 0x80u8 >> (x & 7);
        if let Some(b) = self.data.get_mut(i) {
            if v != 0 { *b |= mask } else { *b &= !mask }
        }
    }

    fn copy_row(&mut self, from: u32, to: u32) {
        let s = self.stride as usize;
        let (from, to) = (from as usize * s, to as usize * s);
        if from + s <= self.data.len() && to + s <= self.data.len() {
            self.data.copy_within(from..from + s, to);
        }
    }

    /// Copy of the `w`x`h` area at (x, y); pixels outside read as 0.
    fn extract(&self, x: i64, y: i64, w: u32, h: u32) -> Result<Bitmap, CodecError> {
        let mut out = Bitmap::new(w, h)?;
        for yy in 0..i64::from(h) {
            for xx in 0..i64::from(w) {
                if self.get(x + xx, y + yy) != 0 {
                    out.put(xx, yy, 1);
                }
            }
        }
        Ok(out)
    }

    /// Splits off columns `x0..x0 + w` (used to cut collective bitmaps into symbols and patterns).
    fn columns(&self, x0: u32, w: u32) -> Result<Bitmap, CodecError> {
        self.extract(i64::from(x0), 0, w, self.height)
    }

    /// Grows the height to `h` rows of `value` (striped pages of unknown height).
    fn grow(&mut self, h: u32, value: u8) -> Result<(), CodecError> {
        if h <= self.height {
            return Ok(());
        }
        if h > MAX_DIM || u64::from(self.width) * u64::from(h) > MAX_PIXELS {
            return Err(err("page too large"));
        }
        let fill = if value != 0 { 0xff } else { 0 };
        self.data.resize(self.stride as usize * h as usize, fill);
        self.height = h;
        Ok(())
    }
}

/// Draws `src` onto `dst` at (x, y) with a JBIG2 combination operator: 0 OR, 1 AND, 2 XOR, 3 XNOR, 4 REPLACE.
fn combine(dst: &mut Bitmap, src: &Bitmap, x: i64, y: i64, op: u8) {
    let x0 = (-x).max(0);
    let x1 = i64::from(src.width).min(i64::from(dst.width) - x);
    let y0 = (-y).max(0);
    let y1 = i64::from(src.height).min(i64::from(dst.height) - y);
    for sy in y0..y1 {
        for sx in x0..x1 {
            let s = src.get(sx, sy);
            let (dx, dy) = (x + sx, y + sy);
            let r = match op {
                0 => {
                    if s == 0 {
                        continue;
                    }
                    1
                }
                1 => dst.get(dx, dy) & s,
                2 => dst.get(dx, dy) ^ s,
                3 => 1 ^ dst.get(dx, dy) ^ s,
                _ => s,
            };
            dst.put(dx, dy, r);
        }
    }
}

/// Counts decoded and drawn pixels against [`WORK_LIMIT`].
struct Work(u64);

impl Work {
    fn spend(&mut self, n: u64) -> Result<(), CodecError> {
        self.0 = self.0.checked_sub(n).ok_or_else(|| err("work limit exceeded"))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------------------------
// MQ arithmetic decoder (Annex E), in the software conventions pdf.js and the JPEG 2000 spec use.
// ---------------------------------------------------------------------------------------------------------------

/// (Qe, NMPS, NLPS, SWITCH)
#[rustfmt::skip]
const QE: [(u32, u8, u8, bool); 47] = [
    (0x5601, 1, 1, true), (0x3401, 2, 6, false), (0x1801, 3, 9, false), (0x0ac1, 4, 12, false),
    (0x0521, 5, 29, false), (0x0221, 38, 33, false), (0x5601, 7, 6, true), (0x5401, 8, 14, false),
    (0x4801, 9, 14, false), (0x3801, 10, 14, false), (0x3001, 11, 17, false), (0x2401, 12, 18, false),
    (0x1c01, 13, 20, false), (0x1601, 29, 21, false), (0x5601, 15, 14, true), (0x5401, 16, 14, false),
    (0x5101, 17, 15, false), (0x4801, 18, 16, false), (0x3801, 19, 17, false), (0x3401, 20, 18, false),
    (0x3001, 21, 19, false), (0x2801, 22, 19, false), (0x2401, 23, 20, false), (0x2201, 24, 21, false),
    (0x1c01, 25, 22, false), (0x1801, 26, 23, false), (0x1601, 27, 24, false), (0x1401, 28, 25, false),
    (0x1201, 29, 26, false), (0x1101, 30, 27, false), (0x0ac1, 31, 28, false), (0x09c1, 32, 29, false),
    (0x08a1, 33, 30, false), (0x0521, 34, 31, false), (0x0441, 35, 32, false), (0x02a1, 36, 33, false),
    (0x0221, 37, 34, false), (0x0141, 38, 35, false), (0x0111, 39, 36, false), (0x0085, 40, 37, false),
    (0x0049, 41, 38, false), (0x0025, 42, 39, false), (0x0015, 43, 40, false), (0x0009, 44, 41, false),
    (0x0005, 45, 42, false), (0x0001, 45, 43, false), (0x5601, 46, 46, false),
];

struct Mq<'a> {
    data: &'a [u8],
    bp: usize,
    chigh: u32,
    clow: u32,
    a: u32,
    ct: u32,
    /// Bytes fed after the data ran out (or hit a marker).
    invented: u32,
}

impl<'a> Mq<'a> {
    fn new(data: &'a [u8]) -> Self {
        let mut m = Mq { data, bp: 0, chigh: 0, clow: 0, a: 0, ct: 0, invented: 0 };
        m.chigh = m.byte(0);
        m.byte_in();
        m.chigh = ((m.chigh << 7) & 0xffff) | ((m.clow >> 9) & 0x7f);
        m.clow = (m.clow << 7) & 0xffff;
        m.ct = m.ct.saturating_sub(7);
        m.a = 0x8000;
        m
    }

    /// Bytes past the end read as 0xff, which the decoder treats like a marker.
    fn byte(&self, i: usize) -> u32 {
        u32::from(self.data.get(i).copied().unwrap_or(0xff))
    }

    fn byte_in(&mut self) {
        if self.byte(self.bp) == 0xff {
            if self.byte(self.bp + 1) > 0x8f {
                self.clow += 0xff00;
                self.ct = 8;
                self.invented = self.invented.saturating_add(1);
            } else {
                self.bp += 1;
                self.clow += self.byte(self.bp) << 9;
                self.ct = 7;
            }
        } else {
            self.bp += 1;
            if self.bp >= self.data.len() {
                self.invented = self.invented.saturating_add(1);
            }
            self.clow += self.byte(self.bp) << 8;
            self.ct = 8;
        }
        if self.clow > 0xffff {
            self.chigh += self.clow >> 16;
            self.clow &= 0xffff;
        }
    }

    fn exhausted(&self) -> bool {
        self.invented > MQ_OVERRUN
    }

    /// Decodes one decision in context `cx[i]` (state index << 1 | MPS).
    #[inline]
    fn decode(&mut self, cx: &mut [u8], i: usize) -> u32 {
        let Some(&state) = cx.get(i) else { return 0 };
        let mut index = usize::from(state >> 1).min(46);
        let mut mps = u32::from(state & 1);
        let (qe, nmps, nlps, switch) = QE[index];
        let mut a = self.a.wrapping_sub(qe);
        let d;
        if self.chigh < qe {
            if a < qe {
                a = qe;
                d = mps;
                index = usize::from(nmps);
            } else {
                a = qe;
                d = 1 ^ mps;
                if switch {
                    mps = d;
                }
                index = usize::from(nlps);
            }
        } else {
            self.chigh -= qe;
            if a & 0x8000 != 0 {
                self.a = a;
                return mps;
            }
            if a < qe {
                d = 1 ^ mps;
                if switch {
                    mps = d;
                }
                index = usize::from(nlps);
            } else {
                d = mps;
                index = usize::from(nmps);
            }
        }
        loop {
            if self.ct == 0 {
                self.byte_in();
            }
            a <<= 1;
            self.chigh = ((self.chigh << 1) & 0xffff) | ((self.clow >> 15) & 1);
            self.clow = (self.clow << 1) & 0xffff;
            self.ct = self.ct.saturating_sub(1);
            if a & 0x8000 != 0 || a == 0 {
                break;
            }
        }
        self.a = a;
        cx[i] = ((index as u8) << 1) | mps as u8;
        d
    }
}

/// Integer decoding (A.2); `None` is OOB, also returned for values outside i32.
fn decode_int(mq: &mut Mq, cx: &mut [u8]) -> Option<i64> {
    let mut prev = 1usize;
    let mut read = |mq: &mut Mq, n: u32| -> i64 {
        let mut v = 0i64;
        for _ in 0..n {
            let b = mq.decode(cx, prev);
            prev = if prev < 256 { (prev << 1) | b as usize } else { (((prev << 1) | b as usize) & 511) | 256 };
            v = (v << 1) | i64::from(b);
        }
        v
    };
    let sign = read(mq, 1);
    let v = if read(mq, 1) == 0 {
        read(mq, 2)
    } else if read(mq, 1) == 0 {
        read(mq, 4) + 4
    } else if read(mq, 1) == 0 {
        read(mq, 6) + 20
    } else if read(mq, 1) == 0 {
        read(mq, 8) + 84
    } else if read(mq, 1) == 0 {
        read(mq, 12) + 340
    } else {
        read(mq, 32) + 4436
    };
    let v = if sign == 0 {
        v
    } else if v == 0 {
        return None;
    } else {
        -v
    };
    (i64::from(i32::MIN)..=i64::from(i32::MAX)).contains(&v).then_some(v)
}

/// Symbol ID decoding (A.3) with `len` bits; `cx` holds `2 << len` contexts.
fn decode_iaid(mq: &mut Mq, cx: &mut [u8], len: u32) -> u32 {
    let mut prev = 1usize;
    for _ in 0..len {
        prev = (prev << 1) | mq.decode(cx, prev) as usize;
    }
    (prev - (1 << len)) as u32
}

/// All arithmetic contexts of one segment; procedures with the same name share them, as the standard requires.
#[derive(Default)]
struct Contexts {
    iadh: Vec<u8>,
    iadw: Vec<u8>,
    iaex: Vec<u8>,
    iaai: Vec<u8>,
    iadt: Vec<u8>,
    iafs: Vec<u8>,
    iads: Vec<u8>,
    iait: Vec<u8>,
    iari: Vec<u8>,
    iardw: Vec<u8>,
    iardh: Vec<u8>,
    iardx: Vec<u8>,
    iardy: Vec<u8>,
    iaid: Vec<u8>,
    gb: Vec<u8>,
    gr: Vec<u8>,
}

impl Contexts {
    fn new() -> Contexts {
        let n = || vec![0u8; 512];
        Contexts {
            iadh: n(),
            iadw: n(),
            iaex: n(),
            iaai: n(),
            iadt: n(),
            iafs: n(),
            iads: n(),
            iait: n(),
            iari: n(),
            iardw: n(),
            iardh: n(),
            iardx: n(),
            iardy: n(),
            iaid: Vec::new(),
            gb: Vec::new(),
            gr: Vec::new(),
        }
    }
}

fn ensure(cx: &mut Vec<u8>, size: usize) {
    if cx.len() < size {
        cx.resize(size, 0);
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Generic region decoding (6.2)
// ---------------------------------------------------------------------------------------------------------------

/// Context layout of a generic template, bit for bit as in T.88 figures 3-6: the fixed pixels of the two rows
/// above form shift registers (`lead` is the rightmost x offset), the current row's pixels to the left another,
/// and the AT pixels sit at fixed bits.
struct GenericTemplate {
    row0_bits: u32,
    row1: (u32, u32, i64),
    row2: Option<(u32, u32, i64)>,
    at_bits: &'static [u32],
    default_at: &'static [(i32, i32)],
    context_bits: u32,
    /// The context TPGDON's SLTP bit is coded in.
    sltp: usize,
}

const GENERIC: [GenericTemplate; 4] = [
    GenericTemplate {
        row0_bits: 4,
        row1: (5, 5, 2),
        row2: Some((12, 3, 1)),
        at_bits: &[4, 10, 11, 15],
        default_at: &[(3, -1), (-3, -1), (2, -2), (-2, -2)],
        context_bits: 16,
        sltp: 0x9b25,
    },
    GenericTemplate {
        row0_bits: 3,
        row1: (4, 5, 2),
        row2: Some((9, 4, 2)),
        at_bits: &[3],
        default_at: &[(3, -1)],
        context_bits: 13,
        sltp: 0x0795,
    },
    GenericTemplate {
        row0_bits: 2,
        row1: (3, 4, 1),
        row2: Some((7, 3, 1)),
        at_bits: &[2],
        default_at: &[(2, -1)],
        context_bits: 10,
        sltp: 0x00e5,
    },
    GenericTemplate {
        row0_bits: 4,
        row1: (5, 5, 1),
        row2: None,
        at_bits: &[4],
        default_at: &[(2, -1)],
        context_bits: 10,
        sltp: 0x0195,
    },
];

/// Generic region decoding with the MQ coder (6.2.5). Rows the data does not reach stay 0.
#[allow(clippy::too_many_arguments)]
fn decode_generic(
    mq: &mut Mq,
    cx: &mut Vec<u8>,
    width: u32,
    height: u32,
    template: u8,
    tpgdon: bool,
    at: &[(i32, i32)],
    skip: Option<&Bitmap>,
    work: &mut Work,
) -> Result<Bitmap, CodecError> {
    let t = &GENERIC[usize::from(template & 3)];
    ensure(cx, 1 << t.context_bits);
    let mut bm = Bitmap::new(width, height)?;
    let at: Vec<(i64, i64)> = (0..t.at_bits.len())
        .map(|i| at.get(i).copied().unwrap_or(t.default_at[i]))
        .map(|(x, y)| (i64::from(x), i64::from(y)))
        .collect();
    let mask0 = (1u32 << t.row0_bits) - 1;
    let (r1_pos, r1_len, r1_lead) = t.row1;
    let mask1 = (1u32 << r1_len) - 1;
    let (r2_pos, r2_len, r2_lead) = t.row2.unwrap_or((0, 0, 0));
    let mask2 = (1u32 << r2_len) - 1;
    let mut ltp = 0;
    for y in 0..i64::from(height) {
        if mq.exhausted() {
            break;
        }
        work.spend(u64::from(width))?;
        if tpgdon {
            ltp ^= mq.decode(cx, t.sltp);
            if ltp != 0 {
                if y > 0 {
                    bm.copy_row(y as u32 - 1, y as u32);
                }
                continue;
            }
        }
        let mut w0 = 0u32;
        let mut w1 = 0u32;
        for dx in r1_lead - i64::from(r1_len) + 1..r1_lead {
            w1 = (w1 << 1) | bm.get(dx, y - 1);
        }
        let mut w2 = 0u32;
        if t.row2.is_some() {
            for dx in r2_lead - i64::from(r2_len) + 1..r2_lead {
                w2 = (w2 << 1) | bm.get(dx, y - 2);
            }
        }
        for x in 0..i64::from(width) {
            w1 = ((w1 << 1) | bm.get(x + r1_lead, y - 1)) & mask1;
            if t.row2.is_some() {
                w2 = ((w2 << 1) | bm.get(x + r2_lead, y - 2)) & mask2;
            }
            let mut ctx = w0 | (w1 << r1_pos) | (w2 << r2_pos);
            for (&(ax, ay), &bit) in at.iter().zip(t.at_bits) {
                ctx |= bm.get(x + ax, y + ay) << bit;
            }
            let pixel = if skip.is_some_and(|s| s.get(x, y) != 0) { 0 } else { mq.decode(cx, ctx as usize) };
            if pixel != 0 {
                bm.put(x, y, 1);
            }
            w0 = ((w0 << 1) | pixel) & mask0;
        }
    }
    Ok(bm)
}

/// Generic region decoding with MMR (6.2.6): returns the bitmap and the bytes consumed.
fn decode_generic_mmr(data: &[u8], width: u32, height: u32) -> Result<(Bitmap, usize), CodecError> {
    if width == 0 || height == 0 {
        return Ok((Bitmap::new(width, height)?, 0));
    }
    let empty = Bitmap::new(width, height)?;
    let (bits, used) = super::ccitt::decode_mmr(data, width, height)?;
    Ok((Bitmap { data: bits, ..empty }, used))
}

// ---------------------------------------------------------------------------------------------------------------
// Generic refinement region decoding (6.3)
// ---------------------------------------------------------------------------------------------------------------

/// Refinement of `reference` into a `width`x`height` bitmap; reference pixel (x - dx, y - dy) sits under (x, y).
#[allow(clippy::too_many_arguments)]
fn decode_refinement(
    mq: &mut Mq,
    cx: &mut Vec<u8>,
    width: u32,
    height: u32,
    template: u8,
    reference: &Bitmap,
    dx: i64,
    dy: i64,
    tpgron: bool,
    at: &[(i32, i32); 2],
    work: &mut Work,
) -> Result<Bitmap, CodecError> {
    let t0 = template & 1 == 0;
    ensure(cx, if t0 { 1 << 13 } else { 1 << 10 });
    let mut bm = Bitmap::new(width, height)?;
    let (a0x, a0y) = (i64::from(at[0].0), i64::from(at[0].1));
    let (a1x, a1y) = (i64::from(at[1].0), i64::from(at[1].1));
    let r = reference;
    let mut ltp = 0;
    for y in 0..i64::from(height) {
        if mq.exhausted() {
            break;
        }
        work.spend(u64::from(width))?;
        if tpgron {
            ltp ^= mq.decode(cx, if t0 { 0x0020 } else { 0x0004 });
        }
        for x in 0..i64::from(width) {
            let (rx, ry) = (x - dx, y - dy);
            if ltp != 0 {
                // Typical prediction: a uniform 3x3 reference neighbourhood is copied without coding.
                let v = r.get(rx, ry);
                let uniform = (-1..=1).all(|j| (-1..=1).all(|i| r.get(rx + i, ry + j) == v));
                if uniform {
                    if v != 0 {
                        bm.put(x, y, 1);
                    }
                    continue;
                }
            }
            let ctx = if t0 {
                (bm.get(x, y - 1) << 12)
                    | (bm.get(x + 1, y - 1) << 11)
                    | (bm.get(x - 1, y) << 10)
                    | (bm.get(x + a0x, y + a0y) << 9)
                    | (r.get(rx, ry - 1) << 8)
                    | (r.get(rx + 1, ry - 1) << 7)
                    | (r.get(rx - 1, ry) << 6)
                    | (r.get(rx, ry) << 5)
                    | (r.get(rx + 1, ry) << 4)
                    | (r.get(rx - 1, ry + 1) << 3)
                    | (r.get(rx, ry + 1) << 2)
                    | (r.get(rx + 1, ry + 1) << 1)
                    | r.get(rx + a1x, ry + a1y)
            } else {
                (bm.get(x - 1, y - 1) << 9)
                    | (bm.get(x, y - 1) << 8)
                    | (bm.get(x + 1, y - 1) << 7)
                    | (bm.get(x - 1, y) << 6)
                    | (r.get(rx, ry - 1) << 5)
                    | (r.get(rx - 1, ry) << 4)
                    | (r.get(rx, ry) << 3)
                    | (r.get(rx + 1, ry) << 2)
                    | (r.get(rx, ry + 1) << 1)
                    | r.get(rx + 1, ry + 1)
            };
            if mq.decode(cx, ctx as usize) != 0 {
                bm.put(x, y, 1);
            }
        }
    }
    Ok(bm)
}

// ---------------------------------------------------------------------------------------------------------------
// Huffman tables (Annex B) and the bit reader they use
// ---------------------------------------------------------------------------------------------------------------

/// MSB-first reader for the Huffman-coded parts; running off the end is an error.
struct HBits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> HBits<'a> {
    fn bit(&mut self) -> Result<u32, CodecError> {
        let b = *self.data.get(self.pos / 8).ok_or_else(|| err("Huffman data ends early"))?;
        let v = u32::from(b >> (7 - self.pos % 8)) & 1;
        self.pos += 1;
        Ok(v)
    }

    fn bits(&mut self, n: u32) -> Result<u64, CodecError> {
        let mut v = 0u64;
        for _ in 0..n {
            v = (v << 1) | u64::from(self.bit()?);
        }
        Ok(v)
    }

    fn align(&mut self) {
        self.pos = self.pos.div_ceil(8) * 8;
    }

    /// Current position in whole bytes (after `align`).
    fn byte_pos(&self) -> usize {
        self.pos.div_ceil(8)
    }

    fn skip_bytes(&mut self, n: usize) {
        self.pos = (self.byte_pos().saturating_add(n)).saturating_mul(8);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LineKind {
    Normal,
    /// The value is RANGELOW minus the offset.
    Lower,
    Oob,
}

#[derive(Clone, Copy, Debug)]
struct Line {
    low: i64,
    prefix: u32,
    range: u32,
    kind: LineKind,
}

/// A table with prefix codes assigned as B.3 describes, decoded canonically.
#[derive(Debug)]
struct HuffTable {
    lines: Vec<Line>,
    /// Code of each line (0 for unused lines with prefix length 0); the tests' encoders write these.
    #[cfg_attr(not(test), allow(dead_code))]
    codes: Vec<u32>,
    /// Per prefix length: first code and the lines that use the length, in order.
    first: Vec<u32>,
    by_len: Vec<Vec<usize>>,
}

impl HuffTable {
    fn new(lines: Vec<Line>) -> Result<HuffTable, CodecError> {
        let max = lines.iter().map(|l| l.prefix).max().unwrap_or(0) as usize;
        if max > 32 || lines.iter().any(|l| l.range > 32) {
            return Err(err("bad Huffman table"));
        }
        let mut by_len: Vec<Vec<usize>> = vec![Vec::new(); max + 1];
        for (i, l) in lines.iter().enumerate() {
            if l.prefix > 0 {
                by_len[l.prefix as usize].push(i);
            }
        }
        let mut first = vec![0u32; max + 1];
        let mut codes = vec![0u32; lines.len()];
        let mut code = 0u64;
        for len in 1..=max {
            // B.3: FIRSTCODE[len] = (FIRSTCODE[len - 1] + LENCOUNT[len - 1]) * 2, with nothing of length 0.
            code = (code + by_len[len - 1].len() as u64) << 1;
            if code + by_len[len].len() as u64 > 1u64 << len {
                return Err(err("overfull Huffman table"));
            }
            first[len] = code as u32;
            for (k, &i) in by_len[len].iter().enumerate() {
                codes[i] = code as u32 + k as u32;
            }
        }
        Ok(HuffTable { lines, codes, first, by_len })
    }

    /// Decodes one value; `None` is OOB.
    fn decode(&self, r: &mut HBits) -> Result<Option<i64>, CodecError> {
        let mut code = 0u32;
        for len in 1..self.first.len() {
            code = (code << 1) | r.bit()?;
            let k = code.wrapping_sub(self.first[len]) as usize;
            if let Some(&i) = self.by_len[len].get(k).filter(|_| code >= self.first[len]) {
                let line = self.lines[i];
                return match line.kind {
                    LineKind::Oob => Ok(None),
                    LineKind::Normal => Ok(Some(line.low + r.bits(line.range)? as i64)),
                    LineKind::Lower => Ok(Some(line.low - r.bits(line.range)? as i64)),
                };
            }
        }
        Err(err("invalid Huffman code"))
    }

    /// Decodes a value that must not be OOB.
    fn value(&self, r: &mut HBits) -> Result<i64, CodecError> {
        self.decode(r)?.ok_or_else(|| err("unexpected OOB"))
    }
}

/// (RANGELOW, PREFLEN, RANGELEN, kind, code)
type TableRow = (i64, u32, u32, u8, u32);

/// Standard tables B.1-B.15 as (RANGELOW, PREFLEN, RANGELEN, kind, code): 'n' normal, 'l' lower range, 'o' OOB.
/// The codes are the ones printed in the standard; the B.3 assignment reproduces them (checked in tests).
#[rustfmt::skip]
const STANDARD_TABLES: [&[TableRow]; 15] = [
    &[(0, 1, 4, b'n', 0x0), (16, 2, 8, b'n', 0x2), (272, 3, 16, b'n', 0x6), (65808, 3, 32, b'n', 0x7)],
    &[(0, 1, 0, b'n', 0x0), (1, 2, 0, b'n', 0x2), (2, 3, 0, b'n', 0x6), (3, 4, 3, b'n', 0xe), (11, 5, 6, b'n', 0x1e),
      (75, 6, 32, b'n', 0x3e), (0, 6, 0, b'o', 0x3f)],
    &[(-256, 8, 8, b'n', 0xfe), (0, 1, 0, b'n', 0x0), (1, 2, 0, b'n', 0x2), (2, 3, 0, b'n', 0x6), (3, 4, 3, b'n', 0xe),
      (11, 5, 6, b'n', 0x1e), (-257, 8, 32, b'l', 0xff), (75, 7, 32, b'n', 0x7e), (0, 6, 0, b'o', 0x3e)],
    &[(1, 1, 0, b'n', 0x0), (2, 2, 0, b'n', 0x2), (3, 3, 0, b'n', 0x6), (4, 4, 3, b'n', 0xe), (12, 5, 6, b'n', 0x1e),
      (76, 5, 32, b'n', 0x1f)],
    &[(-255, 7, 8, b'n', 0x7e), (1, 1, 0, b'n', 0x0), (2, 2, 0, b'n', 0x2), (3, 3, 0, b'n', 0x6), (4, 4, 3, b'n', 0xe),
      (12, 5, 6, b'n', 0x1e), (-256, 7, 32, b'l', 0x7f), (76, 6, 32, b'n', 0x3e)],
    &[(-2048, 5, 10, b'n', 0x1c), (-1024, 4, 9, b'n', 0x8), (-512, 4, 8, b'n', 0x9), (-256, 4, 7, b'n', 0xa),
      (-128, 5, 6, b'n', 0x1d), (-64, 5, 5, b'n', 0x1e), (-32, 4, 5, b'n', 0xb), (0, 2, 7, b'n', 0x0),
      (128, 3, 7, b'n', 0x2), (256, 3, 8, b'n', 0x3), (512, 4, 9, b'n', 0xc), (1024, 4, 10, b'n', 0xd),
      (-2049, 6, 32, b'l', 0x3e), (2048, 6, 32, b'n', 0x3f)],
    &[(-1024, 4, 9, b'n', 0x8), (-512, 3, 8, b'n', 0x0), (-256, 4, 7, b'n', 0x9), (-128, 5, 6, b'n', 0x1a),
      (-64, 5, 5, b'n', 0x1b), (-32, 4, 5, b'n', 0xa), (0, 4, 5, b'n', 0xb), (32, 5, 5, b'n', 0x1c),
      (64, 5, 6, b'n', 0x1d), (128, 4, 7, b'n', 0xc), (256, 3, 8, b'n', 0x1), (512, 3, 9, b'n', 0x2),
      (1024, 3, 10, b'n', 0x3), (-1025, 5, 32, b'l', 0x1e), (2048, 5, 32, b'n', 0x1f)],
    &[(-15, 8, 3, b'n', 0xfc), (-7, 9, 1, b'n', 0x1fc), (-5, 8, 1, b'n', 0xfd), (-3, 9, 0, b'n', 0x1fd),
      (-2, 7, 0, b'n', 0x7c), (-1, 4, 0, b'n', 0xa), (0, 2, 1, b'n', 0x0), (2, 5, 0, b'n', 0x1a), (3, 6, 0, b'n', 0x3a),
      (4, 3, 4, b'n', 0x4), (20, 6, 1, b'n', 0x3b), (22, 4, 4, b'n', 0xb), (38, 4, 5, b'n', 0xc),
      (70, 5, 6, b'n', 0x1b), (134, 5, 7, b'n', 0x1c), (262, 6, 7, b'n', 0x3c), (390, 7, 8, b'n', 0x7d),
      (646, 6, 10, b'n', 0x3d), (-16, 9, 32, b'l', 0x1fe), (1670, 9, 32, b'n', 0x1ff), (0, 2, 0, b'o', 0x1)],
    &[(-31, 8, 4, b'n', 0xfc), (-15, 9, 2, b'n', 0x1fc), (-11, 8, 2, b'n', 0xfd), (-7, 9, 1, b'n', 0x1fd),
      (-5, 7, 1, b'n', 0x7c), (-3, 4, 1, b'n', 0xa), (-1, 3, 1, b'n', 0x2), (1, 3, 1, b'n', 0x3), (3, 5, 1, b'n', 0x1a),
      (5, 6, 1, b'n', 0x3a), (7, 3, 5, b'n', 0x4), (39, 6, 2, b'n', 0x3b), (43, 4, 5, b'n', 0xb),
      (75, 4, 6, b'n', 0xc), (139, 5, 7, b'n', 0x1b), (267, 5, 8, b'n', 0x1c), (523, 6, 8, b'n', 0x3c),
      (779, 7, 9, b'n', 0x7d), (1291, 6, 11, b'n', 0x3d), (-32, 9, 32, b'l', 0x1fe), (3339, 9, 32, b'n', 0x1ff),
      (0, 2, 0, b'o', 0x0)],
    &[(-21, 7, 4, b'n', 0x7a), (-5, 8, 0, b'n', 0xfc), (-4, 7, 0, b'n', 0x7b), (-3, 5, 0, b'n', 0x18),
      (-2, 2, 2, b'n', 0x0), (2, 5, 0, b'n', 0x19), (3, 6, 0, b'n', 0x36), (4, 7, 0, b'n', 0x7c), (5, 8, 0, b'n', 0xfd),
      (6, 2, 6, b'n', 0x1), (70, 5, 5, b'n', 0x1a), (102, 6, 5, b'n', 0x37), (134, 6, 6, b'n', 0x38),
      (198, 6, 7, b'n', 0x39), (326, 6, 8, b'n', 0x3a), (582, 6, 9, b'n', 0x3b), (1094, 6, 10, b'n', 0x3c),
      (2118, 7, 11, b'n', 0x7d), (-22, 8, 32, b'l', 0xfe), (4166, 8, 32, b'n', 0xff), (0, 2, 0, b'o', 0x2)],
    &[(1, 1, 0, b'n', 0x0), (2, 2, 1, b'n', 0x2), (4, 4, 0, b'n', 0xc), (5, 4, 1, b'n', 0xd), (7, 5, 1, b'n', 0x1c),
      (9, 5, 2, b'n', 0x1d), (13, 6, 2, b'n', 0x3c), (17, 7, 2, b'n', 0x7a), (21, 7, 3, b'n', 0x7b),
      (29, 7, 4, b'n', 0x7c), (45, 7, 5, b'n', 0x7d), (77, 7, 6, b'n', 0x7e), (141, 7, 32, b'n', 0x7f)],
    &[(1, 1, 0, b'n', 0x0), (2, 2, 0, b'n', 0x2), (3, 3, 1, b'n', 0x6), (5, 5, 0, b'n', 0x1c), (6, 5, 1, b'n', 0x1d),
      (8, 6, 1, b'n', 0x3c), (10, 7, 0, b'n', 0x7a), (11, 7, 1, b'n', 0x7b), (13, 7, 2, b'n', 0x7c),
      (17, 7, 3, b'n', 0x7d), (25, 7, 4, b'n', 0x7e), (41, 8, 5, b'n', 0xfe), (73, 8, 32, b'n', 0xff)],
    &[(1, 1, 0, b'n', 0x0), (2, 3, 0, b'n', 0x4), (3, 4, 0, b'n', 0xc), (4, 5, 0, b'n', 0x1c), (5, 4, 1, b'n', 0xd),
      (7, 3, 3, b'n', 0x5), (15, 6, 1, b'n', 0x3a), (17, 6, 2, b'n', 0x3b), (21, 6, 3, b'n', 0x3c),
      (29, 6, 4, b'n', 0x3d), (45, 6, 5, b'n', 0x3e), (77, 7, 6, b'n', 0x7e), (141, 7, 32, b'n', 0x7f)],
    &[(-2, 3, 0, b'n', 0x4), (-1, 3, 0, b'n', 0x5), (0, 1, 0, b'n', 0x0), (1, 3, 0, b'n', 0x6), (2, 3, 0, b'n', 0x7)],
    &[(-24, 7, 4, b'n', 0x7c), (-8, 6, 2, b'n', 0x3c), (-4, 5, 1, b'n', 0x1c), (-2, 4, 0, b'n', 0xc),
      (-1, 3, 0, b'n', 0x4), (0, 1, 0, b'n', 0x0), (1, 3, 0, b'n', 0x5), (2, 4, 0, b'n', 0xd), (3, 5, 1, b'n', 0x1d),
      (5, 6, 2, b'n', 0x3d), (9, 7, 4, b'n', 0x7d), (-25, 7, 32, b'l', 0x7e), (25, 7, 32, b'n', 0x7f)],
];

fn standard_table(n: usize) -> Rc<HuffTable> {
    thread_local! {
        static TABLES: Vec<Rc<HuffTable>> = STANDARD_TABLES
            .iter()
            .map(|t| {
                let lines = t
                    .iter()
                    .map(|&(low, prefix, range, kind, _)| Line {
                        low,
                        prefix,
                        range,
                        kind: match kind {
                            b'l' => LineKind::Lower,
                            b'o' => LineKind::Oob,
                            _ => LineKind::Normal,
                        },
                    })
                    .collect();
                Rc::new(HuffTable::new(lines).unwrap_or(HuffTable { lines: Vec::new(), codes: Vec::new(), first: Vec::new(), by_len: Vec::new() }))
            })
            .collect();
    }
    TABLES.with(|t| t[(n - 1).min(14)].clone())
}

/// A code table segment (7.4.13 / B.2).
fn parse_table_segment(data: &[u8]) -> Result<HuffTable, CodecError> {
    let flags = *data.first().ok_or_else(|| err("short table segment"))?;
    let low = i64::from(be32(data, 1).ok_or_else(|| err("short table segment"))? as i32);
    let high = i64::from(be32(data, 5).ok_or_else(|| err("short table segment"))? as i32);
    let prefix_bits = u32::from((flags >> 1) & 7) + 1;
    let range_bits = u32::from((flags >> 4) & 7) + 1;
    let mut r = HBits { data: data.get(9..).unwrap_or(&[]), pos: 0 };
    let mut lines = Vec::new();
    let mut cur = low;
    while cur < high {
        let prefix = r.bits(prefix_bits)? as u32;
        let range = r.bits(range_bits)? as u32;
        if range > 32 || lines.len() > 1 << 16 {
            return Err(err("bad table segment"));
        }
        lines.push(Line { low: cur, prefix, range, kind: LineKind::Normal });
        cur += 1i64 << range;
    }
    let prefix = r.bits(prefix_bits)? as u32;
    lines.push(Line { low: low - 1, prefix, range: 32, kind: LineKind::Lower });
    let prefix = r.bits(prefix_bits)? as u32;
    lines.push(Line { low: high, prefix, range: 32, kind: LineKind::Normal });
    if flags & 1 != 0 {
        let prefix = r.bits(prefix_bits)? as u32;
        lines.push(Line { low: 0, prefix, range: 0, kind: LineKind::Oob });
    }
    HuffTable::new(lines)
}

// ---------------------------------------------------------------------------------------------------------------
// Segments (7.2)
// ---------------------------------------------------------------------------------------------------------------

fn be32(data: &[u8], pos: usize) -> Option<u32> {
    Some(u32::from_be_bytes(data.get(pos..pos + 4)?.try_into().ok()?))
}

fn be16(data: &[u8], pos: usize) -> Option<u32> {
    Some(u32::from(u16::from_be_bytes(data.get(pos..pos + 2)?.try_into().ok()?)))
}

struct Segment<'a> {
    number: u32,
    kind: u8,
    page: u32,
    refs: Vec<u32>,
    data: &'a [u8],
    /// Row count from the end marker of an immediate generic region of unknown length.
    rows: Option<u32>,
}

fn parse_segments(data: &[u8]) -> Vec<Segment<'_>> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        let Some((seg, next)) = parse_segment(data, pos) else { break };
        let end_of_file = seg.kind == 51;
        out.push(seg);
        if end_of_file || next <= pos {
            break;
        }
        pos = next;
    }
    out
}

/// One segment header (7.2.2) and its data; returns the position of the next header.
fn parse_segment(data: &[u8], pos: usize) -> Option<(Segment<'_>, usize)> {
    let number = be32(data, pos)?;
    let flags = *data.get(pos + 4)?;
    let kind = flags & 0x3f;
    let rts = *data.get(pos + 5)?;
    let mut count = u32::from(rts >> 5);
    let mut p = pos + 6;
    if count == 7 {
        count = be32(data, pos + 5)? & 0x1fff_ffff;
        p = pos + 9 + (count as usize + 1).div_ceil(8);
    } else if count > 4 {
        return None;
    }
    let size = if number <= 256 {
        1
    } else if number <= 65536 {
        2
    } else {
        4
    };
    if p.checked_add(count as usize * size)? > data.len() {
        return None;
    }
    let mut refs = Vec::with_capacity(count as usize);
    for _ in 0..count {
        refs.push(match size {
            1 => u32::from(data[p]),
            2 => be16(data, p)?,
            _ => be32(data, p)?,
        });
        p += size;
    }
    let page = if flags & 0x40 != 0 {
        let v = be32(data, p)?;
        p += 4;
        v
    } else {
        let v = u32::from(*data.get(p)?);
        p += 1;
        v
    };
    let len = be32(data, p)?;
    p += 4;
    let start = p.min(data.len());
    let (end, next, rows) = if len == 0xffff_ffff {
        if kind != 38 {
            return None;
        }
        let (end, rows) = find_generic_end(data, start)?;
        (end, end + 6, Some(rows))
    } else {
        let end = p.saturating_add(len as usize);
        (end.min(data.len()), end, None)
    };
    Some((Segment { number, kind, page, refs, data: &data[start..end.max(start)], rows }, next))
}

/// End of an immediate generic region whose length is unknown (7.2.7): the 0xffac (arithmetic) or 0x0000 (MMR)
/// marker followed by the row count. Returns the marker position and the row count.
fn find_generic_end(data: &[u8], start: usize) -> Option<(usize, u32)> {
    let flags = *data.get(start + 17)?;
    let mmr = flags & 1 != 0;
    let height = be32(data, start + 4)?;
    let at_bytes = if mmr {
        0
    } else if (flags >> 1) & 3 == 0 {
        8
    } else {
        2
    };
    let marker: [u8; 2] = if mmr { [0, 0] } else { [0xff, 0xac] };
    let mut i = start + 18 + at_bytes;
    while i + 6 <= data.len() {
        if data[i..i + 2] == marker {
            let rows = be32(data, i + 2)?;
            if rows <= height {
                return Some((i, rows));
            }
        }
        i += 1;
    }
    None
}

/// Region segment information field (7.4.1).
#[derive(Clone, Copy, Debug)]
struct RegionInfo {
    width: u32,
    height: u32,
    x: i64,
    y: i64,
    op: u8,
}

fn region_info(data: &[u8]) -> Result<RegionInfo, CodecError> {
    let short = || err("short region segment");
    Ok(RegionInfo {
        width: be32(data, 0).ok_or_else(short)?,
        height: be32(data, 4).ok_or_else(short)?,
        x: i64::from(be32(data, 8).ok_or_else(short)?),
        y: i64::from(be32(data, 12).ok_or_else(short)?),
        op: *data.get(16).ok_or_else(short)? & 7,
    })
}

fn read_at(data: &[u8], pos: usize, n: usize) -> Result<Vec<(i32, i32)>, CodecError> {
    let bytes = data.get(pos..pos + 2 * n).ok_or_else(|| err("short segment"))?;
    Ok(bytes.as_chunks::<2>().0.iter().map(|&[x, y]| (i32::from(x as i8), i32::from(y as i8))).collect())
}

fn ceil_log2(n: u64) -> u32 {
    if n <= 1 { 0 } else { 64 - (n - 1).leading_zeros() }
}

// ---------------------------------------------------------------------------------------------------------------
// Text regions (6.4)
// ---------------------------------------------------------------------------------------------------------------

/// Huffman tables of a text region (7.4.3.1.6) and its symbol ID code.
struct TextTables {
    fs: Rc<HuffTable>,
    ds: Rc<HuffTable>,
    dt: Rc<HuffTable>,
    rdw: Rc<HuffTable>,
    rdh: Rc<HuffTable>,
    rdx: Rc<HuffTable>,
    rdy: Rc<HuffTable>,
    rsize: Rc<HuffTable>,
    /// None: symbol IDs are fixed-length codes of this many bits (aggregate symbols, 6.5.8.2.4).
    ids: Option<HuffTable>,
    id_bits: u32,
}

/// Where a text region's values come from.
enum TextSource<'c, 'd> {
    Arith { mq: &'c mut Mq<'d>, cx: &'c mut Contexts, id_bits: u32 },
    Huffman { r: &'c mut HBits<'d>, t: &'c TextTables, gr: &'c mut Vec<u8> },
}

impl TextSource<'_, '_> {
    fn int(&mut self, which: u8) -> Result<Option<i64>, CodecError> {
        match self {
            TextSource::Arith { mq, cx, .. } => {
                let c = match which {
                    b't' => &mut cx.iadt,
                    b'f' => &mut cx.iafs,
                    b's' => &mut cx.iads,
                    b'i' => &mut cx.iait,
                    b'r' => &mut cx.iari,
                    b'w' => &mut cx.iardw,
                    b'h' => &mut cx.iardh,
                    b'x' => &mut cx.iardx,
                    _ => &mut cx.iardy,
                };
                Ok(decode_int(mq, c))
            }
            TextSource::Huffman { r, t, .. } => match which {
                b't' => t.dt.decode(r),
                b'f' => t.fs.decode(r),
                b's' => t.ds.decode(r),
                b'r' => Ok(Some(i64::from(r.bit()?))),
                b'w' => t.rdw.decode(r),
                b'h' => t.rdh.decode(r),
                b'x' => t.rdx.decode(r),
                _ => t.rdy.decode(r),
            },
        }
    }

    fn value(&mut self, which: u8) -> Result<i64, CodecError> {
        self.int(which)?.ok_or_else(|| err("unexpected OOB in text region"))
    }

    fn curt(&mut self, log_strips: u32) -> Result<i64, CodecError> {
        match self {
            TextSource::Arith { .. } => self.value(b'i'),
            TextSource::Huffman { r, .. } => Ok(r.bits(log_strips)? as i64),
        }
    }

    fn symbol_id(&mut self) -> Result<u32, CodecError> {
        match self {
            TextSource::Arith { mq, cx, id_bits } => {
                ensure(&mut cx.iaid, 2 << *id_bits);
                Ok(decode_iaid(mq, &mut cx.iaid, *id_bits))
            }
            TextSource::Huffman { r, t, .. } => match &t.ids {
                Some(table) => Ok(table.value(r)? as u32),
                None => Ok(r.bits(t.id_bits)? as u32),
            },
        }
    }

    fn exhausted(&self) -> bool {
        match self {
            TextSource::Arith { mq, .. } => mq.exhausted(),
            TextSource::Huffman { r, .. } => r.pos >= r.data.len() * 8,
        }
    }

    /// Refinement of a symbol instance (6.4.11); in Huffman mode the refinement data is an arithmetic-coded
    /// block of BMSIZE bytes.
    #[allow(clippy::too_many_arguments)]
    fn refine(
        &mut self,
        w: u32,
        h: u32,
        template: u8,
        reference: &Bitmap,
        dx: i64,
        dy: i64,
        at: &[(i32, i32); 2],
        work: &mut Work,
    ) -> Result<Bitmap, CodecError> {
        match self {
            TextSource::Arith { mq, cx, .. } => {
                decode_refinement(mq, &mut cx.gr, w, h, template, reference, dx, dy, false, at, work)
            }
            TextSource::Huffman { r, t, gr } => {
                let size = usize::try_from(t.rsize.value(r)?).map_err(|_| err("bad refinement size"))?;
                r.align();
                let start = r.byte_pos();
                let chunk = r.data.get(start..start.saturating_add(size).min(r.data.len())).unwrap_or(&[]);
                let mut mq = Mq::new(chunk);
                let bm = decode_refinement(&mut mq, gr, w, h, template, reference, dx, dy, false, at, work);
                r.skip_bytes(size);
                bm
            }
        }
    }
}

struct TextParams<'s> {
    width: u32,
    height: u32,
    instances: u64,
    log_strips: u32,
    symbols: &'s [Rc<Bitmap>],
    refine: bool,
    transposed: bool,
    ds_offset: i64,
    corner: u8,
    op: u8,
    default_pixel: u8,
    rtemplate: u8,
    rat: [(i32, i32); 2],
}

/// Text region decoding (6.4.5). A decoding error ends the region; what was drawn is kept.
fn decode_text(src: &mut TextSource, p: &TextParams, work: &mut Work) -> Result<Bitmap, CodecError> {
    let mut region = Bitmap::filled(p.width, p.height, p.default_pixel)?;
    let _ = text_instances(src, p, &mut region, work);
    Ok(region)
}

fn text_instances(
    src: &mut TextSource,
    p: &TextParams,
    region: &mut Bitmap,
    work: &mut Work,
) -> Result<(), CodecError> {
    let strips = 1i64 << p.log_strips;
    let mut strip_t = -src.value(b't')? * strips;
    let mut first_s = 0i64;
    let mut n = 0u64;
    while n < p.instances {
        if src.exhausted() {
            return Ok(());
        }
        strip_t += src.value(b't')? * strips;
        first_s += src.value(b'f')?;
        let mut cur_s = first_s;
        loop {
            if src.exhausted() {
                return Ok(());
            }
            let cur_t = if strips == 1 { 0 } else { src.curt(p.log_strips)? };
            let t = strip_t + cur_t;
            let id = src.symbol_id()? as usize;
            let base = p.symbols.get(id).ok_or_else(|| err("symbol ID out of range"))?;
            let refined;
            let ib: &Bitmap = if p.refine && src.value(b'r')? != 0 {
                let (rdw, rdh) = (src.value(b'w')?, src.value(b'h')?);
                let (rdx, rdy) = (src.value(b'x')?, src.value(b'y')?);
                let w = u32::try_from(i64::from(base.width) + rdw).map_err(|_| err("bad refinement width"))?;
                let h = u32::try_from(i64::from(base.height) + rdh).map_err(|_| err("bad refinement height"))?;
                let (dx, dy) = (rdw.div_euclid(2) + rdx, rdh.div_euclid(2) + rdy);
                refined = src.refine(w, h, p.rtemplate, base, dx, dy, &p.rat, work)?;
                &refined
            } else {
                base
            };
            let (wi, hi) = (i64::from(ib.width), i64::from(ib.height));
            work.spend((wi * hi) as u64 + 1)?;
            // REFCORNER: 0 bottom-left, 1 top-left, 2 bottom-right, 3 top-right.
            if !p.transposed && p.corner > 1 {
                cur_s += wi - 1;
            } else if p.transposed && p.corner & 1 == 0 {
                cur_s += hi - 1;
            }
            let s = cur_s;
            let (x, y) = match (p.transposed, p.corner) {
                (false, 1) => (s, t),
                (false, 3) => (s - wi + 1, t),
                (false, 0) => (s, t - hi + 1),
                (false, _) => (s - wi + 1, t - hi + 1),
                (true, 1) => (t, s),
                (true, 3) => (t - wi + 1, s),
                (true, 0) => (t, s - hi + 1),
                (true, _) => (t - wi + 1, s - hi + 1),
            };
            combine(region, ib, x, y, p.op);
            if !p.transposed && p.corner < 2 {
                cur_s += wi - 1;
            } else if p.transposed && p.corner & 1 != 0 {
                cur_s += hi - 1;
            }
            n += 1;
            match src.int(b's')? {
                None => break,
                Some(_) if n >= p.instances => return Ok(()),
                Some(ds) => cur_s += ds + p.ds_offset,
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------
// The page and its segments
// ---------------------------------------------------------------------------------------------------------------

struct Page {
    bitmap: Bitmap,
    default_pixel: u8,
    default_op: u8,
    op_override: bool,
    /// Striped page of unknown height: grows with regions and end-of-stripe segments.
    open_height: bool,
}

#[derive(Default)]
struct State {
    symbols: HashMap<u32, Rc<Vec<Rc<Bitmap>>>>,
    patterns: HashMap<u32, Rc<Vec<Bitmap>>>,
    tables: HashMap<u32, Rc<HuffTable>>,
    /// Results of intermediate region segments, waiting for a refinement.
    regions: HashMap<u32, (RegionInfo, Bitmap)>,
    page: Option<Page>,
    page_number: Option<u32>,
}

/// Decodes a JBIG2Decode stream (embedded organisation) into page 1's bitmap. `globals` is the JBIG2Globals
/// stream, read first.
pub fn decode(data: &[u8], globals: Option<&[u8]>) -> Result<Bitmap, CodecError> {
    let mut st = State::default();
    let mut work = Work(WORK_LIMIT);
    if let Some(g) = globals {
        for seg in parse_segments(g) {
            // A broken global only matters if a region needs it; the error shows up there.
            let _ = st.process(&seg, &mut work);
        }
    }
    let mut first_error = None;
    for seg in parse_segments(data) {
        if let Some(page) = st.page_number
            && seg.page != 0
            && seg.page != page
        {
            continue; // another page's segment
        }
        match seg.kind {
            49 if st.page.is_some() => break, // end of page
            51 => break,
            _ => {}
        }
        if let Err(e) = st.process(&seg, &mut work) {
            if e.0.contains("work limit") {
                return Err(e);
            }
            first_error.get_or_insert(e);
        }
    }
    let page = st.page.ok_or_else(|| first_error.unwrap_or_else(|| err("no page information segment")))?;
    if page.bitmap.width == 0 || page.bitmap.height == 0 {
        return Err(err("empty page"));
    }
    Ok(page.bitmap)
}

impl State {
    fn process(&mut self, seg: &Segment, work: &mut Work) -> Result<(), CodecError> {
        let d = seg.data;
        match seg.kind {
            0 => {
                let syms = self.symbol_dictionary(seg, work)?;
                self.symbols.insert(seg.number, Rc::new(syms));
            }
            4 | 6 | 7 => {
                let info = region_info(d)?;
                let bm = self.text_region(seg, &info, work)?;
                self.place(seg, info, bm)?;
            }
            16 => {
                let pats = pattern_dictionary(d, work)?;
                self.patterns.insert(seg.number, Rc::new(pats));
            }
            20 | 22 | 23 => {
                let info = region_info(d)?;
                let bm = self.halftone_region(seg, &info, work)?;
                self.place(seg, info, bm)?;
            }
            36 | 38 | 39 => {
                let mut info = region_info(d)?;
                if let Some(rows) = seg.rows {
                    info.height = info.height.min(rows);
                }
                let bm = generic_region(d, &info, work)?;
                self.place(seg, info, bm)?;
            }
            40 | 42 | 43 => {
                let info = region_info(d)?;
                let bm = self.refinement_region(seg, &info, work)?;
                self.place(seg, info, bm)?;
            }
            48 => self.page_information(seg)?,
            50 => {
                let end_row = be32(d, 0).ok_or_else(|| err("short end-of-stripe segment"))?;
                if let Some(page) = &mut self.page
                    && page.open_height
                {
                    let h = end_row.saturating_add(1).min(MAX_DIM);
                    page.bitmap.grow(h, page.default_pixel)?;
                }
            }
            53 => {
                let t = parse_table_segment(d)?;
                self.tables.insert(seg.number, Rc::new(t));
            }
            _ => {} // end of page/file, profiles, extensions, unknown: nothing to draw
        }
        Ok(())
    }

    fn page_information(&mut self, seg: &Segment) -> Result<(), CodecError> {
        if self.page.is_some() {
            return Ok(()); // only the first page is rendered
        }
        let d = seg.data;
        let short = || err("short page information segment");
        let width = be32(d, 0).ok_or_else(short)?;
        let height = be32(d, 4).ok_or_else(short)?;
        let flags = *d.get(16).ok_or_else(short)?;
        let default_pixel = (flags >> 2) & 1;
        let open_height = height == 0xffff_ffff;
        let bitmap = Bitmap::filled(width, if open_height { 0 } else { height }, default_pixel)?;
        self.page = Some(Page {
            bitmap,
            default_pixel,
            default_op: (flags >> 3) & 3,
            op_override: flags & 0x40 != 0,
            open_height,
        });
        self.page_number = Some(seg.page);
        Ok(())
    }

    /// Composes an immediate region onto the page, or keeps an intermediate one for a later refinement.
    fn place(&mut self, seg: &Segment, info: RegionInfo, bm: Bitmap) -> Result<(), CodecError> {
        if matches!(seg.kind, 4 | 20 | 36 | 40) {
            self.regions.insert(seg.number, (info, bm));
            return Ok(());
        }
        let page = self.page.as_mut().ok_or_else(|| err("region before page information"))?;
        if page.open_height {
            let bottom = (info.y + i64::from(bm.height)).clamp(0, i64::from(MAX_DIM)) as u32;
            page.bitmap.grow(bottom, page.default_pixel)?;
        }
        let op = if page.op_override { info.op } else { page.default_op };
        combine(&mut page.bitmap, &bm, info.x, info.y, op);
        Ok(())
    }

    /// Symbols exported by the referred-to dictionaries, in reference order.
    fn input_symbols(&self, refs: &[u32]) -> Vec<Rc<Bitmap>> {
        refs.iter().filter_map(|r| self.symbols.get(r)).flat_map(|v| v.iter().cloned()).collect()
    }

    fn custom_tables(&self, refs: &[u32]) -> Vec<Rc<HuffTable>> {
        refs.iter().filter_map(|r| self.tables.get(r).cloned()).collect()
    }

    /// Symbol dictionary segment (7.4.2, decoded per 6.5).
    fn symbol_dictionary(&self, seg: &Segment, work: &mut Work) -> Result<Vec<Rc<Bitmap>>, CodecError> {
        let d = seg.data;
        let flags = be16(d, 0).ok_or_else(|| err("short symbol dictionary"))?;
        let huffman = flags & 1 != 0;
        let refagg = flags & 2 != 0;
        let template = ((flags >> 10) & 3) as u8;
        let rtemplate = ((flags >> 12) & 1) as u8;
        let mut p = 2;
        let mut at = Vec::new();
        if !huffman {
            let n = if template == 0 { 4 } else { 1 };
            at = read_at(d, p, n)?;
            p += 2 * n;
        }
        let mut rat = [(-1, -1), (-1, -1)];
        if refagg && rtemplate == 0 {
            let v = read_at(d, p, 2)?;
            rat = [v[0], v[1]];
            p += 4;
        }
        let num_exported = u64::from(be32(d, p).ok_or_else(|| err("short symbol dictionary"))?);
        let num_new = u64::from(be32(d, p + 4).ok_or_else(|| err("short symbol dictionary"))?);
        p += 8;
        let input = self.input_symbols(&seg.refs);
        if num_new + input.len() as u64 > MAX_SYMBOLS || num_exported > MAX_SYMBOLS {
            return Err(err("too many symbols"));
        }
        let body = d.get(p..).unwrap_or(&[]);

        // Huffman table selection (7.4.2.1.6).
        let mut custom = self.custom_tables(&seg.refs).into_iter();
        let mut pick = |sel: u32, standard: &[usize]| -> Result<Rc<HuffTable>, CodecError> {
            match standard.get(sel as usize) {
                Some(&n) if sel != 3 => Ok(standard_table(n)),
                _ => custom.next().ok_or_else(|| err("missing custom Huffman table")),
            }
        };
        let tables = if huffman {
            let dh = pick((flags >> 2) & 3, &[4, 5])?;
            let dw = pick((flags >> 4) & 3, &[2, 3])?;
            let bmsize = pick(if flags & 0x40 != 0 { 3 } else { 0 }, &[1])?;
            let agg = pick(if flags & 0x80 != 0 { 3 } else { 0 }, &[1])?;
            Some((dh, dw, bmsize, agg))
        } else {
            None
        };

        let total = input.len() as u64 + num_new;
        let id_bits = if huffman { ceil_log2(total).max(1) } else { ceil_log2(total) };
        let mut all: Vec<Rc<Bitmap>> = input.clone();
        let mut mq = Mq::new(body);
        let mut hb = HBits { data: body, pos: 0 };
        let mut cx = Contexts::new();
        let mut gr_huff = Vec::new();
        let result = (|| -> Result<(), CodecError> {
            let mut height: i64 = 0;
            let mut decoded = 0u64;
            while decoded < num_new {
                if !huffman && mq.exhausted() {
                    break;
                }
                let dh = match &tables {
                    Some(t) => t.0.value(&mut hb)?,
                    None => decode_int(&mut mq, &mut cx.iadh).ok_or_else(|| err("OOB height delta"))?,
                };
                height += dh;
                let hc = u32::try_from(height).map_err(|_| err("negative symbol height"))?;
                let mut width: i64 = 0;
                let mut total_width: u64 = 0;
                let mut class_widths = Vec::new();
                loop {
                    let dw = match &tables {
                        Some(t) => t.1.decode(&mut hb)?,
                        None => decode_int(&mut mq, &mut cx.iadw),
                    };
                    let Some(dw) = dw else { break }; // end of the height class
                    if decoded >= num_new || (!huffman && mq.exhausted()) {
                        return Err(err("too many symbols in height class"));
                    }
                    width += dw;
                    let sw = u32::try_from(width).map_err(|_| err("negative symbol width"))?;
                    total_width += u64::from(sw);
                    if !huffman || refagg {
                        let bm = if !refagg {
                            decode_generic(&mut mq, &mut cx.gb, sw, hc, template, false, &at, None, work)?
                        } else {
                            let instances = match &tables {
                                Some(t) => t.3.value(&mut hb)?,
                                None => decode_int(&mut mq, &mut cx.iaai).ok_or_else(|| err("OOB instance count"))?,
                            };
                            self.aggregate_symbol(
                                &mut mq,
                                &mut cx,
                                &mut hb,
                                &mut gr_huff,
                                huffman,
                                instances,
                                sw,
                                hc,
                                &all,
                                id_bits,
                                rtemplate,
                                &rat,
                                work,
                            )?
                        };
                        all.push(Rc::new(bm));
                    } else {
                        class_widths.push(sw);
                    }
                    decoded += 1;
                }
                if let (Some(t), false) = (&tables, refagg) {
                    // Height class collective bitmap (6.5.9).
                    let size = usize::try_from(t.2.value(&mut hb)?).map_err(|_| err("bad bitmap size"))?;
                    hb.align();
                    let tw = u32::try_from(total_width).map_err(|_| err("height class too wide"))?;
                    let start = hb.byte_pos();
                    let collective = if size == 0 {
                        let stride = tw.div_ceil(8) as usize;
                        let mut bm = Bitmap::new(tw, hc)?;
                        let raw = body
                            .get(start..start + stride * hc as usize)
                            .ok_or_else(|| err("short collective bitmap"))?;
                        bm.data.copy_from_slice(raw);
                        hb.skip_bytes(stride * hc as usize);
                        bm
                    } else {
                        let chunk = body.get(start..start.saturating_add(size).min(body.len())).unwrap_or(&[]);
                        hb.skip_bytes(size);
                        decode_generic_mmr(chunk, tw, hc)?.0
                    };
                    work.spend(u64::from(tw) * u64::from(hc))?;
                    let mut x = 0;
                    for w in class_widths {
                        all.push(Rc::new(collective.columns(x, w)?));
                        x += w;
                    }
                }
            }
            Ok(())
        })();
        if let Err(e) = result {
            // Keep the symbols that decoded; a text region referring to missing ones fails on its own.
            if all.len() == input.len() {
                return Err(e);
            }
        }

        // Export flags (6.5.10).
        let mut exported = Vec::new();
        let mut index = 0usize;
        let mut export = false;
        let b1 = standard_table(1);
        while (index as u64) < total {
            let run = if huffman { b1.decode(&mut hb).ok().flatten() } else { decode_int(&mut mq, &mut cx.iaex) };
            let Some(run) = run.filter(|&r| r >= 0) else { break };
            let end = index.saturating_add(run as usize).min(total as usize);
            if export {
                exported.extend(all.get(index..end.min(all.len())).unwrap_or(&[]).iter().cloned());
            }
            index = end;
            export = !export;
            if !huffman && mq.exhausted() {
                break;
            }
        }
        if exported.is_empty() && num_exported > 0 && index == 0 {
            // Unreadable export flags: export what was decoded so dependent regions still render.
            exported = all.split_off(input.len());
        }
        Ok(exported)
    }

    /// A refinement/aggregate-coded symbol (6.5.8.2).
    #[allow(clippy::too_many_arguments)]
    fn aggregate_symbol(
        &self,
        mq: &mut Mq,
        cx: &mut Contexts,
        hb: &mut HBits,
        gr_huff: &mut Vec<u8>,
        huffman: bool,
        instances: i64,
        w: u32,
        h: u32,
        symbols: &[Rc<Bitmap>],
        id_bits: u32,
        rtemplate: u8,
        rat: &[(i32, i32); 2],
        work: &mut Work,
    ) -> Result<Bitmap, CodecError> {
        if instances == 1 {
            let (id, rdx, rdy) = if huffman {
                let b15 = standard_table(15);
                let id = hb.bits(id_bits)? as usize;
                (id, b15.value(hb)?, b15.value(hb)?)
            } else {
                ensure(&mut cx.iaid, 2 << id_bits);
                let id = decode_iaid(mq, &mut cx.iaid, id_bits) as usize;
                let rdx = decode_int(mq, &mut cx.iardx).ok_or_else(|| err("OOB refinement offset"))?;
                let rdy = decode_int(mq, &mut cx.iardy).ok_or_else(|| err("OOB refinement offset"))?;
                (id, rdx, rdy)
            };
            let reference = symbols.get(id).ok_or_else(|| err("refinement symbol out of range"))?;
            if huffman {
                let size = usize::try_from(standard_table(1).value(hb)?).map_err(|_| err("bad bitmap size"))?;
                hb.align();
                let start = hb.byte_pos();
                let chunk = hb.data.get(start..start.saturating_add(size).min(hb.data.len())).unwrap_or(&[]);
                let mut sub = Mq::new(chunk);
                let bm = decode_refinement(&mut sub, gr_huff, w, h, rtemplate, reference, rdx, rdy, false, rat, work);
                hb.skip_bytes(size);
                return bm;
            }
            return decode_refinement(mq, &mut cx.gr, w, h, rtemplate, reference, rdx, rdy, false, rat, work);
        }
        let n = u64::try_from(instances).map_err(|_| err("bad instance count"))?;
        let params = TextParams {
            width: w,
            height: h,
            instances: n,
            log_strips: 0,
            symbols,
            refine: true,
            transposed: false,
            ds_offset: 0,
            corner: 1,
            op: 0,
            default_pixel: 0,
            rtemplate,
            rat: *rat,
        };
        if huffman {
            let tables = TextTables {
                fs: standard_table(6),
                ds: standard_table(8),
                dt: standard_table(11),
                rdw: standard_table(15),
                rdh: standard_table(15),
                rdx: standard_table(15),
                rdy: standard_table(15),
                rsize: standard_table(1),
                ids: None,
                id_bits,
            };
            let mut src = TextSource::Huffman { r: hb, t: &tables, gr: gr_huff };
            decode_text(&mut src, &params, work)
        } else {
            let mut src = TextSource::Arith { mq, cx, id_bits };
            decode_text(&mut src, &params, work)
        }
    }

    /// Text region segment (7.4.3).
    fn text_region(&self, seg: &Segment, info: &RegionInfo, work: &mut Work) -> Result<Bitmap, CodecError> {
        let d = seg.data;
        let flags = be16(d, 17).ok_or_else(|| err("short text region"))?;
        let huffman = flags & 1 != 0;
        let refine = flags & 2 != 0;
        let log_strips = (flags >> 2) & 3;
        let corner = ((flags >> 4) & 3) as u8;
        let transposed = flags & 0x40 != 0;
        let op = ((flags >> 7) & 3) as u8;
        let default_pixel = ((flags >> 9) & 1) as u8;
        let ds_raw = i64::from((flags >> 10) & 0x1f);
        let ds_offset = if ds_raw >= 16 { ds_raw - 32 } else { ds_raw };
        let rtemplate = ((flags >> 15) & 1) as u8;
        let mut p = 19;
        let mut hflags = 0;
        if huffman {
            hflags = be16(d, p).ok_or_else(|| err("short text region"))?;
            p += 2;
        }
        let mut rat = [(-1, -1), (-1, -1)];
        if refine && rtemplate == 0 {
            let v = read_at(d, p, 2)?;
            rat = [v[0], v[1]];
            p += 4;
        }
        let instances = u64::from(be32(d, p).ok_or_else(|| err("short text region"))?);
        p += 4;
        let symbols = self.input_symbols(&seg.refs);
        if symbols.len() as u64 > MAX_SYMBOLS {
            return Err(err("too many symbols"));
        }
        let body = d.get(p..).unwrap_or(&[]);
        let params = TextParams {
            width: info.width,
            height: info.height,
            instances,
            log_strips,
            symbols: &symbols,
            refine,
            transposed,
            ds_offset,
            corner,
            op,
            default_pixel,
            rtemplate,
            rat,
        };
        if !huffman {
            let mut mq = Mq::new(body);
            let mut cx = Contexts::new();
            let mut src = TextSource::Arith { mq: &mut mq, cx: &mut cx, id_bits: ceil_log2(symbols.len() as u64) };
            return decode_text(&mut src, &params, work);
        }
        // Huffman table selection (7.4.3.1.6), custom tables in field order.
        let mut custom = self.custom_tables(&seg.refs).into_iter();
        let mut pick = |sel: u32, standard: &[usize]| -> Result<Rc<HuffTable>, CodecError> {
            match standard.get(sel as usize) {
                Some(&n) if sel != 3 => Ok(standard_table(n)),
                _ => custom.next().ok_or_else(|| err("missing custom Huffman table")),
            }
        };
        let fs = pick(hflags & 3, &[6, 7])?;
        let ds = pick((hflags >> 2) & 3, &[8, 9, 10])?;
        let dt = pick((hflags >> 4) & 3, &[11, 12, 13])?;
        let rdw = pick((hflags >> 6) & 3, &[14, 15])?;
        let rdh = pick((hflags >> 8) & 3, &[14, 15])?;
        let rdx = pick((hflags >> 10) & 3, &[14, 15])?;
        let rdy = pick((hflags >> 12) & 3, &[14, 15])?;
        let rsize = pick(if hflags & 0x4000 != 0 { 3 } else { 0 }, &[1])?;
        let mut r = HBits { data: body, pos: 0 };
        let ids = symbol_id_table(&mut r, symbols.len())?;
        let tables = TextTables { fs, ds, dt, rdw, rdh, rdx, rdy, rsize, ids: Some(ids), id_bits: 0 };
        let mut gr = Vec::new();
        let mut src = TextSource::Huffman { r: &mut r, t: &tables, gr: &mut gr };
        decode_text(&mut src, &params, work)
    }

    /// Pattern-dictionary-based halftone region (7.4.5, decoded per 6.6).
    fn halftone_region(&self, seg: &Segment, info: &RegionInfo, work: &mut Work) -> Result<Bitmap, CodecError> {
        let d = seg.data;
        let short = || err("short halftone region");
        let flags = *d.get(17).ok_or_else(short)?;
        let mmr = flags & 1 != 0;
        let template = (flags >> 1) & 3;
        let enable_skip = flags & 8 != 0;
        let op = (flags >> 4) & 7;
        let default_pixel = (flags >> 7) & 1;
        let gw = be32(d, 18).ok_or_else(short)?;
        let gh = be32(d, 22).ok_or_else(short)?;
        let gx = i64::from(be32(d, 26).ok_or_else(short)? as i32);
        let gy = i64::from(be32(d, 30).ok_or_else(short)? as i32);
        let rx = i64::from(be16(d, 34).ok_or_else(short)?);
        let ry = i64::from(be16(d, 36).ok_or_else(short)?);
        let body = d.get(38..).unwrap_or(&[]);
        let patterns =
            seg.refs.iter().find_map(|r| self.patterns.get(r)).ok_or_else(|| err("missing pattern dictionary"))?;
        let first = patterns.first().ok_or_else(|| err("empty pattern dictionary"))?;
        let (pw, ph) = (i64::from(first.width), i64::from(first.height));
        if u64::from(gw) * u64::from(gh) > MAX_PIXELS {
            return Err(err("halftone grid too large"));
        }
        let mut region = Bitmap::filled(info.width, info.height, default_pixel)?;
        let bpp = ceil_log2(patterns.len() as u64);
        let cell = |m: i64, n: i64| ((gx + m * ry + n * rx) >> 8, (gy + m * rx - n * ry) >> 8);

        // HSKIP (6.6.5.1): grid cells whose pattern falls wholly outside the region are not coded.
        let skip = if enable_skip {
            let mut s = Bitmap::new(gw, gh)?;
            for m in 0..i64::from(gh) {
                for n in 0..i64::from(gw) {
                    let (x, y) = cell(m, n);
                    if x + pw <= 0 || x >= i64::from(info.width) || y + ph <= 0 || y >= i64::from(info.height) {
                        s.put(n, m, 1);
                    }
                }
            }
            Some(s)
        } else {
            None
        };

        // Gray-scale image (Annex C.5): bit planes MSB first, then Gray-code decoding.
        let mut planes: Vec<Bitmap> = Vec::with_capacity(bpp as usize);
        let mut mq = Mq::new(body);
        let mut gb = Vec::new();
        let mut offset = 0usize;
        let at: Vec<(i32, i32)> = vec![(if template <= 1 { 3 } else { 2 }, -1), (-3, -1), (2, -2), (-2, -2)];
        for _ in 0..bpp {
            let plane = if mmr {
                let (bm, used) = decode_generic_mmr(body.get(offset..).unwrap_or(&[]), gw, gh)?;
                offset += used;
                bm
            } else {
                decode_generic(&mut mq, &mut gb, gw, gh, template, false, &at, skip.as_ref(), work)?
            };
            planes.push(plane);
        }
        // planes[0] is the most significant bit.
        for m in 0..i64::from(gh) {
            for n in 0..i64::from(gw) {
                let mut bit = 0;
                let mut value = 0usize;
                for (j, plane) in planes.iter().enumerate() {
                    bit ^= plane.get(n, m);
                    value |= (bit as usize) << (bpp as usize - 1 - j);
                }
                let pattern = &patterns[value.min(patterns.len() - 1)];
                let (x, y) = cell(m, n);
                work.spend((pw * ph) as u64 + 1)?;
                combine(&mut region, pattern, x, y, op);
            }
        }
        Ok(region)
    }

    /// Generic refinement region (7.4.7): refines an intermediate region it refers to, or the page.
    fn refinement_region(&mut self, seg: &Segment, info: &RegionInfo, work: &mut Work) -> Result<Bitmap, CodecError> {
        let d = seg.data;
        let flags = *d.get(17).ok_or_else(|| err("short refinement region"))?;
        let template = flags & 1;
        let tpgron = flags & 2 != 0;
        let mut p = 18;
        let mut at = [(-1, -1), (-1, -1)];
        if template == 0 {
            let v = read_at(d, p, 2)?;
            at = [v[0], v[1]];
            p += 4;
        }
        let reference = match seg.refs.iter().find_map(|r| self.regions.remove(r)) {
            Some((_, bm)) => bm,
            None => {
                let page = self.page.as_ref().ok_or_else(|| err("refinement without a reference"))?;
                page.bitmap.extract(info.x, info.y, info.width, info.height)?
            }
        };
        let mut mq = Mq::new(d.get(p..).unwrap_or(&[]));
        let mut gr = Vec::new();
        decode_refinement(&mut mq, &mut gr, info.width, info.height, template, &reference, 0, 0, tpgron, &at, work)
    }
}

/// Symbol ID Huffman table of a Huffman text region (7.4.3.1.7): run-length coded code lengths.
fn symbol_id_table(r: &mut HBits, count: usize) -> Result<HuffTable, CodecError> {
    let mut run_lines = Vec::with_capacity(35);
    for i in 0..35 {
        run_lines.push(Line { low: i, prefix: r.bits(4)? as u32, range: 0, kind: LineKind::Normal });
    }
    let runs = HuffTable::new(run_lines)?;
    let mut lengths: Vec<u32> = Vec::with_capacity(count);
    while lengths.len() < count {
        let code = runs.value(r)?;
        let (len, repeat) = match code {
            0..=31 => (code as u32, 1),
            32 => (*lengths.last().ok_or_else(|| err("symbol ID table repeats nothing"))?, r.bits(2)? + 3),
            33 => (0, r.bits(3)? + 3),
            _ => (0, r.bits(7)? + 11),
        };
        for _ in 0..repeat {
            if lengths.len() < count {
                lengths.push(len);
            }
        }
    }
    r.align();
    HuffTable::new(
        lengths
            .iter()
            .enumerate()
            .map(|(i, &l)| Line { low: i as i64, prefix: l, range: 0, kind: LineKind::Normal })
            .collect(),
    )
}

/// Pattern dictionary (7.4.4, decoded per 6.7): one collective bitmap cut into GRAYMAX + 1 patterns.
fn pattern_dictionary(d: &[u8], work: &mut Work) -> Result<Vec<Bitmap>, CodecError> {
    let short = || err("short pattern dictionary");
    let flags = *d.first().ok_or_else(short)?;
    let mmr = flags & 1 != 0;
    let template = (flags >> 1) & 3;
    let pw = u32::from(*d.get(1).ok_or_else(short)?);
    let ph = u32::from(*d.get(2).ok_or_else(short)?);
    let count = u64::from(be32(d, 3).ok_or_else(short)?) + 1;
    let total = count
        .checked_mul(u64::from(pw))
        .filter(|&w| w <= u64::from(MAX_DIM))
        .ok_or_else(|| err("pattern dictionary too wide"))?;
    if pw == 0 || ph == 0 {
        return Err(err("empty patterns"));
    }
    let body = d.get(7..).unwrap_or(&[]);
    let collective = if mmr {
        decode_generic_mmr(body, total as u32, ph)?.0
    } else {
        let at = vec![(-(pw as i32), 0), (-3, -1), (2, -2), (-2, -2)];
        let mut mq = Mq::new(body);
        decode_generic(&mut mq, &mut Vec::new(), total as u32, ph, template, false, &at, None, work)?
    };
    (0..count as u32).map(|i| collective.columns(i * pw, pw)).collect()
}

/// Generic region segment (7.4.6).
fn generic_region(d: &[u8], info: &RegionInfo, work: &mut Work) -> Result<Bitmap, CodecError> {
    let flags = *d.get(17).ok_or_else(|| err("short generic region"))?;
    let mmr = flags & 1 != 0;
    let template = (flags >> 1) & 3;
    let tpgdon = flags & 8 != 0;
    if flags & 0x10 != 0 {
        return Err(err("extended generic templates are not supported"));
    }
    if mmr {
        work.spend(u64::from(info.width) * u64::from(info.height))?;
        return Ok(decode_generic_mmr(d.get(18..).unwrap_or(&[]), info.width, info.height)?.0);
    }
    let n = if template == 0 { 4 } else { 1 };
    let at = read_at(d, 18, n)?;
    let mut mq = Mq::new(d.get(18 + 2 * n..).unwrap_or(&[]));
    decode_generic(&mut mq, &mut Vec::new(), info.width, info.height, template, tpgdon, &at, None, work)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    // ----- encoders the tests build streams with -----

    /// MQ encoder (T.88 E.2), the inverse of [`Mq`].
    struct MqEnc {
        a: u32,
        c: u32,
        ct: u32,
        out: Vec<u8>,
    }

    impl MqEnc {
        fn new() -> Self {
            // out[0] stands for the byte before the stream, which the encoder may touch but never emits.
            MqEnc { a: 0x8000, c: 0, ct: 12, out: vec![0] }
        }

        fn encode(&mut self, cx: &mut [u8], i: usize, d: u32) {
            let state = cx[i];
            let (mut index, mut mps) = (usize::from(state >> 1), u32::from(state & 1));
            let (qe, nmps, nlps, switch) = QE[index];
            self.a -= qe;
            if d == mps {
                if self.a & 0x8000 == 0 {
                    if self.a < qe {
                        self.a = qe
                    } else {
                        self.c += qe
                    }
                    index = usize::from(nmps);
                    self.renorm();
                } else {
                    self.c += qe;
                }
            } else {
                if self.a < qe {
                    self.c += qe
                } else {
                    self.a = qe
                }
                if switch {
                    mps = 1 - mps;
                }
                index = usize::from(nlps);
                self.renorm();
            }
            cx[i] = ((index as u8) << 1) | mps as u8;
        }

        fn renorm(&mut self) {
            loop {
                self.a <<= 1;
                self.c <<= 1;
                self.ct -= 1;
                if self.ct == 0 {
                    self.byte_out();
                }
                if self.a & 0x8000 != 0 {
                    break;
                }
            }
        }

        fn byte_out(&mut self) {
            let last = self.out.len() - 1;
            if self.out[last] == 0xff {
                self.out.push((self.c >> 20) as u8);
                self.c &= 0xfffff;
                self.ct = 7;
            } else if self.c < 0x800_0000 {
                self.out.push((self.c >> 19) as u8);
                self.c &= 0x7ffff;
                self.ct = 8;
            } else {
                self.out[last] += 1;
                if self.out[last] == 0xff {
                    self.c &= 0x7ff_ffff;
                    self.out.push((self.c >> 20) as u8);
                    self.c &= 0xfffff;
                    self.ct = 7;
                } else {
                    self.out.push((self.c >> 19) as u8);
                    self.c &= 0x7ffff;
                    self.ct = 8;
                }
            }
        }

        fn finish(mut self) -> Vec<u8> {
            let temp = self.c + self.a;
            self.c |= 0xffff;
            if self.c >= temp {
                self.c -= 0x8000;
            }
            self.c <<= self.ct;
            self.byte_out();
            self.c <<= self.ct;
            self.byte_out();
            if self.out.last() != Some(&0xff) {
                self.out.push(0xff);
            }
            self.out.push(0xac);
            self.out.remove(0);
            self.out
        }

        /// Integer encoding (A.2); `None` writes OOB.
        fn int(&mut self, cx: &mut [u8], v: Option<i64>) {
            let mut prev = 1usize;
            let mut put = |e: &mut MqEnc, n: u32, value: u64| {
                for i in (0..n).rev() {
                    let b = ((value >> i) & 1) as u32;
                    e.encode(cx, prev, b);
                    prev = if prev < 256 { (prev << 1) | b as usize } else { (((prev << 1) | b as usize) & 511) | 256 };
                }
            };
            let (sign, a) = match v {
                None => (1, 0u64),
                Some(v) => (u64::from(v < 0), v.unsigned_abs()),
            };
            put(self, 1, sign);
            let ranges: [(u64, u32, u32, u64); 6] = [
                (0, 1, 2, 0b0),
                (4, 2, 4, 0b10),
                (20, 3, 6, 0b110),
                (84, 4, 8, 0b1110),
                (340, 5, 12, 0b11110),
                (4436, 5, 32, 0b11111),
            ];
            let &(base, plen, bits, prefix) = ranges.iter().rev().find(|r| a >= r.0).unwrap();
            put(self, plen, prefix);
            put(self, bits, a - base);
        }

        fn iaid(&mut self, cx: &mut [u8], len: u32, id: u32) {
            let mut prev = 1usize;
            for i in (0..len).rev() {
                let b = (id >> i) & 1;
                self.encode(cx, prev, b);
                prev = (prev << 1) | b as usize;
            }
        }
    }

    /// Generic template context straight from the T.88 figures, one pixel read per bit.
    fn generic_ctx(bm: &Bitmap, x: i64, y: i64, template: u8, at: &[(i32, i32)]) -> usize {
        // (dx, dy) per context bit, bit 0 first; None marks the AT pixel slots in order.
        let layout: &[Option<(i64, i64)>] = match template {
            0 => &[
                Some((-1, 0)),
                Some((-2, 0)),
                Some((-3, 0)),
                Some((-4, 0)),
                None,
                Some((2, -1)),
                Some((1, -1)),
                Some((0, -1)),
                Some((-1, -1)),
                Some((-2, -1)),
                None,
                None,
                Some((1, -2)),
                Some((0, -2)),
                Some((-1, -2)),
                None,
            ],
            1 => &[
                Some((-1, 0)),
                Some((-2, 0)),
                Some((-3, 0)),
                None,
                Some((2, -1)),
                Some((1, -1)),
                Some((0, -1)),
                Some((-1, -1)),
                Some((-2, -1)),
                Some((2, -2)),
                Some((1, -2)),
                Some((0, -2)),
                Some((-1, -2)),
            ],
            2 => &[
                Some((-1, 0)),
                Some((-2, 0)),
                None,
                Some((1, -1)),
                Some((0, -1)),
                Some((-1, -1)),
                Some((-2, -1)),
                Some((1, -2)),
                Some((0, -2)),
                Some((-1, -2)),
            ],
            _ => &[
                Some((-1, 0)),
                Some((-2, 0)),
                Some((-3, 0)),
                Some((-4, 0)),
                None,
                Some((1, -1)),
                Some((0, -1)),
                Some((-1, -1)),
                Some((-2, -1)),
                Some((-3, -1)),
            ],
        };
        let mut ctx = 0usize;
        let mut a = 0;
        for (bit, slot) in layout.iter().enumerate() {
            let (dx, dy) = slot.unwrap_or_else(|| {
                a += 1;
                (i64::from(at[a - 1].0), i64::from(at[a - 1].1))
            });
            ctx |= (bm.get(x + dx, y + dy) as usize) << bit;
        }
        ctx
    }

    fn encode_generic(
        e: &mut MqEnc,
        cx: &mut Vec<u8>,
        bm: &Bitmap,
        template: u8,
        tpgdon: bool,
        at: &[(i32, i32)],
        skip: Option<&Bitmap>,
    ) {
        ensure(cx, 1 << GENERIC[usize::from(template)].context_bits);
        let mut ltp = 0;
        for y in 0..i64::from(bm.height) {
            if tpgdon {
                let same = (0..i64::from(bm.width)).all(|x| bm.get(x, y) == bm.get(x, y - 1));
                let l = u32::from(same);
                e.encode(cx, GENERIC[usize::from(template)].sltp, l ^ ltp);
                ltp = l;
                if same {
                    continue;
                }
            }
            for x in 0..i64::from(bm.width) {
                if skip.is_some_and(|s| s.get(x, y) != 0) {
                    continue;
                }
                let ctx = generic_ctx(bm, x, y, template, at);
                e.encode(cx, ctx, bm.get(x, y));
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_refinement(
        e: &mut MqEnc,
        cx: &mut Vec<u8>,
        bm: &Bitmap,
        template: u8,
        r: &Bitmap,
        dx: i64,
        dy: i64,
        tpgron: bool,
        at: &[(i32, i32); 2],
    ) {
        ensure(cx, 1 << 13);
        let mut ltp = 0;
        let uniform = |rx: i64, ry: i64| {
            let v = r.get(rx, ry);
            (-1..=1).all(|j| (-1..=1).all(|i| r.get(rx + i, ry + j) == v)).then_some(v)
        };
        for y in 0..i64::from(bm.height) {
            if tpgron {
                let predictable =
                    (0..i64::from(bm.width)).all(|x| uniform(x - dx, y - dy).is_none_or(|v| v == bm.get(x, y)));
                let l = u32::from(predictable);
                e.encode(cx, if template == 0 { 0x20 } else { 0x04 }, l ^ ltp);
                ltp = l;
            }
            for x in 0..i64::from(bm.width) {
                let (rx, ry) = (x - dx, y - dy);
                if ltp != 0 && uniform(rx, ry).is_some() {
                    continue;
                }
                let coding: Vec<(i64, i64)>;
                let refpx: Vec<(i64, i64)>;
                if template == 0 {
                    coding = vec![(0, -1), (1, -1), (-1, 0), (i64::from(at[0].0), i64::from(at[0].1))];
                    refpx = vec![
                        (0, -1),
                        (1, -1),
                        (-1, 0),
                        (0, 0),
                        (1, 0),
                        (-1, 1),
                        (0, 1),
                        (1, 1),
                        (i64::from(at[1].0), i64::from(at[1].1)),
                    ];
                } else {
                    coding = vec![(-1, -1), (0, -1), (1, -1), (-1, 0)];
                    refpx = vec![(0, -1), (-1, 0), (0, 0), (1, 0), (0, 1), (1, 1)];
                }
                let mut ctx = 0usize;
                for &(cx_, cy) in &coding {
                    ctx = (ctx << 1) | bm.get(x + cx_, y + cy) as usize;
                }
                for &(px, py) in &refpx {
                    ctx = (ctx << 1) | r.get(rx + px, ry + py) as usize;
                }
                e.encode(cx, ctx, bm.get(x, y));
            }
        }
    }

    /// MSB-first bit writer for the Huffman-coded tests.
    #[derive(Default)]
    struct BitW {
        out: Vec<u8>,
        n: u32,
    }

    impl BitW {
        fn put(&mut self, v: u64, len: u32) {
            for i in (0..len).rev() {
                if self.n.is_multiple_of(8) {
                    self.out.push(0);
                }
                if (v >> i) & 1 != 0 {
                    *self.out.last_mut().unwrap() |= 0x80 >> (self.n % 8);
                }
                self.n += 1;
            }
        }
        fn align(&mut self) {
            self.n = self.n.div_ceil(8) * 8;
        }
        fn bytes(&mut self, b: &[u8]) {
            self.align();
            self.out.extend_from_slice(b);
            self.n += 8 * b.len() as u32;
        }
        /// Writes `v` with a table (None for OOB).
        fn huff(&mut self, t: &HuffTable, v: Option<i64>) {
            let i = t
                .lines
                .iter()
                .position(|l| match (l.kind, v) {
                    (LineKind::Oob, None) => true,
                    (LineKind::Normal, Some(v)) => v >= l.low && (v - l.low) < (1i64 << l.range) && l.prefix > 0,
                    (LineKind::Lower, Some(v)) => v <= l.low,
                    _ => false,
                })
                .unwrap_or_else(|| panic!("value {v:?} not in table"));
            let l = t.lines[i];
            self.put(u64::from(t.codes[i]), l.prefix);
            if let Some(v) = v {
                let off = if l.kind == LineKind::Lower { l.low - v } else { v - l.low };
                self.put(off as u64, l.range);
            }
        }
    }

    // ----- stream assembly -----

    fn segment(number: u32, kind: u8, refs: &[u32], page: u32, data: &[u8]) -> Vec<u8> {
        let mut v = number.to_be_bytes().to_vec();
        v.push(kind | if page > 255 { 0x40 } else { 0 });
        assert!(refs.len() <= 4);
        v.push((refs.len() as u8) << 5);
        for &r in refs {
            match number {
                0..=256 => v.push(r as u8),
                257..=65536 => v.extend_from_slice(&(r as u16).to_be_bytes()),
                _ => v.extend_from_slice(&r.to_be_bytes()),
            }
        }
        if page > 255 {
            v.extend_from_slice(&page.to_be_bytes());
        } else {
            v.push(page as u8);
        }
        v.extend_from_slice(&(data.len() as u32).to_be_bytes());
        v.extend_from_slice(data);
        v
    }

    fn page_info(w: u32, h: u32, flags: u8, striping: u16) -> Vec<u8> {
        let mut v = Vec::new();
        for x in [w, h, 0, 0] {
            v.extend_from_slice(&x.to_be_bytes());
        }
        v.push(flags);
        v.extend_from_slice(&striping.to_be_bytes());
        v
    }

    fn region(w: u32, h: u32, x: u32, y: u32, op: u8) -> Vec<u8> {
        let mut v = Vec::new();
        for f in [w, h, x, y] {
            v.extend_from_slice(&f.to_be_bytes());
        }
        v.push(op);
        v
    }

    fn at_bytes(at: &[(i32, i32)]) -> Vec<u8> {
        at.iter().flat_map(|&(x, y)| [x as i8 as u8, y as i8 as u8]).collect()
    }

    /// An immediate generic region segment body coded with the MQ coder.
    fn generic_segment(bm: &Bitmap, x: u32, y: u32, op: u8, template: u8, tpgdon: bool, at: &[(i32, i32)]) -> Vec<u8> {
        let mut e = MqEnc::new();
        encode_generic(&mut e, &mut Vec::new(), bm, template, tpgdon, at, None);
        let mut d = region(bm.width, bm.height, x, y, op);
        d.push((template << 1) | if tpgdon { 8 } else { 0 });
        d.extend(at_bytes(at));
        d.extend(e.finish());
        d
    }

    // ----- test images -----

    struct Rng(u32);

    impl Rng {
        fn next(&mut self) -> u32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 17;
            self.0 ^= self.0 << 5;
            self.0
        }
        fn below(&mut self, n: u32) -> u32 {
            self.next() % n.max(1)
        }
    }

    /// Blobs, strokes and a little noise: content the templates have something to learn from.
    fn sample(w: u32, h: u32, seed: u32) -> Bitmap {
        let mut bm = Bitmap::new(w, h).unwrap();
        let mut r = Rng(seed.wrapping_mul(2_654_435_761) | 1);
        for _ in 0..(w * h / 60).max(2) {
            let (x0, y0) = (r.below(w) as i64, r.below(h) as i64);
            let (bw, bh) = (1 + r.below(9) as i64, 1 + r.below(7) as i64);
            for y in y0..y0 + bh {
                for x in x0..x0 + bw {
                    bm.put(x, y, 1);
                }
            }
        }
        for _ in 0..(w * h / 200) {
            bm.put(r.below(w) as i64, r.below(h) as i64, 1);
        }
        // Repeated rows exercise TPGDON.
        if h > 8 {
            for y in h / 2..h / 2 + 3 {
                bm.copy_row(h / 2 - 1, y);
            }
        }
        bm
    }

    fn glyph(w: u32, h: u32, seed: u32) -> Bitmap {
        let mut bm = sample(w, h, seed);
        for y in 0..i64::from(h) {
            bm.put(0, y, 1); // a stem, so every glyph is visibly different from blank
        }
        bm
    }

    fn assert_same(got: &Bitmap, want: &Bitmap, what: &str) {
        assert_eq!((got.width, got.height), (want.width, want.height), "{what}: size");
        for y in 0..i64::from(want.height) {
            for x in 0..i64::from(want.width) {
                assert_eq!(got.get(x, y), want.get(x, y), "{what}: pixel ({x}, {y})");
            }
        }
    }

    /// jbig2dec's rendering of an embedded stream, when the tool is installed.
    fn jbig2dec(stream: &[u8], globals: Option<&[u8]>) -> Option<Bitmap> {
        Command::new("jbig2dec").arg("--version").output().ok()?;
        static RUNS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let run = RUNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("pdf-codecs-jbig2-{}-{run}", std::process::id()));
        std::fs::create_dir_all(&dir).ok()?;
        let page = dir.join("page.jb2");
        let out = dir.join("out.pbm");
        std::fs::write(&page, stream).ok()?;
        let mut cmd = Command::new("jbig2dec");
        cmd.args(["-q", "-t", "pbm", "-o"]).arg(&out);
        if let Some(g) = globals {
            let gp = dir.join("globals.jb2");
            std::fs::write(&gp, g).ok()?;
            cmd.arg(&gp).arg(&page);
        } else {
            cmd.arg("-e").arg(&page);
        }
        let output = cmd.output();
        let pbm = std::fs::read(&out).ok();
        // Clean up before any assertion can fire.
        let _ = std::fs::remove_dir_all(&dir);
        let output = output.ok()?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!stderr.contains("FATAL"), "jbig2dec could not decode the test stream: {stderr}");
        assert!(output.status.success(), "jbig2dec failed on the test stream");
        let pbm = pbm?;
        // P4 header: magic, width, height, single whitespace, then packed rows.
        let text: Vec<&[u8]> = pbm.splitn(4, |b| b.is_ascii_whitespace()).collect();
        let (w, h) =
            (std::str::from_utf8(text[1]).ok()?.parse().ok()?, std::str::from_utf8(text[2]).ok()?.parse().ok()?);
        let mut bm = Bitmap::new(w, h).ok()?;
        let body = text[3];
        let n = bm.data.len();
        bm.data.copy_from_slice(body.get(body.len().checked_sub(n)?..)?);
        Some(bm)
    }

    /// Decodes with us, compares with the expected image and, when available, with jbig2dec.
    fn check(stream: &[u8], globals: Option<&[u8]>, want: &Bitmap, what: &str) {
        check_ours(stream, globals, want, what);
        if let Some(reference) = jbig2dec(stream, globals) {
            assert_same(&reference, want, &format!("{what} (jbig2dec)"));
            eprintln!("jbig2dec agrees: {what}");
        }
    }

    /// Decodes with us only: for cases where jbig2dec departs from T.88 (see the callers).
    fn check_ours(stream: &[u8], globals: Option<&[u8]>, want: &Bitmap, what: &str) {
        let got = decode(stream, globals).unwrap_or_else(|e| panic!("{what}: {e}"));
        assert_same(&got, want, what);
    }

    // ----- tests -----

    #[test]
    fn mq_coder_round_trips() {
        let mut r = Rng(5);
        let decisions: Vec<(usize, u32)> = (0..20000)
            .map(|i| ((r.below(4) as usize), u32::from(r.below(10) < if i % 2 == 0 { 1 } else { 7 })))
            .collect();
        let mut e = MqEnc::new();
        let mut cx = vec![0u8; 4];
        for &(c, d) in &decisions {
            e.encode(&mut cx, c, d);
        }
        let ints: Vec<Option<i64>> = vec![
            Some(0),
            Some(3),
            Some(-3),
            Some(4),
            Some(19),
            Some(20),
            Some(-83),
            Some(84),
            Some(339),
            Some(340),
            Some(4435),
            Some(4436),
            Some(-1_000_000),
            None,
            Some(i64::from(i32::MAX)),
        ];
        let mut icx = vec![0u8; 512];
        for &v in &ints {
            e.int(&mut icx, v);
        }
        let mut idcx = vec![0u8; 2 << 7];
        for id in [0u32, 1, 77, 127] {
            e.iaid(&mut idcx, 7, id);
        }
        let data = e.finish();
        let mut mq = Mq::new(&data);
        let mut cx = vec![0u8; 4];
        for (k, &(c, d)) in decisions.iter().enumerate() {
            assert_eq!(mq.decode(&mut cx, c), d, "decision {k}");
        }
        let mut icx = vec![0u8; 512];
        for &v in &ints {
            assert_eq!(decode_int(&mut mq, &mut icx), v);
        }
        let mut idcx = vec![0u8; 2 << 7];
        for id in [0u32, 1, 77, 127] {
            assert_eq!(decode_iaid(&mut mq, &mut idcx, 7), id);
        }
        assert!(!mq.exhausted());
        for _ in 0..10_000 {
            mq.decode(&mut cx, 0);
        }
        assert!(mq.exhausted(), "running past the data is reported");
    }

    #[test]
    fn standard_tables_match_printed_codes() {
        for (n, table) in STANDARD_TABLES.iter().enumerate() {
            let t = standard_table(n + 1);
            assert_eq!(t.lines.len(), table.len());
            for (i, row) in table.iter().enumerate() {
                assert_eq!(t.codes[i], row.4, "table B.{} line {i}", n + 1);
            }
        }
        // Values decode back through every standard table.
        for n in 1..=15 {
            let t = standard_table(n);
            let values: Vec<Option<i64>> =
                [-3000i64, -300, -26, -17, -2, -1, 0, 1, 2, 3, 12, 75, 76, 140, 1000, 5000, 70000]
                    .iter()
                    .copied()
                    .filter(|&v| {
                        t.lines.iter().any(|l| match l.kind {
                            LineKind::Normal => v >= l.low && v - l.low < 1i64 << l.range,
                            LineKind::Lower => v <= l.low,
                            LineKind::Oob => false,
                        })
                    })
                    .map(Some)
                    .chain(t.lines.iter().any(|l| l.kind == LineKind::Oob).then_some(None))
                    .collect();
            let mut w = BitW::default();
            for &v in &values {
                w.huff(&t, v);
            }
            let mut r = HBits { data: &w.out, pos: 0 };
            for &v in &values {
                assert_eq!(t.decode(&mut r).unwrap(), v, "B.{n}");
            }
        }
    }

    #[test]
    fn generic_regions_arithmetic() {
        type Case<'a> = (u8, bool, &'a [(i32, i32)]);
        let cases: &[Case] = &[
            (0, false, &[(3, -1), (-3, -1), (2, -2), (-2, -2)]),
            (0, true, &[(3, -1), (-3, -1), (2, -2), (-2, -2)]),
            (0, true, &[(-1, -1), (-5, 0), (4, -3), (-7, -2)]),
            (1, false, &[(3, -1)]),
            (1, true, &[(-2, -2)]),
            (2, false, &[(2, -1)]),
            (2, true, &[(-3, 0)]),
            (3, false, &[(2, -1)]),
            (3, true, &[(5, -1)]),
        ];
        for (i, &(template, tpgdon, at)) in cases.iter().enumerate() {
            let bm = sample(97, 43, i as u32 + 1);
            let mut stream = segment(0, 48, &[], 1, &page_info(97, 43, 0, 0));
            stream.extend(segment(1, 38, &[], 1, &generic_segment(&bm, 0, 0, 0, template, tpgdon, at)));
            stream.extend(segment(2, 49, &[], 1, &[]));
            check(&stream, None, &bm, &format!("template {template} tpgdon {tpgdon} at {at:?}"));
        }
    }

    #[test]
    fn generic_region_mmr_and_placement() {
        let bm = sample(150, 60, 9);
        let mmr = super::super::ccitt::tests::encode_mmr(&bm.data, 150, 60);
        let mut d = region(150, 60, 10, 5, 0);
        d.push(1);
        d.extend(&mmr);
        let mut stream = segment(0, 48, &[], 1, &page_info(170, 70, 0, 0));
        stream.extend(segment(1, 38, &[], 1, &d));
        let mut want = Bitmap::new(170, 70).unwrap();
        combine(&mut want, &bm, 10, 5, 0);
        check(&stream, None, &want, "MMR generic region");
    }

    /// A dictionary of `glyphs` (grouped by height, as the encoder requires), arithmetic coded, all exported.
    fn arith_dictionary(glyphs: &[Bitmap], template: u8, at: &[(i32, i32)]) -> Vec<u8> {
        let mut e = MqEnc::new();
        let (mut dh, mut dw, mut ex, mut gb) = (vec![0u8; 512], vec![0u8; 512], vec![0u8; 512], Vec::new());
        let mut h = 0i64;
        let mut i = 0;
        while i < glyphs.len() {
            let hc = glyphs[i].height;
            e.int(&mut dh, Some(i64::from(hc) - h));
            h = i64::from(hc);
            let mut w = 0i64;
            while i < glyphs.len() && glyphs[i].height == hc {
                e.int(&mut dw, Some(i64::from(glyphs[i].width) - w));
                w = i64::from(glyphs[i].width);
                encode_generic(&mut e, &mut gb, &glyphs[i], template, false, at, None);
                i += 1;
            }
            e.int(&mut dw, None);
        }
        e.int(&mut ex, Some(0));
        e.int(&mut ex, Some(glyphs.len() as i64));
        let mut d = Vec::new();
        d.extend_from_slice(&(u16::from(template) << 10).to_be_bytes());
        d.extend(at_bytes(at));
        d.extend_from_slice(&(glyphs.len() as u32).to_be_bytes());
        d.extend_from_slice(&(glyphs.len() as u32).to_be_bytes());
        d.extend(e.finish());
        d
    }

    struct Instance {
        id: usize,
        x: i64,
        y: i64,
        /// Refined instance: the bitmap drawn and (RDX, RDY); RDW/RDH follow from its size.
        refine: Option<(Bitmap, i64, i64)>,
    }

    impl Instance {
        fn plain(id: usize, x: i64, y: i64) -> Instance {
            Instance { id, x, y, refine: None }
        }
        fn bitmap<'a>(&'a self, glyphs: &'a [Bitmap]) -> &'a Bitmap {
            self.refine.as_ref().map_or(&glyphs[self.id], |r| &r.0)
        }
    }

    /// Text region layout in the decoder's terms: per instance (strip T, CURT, CURS before placement, CURS after,
    /// index), derived from the wanted top-left corners.
    fn layout(
        insts: &[Instance],
        glyphs: &[Bitmap],
        strips: i64,
        corner: u8,
        transposed: bool,
    ) -> Vec<(i64, i64, i64, i64, usize)> {
        let mut v: Vec<(i64, i64, i64, i64, usize)> = insts
            .iter()
            .enumerate()
            .map(|(k, it)| {
                let bm = it.bitmap(glyphs);
                let (w, h) = (i64::from(bm.width), i64::from(bm.height));
                let right = corner & 2 != 0;
                let bottom = corner & 1 == 0;
                let (_, t_i, before, extent) = if !transposed {
                    let s_i = it.x + if right { w - 1 } else { 0 };
                    let t_i = it.y + if bottom { h - 1 } else { 0 };
                    (s_i, t_i, if right { s_i - (w - 1) } else { s_i }, w)
                } else {
                    let s_i = it.y + if bottom { h - 1 } else { 0 };
                    let t_i = it.x + if right { w - 1 } else { 0 };
                    (s_i, t_i, if bottom { s_i - (h - 1) } else { s_i }, h)
                };
                let strip = t_i.div_euclid(strips) * strips;
                (strip, t_i - strip, before, before + extent - 1, k)
            })
            .collect();
        v.sort_by_key(|e| (e.0, e.2));
        v
    }

    struct TextOpts {
        log_strips: u32,
        corner: u8,
        transposed: bool,
        op: u8,
        ds_offset: i64,
        default_pixel: u8,
        refine: bool,
        rtemplate: u8,
    }

    const PLAIN: TextOpts = TextOpts {
        log_strips: 0,
        corner: 1,
        transposed: false,
        op: 0,
        ds_offset: 0,
        default_pixel: 0,
        refine: false,
        rtemplate: 0,
    };

    /// Codes the instances of a text region with the MQ coder into `e`, sharing `cx` (also how aggregate
    /// symbols are coded inside a dictionary).
    fn code_text_arith(
        e: &mut MqEnc,
        cx: &mut Contexts,
        insts: &[Instance],
        glyphs: &[Bitmap],
        id_bits: u32,
        o: &TextOpts,
    ) {
        let strips = 1i64 << o.log_strips;
        let rows = layout(insts, glyphs, strips, o.corner, o.transposed);
        ensure(&mut cx.iaid, 2 << id_bits);
        e.int(&mut cx.iadt, Some(0));
        let (mut strip_t, mut first_s) = (0i64, 0i64);
        let mut k = 0;
        while k < rows.len() {
            let strip = rows[k].0;
            e.int(&mut cx.iadt, Some((strip - strip_t) / strips));
            strip_t = strip;
            let mut cur = 0;
            let mut first = true;
            while k < rows.len() && rows[k].0 == strip {
                let (_, curt, before, after, idx) = rows[k];
                let it = &insts[idx];
                if first {
                    e.int(&mut cx.iafs, Some(before - first_s));
                    first_s = before;
                    first = false;
                } else {
                    e.int(&mut cx.iads, Some(before - cur - o.ds_offset));
                }
                if strips > 1 {
                    e.int(&mut cx.iait, Some(curt));
                }
                e.iaid(&mut cx.iaid, id_bits, it.id as u32);
                if o.refine {
                    e.int(&mut cx.iari, Some(i64::from(it.refine.is_some())));
                    if let Some((target, rdx, rdy)) = &it.refine {
                        let base = &glyphs[it.id];
                        let rdw = i64::from(target.width) - i64::from(base.width);
                        let rdh = i64::from(target.height) - i64::from(base.height);
                        e.int(&mut cx.iardw, Some(rdw));
                        e.int(&mut cx.iardh, Some(rdh));
                        e.int(&mut cx.iardx, Some(*rdx));
                        e.int(&mut cx.iardy, Some(*rdy));
                        let (dx, dy) = (rdw.div_euclid(2) + rdx, rdh.div_euclid(2) + rdy);
                        encode_refinement(
                            e,
                            &mut cx.gr,
                            target,
                            o.rtemplate,
                            base,
                            dx,
                            dy,
                            false,
                            &[(-1, -1), (-1, -1)],
                        );
                    }
                }
                cur = after;
                k += 1;
            }
            e.int(&mut cx.iads, None);
        }
    }

    fn text_flags(o: &TextOpts, huffman: bool) -> u16 {
        let ds = (o.ds_offset & 0x1f) as u16;
        u16::from(huffman)
            | u16::from(o.refine) << 1
            | (o.log_strips as u16) << 2
            | u16::from(o.corner) << 4
            | if o.transposed { 0x40 } else { 0 }
            | u16::from(o.op) << 7
            | u16::from(o.default_pixel) << 9
            | ds << 10
            | u16::from(o.rtemplate) << 15
    }

    fn arith_text_region(insts: &[Instance], glyphs: &[Bitmap], w: u32, h: u32, o: &TextOpts) -> Vec<u8> {
        let mut e = MqEnc::new();
        let mut cx = Contexts::new();
        code_text_arith(&mut e, &mut cx, insts, glyphs, ceil_log2(glyphs.len() as u64), o);
        let mut d = region(w, h, 0, 0, 0);
        d.extend_from_slice(&text_flags(o, false).to_be_bytes());
        if o.refine && o.rtemplate == 0 {
            d.extend(at_bytes(&[(-1, -1), (-1, -1)]));
        }
        d.extend_from_slice(&(insts.len() as u32).to_be_bytes());
        d.extend(e.finish());
        d
    }

    fn render(insts: &[Instance], glyphs: &[Bitmap], w: u32, h: u32, op: u8, default_pixel: u8) -> Bitmap {
        let mut bm = Bitmap::filled(w, h, default_pixel).unwrap();
        for it in insts {
            combine(&mut bm, it.bitmap(glyphs), it.x, it.y, op);
        }
        bm
    }

    fn glyph_set() -> Vec<Bitmap> {
        let mut g = Vec::new();
        for (i, &(w, h)) in
            [(5u32, 7u32), (9, 7), (12, 7), (3, 11), (8, 11), (14, 15), (6, 15), (1, 15)].iter().enumerate()
        {
            g.push(glyph(w, h, 100 + i as u32));
        }
        g
    }

    fn instances(glyphs: &[Bitmap], seed: u32, w: u32, h: u32, n: usize) -> Vec<Instance> {
        let mut r = Rng(seed);
        (0..n)
            .map(|_| {
                let id = r.below(glyphs.len() as u32) as usize;
                Instance::plain(id, i64::from(r.below(w + 10)) - 8, i64::from(r.below(h + 10)) - 8)
            })
            .collect()
    }

    const NOMINAL_AT: [(i32, i32); 4] = [(3, -1), (-3, -1), (2, -2), (-2, -2)];

    #[test]
    fn symbol_dictionary_and_text_regions() {
        let glyphs = glyph_set();
        let (w, h) = (120u32, 80u32);
        let dict = arith_dictionary(&glyphs, 0, &NOMINAL_AT);
        let mut n = 0;
        for corner in 0..4u8 {
            for transposed in [false, true] {
                for (log_strips, op, ds_offset, default_pixel) in
                    [(0u32, 0u8, 0i64, 0u8), (2, 2, -3, 0), (1, 0, 5, 1), (0, 3, 0, 0), (1, 1, 2, 1)]
                {
                    n += 1;
                    let insts = instances(&glyphs, n, w, h, 25);
                    let o = TextOpts { log_strips, corner, transposed, op, ds_offset, default_pixel, ..PLAIN };
                    let text = arith_text_region(&insts, &glyphs, w, h, &o);
                    // Alternate between an inline dictionary and one in the globals stream.
                    let globals = (n % 2 == 0).then(|| segment(1, 0, &[], 0, &dict));
                    let mut stream = segment(0, 48, &[], 1, &page_info(w, h, 0, 0));
                    if globals.is_none() {
                        stream.extend(segment(1, 0, &[], 1, &dict));
                    }
                    stream.extend(segment(2, 6, &[1], 1, &text));
                    stream.extend(segment(3, 49, &[], 1, &[]));
                    let want = render(&insts, &glyphs, w, h, op, default_pixel);
                    let what =
                        format!("corner {corner} transposed {transposed} strips {log_strips} op {op} ds {ds_offset}");
                    check(&stream, globals.as_deref(), &want, &what);
                }
            }
        }
        // Other generic templates for the symbols.
        for (template, at) in [(1u8, &[(3, -1)][..]), (2, &[(2, -1)][..]), (3, &[(-2, -1)][..])] {
            let dict = arith_dictionary(&glyphs, template, at);
            let insts = instances(&glyphs, 50 + u32::from(template), w, h, 30);
            let mut stream = segment(0, 48, &[], 1, &page_info(w, h, 0, 0));
            stream.extend(segment(1, 0, &[], 1, &dict));
            stream.extend(segment(2, 6, &[1], 1, &arith_text_region(&insts, &glyphs, w, h, &PLAIN)));
            check(&stream, None, &render(&insts, &glyphs, w, h, 0, 0), &format!("symbol template {template}"));
        }
    }

    fn flip(bm: &Bitmap, pts: &[(i64, i64)]) -> Bitmap {
        let mut b = bm.clone();
        for &(x, y) in pts {
            b.put(x, y, 1 - b.get(x, y));
        }
        b
    }

    #[test]
    fn refinement_and_aggregate_symbols() {
        let base = glyph_set();
        let (a, b) = (&base[0], &base[1]); // 5x7, 9x7
        let rat = [(-1, -1), (-1, -1)];
        // Dictionary 1: a and b, direct coded. Dictionary 2 (refinement/aggregate) refers to it.
        let dict1 = arith_dictionary(&[a.clone(), b.clone()], 0, &NOMINAL_AT);
        let refined = flip(a, &[(2, 3), (4, 0)]);
        let mut agg = Bitmap::new(14, 7).unwrap();
        combine(&mut agg, a, 0, 0, 0);
        combine(&mut agg, b, 5, 0, 0);
        // A refinement of a symbol decoded earlier in the same dictionary, one column wider and shifted.
        let mut refined2 = Bitmap::new(6, 7).unwrap();
        combine(&mut refined2, &refined, 1, 0, 0);
        let refined2 = flip(&refined2, &[(0, 6)]);
        let all = vec![a.clone(), b.clone(), refined.clone(), agg.clone(), refined2.clone()];
        for rtemplate in [0u8, 1] {
            let mut e = MqEnc::new();
            let mut cx = Contexts::new();
            let id_bits = ceil_log2(5);
            ensure(&mut cx.iaid, 2 << id_bits);
            e.int(&mut cx.iadh, Some(7));
            e.int(&mut cx.iadw, Some(5));
            e.int(&mut cx.iaai, Some(1));
            e.iaid(&mut cx.iaid, id_bits, 0);
            e.int(&mut cx.iardx, Some(0));
            e.int(&mut cx.iardy, Some(0));
            encode_refinement(&mut e, &mut cx.gr, &refined, rtemplate, a, 0, 0, false, &rat);
            e.int(&mut cx.iadw, Some(9));
            e.int(&mut cx.iaai, Some(2));
            let parts = [Instance::plain(0, 0, 0), Instance::plain(1, 5, 0)];
            let o = TextOpts { refine: true, rtemplate, ..PLAIN };
            code_text_arith(&mut e, &mut cx, &parts, &all, id_bits, &o);
            e.int(&mut cx.iadw, Some(-8));
            e.int(&mut cx.iaai, Some(1));
            e.iaid(&mut cx.iaid, id_bits, 2);
            e.int(&mut cx.iardx, Some(1));
            e.int(&mut cx.iardy, Some(0));
            encode_refinement(&mut e, &mut cx.gr, &refined2, rtemplate, &refined, 1, 0, false, &rat);
            e.int(&mut cx.iadw, None);
            e.int(&mut cx.iaex, Some(0));
            e.int(&mut cx.iaex, Some(5));
            let mut dict2 = (2u16 | u16::from(rtemplate) << 12).to_be_bytes().to_vec();
            dict2.extend(at_bytes(&NOMINAL_AT));
            if rtemplate == 0 {
                dict2.extend(at_bytes(&rat));
            }
            dict2.extend_from_slice(&5u32.to_be_bytes());
            dict2.extend_from_slice(&3u32.to_be_bytes());
            dict2.extend(e.finish());

            // A text region over dictionary 2 with refined instances.
            let (w, h) = (90u32, 40u32);
            let mut insts = instances(&all, 77 + u32::from(rtemplate), w, h, 16);
            insts[3].refine = Some((flip(&base[1], &[(0, 0), (8, 6)]), 0, 0));
            insts[3].id = 1;
            let mut wider = Bitmap::new(11, 8).unwrap();
            combine(&mut wider, &all[3], 1, 1, 0);
            insts[7].refine = Some((flip(&wider, &[(10, 7), (3, 3)]), 0, 0));
            insts[7].id = 3;
            let o = TextOpts { refine: true, rtemplate, ..PLAIN };
            let text = arith_text_region(&insts, &all, w, h, &o);
            let mut stream = segment(0, 48, &[], 1, &page_info(w, h, 0, 0));
            stream.extend(segment(1, 0, &[], 1, &dict1));
            stream.extend(segment(2, 0, &[1], 1, &dict2));
            stream.extend(segment(3, 6, &[2], 1, &text));
            stream.extend(segment(4, 49, &[], 1, &[]));
            check(&stream, None, &render(&insts, &all, w, h, 0, 0), &format!("refinement template {rtemplate}"));
        }
    }

    /// A Huffman symbol dictionary with standard tables; heights must rise and widths must not fall within a
    /// height class (B.4 and B.2 have no negative values).
    fn huffman_dictionary(glyphs: &[Bitmap], mmr: bool) -> Vec<u8> {
        let (b1, b2, b4) = (standard_table(1), standard_table(2), standard_table(4));
        let mut w = BitW::default();
        let mut h = 0i64;
        let mut i = 0;
        while i < glyphs.len() {
            let hc = glyphs[i].height;
            w.huff(&b4, Some(i64::from(hc) - h));
            h = i64::from(hc);
            let start = i;
            let mut sw = 0i64;
            while i < glyphs.len() && glyphs[i].height == hc {
                w.huff(&b2, Some(i64::from(glyphs[i].width) - sw));
                sw = i64::from(glyphs[i].width);
                i += 1;
            }
            w.huff(&b2, None);
            let total: u32 = glyphs[start..i].iter().map(|g| g.width).sum();
            let mut collective = Bitmap::new(total, hc).unwrap();
            let mut x = 0;
            for g in &glyphs[start..i] {
                combine(&mut collective, g, x, 0, 0);
                x += i64::from(g.width);
            }
            if mmr {
                let data = super::super::ccitt::tests::encode_mmr(&collective.data, total as usize, hc as usize);
                w.huff(&b1, Some(data.len() as i64));
                w.bytes(&data);
            } else {
                w.huff(&b1, Some(0));
                w.bytes(&collective.data);
            }
        }
        w.huff(&b1, Some(0));
        w.huff(&b1, Some(glyphs.len() as i64));
        let mut d = 1u16.to_be_bytes().to_vec();
        d.extend_from_slice(&(glyphs.len() as u32).to_be_bytes());
        d.extend_from_slice(&(glyphs.len() as u32).to_be_bytes());
        d.extend(w.out);
        d
    }

    /// A Huffman text region with B.6/B.8/B.11 (or a custom DS table) and B.15/B.1 for refinements.
    fn huffman_text_region(
        insts: &[Instance],
        glyphs: &[Bitmap],
        w: u32,
        h: u32,
        o: &TextOpts,
        custom_ds: Option<&HuffTable>,
    ) -> Vec<u8> {
        let b8 = standard_table(8);
        let ds_table: &HuffTable = custom_ds.unwrap_or(&b8);
        let mut bw = BitW::default();
        let mut gr = Vec::new();
        // Symbol ID table: every symbol gets an L-bit code; the run-code table has just RUNCODE[L], coded "0".
        let l = ceil_log2(glyphs.len() as u64).max(1);
        for i in 0..35 {
            bw.put(u64::from(i == l), 4);
        }
        for _ in glyphs {
            bw.put(0, 1);
        }
        bw.align();
        code_text_huffman(&mut bw, &mut gr, insts, glyphs, l, o, ds_table);
        let mut d = region(w, h, 0, 0, 0);
        d.extend_from_slice(&text_flags(o, true).to_be_bytes());
        // FS B.6, DS B.8 or custom, DT B.11, RDW/RDH/RDX/RDY B.15, RSIZE B.1.
        let hflags: u16 = if custom_ds.is_some() { 3 << 2 } else { 0 } | 1 << 6 | 1 << 8 | 1 << 10 | 1 << 12;
        d.extend_from_slice(&hflags.to_be_bytes());
        if o.refine && o.rtemplate == 0 {
            d.extend(at_bytes(&[(-1, -1), (-1, -1)]));
        }
        d.extend_from_slice(&(insts.len() as u32).to_be_bytes());
        d.extend(bw.out);
        d
    }

    /// Codes a Huffman text region's instances (from the first DT on) into `bw`, symbol IDs as `l`-bit codes;
    /// also how aggregate symbols of a Huffman dictionary are coded.
    #[allow(clippy::too_many_arguments)]
    fn code_text_huffman(
        bw: &mut BitW,
        gr: &mut Vec<u8>,
        insts: &[Instance],
        glyphs: &[Bitmap],
        l: u32,
        o: &TextOpts,
        ds_table: &HuffTable,
    ) {
        let (b1, b6, b11, b15) = (standard_table(1), standard_table(6), standard_table(11), standard_table(15));
        let strips = 1i64 << o.log_strips;
        let rows = layout(insts, glyphs, strips, o.corner, o.transposed);
        bw.huff(&b11, Some(1));
        let mut strip_t = -strips;
        let mut first_s = 0i64;
        let mut k = 0;
        while k < rows.len() {
            let strip = rows[k].0;
            bw.huff(&b11, Some((strip - strip_t) / strips));
            strip_t = strip;
            let mut cur = 0;
            let mut first = true;
            while k < rows.len() && rows[k].0 == strip {
                let (_, curt, before, after, idx) = rows[k];
                let it = &insts[idx];
                if first {
                    bw.huff(&b6, Some(before - first_s));
                    first_s = before;
                    first = false;
                } else {
                    bw.huff(ds_table, Some(before - cur - o.ds_offset));
                }
                bw.put(curt as u64, o.log_strips);
                bw.put(it.id as u64, l);
                if o.refine {
                    bw.put(u64::from(it.refine.is_some()), 1);
                    if let Some((target, rdx, rdy)) = &it.refine {
                        let base = &glyphs[it.id];
                        let rdw = i64::from(target.width) - i64::from(base.width);
                        let rdh = i64::from(target.height) - i64::from(base.height);
                        for v in [rdw, rdh, *rdx, *rdy] {
                            bw.huff(&b15, Some(v));
                        }
                        let mut e = MqEnc::new();
                        let (dx, dy) = (rdw.div_euclid(2) + rdx, rdh.div_euclid(2) + rdy);
                        // Each refinement gets its own MQ block, but the contexts carry over (as in jbig2dec).
                        encode_refinement(&mut e, gr, target, o.rtemplate, base, dx, dy, false, &[(-1, -1), (-1, -1)]);
                        let data = e.finish();
                        bw.huff(&b1, Some(data.len() as i64));
                        bw.bytes(&data);
                    }
                }
                cur = after;
                k += 1;
            }
            bw.huff(ds_table, None);
        }
    }

    /// Code table segment: HTLOW -8, HTHIGH 24, with OOB.
    fn custom_table() -> (Vec<u8>, HuffTable) {
        let mut bw = BitW::default();
        // Normal lines [-8,-4) [-4,0) [0,8) [8,24) with prefix lengths 2,2,2,4; lower, upper, OOB 4 each.
        for (prefix, range) in [(2u64, 2u64), (2, 2), (2, 3), (4, 4)] {
            bw.put(prefix, 3);
            bw.put(range, 3);
        }
        for _ in 0..3 {
            bw.put(4, 3);
        }
        let mut d = vec![1 | (2 << 1) | (2 << 4)];
        d.extend_from_slice(&(-8i32).to_be_bytes());
        d.extend_from_slice(&24i32.to_be_bytes());
        d.extend(bw.out);
        let t = parse_table_segment(&d).unwrap();
        (d, t)
    }

    #[test]
    fn huffman_dictionaries_and_text_regions() {
        let mut glyphs = glyph_set();
        glyphs.sort_by_key(|g| (g.height, g.width));
        let (w, h) = (130u32, 70u32);
        let (table_seg, table) = custom_table();
        let mut n = 0;
        for mmr in [false, true] {
            for (corner, transposed, log_strips, custom) in
                [(1u8, false, 0u32, false), (0, true, 1, false), (3, false, 2, true), (2, true, 0, true)]
            {
                n += 1;
                let mut r = Rng(n);
                // B.11 has no negative strip deltas, so keep instances at non-negative coordinates.
                let insts: Vec<Instance> = (0..20)
                    .map(|_| {
                        Instance::plain(r.below(8) as usize, i64::from(r.below(w - 10)), i64::from(r.below(h - 10)))
                    })
                    .collect();
                let o = TextOpts { log_strips, corner, transposed, ..PLAIN };
                let text = huffman_text_region(&insts, &glyphs, w, h, &o, custom.then_some(&table));
                let mut stream = segment(0, 48, &[], 1, &page_info(w, h, 0, 0));
                stream.extend(segment(1, 0, &[], 1, &huffman_dictionary(&glyphs, mmr)));
                stream.extend(segment(2, 53, &[], 1, &table_seg));
                stream.extend(segment(3, 7, if custom { &[1, 2] } else { &[1] }, 1, &text));
                let want = render(&insts, &glyphs, w, h, 0, 0);
                check(
                    &stream,
                    None,
                    &want,
                    &format!("Huffman mmr {mmr} corner {corner} transposed {transposed} custom {custom}"),
                );
            }
        }
        // Huffman text region with refined instances (refinement data is an embedded MQ block).
        let mut insts: Vec<Instance> = (0..6).map(|i| Instance::plain(i, 10 + 18 * i as i64, 20)).collect();
        insts[2].refine = Some((flip(&glyphs[2], &[(1, 1), (0, 6)]), 0, 0));
        let mut wide = Bitmap::new(glyphs[4].width + 2, glyphs[4].height + 1).unwrap();
        combine(&mut wide, &glyphs[4], 1, 0, 0);
        insts[4].refine = Some((flip(&wide, &[(0, 0)]), 0, 1));
        for rtemplate in [0u8, 1] {
            let o = TextOpts { refine: true, rtemplate, ..PLAIN };
            let text = huffman_text_region(&insts, &glyphs, w, h, &o, None);
            let mut stream = segment(0, 48, &[], 1, &page_info(w, h, 0, 0));
            stream.extend(segment(1, 0, &[], 1, &huffman_dictionary(&glyphs, false)));
            stream.extend(segment(2, 6, &[1], 1, &text));
            let want = render(&insts, &glyphs, w, h, 0, 0);
            let what = format!("Huffman refinement template {rtemplate}");
            // jbig2dec is no oracle here: it skips the refinement AT field of Huffman text regions, which
            // T.88 7.4.3.1.3, pdf.js and pdfium read, and it misreads the embedded refinement blocks
            // (byte-aligned, RSIZE bytes of MQ data, as pdfium reads them) with either alignment.
            check_ours(&stream, None, &want, &what);
        }
    }

    #[test]
    fn huffman_refinement_and_aggregate_dictionary() {
        let base = glyph_set();
        let (a, b) = (&base[0], &base[1]);
        let rat = [(-1, -1), (-1, -1)];
        let dict1 = arith_dictionary(&[a.clone(), b.clone()], 0, &NOMINAL_AT);
        let refined = flip(a, &[(2, 3), (4, 0)]);
        let mut refined2 = Bitmap::new(6, 7).unwrap();
        combine(&mut refined2, &refined, 1, 0, 0);
        let refined2 = flip(&refined2, &[(0, 6)]);
        let mut agg = Bitmap::new(14, 7).unwrap();
        combine(&mut agg, a, 0, 0, 0);
        combine(&mut agg, b, 5, 0, 0);
        // Decode order: input a, b, then the new symbols; B.2 has no negative widths, so widths rise.
        let all = vec![a.clone(), b.clone(), refined.clone(), refined2.clone(), agg.clone()];
        let (b1, b2, b4, b8, b15) =
            (standard_table(1), standard_table(2), standard_table(4), standard_table(8), standard_table(15));
        for rtemplate in [0u8, 1] {
            let mut bw = BitW::default();
            let mut gr = Vec::new();
            let id_bits = 3; // SBSYMCODELEN for 5 symbols
            let refine_block =
                |bw: &mut BitW, gr: &mut Vec<u8>, target: &Bitmap, id: u64, reference: &Bitmap, rdx: i64| {
                    bw.huff(&b1, Some(1)); // REFAGGNINST
                    bw.put(id, id_bits);
                    bw.huff(&b15, Some(rdx));
                    bw.huff(&b15, Some(0));
                    let mut e = MqEnc::new();
                    encode_refinement(&mut e, gr, target, rtemplate, reference, rdx, 0, false, &rat);
                    let data = e.finish();
                    bw.huff(&b1, Some(data.len() as i64));
                    bw.bytes(&data);
                };
            bw.huff(&b4, Some(7));
            bw.huff(&b2, Some(5));
            refine_block(&mut bw, &mut gr, &refined, 0, a, 0);
            bw.huff(&b2, Some(1));
            refine_block(&mut bw, &mut gr, &refined2, 2, &refined, 1);
            bw.huff(&b2, Some(8));
            bw.huff(&b1, Some(2));
            let parts = [Instance::plain(0, 0, 0), Instance::plain(1, 5, 0)];
            code_text_huffman(
                &mut bw,
                &mut gr,
                &parts,
                &all,
                id_bits,
                &TextOpts { refine: true, rtemplate, ..PLAIN },
                &b8,
            );
            bw.huff(&b2, None);
            bw.huff(&b1, Some(0));
            bw.huff(&b1, Some(5));
            let mut dict2 = (3u16 | u16::from(rtemplate) << 12).to_be_bytes().to_vec();
            if rtemplate == 0 {
                dict2.extend(at_bytes(&rat));
            }
            dict2.extend_from_slice(&5u32.to_be_bytes());
            dict2.extend_from_slice(&3u32.to_be_bytes());
            dict2.extend(bw.out);

            let (w, h) = (90u32, 40u32);
            let insts = instances(&all, 5 + u32::from(rtemplate), w, h, 15);
            let mut stream = segment(0, 48, &[], 1, &page_info(w, h, 0, 0));
            stream.extend(segment(1, 0, &[], 1, &dict1));
            stream.extend(segment(2, 0, &[1], 1, &dict2));
            stream.extend(segment(3, 6, &[2], 1, &arith_text_region(&insts, &all, w, h, &PLAIN)));
            // jbig2dec's Huffman refinement paths fail on these streams (see the text region test).
            let want = render(&insts, &all, w, h, 0, 0);
            check_ours(&stream, None, &want, &format!("Huffman refinement dictionary, template {rtemplate}"));
        }
    }

    #[test]
    fn halftone_regions() {
        // Eight 4x4 patterns of rising density.
        let (pw, ph) = (4u32, 4u32);
        let patterns: Vec<Bitmap> = (0..8)
            .map(|g| {
                let mut p = Bitmap::new(pw, ph).unwrap();
                for k in 0..(g * 2) {
                    p.put((k * 3 % 4) as i64, (k / 2 % 4) as i64, 1);
                }
                p
            })
            .collect();
        let mut collective = Bitmap::new(pw * 8, ph).unwrap();
        for (i, p) in patterns.iter().enumerate() {
            combine(&mut collective, p, i64::from(pw) * i as i64, 0, 0);
        }
        let (rw, rh) = (40u32, 26u32);
        let (gw, gh) = (12u32, 8u32);
        let mut rng = Rng(3);
        let gray: Vec<u32> = (0..gw * gh).map(|_| rng.below(8)).collect();
        for (case, &(mmr, template, skip, op, gx, gy, hrx, hry)) in [
            (false, 0u8, false, 0u8, -512i32, -256i32, 1024u16, 0u16),
            (true, 0, false, 0, 0, 0, 1024, 0),
            (false, 1, true, 2, -700, -300, 1024, 0),
            (false, 2, false, 0, 100, 50, 900, 120),
            (false, 3, true, 0, -2000, 0, 1024, 64),
            (true, 0, false, 4, 256, 256, 1024, 0),
        ]
        .iter()
        .enumerate()
        {
            // Pattern dictionary.
            let mut pd = vec![u8::from(mmr) | template << 1, pw as u8, ph as u8];
            pd.extend_from_slice(&7u32.to_be_bytes());
            if mmr {
                pd.extend(super::super::ccitt::tests::encode_mmr(&collective.data, (pw * 8) as usize, ph as usize));
            } else {
                let mut e = MqEnc::new();
                let at = [(-(pw as i32), 0), (-3, -1), (2, -2), (-2, -2)];
                encode_generic(&mut e, &mut Vec::new(), &collective, template, false, &at, None);
                pd.extend(e.finish());
            }
            // Gray-coded bit planes, most significant first.
            let cell = |m: i64, n: i64| {
                (
                    (i64::from(gx) + m * i64::from(hry) + n * i64::from(hrx)) >> 8,
                    (i64::from(gy) + m * i64::from(hrx) - n * i64::from(hry)) >> 8,
                )
            };
            let mut skipmap = Bitmap::new(gw, gh).unwrap();
            for m in 0..i64::from(gh) {
                for n in 0..i64::from(gw) {
                    let (x, y) = cell(m, n);
                    if skip
                        && (x + i64::from(pw) <= 0
                            || x >= i64::from(rw)
                            || y + i64::from(ph) <= 0
                            || y >= i64::from(rh))
                    {
                        skipmap.put(n, m, 1);
                    }
                }
            }
            let mut planes = Vec::new();
            for j in (0..3).rev() {
                let mut plane = Bitmap::new(gw, gh).unwrap();
                for m in 0..gh {
                    for n in 0..gw {
                        let v =
                            if skipmap.get(i64::from(n), i64::from(m)) != 0 { 0 } else { gray[(m * gw + n) as usize] };
                        let g = v ^ (v >> 1);
                        plane.put(i64::from(n), i64::from(m), (g >> j) & 1);
                    }
                }
                planes.push(plane);
            }
            let mut body = Vec::new();
            if mmr {
                for plane in &planes {
                    body.extend(super::super::ccitt::tests::encode_mmr(&plane.data, gw as usize, gh as usize));
                }
            } else {
                let mut e = MqEnc::new();
                let mut cx = Vec::new();
                let at = [(if template <= 1 { 3 } else { 2 }, -1), (-3, -1), (2, -2), (-2, -2)];
                for plane in &planes {
                    encode_generic(&mut e, &mut cx, plane, template, false, &at, skip.then_some(&skipmap));
                }
                body.extend(e.finish());
            }
            let mut hd = region(rw, rh, 0, 0, 0);
            hd.push(u8::from(mmr) | template << 1 | u8::from(skip) << 3 | op << 4);
            for v in [gw, gh, gx as u32, gy as u32] {
                hd.extend_from_slice(&v.to_be_bytes());
            }
            hd.extend_from_slice(&hrx.to_be_bytes());
            hd.extend_from_slice(&hry.to_be_bytes());
            hd.extend(body);
            let mut want = Bitmap::new(rw, rh).unwrap();
            for m in 0..i64::from(gh) {
                for n in 0..i64::from(gw) {
                    let (x, y) = cell(m, n);
                    combine(&mut want, &patterns[gray[(m * i64::from(gw) + n) as usize] as usize], x, y, op);
                }
            }
            let mut stream = segment(0, 48, &[], 1, &page_info(rw, rh, 0, 0));
            stream.extend(segment(1, 16, &[], 1, &pd));
            stream.extend(segment(2, 22, &[1], 1, &hd));
            stream.extend(segment(3, 49, &[], 1, &[]));
            check(&stream, None, &want, &format!("halftone case {case}"));
        }
    }

    #[test]
    fn striped_page_of_unknown_height() {
        for default_pixel in [0u8, 1] {
            let top = sample(64, 16, 1);
            let bottom = sample(50, 10, 2);
            let mut stream = segment(0, 48, &[], 1, &page_info(64, 0xffff_ffff, default_pixel << 2 | 0x40, 0x8010));
            stream.extend(segment(1, 38, &[], 1, &generic_segment(&top, 0, 0, 4, 0, true, &NOMINAL_AT)));
            stream.extend(segment(2, 50, &[], 1, &15u32.to_be_bytes()));
            stream.extend(segment(3, 38, &[], 1, &generic_segment(&bottom, 7, 16, 4, 1, false, &[(3, -1)])));
            stream.extend(segment(4, 50, &[], 1, &29u32.to_be_bytes()));
            stream.extend(segment(5, 49, &[], 1, &[]));
            let mut want = Bitmap::filled(64, 30, default_pixel).unwrap();
            combine(&mut want, &top, 0, 0, 4);
            combine(&mut want, &bottom, 7, 16, 4);
            // T.88 7.4.10 (and pdfium) grow the page to the last end row + 1; jbig2dec stops at the last region,
            // so it is compared only over the rows it renders.
            check_ours(&stream, None, &want, &format!("striped page, default pixel {default_pixel}"));
            if let Some(reference) = jbig2dec(&stream, None) {
                let rows = want.extract(0, 0, 64, reference.height).unwrap();
                assert_same(&reference, &rows, "striped page (jbig2dec)");
            }
        }
    }

    #[test]
    fn refinement_regions() {
        let (w, h) = (80u32, 50u32);
        let page = sample(w, h, 31);
        let at = [(-1, -1), (-1, -1)];
        for (template, tpgron) in [(0u8, false), (0, true), (1, false), (1, true)] {
            // Immediate refinement of a page area, composed with REPLACE. With no referred-to segment the
            // reference is the page area under the region (T.88 7.4.7.5); jbig2dec and pdfium use the page
            // from its origin instead, which only agrees when the region starts at (0, 0).
            for (rx, ry, rw, rh) in [(0u32, 0u32, w, h), (8, 5, 40, 30)] {
                let reference = page.extract(i64::from(rx), i64::from(ry), rw, rh).unwrap();
                let corner = (i64::from(rw) - 1, i64::from(rh) - 1);
                let target = flip(&reference, &[(0, 0), (5, 5), corner, (20, 10), (21, 10), (3, 17)]);
                let mut e = MqEnc::new();
                encode_refinement(&mut e, &mut Vec::new(), &target, template, &reference, 0, 0, tpgron, &at);
                let mut d = region(rw, rh, rx, ry, 4);
                d.push(template | u8::from(tpgron) << 1);
                if template == 0 {
                    d.extend(at_bytes(&at));
                }
                d.extend(e.finish());
                let mut stream = segment(0, 48, &[], 1, &page_info(w, h, 0x40, 0));
                stream.extend(segment(1, 38, &[], 1, &generic_segment(&page, 0, 0, 0, 0, false, &NOMINAL_AT)));
                stream.extend(segment(2, 42, &[], 1, &d));
                let mut want = page.clone();
                combine(&mut want, &target, i64::from(rx), i64::from(ry), 4);
                let what = format!("page refinement at ({rx}, {ry}) template {template} tpgron {tpgron}");
                if rx == 0 && ry == 0 {
                    check(&stream, None, &want, &what);
                } else {
                    check_ours(&stream, None, &want, &what);
                }
            }

            // Intermediate generic region refined by a segment that refers to it.
            let inter = sample(30, 20, 32);
            let target = flip(&inter, &[(1, 1), (29, 19), (15, 0)]);
            let mut e = MqEnc::new();
            encode_refinement(&mut e, &mut Vec::new(), &target, template, &inter, 0, 0, tpgron, &at);
            let mut d = region(30, 20, 40, 25, 0);
            d.push(template | u8::from(tpgron) << 1);
            if template == 0 {
                d.extend(at_bytes(&at));
            }
            d.extend(e.finish());
            let mut stream = segment(0, 48, &[], 1, &page_info(w, h, 0, 0));
            stream.extend(segment(1, 36, &[], 1, &generic_segment(&inter, 40, 25, 0, 0, false, &NOMINAL_AT)));
            stream.extend(segment(2, 42, &[1], 1, &d));
            let mut want = Bitmap::new(w, h).unwrap();
            combine(&mut want, &target, 40, 25, 0);
            // jbig2dec does not implement intermediate generic regions.
            check_ours(&stream, None, &want, &format!("intermediate refinement template {template} tpgron {tpgron}"));
        }
    }

    /// A segment header with the 0xffffffff "unknown" data length.
    fn unknown_length_segment(number: u32, body: &[u8]) -> Vec<u8> {
        let mut v = segment(number, 38, &[], 1, &[]);
        let n = v.len();
        v[n - 4..].copy_from_slice(&0xffff_ffffu32.to_be_bytes());
        v.extend_from_slice(body);
        v
    }

    #[test]
    fn generic_region_of_unknown_length() {
        let bm = sample(70, 33, 41);
        // Arithmetic: the MQ data ends with 0xffac, then the row count.
        let mut body = generic_segment(&bm, 0, 0, 0, 0, true, &NOMINAL_AT);
        body.extend_from_slice(&33u32.to_be_bytes());
        let mut stream = segment(0, 48, &[], 1, &page_info(70, 40, 0, 0));
        stream.extend(unknown_length_segment(1, &body));
        stream.extend(segment(2, 49, &[], 1, &[]));
        let mut want = Bitmap::new(70, 40).unwrap();
        combine(&mut want, &bm, 0, 0, 0);
        assert_same(&decode(&stream, None).unwrap(), &want, "unknown length, arithmetic");
        // MMR: EOFB, then 0x0000 and the row count; the region info may overstate the height.
        let mut body = region(70, 0xffff_ffff, 0, 0, 0);
        body.push(1);
        body.extend(super::super::ccitt::tests::encode_mmr(&bm.data, 70, 33));
        body.extend_from_slice(&[0, 0]);
        body.extend_from_slice(&33u32.to_be_bytes());
        let mut stream = segment(0, 48, &[], 1, &page_info(70, 40, 0, 0));
        stream.extend(unknown_length_segment(1, &body));
        stream.extend(segment(2, 49, &[], 1, &[]));
        assert_same(&decode(&stream, None).unwrap(), &want, "unknown length, MMR");
    }

    #[test]
    fn unknown_segments_and_other_pages_are_skipped() {
        let bm = sample(40, 20, 51);
        let mut stream = segment(0, 48, &[], 1, &page_info(40, 20, 0, 0));
        stream.extend(segment(1, 62, &[], 1, b"\x20\x00\x00\x00comment"));
        stream.extend(segment(2, 33, &[], 1, &[0xde, 0xad, 0xbe, 0xef, 0xff, 0xac]));
        stream.extend(segment(3, 52, &[], 1, &[0; 12]));
        stream.extend(segment(4, 38, &[], 1, &generic_segment(&bm, 0, 0, 0, 0, false, &NOMINAL_AT)));
        // Another page's segments must not draw on page 1.
        stream.extend(segment(5, 38, &[], 2, &generic_segment(&sample(40, 20, 52), 0, 0, 2, 0, false, &NOMINAL_AT)));
        stream.extend(segment(300, 62, &[0], 1, &[1, 2, 3]));
        stream.extend(segment(301, 49, &[], 1, &[]));
        // Anything after the end of the page is ignored.
        stream.extend(segment(302, 38, &[], 1, &generic_segment(&sample(40, 20, 53), 0, 0, 2, 0, false, &NOMINAL_AT)));
        assert_same(&decode(&stream, None).unwrap(), &bm, "skipping");
        // Long-form referred-to segment list and 4-byte page association parse.
        let mut long = 70000u32.to_be_bytes().to_vec();
        long.extend_from_slice(&[0x40 | 62, 0xe0, 0, 0, 5, 0]);
        for r in 0..5u32 {
            long.extend_from_slice(&r.to_be_bytes());
        }
        long.extend_from_slice(&1u32.to_be_bytes());
        long.extend_from_slice(&2u32.to_be_bytes());
        long.extend_from_slice(&[9, 9]);
        let segs = parse_segments(&long);
        assert_eq!(segs.len(), 1);
        assert_eq!((segs[0].refs.len(), segs[0].page, segs[0].data), (5, 1, &[9u8, 9][..]));
    }

    #[test]
    fn hostile_streams_never_panic() {
        // A stream using most segment types.
        let glyphs = glyph_set();
        let (w, h) = (100u32, 60u32);
        let insts = instances(&glyphs, 9, w, h, 20);
        let mut stream = segment(0, 48, &[], 1, &page_info(w, h, 0, 0));
        stream.extend(segment(1, 0, &[], 1, &arith_dictionary(&glyphs, 0, &NOMINAL_AT)));
        stream.extend(segment(2, 6, &[1], 1, &arith_text_region(&insts, &glyphs, w, h, &PLAIN)));
        let mut sorted = glyphs.clone();
        sorted.sort_by_key(|g| (g.height, g.width));
        stream.extend(segment(3, 0, &[], 1, &huffman_dictionary(&sorted, true)));
        let hinsts: Vec<Instance> = (0..8).map(|i| Instance::plain(i, 5 + 11 * i as i64, 30)).collect();
        stream.extend(segment(4, 6, &[3], 1, &huffman_text_region(&hinsts, &sorted, w, h, &PLAIN, None)));
        stream.extend(segment(5, 38, &[], 1, &generic_segment(&sample(60, 30, 3), 20, 20, 2, 1, true, &[(3, -1)])));
        let full = decode(&stream, None).unwrap();
        for cut in 0..stream.len() {
            if let Ok(bm) = decode(&stream[..cut], None) {
                assert_eq!(bm.data.len(), (bm.stride * bm.height) as usize);
            }
        }
        // Truncating inside the last region keeps everything drawn before it.
        let partial = decode(&stream[..stream.len() - 20], None).unwrap();
        assert_eq!((partial.width, partial.height), (full.width, full.height));
        let mut r = Rng(17);
        for _ in 0..2000 {
            let mut bad = stream.clone();
            for _ in 0..1 + r.below(6) {
                let i = r.below(bad.len() as u32) as usize;
                bad[i] = r.next() as u8;
            }
            let _ = decode(&bad, None);
            let _ = decode(&bad[..r.below(bad.len() as u32) as usize], Some(&bad[..r.below(64) as usize]));
        }
        for len in [0usize, 1, 5, 11, 40, 400] {
            let junk: Vec<u8> = (0..len).map(|_| r.next() as u8).collect();
            assert!(decode(&junk, None).is_err() || len > 11);
            let _ = decode(&junk, Some(&junk));
        }
        // Absurd sizes are refused, not allocated.
        assert!(decode(&segment(0, 48, &[], 1, &page_info(70000, 10, 0, 0)), None).is_err());
        assert!(decode(&segment(0, 48, &[], 1, &page_info(65535, 65535, 0, 0)), None).is_err());
        let mut big = segment(0, 48, &[], 1, &page_info(100, 100, 0, 0));
        let mut d = region(65535, 65535, 0, 0, 0);
        d.extend_from_slice(&[0, 3, 0xff, 0xfd, 0xff, 2, 0xfe, 0xfe, 0xff]);
        big.extend(segment(1, 38, &[], 1, &d));
        let bm = decode(&big, None).unwrap();
        assert!(bm.data.iter().all(|&b| b == 0));
        // A text region claiming four billion instances over almost no data ends quickly.
        let mut d = region(50, 50, 0, 0, 0);
        d.extend_from_slice(&0u16.to_be_bytes());
        d.extend_from_slice(&u32::MAX.to_be_bytes());
        d.extend_from_slice(&[0x55; 16]);
        let mut s = segment(0, 48, &[], 1, &page_info(50, 50, 0, 0));
        s.extend(segment(1, 0, &[], 1, &arith_dictionary(&glyphs, 0, &NOMINAL_AT)));
        s.extend(segment(2, 6, &[1], 1, &d));
        let t = std::time::Instant::now();
        let _ = decode(&s, None);
        assert!(t.elapsed().as_secs() < 5);
        assert!(decode(&[], None).is_err());
    }

    /// `cargo test --release -- --ignored --nocapture jbig2::tests::timing`: a full scanned page.
    #[test]
    #[ignore]
    fn timing() {
        let (w, h) = (2480u32, 3508u32);
        let page = sample(w, h, 77);
        let mut stream = segment(0, 48, &[], 1, &page_info(w, h, 0, 0));
        stream.extend(segment(1, 38, &[], 1, &generic_segment(&page, 0, 0, 0, 0, true, &NOMINAL_AT)));
        // Thread CPU time (Linux), so a busy machine does not inflate the figure.
        let cpu = || {
            std::fs::read_to_string("/proc/thread-self/schedstat")
                .ok()
                .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
                .map(|ns| ns / 1e6)
        };
        let wall = std::time::Instant::now();
        let t0 = cpu();
        let got = decode(&stream, None).unwrap();
        let ms = match (t0, cpu()) {
            (Some(a), Some(b)) => b - a,
            _ => wall.elapsed().as_secs_f64() * 1000.0,
        };
        eprintln!("generic region {w}x{h} template 0 TPGDON ({} KiB): {ms:.1} ms", stream.len() / 1024);
        assert_same(&got, &page, "timing page");
    }
}
