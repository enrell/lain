//! CCITT fax decoding (ITU-T T.4 and T.6) as the PDF CCITTFaxDecode filter uses it, plus the bare T.6 (MMR)
//! decoder JBIG2 shares.
//!
//! A row is kept as its list of changing elements: the positions where the colour flips, starting from white.
//! Two-dimensional coding describes each row against the previous one in exactly those terms, so decoding never
//! touches individual pixels until a finished row is rendered into packed bits.

use super::CodecError;

const MAX_DIM: u32 = 65535;
const MAX_PIXELS: u64 = 1 << 28;

/// The CCITTFaxDecode parameters (PDF 32000-1, table 11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CcittParams {
    /// < 0: pure 2-D (Group 4); 0: pure 1-D (Group 3 MH); > 0: mixed 1-D/2-D with a tag bit per row.
    pub k: i32,
    pub columns: u32,
    /// Number of rows; 0 decodes until the data or an end-of-block code ends.
    pub rows: u32,
    pub end_of_line: bool,
    pub encoded_byte_align: bool,
    pub end_of_block: bool,
    pub black_is_1: bool,
    pub damaged_rows_before_error: u32,
}

impl Default for CcittParams {
    fn default() -> Self {
        CcittParams {
            k: 0,
            columns: 1728,
            rows: 0,
            end_of_line: false,
            encoded_byte_align: false,
            end_of_block: true,
            black_is_1: false,
            damaged_rows_before_error: 0,
        }
    }
}

/// Decodes a CCITTFaxDecode stream into packed 1-bit rows, MSB first, `ceil(columns/8)` bytes per row. Bits are
/// 0 for black unless `black_is_1`, the filter's own output convention. A damaged or truncated stream yields the
/// rows decoded so far; when `rows` is set the image is padded to that height with white rows.
pub fn decode(data: &[u8], p: &CcittParams) -> Result<Vec<u8>, CodecError> {
    check_size(p.columns, p.rows)?;
    let row_bytes = p.columns.div_ceil(8) as usize;
    let max_rows = if p.rows > 0 { p.rows } else { max_rows_for(p.columns) };
    let mut dec = Decoder::new(data, p.columns, p.k, p.end_of_line, p.encoded_byte_align);
    let mut out = Vec::new();
    dec.run(max_rows, p.damaged_rows_before_error, |changes| {
        let start = out.len();
        out.resize(start + row_bytes, 0);
        render_row(changes, p.columns, &mut out[start..]);
    });
    if p.rows > 0 {
        out.resize(row_bytes * p.rows as usize, 0);
    }
    if !p.black_is_1 {
        for b in &mut out {
            *b = !*b;
        }
    }
    Ok(out)
}

/// Decodes `rows` rows of `columns` pixels of pure T.6 (MMR) data, as JBIG2 generic regions use it: no EOLs,
/// no byte alignment, an optional EOFB. Returns packed rows (MSB first, 1 = black) and the number of bytes
/// consumed, rounded up to a whole byte. Rows the data does not reach are left white.
pub fn decode_mmr(data: &[u8], columns: u32, rows: u32) -> Result<(Vec<u8>, usize), CodecError> {
    check_size(columns, rows)?;
    let row_bytes = columns.div_ceil(8) as usize;
    let mut out = vec![0u8; row_bytes * rows as usize];
    let mut dec = Decoder::new(data, columns, -1, false, false);
    let mut y = 0usize;
    dec.run(rows, 0, |changes| {
        if let Some(row) = out.get_mut(y * row_bytes..(y + 1) * row_bytes) {
            render_row(changes, columns, row);
        }
        y += 1;
    });
    // Swallow a trailing EOFB so the caller can continue after this bitmap.
    if dec.bits.peek(24) == 0x00_1001 {
        dec.bits.skip(24);
    }
    let consumed = dec.bits.pos.div_ceil(8).min(data.len());
    Ok((out, consumed))
}

fn check_size(columns: u32, rows: u32) -> Result<(), CodecError> {
    if columns == 0 || columns > MAX_DIM || rows > MAX_DIM {
        return Err(CodecError(format!("ccitt: bad dimensions {columns}x{rows}")));
    }
    if u64::from(columns) * u64::from(rows) > MAX_PIXELS {
        return Err(CodecError(format!("ccitt: image too large ({columns}x{rows})")));
    }
    Ok(())
}

/// Row limit when the height is open-ended: the dimension cap and the pixel cap.
fn max_rows_for(columns: u32) -> u32 {
    (MAX_PIXELS / u64::from(columns.max(1))).min(u64::from(MAX_DIM)) as u32
}

/// Sets the black runs of a row given as changing elements.
fn render_row(changes: &[u32], columns: u32, row: &mut [u8]) {
    let mut k = 0;
    while k < changes.len() {
        let start = changes[k].min(columns);
        let end = changes.get(k + 1).copied().unwrap_or(columns).min(columns);
        fill_ones(row, start as usize, end as usize);
        k += 2;
    }
}

/// Sets bits `[start, end)` of a packed MSB-first row.
fn fill_ones(row: &mut [u8], start: usize, end: usize) {
    if start >= end {
        return;
    }
    let (first, last) = (start / 8, (end - 1) / 8);
    let head = 0xffu8 >> (start % 8);
    let tail = 0xffu8 << (7 - (end - 1) % 8);
    if first == last {
        if let Some(b) = row.get_mut(first) {
            *b |= head & tail;
        }
        return;
    }
    if let Some(b) = row.get_mut(first) {
        *b |= head;
    }
    for b in row.iter_mut().take(last).skip(first + 1) {
        *b = 0xff;
    }
    if let Some(b) = row.get_mut(last) {
        *b |= tail;
    }
}

/// MSB-first bit reader; bits past the end read as zero, which no code word consists of.
struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    end: usize,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Bits { data, pos: 0, end: data.len().saturating_mul(8) }
    }

    /// The next `n` (<= 24) bits, right-aligned.
    fn peek(&self, n: u32) -> u32 {
        let i = self.pos / 8;
        let mut w = 0u32;
        for k in 0..4 {
            w = (w << 8) | u32::from(self.data.get(i + k).copied().unwrap_or(0));
        }
        (w << (self.pos % 8)) >> (32 - n)
    }

    fn skip(&mut self, n: u32) {
        self.pos = self.pos.saturating_add(n as usize);
    }

    fn read(&mut self, n: u32) -> u32 {
        let v = self.peek(n);
        self.skip(n);
        v
    }

    fn align(&mut self) {
        self.pos = self.pos.div_ceil(8).saturating_mul(8);
    }

    fn at_end(&self) -> bool {
        self.pos >= self.end
    }

    fn overrun(&self) -> bool {
        self.pos > self.end
    }
}

const EOL: u32 = 1; // 0000 0000 0001 in a 12-bit peek

/// The 2-D mode codes (T.4 table 4).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Pass,
    Horizontal,
    Vertical(i8),
    /// Extension (uncompressed mode) or an EOL: both end a row here.
    Stop,
}

/// Mode lookup by the next 7 bits: (mode, code length).
fn read_mode(bits: &mut Bits) -> Mode {
    let w = bits.peek(7);
    let (mode, len) = if w & 0x40 != 0 {
        (Mode::Vertical(0), 1)
    } else if w & 0x20 != 0 {
        // 01x
        if w & 0x10 != 0 { (Mode::Vertical(1), 3) } else { (Mode::Vertical(-1), 3) }
    } else if w & 0x10 != 0 {
        (Mode::Horizontal, 3)
    } else if w & 0x08 != 0 {
        (Mode::Pass, 4)
    } else if w & 0x04 != 0 {
        // 00001x
        if w & 0x02 != 0 { (Mode::Vertical(2), 6) } else { (Mode::Vertical(-2), 6) }
    } else if w & 0x02 != 0 {
        // 000001x
        if w & 0x01 != 0 { (Mode::Vertical(3), 7) } else { (Mode::Vertical(-3), 7) }
    } else {
        return Mode::Stop;
    };
    bits.skip(len);
    mode
}

// Run-length code words (T.4 tables 2 and 3) as (code, length, run length).
#[rustfmt::skip]
const WHITE_CODES: &[(u16, u8, u16)] = &[
    (0x35, 8, 0), (0x07, 6, 1), (0x07, 4, 2), (0x08, 4, 3), (0x0b, 4, 4), (0x0c, 4, 5), (0x0e, 4, 6), (0x0f, 4, 7),
    (0x13, 5, 8), (0x14, 5, 9), (0x07, 5, 10), (0x08, 5, 11), (0x08, 6, 12), (0x03, 6, 13), (0x34, 6, 14),
    (0x35, 6, 15), (0x2a, 6, 16), (0x2b, 6, 17), (0x27, 7, 18), (0x0c, 7, 19), (0x08, 7, 20), (0x17, 7, 21),
    (0x03, 7, 22), (0x04, 7, 23), (0x28, 7, 24), (0x2b, 7, 25), (0x13, 7, 26), (0x24, 7, 27), (0x18, 7, 28),
    (0x02, 8, 29), (0x03, 8, 30), (0x1a, 8, 31), (0x1b, 8, 32), (0x12, 8, 33), (0x13, 8, 34), (0x14, 8, 35),
    (0x15, 8, 36), (0x16, 8, 37), (0x17, 8, 38), (0x28, 8, 39), (0x29, 8, 40), (0x2a, 8, 41), (0x2b, 8, 42),
    (0x2c, 8, 43), (0x2d, 8, 44), (0x04, 8, 45), (0x05, 8, 46), (0x0a, 8, 47), (0x0b, 8, 48), (0x52, 8, 49),
    (0x53, 8, 50), (0x54, 8, 51), (0x55, 8, 52), (0x24, 8, 53), (0x25, 8, 54), (0x58, 8, 55), (0x59, 8, 56),
    (0x5a, 8, 57), (0x5b, 8, 58), (0x4a, 8, 59), (0x4b, 8, 60), (0x32, 8, 61), (0x33, 8, 62), (0x34, 8, 63),
    (0x1b, 5, 64), (0x12, 5, 128), (0x17, 6, 192), (0x37, 7, 256), (0x36, 8, 320), (0x37, 8, 384),
    (0x64, 8, 448), (0x65, 8, 512), (0x68, 8, 576), (0x67, 8, 640), (0xcc, 9, 704), (0xcd, 9, 768),
    (0xd2, 9, 832), (0xd3, 9, 896), (0xd4, 9, 960), (0xd5, 9, 1024), (0xd6, 9, 1088), (0xd7, 9, 1152),
    (0xd8, 9, 1216), (0xd9, 9, 1280), (0xda, 9, 1344), (0xdb, 9, 1408), (0x98, 9, 1472), (0x99, 9, 1536),
    (0x9a, 9, 1600), (0x18, 6, 1664), (0x9b, 9, 1728),
];

#[rustfmt::skip]
const BLACK_CODES: &[(u16, u8, u16)] = &[
    (0x37, 10, 0), (0x02, 3, 1), (0x03, 2, 2), (0x02, 2, 3), (0x03, 3, 4), (0x03, 4, 5), (0x02, 4, 6),
    (0x03, 5, 7), (0x05, 6, 8), (0x04, 6, 9), (0x04, 7, 10), (0x05, 7, 11), (0x07, 7, 12), (0x04, 8, 13),
    (0x07, 8, 14), (0x18, 9, 15), (0x17, 10, 16), (0x18, 10, 17), (0x08, 10, 18), (0x67, 11, 19),
    (0x68, 11, 20), (0x6c, 11, 21), (0x37, 11, 22), (0x28, 11, 23), (0x17, 11, 24), (0x18, 11, 25),
    (0xca, 12, 26), (0xcb, 12, 27), (0xcc, 12, 28), (0xcd, 12, 29), (0x68, 12, 30), (0x69, 12, 31),
    (0x6a, 12, 32), (0x6b, 12, 33), (0xd2, 12, 34), (0xd3, 12, 35), (0xd4, 12, 36), (0xd5, 12, 37),
    (0xd6, 12, 38), (0xd7, 12, 39), (0x6c, 12, 40), (0x6d, 12, 41), (0xda, 12, 42), (0xdb, 12, 43),
    (0x54, 12, 44), (0x55, 12, 45), (0x56, 12, 46), (0x57, 12, 47), (0x64, 12, 48), (0x65, 12, 49),
    (0x52, 12, 50), (0x53, 12, 51), (0x24, 12, 52), (0x37, 12, 53), (0x38, 12, 54), (0x27, 12, 55),
    (0x28, 12, 56), (0x58, 12, 57), (0x59, 12, 58), (0x2b, 12, 59), (0x2c, 12, 60), (0x5a, 12, 61),
    (0x66, 12, 62), (0x67, 12, 63),
    (0x0f, 10, 64), (0xc8, 12, 128), (0xc9, 12, 192), (0x5b, 12, 256), (0x33, 12, 320), (0x34, 12, 384),
    (0x35, 12, 448), (0x6c, 13, 512), (0x6d, 13, 576), (0x4a, 13, 640), (0x4b, 13, 704), (0x4c, 13, 768),
    (0x4d, 13, 832), (0x72, 13, 896), (0x73, 13, 960), (0x74, 13, 1024), (0x75, 13, 1088), (0x76, 13, 1152),
    (0x77, 13, 1216), (0x52, 13, 1280), (0x53, 13, 1344), (0x54, 13, 1408), (0x55, 13, 1472),
    (0x5a, 13, 1536), (0x5b, 13, 1600), (0x64, 13, 1664), (0x65, 13, 1728),
];

/// Extended make-up codes shared by both colours (T.4 table 3a).
#[rustfmt::skip]
const EXT_MAKEUP: &[(u16, u8, u16)] = &[
    (0x08, 11, 1792), (0x0c, 11, 1856), (0x0d, 11, 1920), (0x12, 12, 1984), (0x13, 12, 2048), (0x14, 12, 2112),
    (0x15, 12, 2176), (0x16, 12, 2240), (0x17, 12, 2304), (0x1c, 12, 2368), (0x1d, 12, 2432), (0x1e, 12, 2496),
    (0x1f, 12, 2560),
];

const LOOKUP_BITS: u32 = 13;

/// Builds a direct lookup on the next 13 bits: `length << 16 | run`, 0 where no code matches. Overlapping codes
/// stop the build, so a typo in the tables above fails at compile time.
const fn build_lookup(codes: &[(u16, u8, u16)], ext: &[(u16, u8, u16)]) -> [u32; 1 << LOOKUP_BITS] {
    let mut table = [0u32; 1 << LOOKUP_BITS];
    let mut pass = 0;
    while pass < 2 {
        let list = if pass == 0 { codes } else { ext };
        let mut i = 0;
        while i < list.len() {
            let (code, len, run) = list[i];
            let shift = LOOKUP_BITS - len as u32;
            let first = (code as usize) << shift;
            let count = 1usize << shift;
            let mut j = 0;
            while j < count {
                assert!(table[first + j] == 0, "overlapping fax codes");
                table[first + j] = ((len as u32) << 16) | run as u32;
                j += 1;
            }
            i += 1;
        }
        pass += 1;
    }
    table
}

static WHITE_LOOKUP: [u32; 1 << LOOKUP_BITS] = build_lookup(WHITE_CODES, EXT_MAKEUP);
static BLACK_LOOKUP: [u32; 1 << LOOKUP_BITS] = build_lookup(BLACK_CODES, EXT_MAKEUP);

/// Reads one run: any make-up codes followed by a terminating code. None on an invalid code.
fn read_run(bits: &mut Bits, black: bool) -> Option<u32> {
    let table = if black { &BLACK_LOOKUP } else { &WHITE_LOOKUP };
    let mut total = 0u32;
    loop {
        let e = table[bits.peek(LOOKUP_BITS) as usize];
        if e == 0 {
            return None;
        }
        bits.skip(e >> 16);
        let run = e & 0xffff;
        total = total.saturating_add(run);
        if run < 64 {
            return Some(total);
        }
        // Make-up codes only ever add up to a row; anything far longer is garbage.
        if total > MAX_DIM + 2560 {
            return None;
        }
    }
}

/// Where a damaged row stopped: what to keep of it.
struct Damaged {
    a0: u32,
}

struct Decoder<'a> {
    bits: Bits<'a>,
    columns: u32,
    k: i32,
    eol: bool,
    align: bool,
    /// Changing elements of the reference (previous) row, followed by two `columns` sentinels.
    reference: Vec<u32>,
    current: Vec<u32>,
    next_2d: bool,
}

impl<'a> Decoder<'a> {
    fn new(data: &'a [u8], columns: u32, k: i32, eol: bool, align: bool) -> Self {
        Decoder {
            bits: Bits::new(data),
            columns,
            k,
            eol,
            align,
            reference: vec![columns, columns],
            current: Vec::new(),
            next_2d: k < 0,
        }
    }

    /// Decodes up to `max_rows` rows, handing each row's changing elements to `emit`.
    fn run(&mut self, max_rows: u32, damaged_allowed: u32, mut emit: impl FnMut(&[u32])) {
        let mut rows = 0u32;
        let mut damaged = 0u32;
        let checks_eol = self.eol && self.k >= 0;
        while rows < max_rows {
            if !self.between_rows() {
                break;
            }
            let result = if self.next_2d { self.decode_2d() } else { self.decode_1d() };
            let bad = match result {
                Ok(()) => {
                    emit(&self.current);
                    // A row that does not end where an EOL starts was decoded from garbage.
                    checks_eol && !self.bits.overrun() && !self.eol_follows()
                }
                Err(d) => {
                    // Keep what the row decoded, the rest white.
                    if self.current.len() % 2 == 1 {
                        self.current.push(d.a0.min(self.columns));
                    }
                    emit(&self.current);
                    true
                }
            };
            rows += 1;
            if self.bits.overrun() {
                break; // the row ran into the end of the data
            }
            if bad {
                if !checks_eol || damaged >= damaged_allowed || !self.seek_eol() {
                    break;
                }
                damaged += 1;
            }
            self.swap_reference();
        }
    }

    /// After a row in EOL mode: skips fill and reports whether an EOL (or the end of the data) comes next.
    fn eol_follows(&mut self) -> bool {
        while !self.bits.at_end() && self.bits.peek(12) == 0 {
            self.bits.skip(1);
        }
        self.bits.at_end() || self.bits.peek(12) == EOL
    }

    fn swap_reference(&mut self) {
        std::mem::swap(&mut self.reference, &mut self.current);
        self.reference.push(self.columns);
        self.reference.push(self.columns);
    }

    /// Consumes fill bits, EOLs and tag bits before a row. Returns false at the end of the data or on two
    /// consecutive EOLs (EOFB in T.6, RTC in T.4).
    fn between_rows(&mut self) -> bool {
        if self.align && (self.k < 0 || !self.eol) {
            // Encoders differ on whether EOFB itself is byte aligned: look for it before skipping to the boundary.
            if self.k < 0 && self.bits.peek(24) == 0x00_1001 {
                return false;
            }
            self.bits.align();
        }
        let mut eols = 0;
        loop {
            if self.bits.at_end() {
                return false;
            }
            let w = self.bits.peek(12);
            if w == 0 {
                // Fill: twelve zeros never start a code word.
                self.bits.skip(1);
                continue;
            }
            if w != EOL {
                break;
            }
            self.bits.skip(12);
            eols += 1;
            if eols >= 2 {
                return false;
            }
            if self.k > 0 {
                self.next_2d = self.bits.read(1) == 0;
            }
        }
        // Mixed coding carries the tag bit even where the stream has no EOLs.
        if self.k > 0 && eols == 0 {
            self.next_2d = self.bits.read(1) == 0;
        }
        !self.bits.at_end()
    }

    /// Skips to the next EOL after a damaged row, leaving it for `between_rows`.
    fn seek_eol(&mut self) -> bool {
        while !self.bits.at_end() {
            if self.bits.peek(12) == EOL {
                return true;
            }
            self.bits.skip(1);
        }
        false
    }

    fn decode_1d(&mut self) -> Result<(), Damaged> {
        self.current.clear();
        let mut a0 = 0u32;
        let mut black = false;
        while a0 < self.columns {
            let run = read_run(&mut self.bits, black).ok_or(Damaged { a0 })?;
            a0 = a0.saturating_add(run).min(self.columns);
            self.current.push(a0);
            black = !black;
        }
        Ok(())
    }

    fn decode_2d(&mut self) -> Result<(), Damaged> {
        self.current.clear();
        let columns = i64::from(self.columns);
        let reference = &self.reference;
        let current = &mut self.current;
        // a0 starts on an imaginary white element just left of the row.
        let mut a0: i64 = -1;
        let mut black = false;
        let mut i = 0usize;
        while a0 < columns {
            let damaged = Damaged { a0: a0.max(0) as u32 };
            // b1: first changing element right of a0 whose colour is opposite to a0's, i.e. an index of the
            // right parity (even indices turn black). a0 only moves right, so stepping back one entry is enough.
            if i > 0 && i64::from(reference[i - 1]) > a0 {
                i -= 1;
            }
            while i + 1 < reference.len() && (i64::from(reference[i]) <= a0 || (i % 2 == 1) != black) {
                i += 1;
            }
            let b1 = i64::from(reference.get(i).copied().unwrap_or(self.columns));
            let b2 = i64::from(reference.get(i + 1).copied().unwrap_or(self.columns));
            match read_mode(&mut self.bits) {
                Mode::Pass => a0 = b2,
                Mode::Horizontal => {
                    let start = a0.max(0);
                    let r1 = read_run(&mut self.bits, black).ok_or(damaged)?;
                    let r2 = read_run(&mut self.bits, !black).ok_or(Damaged { a0: start as u32 })?;
                    let a1 = (start + i64::from(r1)).min(columns);
                    let a2 = (a1 + i64::from(r2)).min(columns);
                    current.push(a1 as u32);
                    current.push(a2 as u32);
                    a0 = a2;
                }
                Mode::Vertical(d) => {
                    let a1 = b1 + i64::from(d);
                    if a1 < a0.max(0) || a1 > columns {
                        return Err(damaged);
                    }
                    current.push(a1 as u32);
                    a0 = a1;
                    black = !black;
                }
                Mode::Stop => return Err(damaged),
            }
            if self.bits.overrun() {
                return Err(Damaged { a0: a0.clamp(0, columns) as u32 });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use std::process::Command;

    /// T.6 (MMR) encoding of packed rows (MSB first, 1 = black) with a trailing EOFB, for other modules' tests.
    pub(in super::super) fn encode_mmr(packed: &[u8], width: usize, height: usize) -> Vec<u8> {
        let rb = width.div_ceil(8);
        let px = (0..width * height)
            .map(|i| packed[(i / width) * rb + (i % width) / 8] & (0x80 >> (i % width % 8)) != 0)
            .collect();
        encode(&Img { w: width, h: height, px }, EncOpts { k: -1, eol: false, align: false, terminator: true })
    }

    /// A 1 = black bitmap as one bool per pixel.
    #[derive(Clone)]
    struct Img {
        w: usize,
        h: usize,
        px: Vec<bool>,
    }

    impl Img {
        /// Text-like content: runs of random length with some vertical coherence.
        fn random(w: usize, h: usize, seed: u32) -> Img {
            let mut s = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
            let mut next = move || {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                s
            };
            let mut px = vec![false; w * h];
            for y in 0..h {
                let copy = y > 0 && next() % 3 != 0;
                let mut x = 0;
                let mut black = next() % 2 == 0;
                while x < w {
                    let run = match next() % 10 {
                        0 => (next() % 3000) as usize,
                        1..=3 => (next() % 64) as usize,
                        _ => (next() % 12) as usize,
                    };
                    for xx in x..(x + run).min(w) {
                        px[y * w + xx] = if copy && next() % 8 != 0 { px[(y - 1) * w + xx] } else { black };
                    }
                    x += run.max(1);
                    black = !black;
                }
            }
            Img { w, h, px }
        }

        fn packed(&self, black_is_1: bool) -> Vec<u8> {
            let rb = self.w.div_ceil(8);
            let mut out = vec![0u8; rb * self.h];
            for y in 0..self.h {
                for x in 0..self.w {
                    if self.px[y * self.w + x] == black_is_1 {
                        out[y * rb + x / 8] |= 0x80 >> (x % 8);
                    }
                }
            }
            if !black_is_1 {
                // The decoder leaves row padding as inverted zeros.
                for y in 0..self.h {
                    for x in self.w..rb * 8 {
                        out[y * rb + x / 8] |= 0x80 >> (x % 8);
                    }
                }
            }
            out
        }

        fn changes(&self, y: usize) -> Vec<u32> {
            let mut c = Vec::new();
            let mut prev = false;
            for x in 0..self.w {
                let p = self.px[y * self.w + x];
                if p != prev {
                    c.push(x as u32);
                    prev = p;
                }
            }
            c
        }
    }

    struct BitWriter {
        out: Vec<u8>,
        acc: u64,
        n: u32,
    }

    impl BitWriter {
        fn new() -> Self {
            BitWriter { out: Vec::new(), acc: 0, n: 0 }
        }
        fn put(&mut self, code: u32, len: u32) {
            self.acc = (self.acc << len) | u64::from(code);
            self.n += len;
            while self.n >= 8 {
                self.out.push((self.acc >> (self.n - 8)) as u8);
                self.n -= 8;
            }
        }
        fn bit_pos(&self) -> usize {
            self.out.len() * 8 + self.n as usize
        }
        fn align(&mut self) {
            if self.n > 0 {
                let pad = 8 - self.n;
                self.put(0, pad);
            }
        }
        /// Zero fill so that the next 12-bit EOL ends on a byte boundary.
        fn fill_for_eol(&mut self) {
            let pad = (8 + 4 - self.n % 8) % 8;
            self.put(0, pad);
        }
        fn finish(mut self) -> Vec<u8> {
            self.align();
            self.out
        }
    }

    fn put_run(w: &mut BitWriter, mut run: u32, black: bool) {
        let table = if black { BLACK_CODES } else { WHITE_CODES };
        let find = |list: &[(u16, u8, u16)], r: u32| list.iter().find(|c| u32::from(c.2) == r).copied();
        while run >= 64 {
            let m = (run / 64 * 64).min(2560);
            let (code, len, _) = find(table, m).or_else(|| find(EXT_MAKEUP, m)).unwrap();
            w.put(u32::from(code), u32::from(len));
            run -= m;
        }
        let (code, len, _) = find(table, run).unwrap();
        w.put(u32::from(code), u32::from(len));
    }

    fn encode_1d_row(w: &mut BitWriter, changes: &[u32], columns: u32) {
        let mut pos = 0;
        let mut black = false;
        for &c in changes.iter().chain(std::iter::once(&columns)) {
            put_run(w, c - pos, black);
            pos = c;
            black = !black;
            if c == columns {
                break;
            }
        }
    }

    fn encode_2d_row(w: &mut BitWriter, reference: &[u32], current: &[u32], columns: u32) {
        let with = |v: &[u32]| {
            let mut v = v.to_vec();
            v.push(columns);
            v.push(columns);
            v
        };
        let (r, c) = (with(reference), with(current));
        let cols = i64::from(columns);
        let mut a0: i64 = -1;
        let mut black = false;
        while a0 < cols {
            let a1_idx = c.iter().position(|&x| i64::from(x) > a0).unwrap();
            let a1 = i64::from(c[a1_idx]);
            let a2 = i64::from(c[(a1_idx + 1).min(c.len() - 1)]);
            let mut bi = 0;
            while bi + 1 < r.len() && (i64::from(r[bi]) <= a0 || (bi % 2 == 1) != black) {
                bi += 1;
            }
            let b1 = i64::from(r[bi]);
            let b2 = i64::from(r[(bi + 1).min(r.len() - 1)]);
            if b2 < a1 {
                w.put(0b0001, 4);
                a0 = b2;
            } else if (a1 - b1).abs() <= 3 {
                let (code, len) = match a1 - b1 {
                    0 => (1, 1),
                    1 => (0b011, 3),
                    2 => (0b000011, 6),
                    3 => (0b0000011, 7),
                    -1 => (0b010, 3),
                    -2 => (0b000010, 6),
                    _ => (0b0000010, 7),
                };
                w.put(code, len);
                a0 = a1;
                black = !black;
            } else {
                w.put(0b001, 3);
                put_run(w, (a1 - a0.max(0)) as u32, black);
                put_run(w, (a2 - a1) as u32, !black);
                a0 = a2;
            }
        }
    }

    #[derive(Clone, Copy)]
    struct EncOpts {
        k: i32,
        eol: bool,
        align: bool,
        terminator: bool,
    }

    /// Encodes like a fax machine would for the given options; `k > 0` codes every k-th row in 1-D.
    fn encode(img: &Img, o: EncOpts) -> Vec<u8> {
        encode_rows(img, o).0
    }

    /// Also returns the bit position where each row's code words start.
    fn encode_rows(img: &Img, o: EncOpts) -> (Vec<u8>, Vec<usize>) {
        let mut starts = Vec::new();
        let mut w = BitWriter::new();
        let columns = img.w as u32;
        let mut reference: Vec<u32> = Vec::new();
        for y in 0..img.h {
            let cur = img.changes(y);
            let two_d = o.k < 0 || (o.k > 0 && y % o.k as usize != 0);
            if o.eol {
                if o.align {
                    w.fill_for_eol();
                }
                w.put(1, 12);
                if o.k > 0 {
                    w.put(u32::from(!two_d), 1);
                }
            } else {
                if o.align {
                    w.align();
                }
                if o.k > 0 {
                    w.put(u32::from(!two_d), 1);
                }
            }
            starts.push(w.bit_pos());
            if two_d {
                encode_2d_row(&mut w, &reference, &cur, columns);
            } else {
                encode_1d_row(&mut w, &cur, columns);
            }
            reference = cur;
        }
        if o.terminator {
            if o.k < 0 {
                w.put(1, 12);
                w.put(1, 12);
            } else {
                for _ in 0..6 {
                    w.put(1, 12);
                    if o.k > 0 {
                        w.put(1, 1);
                    }
                }
            }
        }
        (w.finish(), starts)
    }

    fn params(img: &Img, o: EncOpts, rows: bool) -> CcittParams {
        CcittParams {
            k: o.k,
            columns: img.w as u32,
            rows: if rows { img.h as u32 } else { 0 },
            end_of_line: o.eol,
            encoded_byte_align: o.align,
            end_of_block: o.terminator,
            black_is_1: true,
            damaged_rows_before_error: 0,
        }
    }

    const SIZES: &[(usize, usize)] = &[(1, 1), (7, 5), (8, 9), (100, 40), (1728, 30), (2600, 12), (5000, 3)];

    #[test]
    fn tables_are_complete() {
        // Every run 0..=63 and every make-up value appears once per colour.
        for codes in [WHITE_CODES, BLACK_CODES] {
            for r in 0..64 {
                assert_eq!(codes.iter().filter(|c| c.2 == r).count(), 1, "run {r}");
            }
            for m in (64..=1728).step_by(64) {
                assert_eq!(codes.iter().filter(|c| c.2 == m).count(), 1, "make-up {m}");
            }
        }
        assert_eq!(WHITE_CODES.len(), 91);
        assert_eq!(BLACK_CODES.len(), 91);
    }

    #[test]
    fn round_trips_all_modes() {
        let modes = [
            EncOpts { k: -1, eol: false, align: false, terminator: true },
            EncOpts { k: -1, eol: false, align: false, terminator: false },
            EncOpts { k: -1, eol: false, align: true, terminator: true },
            EncOpts { k: 0, eol: false, align: false, terminator: false },
            EncOpts { k: 0, eol: true, align: false, terminator: true },
            EncOpts { k: 0, eol: true, align: true, terminator: true },
            EncOpts { k: 0, eol: false, align: true, terminator: false },
            EncOpts { k: 2, eol: true, align: false, terminator: true },
            EncOpts { k: 4, eol: true, align: true, terminator: true },
            EncOpts { k: 3, eol: false, align: false, terminator: false },
        ];
        for (n, &(w, h)) in SIZES.iter().enumerate() {
            let img = Img::random(w, h, n as u32 + 7);
            for o in modes {
                let data = encode(&img, o);
                for with_rows in [true, false] {
                    let p = params(&img, o, with_rows);
                    let got = decode(&data, &p).unwrap();
                    assert!(
                        got == img.packed(true),
                        "{w}x{h} k={} eol={} align={} rows={with_rows}",
                        o.k,
                        o.eol,
                        o.align
                    );
                }
            }
        }
    }

    #[test]
    fn black_is_0_output_and_padding() {
        let img = Img::random(13, 6, 3);
        let o = EncOpts { k: -1, eol: false, align: false, terminator: true };
        let mut p = params(&img, o, true);
        p.black_is_1 = false;
        assert_eq!(decode(&encode(&img, o), &p).unwrap(), img.packed(false));
        // Missing rows are padded white.
        p.rows = 10;
        let got = decode(&encode(&img, o), &p).unwrap();
        assert_eq!(got.len(), 20);
        assert!(got[12..].iter().all(|&b| b == 0xff));
    }

    #[test]
    fn mmr_reports_consumed_bytes() {
        let a = Img::random(300, 20, 11);
        let b = Img::random(300, 15, 12);
        let o = EncOpts { k: -1, eol: false, align: false, terminator: true };
        let ea = encode(&a, o);
        let mut data = ea.clone();
        data.extend_from_slice(&encode(&b, o));
        let (bits_a, used) = decode_mmr(&data, 300, 20).unwrap();
        assert_eq!(bits_a, a.packed(true));
        assert_eq!(used, ea.len());
        let (bits_b, _) = decode_mmr(&data[used..], 300, 15).unwrap();
        assert_eq!(bits_b, b.packed(true));
        // Without EOFB, exactly the coded rows.
        let o2 = EncOpts { terminator: false, ..o };
        let (bits, _) = decode_mmr(&encode(&a, o2), 300, 20).unwrap();
        assert_eq!(bits, a.packed(true));
        assert_eq!(decode_mmr(&[], 10, 0).unwrap().0.len(), 0);
    }

    #[test]
    fn truncation_and_garbage_never_panic() {
        let img = Img::random(500, 40, 5);
        let rb = 63;
        let full = img.packed(true);
        for o in [
            EncOpts { k: -1, eol: false, align: false, terminator: true },
            EncOpts { k: 0, eol: true, align: false, terminator: true },
            EncOpts { k: 2, eol: true, align: false, terminator: true },
        ] {
            let (data, starts) = encode_rows(&img, o);
            let mut last_good = 0;
            for cut in (0..=data.len()).step_by(5).chain(std::iter::once(data.len())) {
                let p = params(&img, o, true);
                let got = decode(&data[..cut], &p).unwrap();
                assert_eq!(got.len(), full.len());
                // The rows whose code words lie wholly inside the kept bytes must decode exactly, and more data
                // never decodes fewer rows.
                let good = (0..img.h).take_while(|&y| got[y * rb..(y + 1) * rb] == full[y * rb..(y + 1) * rb]).count();
                let complete = starts.iter().skip(1).filter(|&&bit| bit <= cut * 8).count();
                assert!(good >= complete, "k={} cut {cut}: {good} rows < {complete}", o.k);
                assert!(good >= last_good);
                last_good = good;
                let mut p0 = p.clone();
                p0.rows = 0;
                let open = decode(&data[..cut], &p0).unwrap();
                assert!(open.len() <= full.len());
            }
            assert_eq!(last_good, img.h);
        }
        let mut seed = 99u32;
        for len in [1usize, 2, 5, 50, 500] {
            let junk: Vec<u8> = (0..len)
                .map(|_| {
                    seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    (seed >> 24) as u8
                })
                .collect();
            for k in [-1, 0, 1, 5] {
                for eol in [false, true] {
                    let p = CcittParams {
                        k,
                        columns: 77,
                        end_of_line: eol,
                        damaged_rows_before_error: 3,
                        ..Default::default()
                    };
                    let got = decode(&junk, &p).unwrap();
                    assert_eq!(got.len() % 10, 0);
                    let _ = decode_mmr(&junk, 77, 9).unwrap();
                }
            }
        }
        assert!(decode(&[0xff; 4], &CcittParams { columns: 0, ..Default::default() }).is_err());
        assert!(decode(&[], &CcittParams { columns: 70000, ..Default::default() }).is_err());
        assert!(decode(&[], &CcittParams { columns: 65535, rows: 65535, ..Default::default() }).is_err());
        assert_eq!(decode(&[], &CcittParams::default()).unwrap().len(), 0);
    }

    #[test]
    fn damaged_rows_resync_on_eol() {
        let img = Img::random(200, 12, 21);
        for o in [
            EncOpts { k: 0, eol: true, align: true, terminator: true },
            EncOpts { k: 2, eol: true, align: false, terminator: true },
        ] {
            let (mut data, starts) = encode_rows(&img, o);
            // Garble row 4 without creating anything that looks like an EOL.
            let p4 = starts[4] / 8 + 2;
            data[p4 - 1] = 0xff;
            data[p4] = 0x00;
            data[p4 + 1] = 0xff;
            let rb = 25;
            let full = img.packed(true);
            let mut p = params(&img, o, true);
            p.damaged_rows_before_error = 5;
            let lenient = decode(&data, &p).unwrap();
            assert_eq!(&lenient[..4 * rb], &full[..4 * rb]);
            // Row 5 is 2-D coded against the damaged row in mixed mode, so only the 1-D rows after it must match.
            let from = if o.k > 0 { 6 } else { 5 };
            assert_eq!(&lenient[from * rb..], &full[from * rb..], "k={}: rows after the damage should resync", o.k);
            p.damaged_rows_before_error = 0;
            let strict = decode(&data, &p).unwrap();
            assert_eq!(&strict[..4 * rb], &full[..4 * rb]);
            assert!(strict[5 * rb..].iter().all(|&b| b == 0), "strict mode stops at the damage");
        }
    }

    /// Finds the single strip of a little- or big-endian TIFF written with one strip.
    fn tiff_strip(tif: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
        let le = tif.get(..2)? == b"II";
        let u16_at = |p: usize| -> Option<u32> {
            let b: [u8; 2] = tif.get(p..p + 2)?.try_into().ok()?;
            Some(u32::from(if le { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) }))
        };
        let u32_at = |p: usize| -> Option<u32> {
            let b: [u8; 4] = tif.get(p..p + 4)?.try_into().ok()?;
            Some(if le { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
        };
        let ifd = u32_at(4)? as usize;
        let n = u16_at(ifd)? as usize;
        let (mut off, mut len, mut photometric, mut t4) = (None, None, 0, 0);
        for i in 0..n {
            let e = ifd + 2 + i * 12;
            let tag = u16_at(e)?;
            let typ = u16_at(e + 2)?;
            let count = u32_at(e + 4)?;
            let val = if typ == 3 { u16_at(e + 8)? } else { u32_at(e + 8)? };
            match tag {
                262 => photometric = val,
                273 if count == 1 => off = Some(val),
                279 if count == 1 => len = Some(val),
                292 => t4 = val,
                _ => {}
            }
        }
        let (off, len) = (off? as usize, len? as usize);
        Some((tif.get(off..off + len)?.to_vec(), photometric, t4))
    }

    /// Cross-checks the code tables against libtiff's encoder through ImageMagick, when it is installed.
    #[test]
    fn matches_libtiff_encoder() {
        if Command::new("magick").arg("-version").output().is_err() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("pdf-codecs-ccitt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut compared = Vec::new();
        for (n, &(w, h)) in [(1728usize, 40usize), (333, 21), (3000, 7)].iter().enumerate() {
            let img = Img::random(w, h, 40 + n as u32);
            let pbm_path = dir.join("in.pbm");
            let mut pbm = format!("P4\n{w} {h}\n").into_bytes();
            pbm.extend_from_slice(&img.packed(true));
            std::fs::write(&pbm_path, &pbm).unwrap();
            for (compress, k, eol) in [("Group4", -1, false), ("Fax", 0, true)] {
                let out = dir.join(format!("out-{compress}.tif"));
                let status = Command::new("magick")
                    .arg(&pbm_path)
                    .args(["-compress", compress, "-define", &format!("tiff:rows-per-strip={h}")])
                    .arg(&out)
                    .status();
                let Ok(status) = status else { continue };
                if !status.success() {
                    continue;
                }
                let tif = std::fs::read(&out).unwrap();
                let Some((strip, photometric, t4)) = tiff_strip(&tif) else { continue };
                let p = CcittParams {
                    k: if compress == "Fax" && t4 & 1 != 0 { 1 } else { k },
                    columns: w as u32,
                    rows: h as u32,
                    end_of_line: eol,
                    encoded_byte_align: t4 & 4 != 0,
                    end_of_block: false,
                    black_is_1: photometric == 0,
                    damaged_rows_before_error: 0,
                };
                let mut got = decode(&strip, &p).unwrap();
                if !p.black_is_1 {
                    // Clear the inverted row padding before comparing.
                    let rb = w.div_ceil(8);
                    for y in 0..h {
                        for x in w..rb * 8 {
                            got[y * rb + x / 8] &= !(0x80 >> (x % 8));
                        }
                    }
                }
                assert!(got == img.packed(true), "{compress} {w}x{h} differs from libtiff (photometric {photometric})");
                compared.push((compress, photometric, t4));
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
        eprintln!("compared against libtiff: {compared:?}");
        assert!(compared.len() >= 4, "ImageMagick produced too few fax TIFFs: {compared:?}");
    }
}
