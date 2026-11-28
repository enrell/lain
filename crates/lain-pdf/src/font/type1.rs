//! Type 1 font programs: the PDF `FontFile` stream (cleartext part, then
//! the eexec-encrypted part, binary or hex), PFB files (0x80 segment
//! headers) and PFA files.
//!
//! Parsing decrypts the private part once and records where each
//! subroutine and charstring is; a charstring is interpreted only when
//! its outline or width is asked for. The PostScript is not executed:
//! a small tokenizer finds the few entries a renderer needs.

use super::FontError;
use super::encoding::STANDARD;
use crate::geom::{Matrix, Path, Seg};

/// Subroutine nesting limit.
const MAX_SUBR_DEPTH: usize = 10;
/// The Type 1 spec allows 24 operands; broken fonts get more room.
const MAX_STACK: usize = 64;
/// Path segments one glyph may produce before it is refused.
const MAX_SEGMENTS: usize = 20_000;
/// Operators and operands one glyph may execute.
const MAX_OPS: usize = 100_000;
/// Entries read from `/Subrs` and `/CharStrings`.
const MAX_ENTRIES: usize = 65_536;
/// Points one Flex sequence may collect (it needs 7).
const MAX_FLEX_POINTS: usize = 16;

const DEFAULT_MATRIX: [f64; 6] = [0.001, 0.0, 0.0, 0.001, 0.0, 0.0];
const EEXEC_KEY: u16 = 55665;
const CHARSTRING_KEY: u16 = 4330;

fn err(msg: &str) -> FontError {
    FontError(format!("type1: {msg}"))
}

/// Type 1 decryption (Adobe Type 1 Font Format, chapter 7).
fn decrypt(data: &mut [u8], key: u16) {
    let mut r = key;
    for b in data.iter_mut() {
        let c = *b;
        *b = c ^ (r >> 8) as u8;
        r = (c as u16).wrapping_add(r).wrapping_mul(52845).wrapping_add(22719);
    }
}

fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n' | b'\x0C' | b'\0')
}

fn is_delim(b: u8) -> bool {
    matches!(b, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%')
}

/// One PostScript token; strings and dictionary brackets carry no value
/// this module needs.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Tok<'a> {
    Name(&'a [u8]),
    Num(f64),
    Word(&'a [u8]),
    Open,
    Close,
    Other,
}

/// The next token at or after `*pos`; `None` at the end of the data.
fn next_tok<'a>(d: &'a [u8], pos: &mut usize) -> Option<Tok<'a>> {
    loop {
        while *pos < d.len() && is_space(d[*pos]) {
            *pos += 1;
        }
        if *pos < d.len() && d[*pos] == b'%' {
            while *pos < d.len() && d[*pos] != b'\n' && d[*pos] != b'\r' {
                *pos += 1;
            }
            continue;
        }
        break;
    }
    let start = *pos;
    let b = *d.get(start)?;
    *pos += 1;
    match b {
        b'(' => {
            // A string, with nested parentheses and backslash escapes.
            let mut depth = 1;
            while *pos < d.len() && depth > 0 {
                match d[*pos] {
                    b'\\' => *pos += 1,
                    b'(' => depth += 1,
                    b')' => depth -= 1,
                    _ => {}
                }
                *pos += 1;
            }
            *pos = (*pos).min(d.len());
            Some(Tok::Other)
        }
        b'<' => {
            if d.get(*pos) == Some(&b'<') {
                *pos += 1;
            } else {
                while *pos < d.len() && d[*pos] != b'>' {
                    *pos += 1;
                }
                *pos = (*pos + 1).min(d.len());
            }
            Some(Tok::Other)
        }
        b'>' => {
            if d.get(*pos) == Some(&b'>') {
                *pos += 1;
            }
            Some(Tok::Other)
        }
        b'[' | b'{' => Some(Tok::Open),
        b']' | b'}' => Some(Tok::Close),
        b')' => Some(Tok::Other),
        b'/' => {
            if d.get(*pos) == Some(&b'/') {
                *pos += 1;
            }
            let s = *pos;
            while *pos < d.len() && !is_space(d[*pos]) && !is_delim(d[*pos]) {
                *pos += 1;
            }
            Some(Tok::Name(&d[s..*pos]))
        }
        _ => {
            while *pos < d.len() && !is_space(d[*pos]) && !is_delim(d[*pos]) {
                *pos += 1;
            }
            let word = &d[start..*pos];
            Some(match parse_number(word) {
                Some(v) => Tok::Num(v),
                None => Tok::Word(word),
            })
        }
    }
}

/// A PostScript number: integer, real or `radix#digits`.
fn parse_number(w: &[u8]) -> Option<f64> {
    let s = std::str::from_utf8(w).ok()?;
    let first = *w.first()?;
    if !(first.is_ascii_digit() || matches!(first, b'-' | b'+' | b'.')) {
        return None;
    }
    if let Some((radix, digits)) = s.split_once('#') {
        let radix: u32 = radix.parse().ok().filter(|r| (2..=36).contains(r))?;
        return i64::from_str_radix(digits, radix).ok().map(|v| v as f64);
    }
    s.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// The cleartext part and the decrypted private part of a font program.
fn split_program(data: &[u8]) -> Result<(Vec<u8>, Vec<u8>), FontError> {
    if data.first() == Some(&0x80) {
        return split_pfb(data);
    }
    let Some(at) = find_eexec(data) else {
        // No encrypted part: some producers write the whole font in the
        // clear; the private entries are then found in it directly.
        return Ok((data.to_vec(), data.to_vec()));
    };
    let clear = data[..at].to_vec();
    let mut pos = at + 5;
    while pos < data.len() && is_space(data[pos]) {
        pos += 1;
    }
    let rest = &data[pos..];
    // Hex when the first four bytes are hex digits (the spec's test).
    let hex = rest.len() >= 4 && rest[..4].iter().all(|b| b.is_ascii_hexdigit());
    let mut enc = if hex { decode_hex(rest) } else { rest.to_vec() };
    decrypt(&mut enc, EEXEC_KEY);
    let private = enc.get(4..).unwrap_or(&[]).to_vec();
    Ok((clear, private))
}

fn split_pfb(data: &[u8]) -> Result<(Vec<u8>, Vec<u8>), FontError> {
    let (mut clear, mut enc) = (Vec::new(), Vec::new());
    let mut pos = 0;
    while pos + 2 <= data.len() && data[pos] == 0x80 {
        let kind = data[pos + 1];
        if kind == 3 {
            break;
        }
        let Some(len) = data.get(pos + 2..pos + 6) else {
            break;
        };
        let len = u32::from_le_bytes([len[0], len[1], len[2], len[3]]) as usize;
        let start = pos + 6;
        let end = start.saturating_add(len).min(data.len());
        match kind {
            // ASCII segments after the binary one are the zeros trailer.
            1 if enc.is_empty() => clear.extend_from_slice(&data[start..end]),
            2 => enc.extend_from_slice(&data[start..end]),
            _ => {}
        }
        pos = end;
    }
    if enc.is_empty() {
        return Err(err("PFB without a binary segment"));
    }
    decrypt(&mut enc, EEXEC_KEY);
    Ok((clear, enc.get(4..).unwrap_or(&[]).to_vec()))
}

/// The `eexec` operator: the word on its own, not inside a comment or a
/// string that merely mentions it.
fn find_eexec(data: &[u8]) -> Option<usize> {
    let mut from = 0;
    while let Some(i) = data.get(from..)?.windows(5).position(|w| w == b"eexec") {
        let at = from + i;
        let before = at.checked_sub(1).map(|p| data[p]);
        let after = data.get(at + 5).copied();
        if before.is_none_or(is_space) && after.is_none_or(is_space) {
            return Some(at);
        }
        from = at + 5;
    }
    None
}

/// Hex digits up to the first byte that is neither hex nor white space.
fn decode_hex(d: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(d.len() / 2);
    let mut high: Option<u8> = None;
    for &b in d {
        let v = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            _ if is_space(b) => continue,
            _ => break,
        };
        match high.take() {
            Some(h) => out.push((h << 4) | v),
            None => high = Some(v),
        }
    }
    out
}

/// A Type 1 font. It owns the decrypted private part.
pub struct Type1 {
    font_matrix: [f64; 6],
    bbox: Option<[f64; 4]>,
    encoding: Vec<Option<String>>,
    private: Vec<u8>,
    subrs: Vec<Option<(usize, usize)>>,
    /// (name, start, end) in CharStrings order.
    charstrings: Vec<(String, usize, usize)>,
    /// Indices into `charstrings` sorted by name.
    by_name: Vec<u32>,
}

impl std::fmt::Debug for Type1 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Type1")
            .field("glyphs", &self.charstrings.len())
            .field("subrs", &self.subrs.len())
            .field("font_matrix", &self.font_matrix)
            .finish()
    }
}

/// What the PostScript scan found.
#[derive(Default)]
struct Scan {
    matrix: Option<[f64; 6]>,
    bbox: Option<[f64; 4]>,
    encoding: Option<Vec<Option<String>>>,
    len_iv: Option<f64>,
    subrs: Option<Vec<Option<(usize, usize)>>>,
    charstrings: Option<Vec<(String, usize, usize)>>,
}

/// Up to `n` numbers inside the brackets that follow (`[...]` or `{...}`).
fn read_array(d: &[u8], pos: &mut usize, n: usize) -> Option<Vec<f64>> {
    if next_tok(d, pos)? != Tok::Open {
        return None;
    }
    let mut out = Vec::new();
    loop {
        match next_tok(d, pos)? {
            Tok::Num(v) if out.len() < n => out.push(v),
            Tok::Close => break,
            _ => return None,
        }
    }
    (out.len() == n).then_some(out)
}

fn read_encoding(d: &[u8], pos: &mut usize) -> Option<Vec<Option<String>>> {
    let mut out: Vec<Option<String>> = STANDARD.iter().map(|n| n.map(str::to_string)).collect();
    match next_tok(d, pos)? {
        Tok::Num(_) => {}
        // StandardEncoding, or a named encoding this module does not hold.
        _ => return Some(out),
    }
    out.iter_mut().for_each(|e| *e = None);
    // `dup code /name put` entries until the closing `def`.
    let mut steps = 0;
    while let Some(tok) = next_tok(d, pos) {
        steps += 1;
        if steps > 10_000 {
            break;
        }
        match tok {
            Tok::Word(b"dup") => {
                let save = *pos;
                if let (Some(Tok::Num(code)), Some(Tok::Name(name))) = (next_tok(d, pos), next_tok(d, pos)) {
                    if (0.0..256.0).contains(&code) && name != b".notdef" {
                        out[code as usize] = Some(String::from_utf8_lossy(name).into_owned());
                    }
                } else {
                    *pos = save;
                }
            }
            Tok::Word(b"def") | Tok::Word(b"readonly") => break,
            _ => {}
        }
    }
    Some(out)
}

/// `count array` then `dup index len RD <binary> NP` entries.
fn read_subrs(d: &[u8], pos: &mut usize) -> Vec<Option<(usize, usize)>> {
    let count = match next_tok(d, pos) {
        Some(Tok::Num(n)) if n >= 0.0 => (n as usize).min(MAX_ENTRIES),
        _ => return Vec::new(),
    };
    let mut subrs = vec![None; count];
    loop {
        let save = *pos;
        match next_tok(d, pos) {
            Some(Tok::Word(b"dup")) => {
                let (Some(Tok::Num(i)), Some(Tok::Num(len)), Some(_)) =
                    (next_tok(d, pos), next_tok(d, pos), next_tok(d, pos))
                else {
                    break;
                };
                let Some((start, end)) = binary_after(d, pos, len) else {
                    break;
                };
                if i >= 0.0 && (i as usize) < count {
                    subrs[i as usize] = Some((start, end));
                }
            }
            Some(Tok::Word(b"array" | b"NP" | b"|" | b"noaccess" | b"put")) => {}
            _ => {
                *pos = save;
                break;
            }
        }
    }
    subrs
}

/// The binary data that follows an RD token: one space, then `len` bytes.
fn binary_after(d: &[u8], pos: &mut usize, len: f64) -> Option<(usize, usize)> {
    if len < 0.0 {
        return None;
    }
    let start = pos.checked_add(1)?;
    let end = start.checked_add(len as usize)?;
    if end > d.len() {
        return None;
    }
    *pos = end;
    Some((start, end))
}

/// `count dict dup begin` then `/name len RD <binary> ND` entries to `end`.
fn read_charstrings(d: &[u8], pos: &mut usize) -> Vec<(String, usize, usize)> {
    let mut out = Vec::new();
    loop {
        let save = *pos;
        match next_tok(d, pos) {
            Some(Tok::Name(name)) => {
                let (Some(Tok::Num(len)), Some(_)) = (next_tok(d, pos), next_tok(d, pos)) else {
                    break;
                };
                let Some((start, end)) = binary_after(d, pos, len) else {
                    break;
                };
                out.push((String::from_utf8_lossy(name).into_owned(), start, end));
                if out.len() >= MAX_ENTRIES {
                    break;
                }
            }
            Some(Tok::Num(_)) if out.is_empty() => {}
            Some(Tok::Word(b"dict" | b"dup" | b"begin" | b"ND" | b"|-" | b"noaccess" | b"def" | b"readonly")) => {}
            _ => {
                *pos = save;
                break;
            }
        }
    }
    out
}

/// Walks the tokens of `d` and records the entries this module needs.
/// `binary` says whether Subrs and CharStrings may be read here.
fn scan(d: &[u8], scan: &mut Scan, binary: bool) {
    let mut pos = 0;
    while let Some(tok) = next_tok(d, &mut pos) {
        let Tok::Name(name) = tok else {
            continue;
        };
        match name {
            b"FontMatrix" if scan.matrix.is_none() => {
                scan.matrix = read_array(d, &mut pos, 6).map(|m| [m[0], m[1], m[2], m[3], m[4], m[5]]);
            }
            b"FontBBox" if scan.bbox.is_none() => {
                scan.bbox = read_array(d, &mut pos, 4).map(|m| [m[0], m[1], m[2], m[3]]);
            }
            b"Encoding" if scan.encoding.is_none() => scan.encoding = read_encoding(d, &mut pos),
            b"lenIV" if scan.len_iv.is_none() => {
                if let Some(Tok::Num(v)) = next_tok(d, &mut pos) {
                    scan.len_iv = Some(v);
                }
            }
            b"Subrs" if binary && scan.subrs.is_none() => scan.subrs = Some(read_subrs(d, &mut pos)),
            b"CharStrings" if binary && scan.charstrings.is_none() => {
                scan.charstrings = Some(read_charstrings(d, &mut pos));
            }
            _ => {}
        }
    }
}

impl Type1 {
    /// PDF `FontFile` data (cleartext part followed by the eexec part,
    /// binary or hex), a PFB (0x80 segment headers) or a PFA.
    pub fn parse(data: &[u8]) -> Result<Type1, FontError> {
        let (clear, mut private) = split_program(data)?;
        let mut found = Scan::default();
        scan(&clear, &mut found, false);
        scan(&private, &mut found, true);
        let charstrings = found.charstrings.unwrap_or_default();
        if charstrings.is_empty() {
            return Err(err("no CharStrings"));
        }
        let mut subrs = found.subrs.unwrap_or_default();
        // lenIV -1 means unencrypted charstrings; otherwise decrypt each
        // one in place and skip its lenIV leading bytes.
        let len_iv = found.len_iv.filter(|v| (-1.0..=64.0).contains(v)).unwrap_or(4.0) as i64;
        let mut fix = |range: (usize, usize)| -> (usize, usize) {
            let (start, end) = range;
            if len_iv < 0 {
                return range;
            }
            decrypt(&mut private[start..end], CHARSTRING_KEY);
            ((start + len_iv as usize).min(end), end)
        };
        for s in subrs.iter_mut().flatten() {
            *s = fix(*s);
        }
        let charstrings: Vec<(String, usize, usize)> = charstrings
            .into_iter()
            .map(|(name, s, e)| {
                let (s, e) = fix((s, e));
                (name, s, e)
            })
            .collect();
        let mut by_name: Vec<u32> = (0..charstrings.len() as u32).collect();
        // Stable: a repeated name keeps its first definition.
        by_name.sort_by(|&a, &b| charstrings[a as usize].0.cmp(&charstrings[b as usize].0));
        by_name.dedup_by(|b, a| charstrings[*a as usize].0 == charstrings[*b as usize].0);
        let font_matrix = found
            .matrix
            .filter(|m| m.iter().all(|v| v.is_finite()) && (m[0] * m[3] - m[1] * m[2]).abs() > 1e-12)
            .unwrap_or(DEFAULT_MATRIX);
        let encoding = found.encoding.unwrap_or_else(|| STANDARD.iter().map(|n| n.map(str::to_string)).collect());
        Ok(Type1 { font_matrix, bbox: found.bbox, encoding, private, subrs, charstrings, by_name })
    }

    /// The FontMatrix (default `[0.001 0 0 0.001 0 0]`).
    pub fn font_matrix(&self) -> [f64; 6] {
        self.font_matrix
    }

    /// The FontBBox, in character space.
    pub fn font_bbox(&self) -> Option<[f64; 4]> {
        self.bbox
    }

    /// The glyph names in CharStrings order.
    pub fn glyph_names(&self) -> Vec<String> {
        self.charstrings.iter().map(|c| c.0.clone()).collect()
    }

    /// The font's encoding, 256 entries: StandardEncoding or the font's
    /// own `dup code /name put` array.
    pub fn encoding(&self) -> Vec<Option<String>> {
        self.encoding.clone()
    }

    fn charstring(&self, name: &str) -> Option<&[u8]> {
        let i = self.by_name.binary_search_by(|&i| self.charstrings[i as usize].0.as_str().cmp(name)).ok()?;
        let (_, s, e) = &self.charstrings[self.by_name[i] as usize];
        self.private.get(*s..*e)
    }

    fn subr(&self, i: f64) -> Option<&[u8]> {
        if !(0.0..self.subrs.len() as f64).contains(&i) {
            return None;
        }
        let (s, e) = self.subrs[i as usize]?;
        self.private.get(s..e)
    }

    /// The outline of a glyph in character space (before the font
    /// matrix), y up, with cubic curves; every contour is closed. `None`
    /// when the glyph is missing or its charstring is broken.
    pub fn glyph_path(&self, name: &str) -> Option<Path> {
        let mut t1 = Interp::new(self, false);
        t1.run(self.charstring(name)?)?;
        let mut path = std::mem::take(&mut t1.path);
        if let Some((asb, adx, ady, bchar, achar)) = t1.seac {
            // Accented character: the accent's origin lands at
            // adx - asb + sbx (the composite's own sidebearing).
            let base = self.seac_component(bchar)?;
            let accent = self.seac_component(achar)?;
            path.segs.extend(base.segs);
            path.extend_transformed(&accent, &Matrix::translate(adx - asb + t1.sbx, ady));
            if path.segs.len() > MAX_SEGMENTS {
                return None;
            }
        }
        Some(path)
    }

    fn seac_component(&self, code: f64) -> Option<Path> {
        if !(0.0..256.0).contains(&code) {
            return None;
        }
        let mut t1 = Interp::new(self, false);
        t1.no_seac = true;
        t1.run(self.charstring(STANDARD[code as usize]?)?)?;
        Some(t1.path)
    }

    /// The advance width from the glyph's `hsbw` or `sbw`.
    pub fn advance_width(&self, name: &str) -> Option<f64> {
        let mut t1 = Interp::new(self, true);
        t1.run(self.charstring(name)?)?;
        t1.width
    }
}

enum Flow {
    Return,
    End,
}

/// Points collected during a Flex sequence, and where it started.
struct Flex {
    points: Vec<(f64, f64)>,
    start: (f64, f64),
}

/// The Type 1 charstring interpreter for one glyph.
struct Interp<'a> {
    font: &'a Type1,
    stack: [f64; MAX_STACK],
    sp: usize,
    /// The PostScript operand stack `callothersubr` leaves results on.
    ps: Vec<f64>,
    x: f64,
    y: f64,
    sbx: f64,
    open: bool,
    path: Path,
    width: Option<f64>,
    width_only: bool,
    flex: Option<Flex>,
    ops: usize,
    seac: Option<(f64, f64, f64, f64, f64)>,
    no_seac: bool,
}

impl<'a> Interp<'a> {
    fn new(font: &'a Type1, width_only: bool) -> Interp<'a> {
        Interp {
            font,
            stack: [0.0; MAX_STACK],
            sp: 0,
            ps: Vec::new(),
            x: 0.0,
            y: 0.0,
            sbx: 0.0,
            open: false,
            path: Path::new(),
            width: None,
            width_only,
            flex: None,
            ops: 0,
            seac: None,
            no_seac: false,
        }
    }

    fn run(&mut self, cs: &[u8]) -> Option<()> {
        self.exec(cs, 0)?;
        self.close_contour();
        (self.path.segs.len() <= MAX_SEGMENTS).then_some(())
    }

    fn push(&mut self, v: f64) -> Option<()> {
        if self.sp >= MAX_STACK {
            return None;
        }
        self.stack[self.sp] = if v.is_finite() { v } else { 0.0 };
        self.sp += 1;
        Some(())
    }

    fn pop(&mut self) -> Option<f64> {
        self.sp = self.sp.checked_sub(1)?;
        Some(self.stack[self.sp])
    }

    fn arg(&self, i: usize) -> f64 {
        if i < self.sp { self.stack[i] } else { 0.0 }
    }

    /// Closes the open contour; one that is only a move is dropped.
    fn close_contour(&mut self) {
        if self.open {
            if matches!(self.path.segs.last(), Some(Seg::Move(_))) {
                self.path.segs.pop();
            } else {
                self.path.close();
            }
            self.open = false;
        }
    }

    /// Type 1's closepath leaves the current point where it is.
    fn move_to(&mut self, dx: f64, dy: f64) {
        self.x += dx;
        self.y += dy;
        if self.flex.is_some() {
            // Inside Flex a move only positions the next Flex point.
            return;
        }
        self.close_contour();
        self.path.move_to(self.x, self.y);
        self.open = true;
    }

    fn ensure_open(&mut self) {
        if !self.open {
            self.path.move_to(self.x, self.y);
            self.open = true;
        }
    }

    fn line(&mut self, dx: f64, dy: f64) {
        self.ensure_open();
        self.x += dx;
        self.y += dy;
        self.path.line_to(self.x, self.y);
    }

    fn curve(&mut self, dxa: f64, dya: f64, dxb: f64, dyb: f64, dxc: f64, dyc: f64) {
        self.ensure_open();
        let (x1, y1) = (self.x + dxa, self.y + dya);
        let (x2, y2) = (x1 + dxb, y1 + dyb);
        let (x3, y3) = (x2 + dxc, y2 + dyc);
        self.path.cubic_to(x1, y1, x2, y2, x3, y3);
        (self.x, self.y) = (x3, y3);
    }

    fn exec(&mut self, cs: &[u8], depth: usize) -> Option<Flow> {
        let mut i = 0;
        while i < cs.len() {
            self.ops += 1;
            if self.ops > MAX_OPS || self.path.segs.len() > MAX_SEGMENTS {
                return None;
            }
            let v = cs[i];
            i += 1;
            match v {
                32..=246 => self.push(v as f64 - 139.0)?,
                247..=250 => {
                    let w = *cs.get(i)? as f64;
                    i += 1;
                    self.push((v as f64 - 247.0) * 256.0 + w + 108.0)?;
                }
                251..=254 => {
                    let w = *cs.get(i)? as f64;
                    i += 1;
                    self.push(-(v as f64 - 251.0) * 256.0 - w - 108.0)?;
                }
                255 => {
                    let b = cs.get(i..i + 4)?;
                    i += 4;
                    self.push(i32::from_be_bytes([b[0], b[1], b[2], b[3]]) as f64)?;
                }
                // hstem, vstem: hints only.
                1 | 3 => self.sp = 0,
                4 => {
                    self.move_to(0.0, self.arg(0));
                    self.sp = 0;
                }
                5 => {
                    self.line(self.arg(0), self.arg(1));
                    self.sp = 0;
                }
                6 => {
                    self.line(self.arg(0), 0.0);
                    self.sp = 0;
                }
                7 => {
                    self.line(0.0, self.arg(0));
                    self.sp = 0;
                }
                8 => {
                    let a: [f64; 6] = std::array::from_fn(|k| self.arg(k));
                    self.curve(a[0], a[1], a[2], a[3], a[4], a[5]);
                    self.sp = 0;
                }
                9 => {
                    self.close_contour();
                    self.sp = 0;
                }
                10 => {
                    if depth >= MAX_SUBR_DEPTH {
                        return None;
                    }
                    let n = self.pop()?;
                    let subr = self.font.subr(n)?;
                    if let Flow::End = self.exec(subr, depth + 1)? {
                        return Some(Flow::End);
                    }
                }
                11 => return Some(Flow::Return),
                13 => {
                    // hsbw: sidebearing point and width.
                    self.sbx = self.arg(0);
                    (self.x, self.y) = (self.sbx, 0.0);
                    self.width = Some(self.arg(1));
                    self.sp = 0;
                    if self.width_only {
                        return Some(Flow::End);
                    }
                }
                14 => {
                    self.close_contour();
                    return Some(Flow::End);
                }
                21 => {
                    self.move_to(self.arg(0), self.arg(1));
                    self.sp = 0;
                }
                22 => {
                    self.move_to(self.arg(0), 0.0);
                    self.sp = 0;
                }
                30 => {
                    let a: [f64; 4] = std::array::from_fn(|k| self.arg(k));
                    self.curve(0.0, a[0], a[1], a[2], a[3], 0.0);
                    self.sp = 0;
                }
                31 => {
                    let a: [f64; 4] = std::array::from_fn(|k| self.arg(k));
                    self.curve(a[0], 0.0, a[1], a[2], 0.0, a[3]);
                    self.sp = 0;
                }
                12 => {
                    let b = *cs.get(i)?;
                    i += 1;
                    if let Flow::End = self.escape(b)? {
                        return Some(Flow::End);
                    }
                }
                _ => self.sp = 0,
            }
        }
        Some(Flow::Return)
    }

    fn escape(&mut self, b: u8) -> Option<Flow> {
        match b {
            // dotsection, vstem3, hstem3: hints only.
            0..=2 => self.sp = 0,
            6 => {
                // seac: asb adx ady bchar achar; it ends the charstring.
                let a: [f64; 5] = std::array::from_fn(|k| self.arg(k));
                self.sp = 0;
                if !self.no_seac {
                    self.seac = Some((a[0], a[1], a[2], a[3], a[4]));
                }
                self.close_contour();
                return Some(Flow::End);
            }
            7 => {
                // sbw: sidebearing point (x, y) and width vector.
                self.sbx = self.arg(0);
                (self.x, self.y) = (self.sbx, self.arg(1));
                self.width = Some(self.arg(2));
                self.sp = 0;
                if self.width_only {
                    return Some(Flow::End);
                }
            }
            12 => {
                let (b, a) = (self.pop()?, self.pop()?);
                self.push(if b == 0.0 { 0.0 } else { a / b })?;
            }
            16 => self.call_other()?,
            17 => {
                let v = self.ps.pop().unwrap_or(0.0);
                self.push(v)?;
            }
            33 => {
                (self.x, self.y) = (self.arg(0), self.arg(1));
                self.sp = 0;
            }
            _ => self.sp = 0,
        }
        Some(Flow::Return)
    }

    /// callothersubr: Flex (0, 1, 2) and hint replacement (3) are done
    /// here; for any other the arguments come back through `pop` in their
    /// original order, which is what fonts expect when the procedure is
    /// unknown.
    fn call_other(&mut self) -> Option<()> {
        let other = self.pop()?;
        let n = self.pop()?;
        if n < 0.0 || n as usize > self.sp {
            return None;
        }
        let n = n as usize;
        let args: Vec<f64> = self.stack[self.sp - n..self.sp].to_vec();
        self.sp -= n;
        self.ps.clear();
        match other as i64 {
            1 => self.flex = Some(Flex { points: Vec::new(), start: (self.x, self.y) }),
            2 => {
                let point = (self.x, self.y);
                if let Some(Flex { points, .. }) = &mut self.flex {
                    if points.len() >= MAX_FLEX_POINTS {
                        return None;
                    }
                    points.push(point);
                }
            }
            0 => {
                if let Some(Flex { points, start }) = self.flex.take() {
                    if !self.open {
                        self.path.move_to(start.0, start.1);
                        self.open = true;
                    }
                    // points[0] is the reference point; 1..=3 and 4..=6 are
                    // the two curves.
                    if points.len() >= 7 {
                        let p = &points;
                        self.path.cubic_to(p[1].0, p[1].1, p[2].0, p[2].1, p[3].0, p[3].1);
                        self.path.cubic_to(p[4].0, p[4].1, p[5].0, p[5].1, p[6].0, p[6].1);
                    } else if let Some(&last) = points.last() {
                        self.path.line_to(last.0, last.1);
                    }
                    if let Some(&last) = points.last() {
                        (self.x, self.y) = last;
                    }
                }
                // The end point goes back for `pop pop setcurrentpoint`.
                if n >= 3 {
                    self.ps.extend([args[2], args[1]]);
                } else {
                    self.ps.extend([self.y, self.x]);
                }
            }
            // Hint replacement: `pop` returns the subroutine number.
            _ => self.ps.extend(args.iter().rev()),
        }
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::truetype::tests::{find_fonts, fuzz, l, m, z};
    use crate::geom::{Point, Seg};

    fn c(x1: f64, y1: f64, x2: f64, y2: f64, x: f64, y: f64) -> Seg {
        Seg::Cubic(Point::new(x1, y1), Point::new(x2, y2), Point::new(x, y))
    }

    /// Assembles a Type 1 charstring from numbers and operator names.
    fn cs(src: &str) -> Vec<u8> {
        let mut out = Vec::new();
        for tok in src.split_whitespace() {
            if let Ok(i) = tok.parse::<i32>() {
                match i {
                    -107..=107 => out.push((i + 139) as u8),
                    108..=1131 => out.extend([((i - 108) >> 8) as u8 + 247, ((i - 108) & 0xFF) as u8]),
                    -1131..=-108 => out.extend([((-i - 108) >> 8) as u8 + 251, ((-i - 108) & 0xFF) as u8]),
                    _ => {
                        out.push(255);
                        out.extend(i.to_be_bytes());
                    }
                }
                continue;
            }
            let code: &[u8] = match tok {
                "hstem" => &[1],
                "vstem" => &[3],
                "vmoveto" => &[4],
                "rlineto" => &[5],
                "hlineto" => &[6],
                "vlineto" => &[7],
                "rrcurveto" => &[8],
                "closepath" => &[9],
                "callsubr" => &[10],
                "return" => &[11],
                "hsbw" => &[13],
                "endchar" => &[14],
                "rmoveto" => &[21],
                "hmoveto" => &[22],
                "vhcurveto" => &[30],
                "hvcurveto" => &[31],
                "dotsection" => &[12, 0],
                "vstem3" => &[12, 1],
                "hstem3" => &[12, 2],
                "seac" => &[12, 6],
                "sbw" => &[12, 7],
                "div" => &[12, 12],
                "callothersubr" => &[12, 16],
                "pop" => &[12, 17],
                "setcurrentpoint" => &[12, 33],
                _ => panic!("unknown operator {tok}"),
            };
            out.extend(code);
        }
        out
    }

    fn encrypt(plain: &[u8], key: u16) -> Vec<u8> {
        let mut r = key;
        plain
            .iter()
            .map(|&p| {
                let c = p ^ (r >> 8) as u8;
                r = (c as u16).wrapping_add(r).wrapping_mul(52845).wrapping_add(22719);
                c
            })
            .collect()
    }

    /// A charstring with its lenIV bytes, encrypted.
    fn enc_cs(src: &str) -> Vec<u8> {
        let mut plain = vec![0x11, 0x22, 0x33, 0x44];
        plain.extend(cs(src));
        encrypt(&plain, CHARSTRING_KEY)
    }

    const CLEAR: &str = "%!PS-AdobeFont-1.0: TestT1 001.000\r%%Title: (test)\r12 dict begin\r/FontName /TestT1 def\r\
        /FontInfo 2 dict dup begin /Notice (Copyright (c) /Encoding [9 9] none\\) here, see eexec) readonly def end readonly def\r\
        /FontMatrix [0.001 0 0 0.001 0 0] readonly def\r/FontBBox {-10 -60 900 880} readonly def\r\
        /Encoding 256 array\r0 1 255 {1 index exch /.notdef put} for\r\
        dup 65 /A put\rdup 97 /square put\rdup 98 /flexed put\rdup 192 /Aacute put\rreadonly def\r\
        currentdict end\rcurrentfile eexec\r";

    /// The private part in the clear (before eexec encryption).
    fn private_part() -> Vec<u8> {
        let subrs = [
            "3 0 callothersubr pop pop setcurrentpoint return",
            "0 1 callothersubr return",
            "0 2 callothersubr return",
            "return",
            "10 0 rlineto return",
            "0 10 hstem return",
            "6 callsubr return",
        ];
        let glyphs = [
            (".notdef", "0 500 hsbw endchar"),
            ("square", "50 600 hsbw 0 0 rmoveto 100 hlineto 100 vlineto -100 hlineto closepath endchar"),
            (
                "curves",
                "0 700 hsbw 10 20 rmoveto 10 20 30 40 50 60 rrcurveto 5 10 15 20 vhcurveto 1 2 3 4 hvcurveto \
                 4 callsubr 300 3 div 0 rlineto 5 1 3 callothersubr pop callsubr closepath 30 40 rmoveto \
                 0 10 rlineto endchar",
            ),
            (
                "flexed",
                "0 800 hsbw 0 0 rmoveto 1 callsubr 50 0 rmoveto 2 callsubr -40 10 rmoveto 2 callsubr \
                 20 10 rmoveto 2 callsubr 20 0 rmoveto 2 callsubr 20 0 rmoveto 2 callsubr 20 -10 rmoveto \
                 2 callsubr 20 -10 rmoveto 2 callsubr 50 110 0 0 callsubr 0 -50 rlineto closepath endchar",
            ),
            ("A", "0 600 hsbw 0 0 rmoveto 300 600 rlineto 300 -600 rlineto closepath endchar"),
            ("acute", "100 300 hsbw 0 700 rmoveto 100 100 rlineto closepath endchar"),
            ("Aacute", "20 600 hsbw 100 150 50 65 194 seac"),
            ("implicit", "0 500 hsbw 0 0 rmoveto 100 0 rlineto 0 100 rmoveto 50 0 rlineto endchar"),
            (
                "sbwglyph",
                "10 20 700 0 sbw 0 0 rmoveto 1 2 3 4 5 6 hstem3 dotsection 7 8 9 10 11 12 vstem3 \
                 50 0 rlineto 0 5 vstem 4 vmoveto 3 hmoveto 1 1 rlineto endchar",
            ),
            ("deep", "0 500 hsbw 0 0 rmoveto 6 callsubr endchar"),
            ("big", "-2000 70000 hsbw 70000 -70000 rmoveto 1 1 rlineto endchar"),
        ];
        let mut p = b"\x51\x52\x53\x54dup /Private 9 dict dup begin\n/RD{string currentfile exch readstring pop}executeonly def\n\
            /ND{noaccess def}executeonly def\n/NP{noaccess put}executeonly def\n/-|{string currentfile exch readstring pop}executeonly def\n\
            /MinFeature{16 16} def\n/password 5839 def\n/BlueValues [-20 0 600 620] def\n/lenIV 4 def\n\
            /OtherSubrs [ {} {} {} {systemdict /internaldict known not {pop 3} {1183615869 systemdict /internaldict get exec dup /startlock known {/startlock get exec} {dup /strtlck known {/strtlck get exec} {pop 3} ifelse} ifelse} ifelse} executeonly ] ND\n"
            .to_vec();
        p.extend(format!("/Subrs {} array\n", subrs.len()).as_bytes());
        for (i, s) in subrs.iter().enumerate() {
            let e = enc_cs(s);
            let (rd, np) = [("RD", "NP"), ("-|", "|"), ("RD", "noaccess put")][i % 3];
            p.extend(format!("dup {i} {} {rd} ", e.len()).as_bytes());
            p.extend(e);
            p.extend(format!(" {np}\n").as_bytes());
        }
        p.extend(b"ND\n2 index /CharStrings ");
        p.extend(format!("{} dict dup begin\n", glyphs.len()).as_bytes());
        for (i, (name, s)) in glyphs.iter().enumerate() {
            let e = enc_cs(s);
            let (rd, nd) = [("RD", "ND"), ("-|", "|-"), ("RD", "noaccess def")][i % 3];
            p.extend(format!("/{name} {} {rd} ", e.len()).as_bytes());
            p.extend(e);
            p.extend(format!(" {nd}\n").as_bytes());
        }
        p.extend(b"end\nend\nreadonly put\nnoaccess put\ndup /FontName get exch definefont pop\nmark currentfile closefile\n");
        p
    }

    const TRAILER: &str = "\n0000000000000000000000000000000000000000000000000000000000000000\ncleartomark\n";

    /// PDF FontFile layout: cleartext, binary eexec part, trailer.
    fn binary_font() -> Vec<u8> {
        let mut out = CLEAR.as_bytes().to_vec();
        out.extend(encrypt(&private_part(), EEXEC_KEY));
        out.extend(TRAILER.as_bytes());
        out
    }

    fn hex_font() -> Vec<u8> {
        let mut out = CLEAR.replace('\r', "\n").into_bytes();
        for (i, b) in encrypt(&private_part(), EEXEC_KEY).iter().enumerate() {
            out.extend(format!("{b:02x}").as_bytes());
            if i % 32 == 31 {
                out.push(b'\n');
            }
        }
        out.extend(TRAILER.as_bytes());
        out
    }

    fn pfb_font() -> Vec<u8> {
        let mut out = Vec::new();
        let enc = encrypt(&private_part(), EEXEC_KEY);
        for (kind, seg) in [(1u8, CLEAR.as_bytes()), (2, &enc[..]), (1, TRAILER.as_bytes())] {
            out.extend([0x80, kind]);
            out.extend((seg.len() as u32).to_le_bytes());
            out.extend(seg);
        }
        out.extend([0x80, 3]);
        out
    }

    fn segs(font: &Type1, name: &str) -> Vec<Seg> {
        font.glyph_path(name).unwrap_or_else(|| panic!("glyph {name}")).segs
    }

    #[test]
    fn layouts_parse_alike() {
        for data in [binary_font(), hex_font(), pfb_font()] {
            let font = Type1::parse(&data).unwrap();
            assert_eq!(font.font_matrix(), [0.001, 0.0, 0.0, 0.001, 0.0, 0.0]);
            assert_eq!(font.font_bbox(), Some([-10.0, -60.0, 900.0, 880.0]));
            assert_eq!(font.glyph_names().len(), 11);
            assert_eq!(font.glyph_names()[1], "square");
            assert_eq!(segs(&font, "square"), vec![m(50.0, 0.0), l(150.0, 0.0), l(150.0, 100.0), l(50.0, 100.0), z()]);
            let enc = font.encoding();
            assert_eq!(enc.len(), 256);
            assert_eq!(enc[65].as_deref(), Some("A"));
            assert_eq!(enc[97].as_deref(), Some("square"));
            assert_eq!(enc[192].as_deref(), Some("Aacute"));
            assert_eq!(enc[66], None);
            assert_eq!(enc[32], None);
        }
    }

    #[test]
    fn charstring_operators() {
        let font = Type1::parse(&binary_font()).unwrap();
        assert_eq!(font.glyph_path(".notdef"), Some(Path::new()));
        assert_eq!(
            segs(&font, "curves"),
            vec![
                m(10.0, 20.0),
                c(20.0, 40.0, 50.0, 80.0, 100.0, 140.0),
                c(100.0, 145.0, 110.0, 160.0, 130.0, 160.0),
                c(131.0, 160.0, 133.0, 163.0, 133.0, 167.0),
                l(143.0, 167.0),
                l(243.0, 167.0),
                z(),
                m(273.0, 207.0),
                l(273.0, 217.0),
                z(),
            ]
        );
        assert_eq!(
            segs(&font, "implicit"),
            vec![m(0.0, 0.0), l(100.0, 0.0), z(), m(100.0, 100.0), l(150.0, 100.0), z()]
        );
        assert_eq!(segs(&font, "sbwglyph"), vec![m(10.0, 20.0), l(60.0, 20.0), z(), m(63.0, 24.0), l(64.0, 25.0), z()]);
        assert_eq!(segs(&font, "big"), vec![m(68000.0, -70000.0), l(68001.0, -69999.0), z()]);
        assert_eq!(font.glyph_path("deep"), None);
        assert_eq!(font.glyph_path("missing"), None);
    }

    #[test]
    fn flex_and_seac() {
        let font = Type1::parse(&pfb_font()).unwrap();
        assert_eq!(
            segs(&font, "flexed"),
            vec![
                m(0.0, 0.0),
                c(10.0, 10.0, 30.0, 20.0, 50.0, 20.0),
                c(70.0, 20.0, 90.0, 10.0, 110.0, 0.0),
                l(110.0, -50.0),
                z(),
            ]
        );
        // The accent lands at adx - asb + sbx = 150 - 100 + 20.
        assert_eq!(
            segs(&font, "Aacute"),
            vec![m(0.0, 0.0), l(300.0, 600.0), l(600.0, 0.0), z(), m(170.0, 750.0), l(270.0, 850.0), z()]
        );
    }

    #[test]
    fn widths() {
        let font = Type1::parse(&hex_font()).unwrap();
        assert_eq!(font.advance_width(".notdef"), Some(500.0));
        assert_eq!(font.advance_width("square"), Some(600.0));
        assert_eq!(font.advance_width("Aacute"), Some(600.0));
        assert_eq!(font.advance_width("sbwglyph"), Some(700.0));
        assert_eq!(font.advance_width("big"), Some(70000.0));
        assert_eq!(font.advance_width("missing"), None);
    }

    #[test]
    fn standard_encoding_and_cleartext_fonts() {
        // No custom encoding, no eexec, lenIV -1: everything in the clear.
        let mut data = b"%!FontType1\n/FontName /Plain def\n/Encoding StandardEncoding def\n/lenIV -1 def\n\
            /Subrs 0 array\n/CharStrings 1 dict dup begin\n"
            .to_vec();
        let g = cs("0 250 hsbw 0 0 rmoveto 10 10 rlineto endchar");
        data.extend(format!("/space {} RD ", g.len()).as_bytes());
        data.extend(g);
        data.extend(b" ND\nend\n");
        let font = Type1::parse(&data).unwrap();
        assert_eq!(font.encoding()[32].as_deref(), Some("space"));
        assert_eq!(font.encoding()[0x27].as_deref(), Some("quoteright"));
        assert_eq!(font.font_matrix(), DEFAULT_MATRIX);
        assert_eq!(segs(&font, "space"), vec![m(0.0, 0.0), l(10.0, 10.0), z()]);
        assert!(Type1::parse(b"%!PS-AdobeFont-1.0 nothing here").is_err());
        assert!(Type1::parse(&[]).is_err());
        assert!(Type1::parse(&[0x80, 1, 0, 0, 0, 0]).is_err());
    }

    fn exercise(font: &Type1) {
        for name in font.glyph_names().iter().take(32) {
            let _ = font.glyph_path(name);
            let _ = font.advance_width(name);
        }
        let _ = font.encoding();
        let _ = format!("{font:?}");
    }

    #[test]
    fn hostile_inputs_never_panic() {
        for data in [binary_font(), hex_font(), pfb_font()] {
            fuzz(&data, 5_000, |d| {
                if let Ok(font) = Type1::parse(&d) {
                    exercise(&font);
                }
            });
            for cut in (0..data.len()).step_by(3) {
                if let Ok(font) = Type1::parse(&data[..cut]) {
                    exercise(&font);
                }
            }
            for i in 0..data.len() {
                for v in [0x00, 0xFF, b' ', b'/', b'('] {
                    let mut d = data.clone();
                    d[i] = v;
                    if let Ok(font) = Type1::parse(&d) {
                        exercise(&font);
                    }
                }
            }
        }
    }

    #[test]
    fn system_type1_fonts() {
        let dirs = ["/usr/share/fonts", "/usr/share/ghostscript/Resource/Font"];
        let mut files = find_fonts(&dirs, &["pfb", "pfa", "t1"], 8);
        // Ghostscript ships the URW base 35 as PFA files without extension.
        for name in ["NimbusSans-Regular", "C059-Roman", "NimbusMonoPS-Regular", "StandardSymbolsPS", "D050000L"] {
            files.push(std::path::PathBuf::from(format!("/usr/share/ghostscript/Resource/Font/{name}")));
        }
        for (n, path) in files.into_iter().enumerate() {
            let Ok(data) = std::fs::read(&path) else {
                continue;
            };
            if n == 0 {
                for k in 1..8 {
                    if let Ok(font) = Type1::parse(&data[..data.len() * k / 8]) {
                        font.glyph_names().iter().for_each(|g| drop(font.glyph_path(g)));
                    }
                }
            }
            let font = Type1::parse(&data).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let names = font.glyph_names();
            let mut drawn = 0;
            for name in &names {
                if font.glyph_path(name).is_some_and(|p| !p.is_empty()) {
                    drawn += 1;
                }
                assert!(font.advance_width(name).is_some(), "{} {name}", path.display());
            }
            assert!(drawn > names.len() / 2, "{}: {drawn} of {}", path.display(), names.len());
            let bbox = font.font_bbox().unwrap();
            for name in ["A", "o", "g", "Aacute"] {
                let Some(p) = font.glyph_path(name) else {
                    continue;
                };
                let (x0, y0, x1, y1) = p.bounds().unwrap();
                assert!(x0 >= bbox[0] - 50.0 && y0 >= bbox[1] - 50.0, "{} {name}", path.display());
                assert!(x1 <= bbox[2] + 50.0 && y1 <= bbox[3] + 50.0, "{} {name}", path.display());
                assert!(x1 > x0 && y1 > y0);
            }
        }
    }
}
