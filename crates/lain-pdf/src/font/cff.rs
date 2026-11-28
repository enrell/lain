//! CFF (Compact Font Format, Adobe TN #5176) font programs with Type 2
//! charstrings (TN #5177): the PDF `FontFile3` streams `/Type1C` and
//! `/CIDFontType0C`, and the `CFF ` table of OpenType fonts.
//!
//! Parsing reads the INDEX headers, the DICTs and the charset; a glyph's
//! charstring is interpreted only when its outline or width is asked for.
//! Every read is bounds-checked and the interpreter has hard limits
//! (stack, subroutine depth, operator budget, path size), so a hostile
//! font costs bounded time and fails per glyph.

use super::FontError;
use super::encoding::STANDARD;
use crate::geom::{Matrix, Path, Seg};
use std::sync::OnceLock;

/// Subroutine nesting limit (the Type 2 spec says 10).
const MAX_SUBR_DEPTH: usize = 10;
/// Argument stack size: 48 in Type 2, 513 in CFF2; the larger value
/// only makes broken fonts work, it costs nothing.
const MAX_STACK: usize = 513;
/// Path segments one glyph may produce before it is refused.
const MAX_SEGMENTS: usize = 20_000;
/// Operators and operands one glyph may execute: subroutines can call
/// each other many times over, and this bounds the work (real CJK
/// glyphs need about 2 500).
const MAX_OPS: usize = 100_000;
/// Font DICTs read from an FDArray (FDSelect stores 8-bit indices).
const MAX_FDS: usize = 256;

const DEFAULT_MATRIX: [f64; 6] = [0.001, 0.0, 0.0, 0.001, 0.0, 0.0];

fn err(msg: &str) -> FontError {
    FontError(format!("cff: {msg}"))
}

fn be_u16(d: &[u8], off: usize) -> Option<u16> {
    let b = d.get(off..off.checked_add(2)?)?;
    Some(u16::from_be_bytes([b[0], b[1]]))
}

/// An INDEX: `count` objects whose byte ranges come from the offset array.
#[derive(Debug, Clone, Copy, Default)]
struct Index {
    count: usize,
    off_size: usize,
    /// Position of the offset array.
    offsets: usize,
    /// Offsets are relative to this position (the byte before the data).
    base: usize,
    /// First byte after the INDEX.
    end: usize,
}

impl Index {
    fn parse(d: &[u8], pos: usize) -> Option<Index> {
        let count = be_u16(d, pos)? as usize;
        if count == 0 {
            return Some(Index { end: pos + 2, ..Index::default() });
        }
        let off_size = *d.get(pos + 2)? as usize;
        if !(1..=4).contains(&off_size) {
            return None;
        }
        let offsets = pos + 3;
        let base = offsets.checked_add((count + 1) * off_size)? - 1;
        let mut idx = Index { count, off_size, offsets, base, end: 0 };
        let last = idx.offset(d, count)?;
        idx.end = base.checked_add(last)?;
        Some(idx)
    }

    fn offset(&self, d: &[u8], i: usize) -> Option<usize> {
        let at = self.offsets.checked_add(i.checked_mul(self.off_size)?)?;
        let b = d.get(at..at.checked_add(self.off_size)?)?;
        Some(b.iter().fold(0usize, |v, &x| (v << 8) | x as usize))
    }

    fn get<'a>(&self, d: &'a [u8], i: usize) -> Option<&'a [u8]> {
        if i >= self.count {
            return None;
        }
        let (a, b) = (self.offset(d, i)?, self.offset(d, i + 1)?);
        if a == 0 || b < a {
            return None;
        }
        d.get(self.base.checked_add(a)?..self.base.checked_add(b)?)
    }
}

/// DICT operators, escaped ones as `0x0C00 | b1`.
mod op {
    pub const FONT_BBOX: u16 = 5;
    pub const CHARSET: u16 = 15;
    pub const ENCODING: u16 = 16;
    pub const CHAR_STRINGS: u16 = 17;
    pub const PRIVATE: u16 = 18;
    pub const SUBRS: u16 = 19;
    pub const DEFAULT_WIDTH_X: u16 = 20;
    pub const NOMINAL_WIDTH_X: u16 = 21;
    pub const CHARSTRING_TYPE: u16 = 0x0C06;
    pub const FONT_MATRIX: u16 = 0x0C07;
    pub const ROS: u16 = 0x0C1E;
    pub const FD_ARRAY: u16 = 0x0C24;
    pub const FD_SELECT: u16 = 0x0C25;
}

/// Calls `f(operator, operands)` for each DICT entry. At most 48 operands
/// are kept per operator (the spec's limit); the rest are dropped.
fn parse_dict(d: &[u8], mut f: impl FnMut(u16, &[f64])) {
    let mut operands = [0.0f64; 48];
    let mut n = 0;
    let mut i = 0;
    while i < d.len() {
        let b0 = d[i];
        let (value, len) = match b0 {
            0..=21 => {
                let (code, len) = if b0 == 12 {
                    match d.get(i + 1) {
                        Some(&b1) => (0x0C00 | b1 as u16, 2),
                        None => return,
                    }
                } else {
                    (b0 as u16, 1)
                };
                f(code, &operands[..n]);
                n = 0;
                i += len;
                continue;
            }
            28 => match d.get(i + 1..i + 3) {
                Some(b) => (i16::from_be_bytes([b[0], b[1]]) as f64, 3),
                None => return,
            },
            29 => match d.get(i + 1..i + 5) {
                Some(b) => (i32::from_be_bytes([b[0], b[1], b[2], b[3]]) as f64, 5),
                None => return,
            },
            30 => {
                let (v, len) = parse_real(&d[i + 1..]);
                (v, len + 1)
            }
            32..=246 => (b0 as f64 - 139.0, 1),
            247..=250 => match d.get(i + 1) {
                Some(&b1) => ((b0 as f64 - 247.0) * 256.0 + b1 as f64 + 108.0, 2),
                None => return,
            },
            251..=254 => match d.get(i + 1) {
                Some(&b1) => (-(b0 as f64 - 251.0) * 256.0 - b1 as f64 - 108.0, 2),
                None => return,
            },
            // Reserved bytes: skip them, they carry no operand.
            _ => {
                i += 1;
                continue;
            }
        };
        if n < operands.len() {
            operands[n] = value;
            n += 1;
        }
        i += len;
    }
}

/// A DICT real (packed BCD nibbles up to the 0xF terminator) and the
/// bytes it used. Unparsable reals read as 0.
fn parse_real(d: &[u8]) -> (f64, usize) {
    let mut s = String::new();
    let mut used = 0;
    'bytes: for &b in d {
        used += 1;
        for nib in [b >> 4, b & 0x0F] {
            match nib {
                0..=9 => s.push((b'0' + nib) as char),
                0xA => s.push('.'),
                0xB => s.push('E'),
                0xC => s.push_str("E-"),
                0xE => s.push('-'),
                0xF => break 'bytes,
                _ => {}
            }
        }
        if s.len() > 64 {
            break;
        }
    }
    (s.parse::<f64>().ok().filter(|v| v.is_finite()).unwrap_or(0.0), used)
}

/// A DICT operand used as an offset or count.
fn to_usize(v: f64) -> Option<usize> {
    (v.is_finite() && v >= 0.0 && v <= u32::MAX as f64).then_some(v as usize)
}

fn matrix6(ops: &[f64]) -> Option<[f64; 6]> {
    let m: [f64; 6] = ops.get(..6)?.try_into().ok()?;
    // A singular matrix would make every glyph vanish; ignore it.
    (m.iter().all(|v| v.is_finite()) && (m[0] * m[3] - m[1] * m[2]).abs() > 1e-12).then_some(m)
}

/// The Top DICT entries this module uses.
#[derive(Default)]
struct TopDict {
    charset: usize,
    encoding: usize,
    char_strings: Option<usize>,
    private: Option<(usize, usize)>,
    font_matrix: Option<[f64; 6]>,
    charstring_type: f64,
    ros: bool,
    fd_array: Option<usize>,
    fd_select: Option<usize>,
    bbox: Option<[f64; 4]>,
}

fn read_top(d: &[u8]) -> TopDict {
    let mut t = TopDict { charstring_type: 2.0, ..TopDict::default() };
    parse_dict(d, |code, ops| {
        let first = ops.first().copied().and_then(to_usize);
        match code {
            op::CHARSET => t.charset = first.unwrap_or(0),
            op::ENCODING => t.encoding = first.unwrap_or(0),
            op::CHAR_STRINGS => t.char_strings = first,
            op::PRIVATE if ops.len() >= 2 => {
                t.private = to_usize(ops[0]).zip(to_usize(ops[1]));
            }
            op::FONT_MATRIX => t.font_matrix = matrix6(ops),
            op::CHARSTRING_TYPE => t.charstring_type = ops.first().copied().unwrap_or(2.0),
            op::ROS => t.ros = true,
            op::FD_ARRAY => t.fd_array = first,
            op::FD_SELECT => t.fd_select = first,
            op::FONT_BBOX if ops.len() >= 4 => t.bbox = Some([ops[0], ops[1], ops[2], ops[3]]),
            _ => {}
        }
    });
    t
}

/// What a glyph's charstring needs from its (Font DICT's) Private DICT.
#[derive(Debug, Clone, Default)]
struct PrivateInfo {
    subrs: Index,
    default_width: f64,
    nominal_width: f64,
    /// Maps this FD's glyph space onto the space `font_matrix` expects,
    /// when its FontMatrix differs from the first FD's.
    adjust: Option<Matrix>,
}

fn read_private(d: &[u8], size_offset: Option<(usize, usize)>) -> PrivateInfo {
    let mut info = PrivateInfo::default();
    let Some((size, offset)) = size_offset else {
        return info;
    };
    let Some(dict) = d.get(offset..offset.saturating_add(size).min(d.len())) else {
        return info;
    };
    let mut subrs = None;
    parse_dict(dict, |code, ops| match code {
        op::SUBRS => subrs = ops.first().copied().and_then(to_usize),
        op::DEFAULT_WIDTH_X => info.default_width = ops.first().copied().unwrap_or(0.0),
        op::NOMINAL_WIDTH_X => info.nominal_width = ops.first().copied().unwrap_or(0.0),
        _ => {}
    });
    // Subrs is relative to the Private DICT; a bad one just means none.
    if let Some(index) = subrs.and_then(|s| offset.checked_add(s)).and_then(|p| Index::parse(d, p)) {
        info.subrs = index;
    }
    info
}

#[derive(Debug, Clone, Copy)]
enum FdSelect {
    Format0(usize),
    Format3 { ranges: usize, count: usize },
}

#[derive(Debug, Clone, Copy)]
enum EncodingKind {
    Standard,
    Expert,
    Custom(usize),
}

/// A CFF font: the first font of the FontSet. It owns the data.
pub struct Cff {
    data: Vec<u8>,
    char_strings: Index,
    gsubrs: Index,
    strings: Index,
    /// gid -> SID (name-keyed) or CID (CID-keyed).
    charset: Vec<u16>,
    encoding: EncodingKind,
    cid: bool,
    type2: bool,
    fds: Vec<PrivateInfo>,
    fd_select: Option<FdSelect>,
    font_matrix: [f64; 6],
    bbox: Option<[f64; 4]>,
    names: OnceLock<Vec<(String, u16)>>,
    cids: OnceLock<Vec<u16>>,
}

impl std::fmt::Debug for Cff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cff")
            .field("bytes", &self.data.len())
            .field("glyphs", &self.char_strings.count)
            .field("cid", &self.cid)
            .field("fds", &self.fds.len())
            .field("font_matrix", &self.font_matrix)
            .finish()
    }
}

/// How a charstring run ended.
enum Flow {
    Return,
    End,
}

impl Cff {
    /// A bare CFF: PDF `FontFile3` with `/Type1C` or `/CIDFontType0C`.
    /// The first font of the FontSet is used.
    pub fn parse(data: Vec<u8>) -> Result<Cff, FontError> {
        let d = &data;
        let (&major, &hdr_size) = (d.first().ok_or_else(|| err("empty"))?, d.get(2).ok_or_else(|| err("no header"))?);
        if major != 1 {
            return Err(err("not a CFF version 1 font"));
        }
        let names = Index::parse(d, hdr_size as usize).ok_or_else(|| err("bad Name INDEX"))?;
        let tops = Index::parse(d, names.end).ok_or_else(|| err("bad Top DICT INDEX"))?;
        let strings = Index::parse(d, tops.end);
        let gsubrs = strings.and_then(|s| Index::parse(d, s.end)).unwrap_or_default();
        let strings = strings.unwrap_or_default();
        let top = read_top(tops.get(d, 0).ok_or_else(|| err("no Top DICT"))?);
        let cs_offset = top.char_strings.ok_or_else(|| err("no CharStrings"))?;
        let char_strings = Index::parse(d, cs_offset).ok_or_else(|| err("bad CharStrings INDEX"))?;
        if char_strings.count == 0 {
            return Err(err("no glyphs"));
        }
        let n = char_strings.count;
        let cid = top.ros;
        let charset = read_charset(d, top.charset, n, cid);

        let mut font_matrix = top.font_matrix.unwrap_or(DEFAULT_MATRIX);
        let mut fds = Vec::new();
        let mut fd_select = None;
        if cid && let Some(fd_index) = top.fd_array.and_then(|o| Index::parse(d, o)) {
            let mut matrices = Vec::new();
            for i in 0..fd_index.count.min(MAX_FDS) {
                let (mut private, mut matrix) = (None, None);
                if let Some(dict) = fd_index.get(d, i) {
                    parse_dict(dict, |code, ops| match code {
                        op::PRIVATE if ops.len() >= 2 => private = to_usize(ops[0]).zip(to_usize(ops[1])),
                        op::FONT_MATRIX => matrix = matrix6(ops),
                        _ => {}
                    });
                }
                fds.push(read_private(d, private));
                // The FD's matrix applies first, then the Top DICT's; the
                // Top DICT's default only counts when the FD has none.
                let m = match (matrix, top.font_matrix) {
                    (Some(f), Some(t)) => Matrix::from_array(f).then(&Matrix::from_array(t)),
                    (Some(f), None) => Matrix::from_array(f),
                    (None, t) => Matrix::from_array(t.unwrap_or(DEFAULT_MATRIX)),
                };
                matrices.push(m);
            }
            if let Some(&m0) = matrices.first() {
                font_matrix = [m0.a, m0.b, m0.c, m0.d, m0.e, m0.f];
                if let Some(inv) = m0.invert() {
                    for (fd, m) in fds.iter_mut().zip(&matrices) {
                        if *m != m0 {
                            fd.adjust = Some(m.then(&inv));
                        }
                    }
                }
            }
            fd_select = top.fd_select.and_then(|o| read_fd_select(d, o, n));
        }
        if fds.is_empty() {
            // Name-keyed, or a CID font without a usable FDArray.
            fds.push(read_private(d, top.private));
        }
        let encoding = match top.encoding {
            0 => EncodingKind::Standard,
            1 => EncodingKind::Expert,
            o => EncodingKind::Custom(o),
        };
        Ok(Cff {
            char_strings,
            gsubrs,
            strings,
            charset,
            encoding,
            cid,
            type2: top.charstring_type == 2.0,
            fds,
            fd_select,
            font_matrix,
            bbox: top.bbox,
            names: OnceLock::new(),
            cids: OnceLock::new(),
            data,
        })
    }

    /// The `CFF ` table of an OpenType (`OTTO`) font, or of the first face
    /// of a collection.
    pub fn from_opentype(data: &[u8]) -> Result<Cff, FontError> {
        let table = super::truetype::sfnt_table(data, b"CFF ").ok_or_else(|| err("no CFF table"))?;
        Cff::parse(table.to_vec())
    }

    /// The FontMatrix mapping glyph space to text space: the Top DICT's
    /// (default `[0.001 0 0 0.001 0 0]`), or for CID fonts the first Font
    /// DICT's matrix followed by the Top DICT's. Glyphs of a Font DICT with
    /// another matrix are returned already mapped into this one's space.
    pub fn font_matrix(&self) -> [f64; 6] {
        self.font_matrix
    }

    /// The Top DICT FontBBox, in glyph space.
    pub fn font_bbox(&self) -> Option<[f64; 4]> {
        self.bbox
    }

    pub fn is_cid(&self) -> bool {
        self.cid
    }

    pub fn num_glyphs(&self) -> u16 {
        self.char_strings.count.min(u16::MAX as usize) as u16
    }

    fn sid_name(&self, sid: u16) -> Option<String> {
        let sid = sid as usize;
        if sid < STANDARD_STRINGS.len() {
            return Some(STANDARD_STRINGS[sid].to_string());
        }
        let s = self.strings.get(&self.data, sid - STANDARD_STRINGS.len())?;
        Some(String::from_utf8_lossy(s).into_owned())
    }

    /// The glyph name (name-keyed fonts only).
    pub fn glyph_name(&self, gid: u16) -> Option<String> {
        if self.cid {
            return None;
        }
        self.sid_name(*self.charset.get(gid as usize)?)
    }

    /// The glyph with this name (name-keyed fonts, via charset and strings).
    pub fn gid_for_name(&self, name: &str) -> Option<u16> {
        if self.cid {
            return None;
        }
        let names = self.names.get_or_init(|| {
            let mut v: Vec<(String, u16)> = (0..self.charset.len())
                .filter_map(|gid| Some((self.sid_name(self.charset[gid])?, gid as u16)))
                .collect();
            v.sort();
            v.dedup_by(|b, a| a.0 == b.0);
            v
        });
        let i = names.binary_search_by(|e| e.0.as_str().cmp(name)).ok()?;
        Some(names[i].1)
    }

    /// CID-keyed: the glyph the charset gives this CID. Name-keyed: the
    /// CID is the glyph index, when it is in range.
    pub fn gid_for_cid(&self, cid: u16) -> Option<u16> {
        if !self.cid {
            return (cid < self.num_glyphs()).then_some(cid);
        }
        let map = self.cids.get_or_init(|| {
            let max = self.charset.iter().copied().max().unwrap_or(0) as usize;
            let mut map = vec![u16::MAX; max + 1];
            for (gid, &c) in self.charset.iter().enumerate().rev() {
                map[c as usize] = gid as u16;
            }
            map
        });
        map.get(cid as usize).copied().filter(|&g| g != u16::MAX)
    }

    /// The built-in encoding as code -> glyph name: Standard or Expert, or
    /// the font's own (formats 0 and 1, with supplements). All `None` for
    /// CID-keyed fonts, which have none.
    pub fn encoding(&self) -> Vec<Option<String>> {
        let mut out = vec![None; 256];
        if self.cid {
            return out;
        }
        match self.encoding {
            EncodingKind::Standard => {
                for (slot, name) in out.iter_mut().zip(STANDARD.iter()) {
                    *slot = name.map(str::to_string);
                }
            }
            EncodingKind::Expert => {
                for (slot, &sid) in out.iter_mut().zip(EXPERT_ENCODING.iter()) {
                    if sid != 0 {
                        *slot = self.sid_name(sid);
                    }
                }
            }
            EncodingKind::Custom(o) => self.custom_encoding(o, &mut out),
        }
        out
    }

    fn custom_encoding(&self, o: usize, out: &mut [Option<String>]) {
        let d = &self.data;
        let Some(&format) = d.get(o) else {
            return;
        };
        let mut pos = o + 1;
        let mut gid = 1usize;
        let set = |code: usize, gid: usize, out: &mut [Option<String>]| {
            if let (Some(slot), Some(&sid)) = (out.get_mut(code), self.charset.get(gid)) {
                *slot = self.sid_name(sid);
            }
        };
        match format & 0x7F {
            0 => {
                let n = d.get(pos).copied().unwrap_or(0) as usize;
                for i in 0..n {
                    let Some(&code) = d.get(pos + 1 + i) else {
                        break;
                    };
                    set(code as usize, gid, out);
                    gid += 1;
                }
                pos += 1 + n;
            }
            1 => {
                let n = d.get(pos).copied().unwrap_or(0) as usize;
                for i in 0..n {
                    let (Some(&first), Some(&left)) = (d.get(pos + 1 + 2 * i), d.get(pos + 2 + 2 * i)) else {
                        break;
                    };
                    for code in first as usize..=first as usize + left as usize {
                        set(code, gid, out);
                        gid += 1;
                    }
                }
                pos += 1 + 2 * n;
            }
            _ => return,
        }
        if format & 0x80 != 0 {
            let n = d.get(pos).copied().unwrap_or(0) as usize;
            for i in 0..n {
                let at = pos + 1 + 3 * i;
                let (Some(&code), Some(sid)) = (d.get(at), be_u16(d, at + 1)) else {
                    break;
                };
                out[code as usize] = self.sid_name(sid);
            }
        }
    }

    /// The Private DICT data of a glyph's Font DICT (`None` when FDSelect
    /// names a Font DICT that does not exist).
    fn private_for(&self, gid: u16) -> Option<&PrivateInfo> {
        let fd = match self.fd_select {
            None => 0,
            Some(FdSelect::Format0(o)) => *self.data.get(o.checked_add(1 + gid as usize)?)? as usize,
            Some(FdSelect::Format3 { ranges, count }) => {
                // The last range starting at or before gid.
                let first = |i: usize| be_u16(&self.data, ranges + 3 * i);
                let (mut lo, mut hi) = (0, count);
                while lo < hi {
                    let mid = (lo + hi) / 2;
                    if first(mid)? <= gid {
                        lo = mid + 1;
                    } else {
                        hi = mid;
                    }
                }
                *self.data.get(ranges + 3 * lo.checked_sub(1)? + 2)? as usize
            }
        };
        self.fds.get(fd)
    }

    /// The outline of a glyph in glyph space (before the font matrix), y
    /// up, with cubic curves; every contour is closed. `None` when the glyph
    /// is out of range or its charstring is broken.
    pub fn glyph_path(&self, gid: u16) -> Option<Path> {
        let private = self.private_for(gid)?;
        let mut t2 = Interp::new(self, private, false);
        t2.run_glyph(gid)?;
        let mut path = std::mem::take(&mut t2.path);
        if let Some((adx, ady, bchar, achar)) = t2.seac {
            // Accented character: base and accent by StandardEncoding code.
            let base = self.seac_component(bchar)?;
            let accent = self.seac_component(achar)?;
            path.segs.extend(base.segs);
            path.extend_transformed(&accent, &Matrix::translate(adx, ady));
            if path.segs.len() > MAX_SEGMENTS {
                return None;
            }
        }
        Some(match private.adjust {
            Some(m) => path.transformed(&m),
            None => path,
        })
    }

    fn seac_component(&self, code: f64) -> Option<Path> {
        if self.cid || !(0.0..256.0).contains(&code) {
            return None;
        }
        let gid = self.gid_for_name(STANDARD[code as usize]?)?;
        let mut t2 = Interp::new(self, self.private_for(gid)?, false);
        t2.no_seac = true;
        t2.run_glyph(gid)?;
        Some(t2.path)
    }

    /// The advance width in glyph space: the charstring's optional first
    /// operand plus nominalWidthX, else defaultWidthX.
    pub fn advance_width(&self, gid: u16) -> Option<f64> {
        let private = self.private_for(gid)?;
        let mut t2 = Interp::new(self, private, true);
        t2.run_glyph(gid)?;
        let w = match t2.width {
            Some(w) => private.nominal_width + w,
            None => private.default_width,
        };
        Some(match private.adjust {
            Some(m) => m.apply_vector(w, 0.0).x,
            None => w,
        })
    }
}

/// gid -> SID/CID from the charset at `offset` (0, 1, 2: predefined).
fn read_charset(d: &[u8], offset: usize, n: usize, cid: bool) -> Vec<u16> {
    let predefined: Option<&[u16]> = match (offset, cid) {
        (0, false) => return (0..n.min(229) as u16).collect(),
        (1, false) => Some(&EXPERT_CHARSET),
        (2, false) => Some(&EXPERT_SUBSET_CHARSET),
        // A CID font must have its own charset; without one, CID = gid.
        (0..=2, true) => return (0..n.min(u16::MAX as usize + 1)).map(|g| g as u16).collect(),
        _ => None,
    };
    if let Some(p) = predefined {
        return p.iter().copied().take(n).collect();
    }
    let mut out = vec![0u16];
    let Some(&format) = d.get(offset) else {
        return out;
    };
    let mut pos = offset + 1;
    match format {
        0 => {
            while out.len() < n {
                let Some(v) = be_u16(d, pos) else {
                    break;
                };
                out.push(v);
                pos += 2;
            }
        }
        1 | 2 => {
            while out.len() < n {
                let first = be_u16(d, pos);
                let left = if format == 1 { d.get(pos + 2).map(|&b| b as u16) } else { be_u16(d, pos + 2) };
                let (Some(first), Some(left)) = (first, left) else {
                    break;
                };
                pos += if format == 1 { 3 } else { 4 };
                for k in 0..=left {
                    if out.len() >= n {
                        break;
                    }
                    out.push(first.wrapping_add(k));
                }
            }
        }
        _ => {}
    }
    out
}

fn read_fd_select(d: &[u8], offset: usize, n: usize) -> Option<FdSelect> {
    match *d.get(offset)? {
        0 => d.get(offset + n).map(|_| FdSelect::Format0(offset)),
        3 => {
            let count = be_u16(d, offset + 1)? as usize;
            let ranges = offset + 3;
            // Keep only the ranges that are present.
            let count = count.min(d.len().saturating_sub(ranges) / 3);
            (count > 0).then_some(FdSelect::Format3 { ranges, count })
        }
        _ => None,
    }
}

fn subr_bias(count: usize) -> i64 {
    if count < 1240 {
        107
    } else if count < 33900 {
        1131
    } else {
        32768
    }
}

/// The Type 2 charstring interpreter for one glyph.
struct Interp<'a> {
    cff: &'a Cff,
    private: &'a PrivateInfo,
    stack: [f64; MAX_STACK],
    sp: usize,
    transient: [f64; 32],
    x: f64,
    y: f64,
    open: bool,
    path: Path,
    stems: usize,
    /// The first stack-clearing operator has been seen (the width is
    /// only possible there).
    width_seen: bool,
    width: Option<f64>,
    /// Stop as soon as the width is known.
    width_only: bool,
    ops: usize,
    seac: Option<(f64, f64, f64, f64)>,
    no_seac: bool,
}

impl<'a> Interp<'a> {
    fn new(cff: &'a Cff, private: &'a PrivateInfo, width_only: bool) -> Interp<'a> {
        Interp {
            cff,
            private,
            stack: [0.0; MAX_STACK],
            sp: 0,
            transient: [0.0; 32],
            x: 0.0,
            y: 0.0,
            open: false,
            path: Path::new(),
            stems: 0,
            width_seen: false,
            width: None,
            width_only,
            ops: 0,
            seac: None,
            no_seac: false,
        }
    }

    fn run_glyph(&mut self, gid: u16) -> Option<()> {
        if !self.cff.type2 {
            return None;
        }
        let cs = self.cff.char_strings.get(&self.cff.data, gid as usize)?;
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

    /// The width is an extra first operand on the first stack-clearing
    /// operator; `extra` says whether this operator has one more operand
    /// than it takes.
    fn take_width(&mut self, extra: bool) {
        if self.width_seen {
            return;
        }
        self.width_seen = true;
        if extra && self.sp > 0 {
            self.width = Some(self.stack[0]);
            self.stack.copy_within(1..self.sp, 0);
            self.sp -= 1;
        }
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

    fn move_to(&mut self, dx: f64, dy: f64) {
        self.close_contour();
        self.x += dx;
        self.y += dy;
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

    fn call(&mut self, subrs: Index, depth: usize) -> Option<Flow> {
        if depth >= MAX_SUBR_DEPTH {
            return None;
        }
        let i = (self.pop()? as i64).checked_add(subr_bias(subrs.count))?;
        let cs = subrs.get(&self.cff.data, usize::try_from(i).ok()?)?;
        self.exec(cs, depth + 1)
    }

    /// Runs one charstring (the glyph's, or a subroutine at `depth`).
    fn exec(&mut self, cs: &[u8], depth: usize) -> Option<Flow> {
        let mut i = 0;
        while i < cs.len() {
            self.ops += 1;
            if self.ops > MAX_OPS || self.path.segs.len() > MAX_SEGMENTS {
                return None;
            }
            let b0 = cs[i];
            i += 1;
            match b0 {
                32..=246 => self.push(b0 as f64 - 139.0)?,
                247..=250 => {
                    let b1 = *cs.get(i)? as f64;
                    i += 1;
                    self.push((b0 as f64 - 247.0) * 256.0 + b1 + 108.0)?;
                }
                251..=254 => {
                    let b1 = *cs.get(i)? as f64;
                    i += 1;
                    self.push(-(b0 as f64 - 251.0) * 256.0 - b1 - 108.0)?;
                }
                28 => {
                    let b = cs.get(i..i + 2)?;
                    i += 2;
                    self.push(i16::from_be_bytes([b[0], b[1]]) as f64)?;
                }
                255 => {
                    let b = cs.get(i..i + 4)?;
                    i += 4;
                    self.push(i32::from_be_bytes([b[0], b[1], b[2], b[3]]) as f64 / 65536.0)?;
                }
                // hstem, vstem, hstemhm, vstemhm
                1 | 3 | 18 | 23 => {
                    self.take_width(self.sp % 2 == 1);
                    if self.width_only {
                        return Some(Flow::End);
                    }
                    self.stems += self.sp / 2;
                    self.sp = 0;
                }
                // hintmask, cntrmask: pending operands are an implied vstem.
                19 | 20 => {
                    self.take_width(self.sp % 2 == 1);
                    if self.width_only {
                        return Some(Flow::End);
                    }
                    self.stems += self.sp / 2;
                    self.sp = 0;
                    i = i.checked_add(self.stems.div_ceil(8))?;
                }
                21 => {
                    self.take_width(self.sp > 2);
                    if self.width_only {
                        return Some(Flow::End);
                    }
                    self.move_to(self.arg(0), self.arg(1));
                    self.sp = 0;
                }
                22 | 4 => {
                    self.take_width(self.sp > 1);
                    if self.width_only {
                        return Some(Flow::End);
                    }
                    let d = self.arg(0);
                    if b0 == 22 {
                        self.move_to(d, 0.0)
                    } else {
                        self.move_to(0.0, d)
                    }
                    self.sp = 0;
                }
                14 => {
                    self.take_width(self.sp == 1 || self.sp == 5);
                    if self.width_only {
                        return Some(Flow::End);
                    }
                    if self.sp >= 4 && !self.no_seac {
                        self.seac = Some((self.arg(0), self.arg(1), self.arg(2), self.arg(3)));
                    }
                    self.close_contour();
                    self.sp = 0;
                    return Some(Flow::End);
                }
                5 => {
                    let mut k = 0;
                    while k + 2 <= self.sp {
                        self.line(self.stack[k], self.stack[k + 1]);
                        k += 2;
                    }
                    self.sp = 0;
                }
                6 | 7 => {
                    let mut horizontal = b0 == 6;
                    for k in 0..self.sp {
                        let d = self.stack[k];
                        if horizontal {
                            self.line(d, 0.0)
                        } else {
                            self.line(0.0, d)
                        }
                        horizontal = !horizontal;
                    }
                    self.sp = 0;
                }
                8 => {
                    let mut k = 0;
                    while k + 6 <= self.sp {
                        let s = &self.stack;
                        let a = (s[k], s[k + 1], s[k + 2], s[k + 3], s[k + 4], s[k + 5]);
                        self.curve(a.0, a.1, a.2, a.3, a.4, a.5);
                        k += 6;
                    }
                    self.sp = 0;
                }
                24 => {
                    // rcurveline: curves, then one line.
                    let mut k = 0;
                    while self.sp >= 2 && k + 6 <= self.sp - 2 {
                        let s = &self.stack;
                        let a = (s[k], s[k + 1], s[k + 2], s[k + 3], s[k + 4], s[k + 5]);
                        self.curve(a.0, a.1, a.2, a.3, a.4, a.5);
                        k += 6;
                    }
                    if k + 2 <= self.sp {
                        self.line(self.stack[k], self.stack[k + 1]);
                    }
                    self.sp = 0;
                }
                25 => {
                    // rlinecurve: lines, then one curve.
                    let mut k = 0;
                    while self.sp >= 6 && k + 2 <= self.sp - 6 {
                        self.line(self.stack[k], self.stack[k + 1]);
                        k += 2;
                    }
                    if k + 6 <= self.sp {
                        let s = &self.stack;
                        let a = (s[k], s[k + 1], s[k + 2], s[k + 3], s[k + 4], s[k + 5]);
                        self.curve(a.0, a.1, a.2, a.3, a.4, a.5);
                    }
                    self.sp = 0;
                }
                26 | 27 => {
                    // vvcurveto / hhcurveto: an odd count leads with the
                    // first curve's off-axis delta.
                    let (mut k, mut lead) = if self.sp % 2 == 1 { (1, self.stack[0]) } else { (0, 0.0) };
                    while k + 4 <= self.sp {
                        let s = &self.stack;
                        let (a, b, c, d) = (s[k], s[k + 1], s[k + 2], s[k + 3]);
                        if b0 == 26 {
                            self.curve(lead, a, b, c, 0.0, d)
                        } else {
                            self.curve(a, lead, b, c, d, 0.0)
                        }
                        lead = 0.0;
                        k += 4;
                    }
                    self.sp = 0;
                }
                30 | 31 => {
                    // vhcurveto / hvcurveto: alternating tangents, and an
                    // optional last delta on the final curve.
                    let mut horizontal = b0 == 31;
                    let mut k = 0;
                    while k + 4 <= self.sp {
                        let s = &self.stack;
                        let last = if self.sp - k == 5 { s[k + 4] } else { 0.0 };
                        let (a, b, c, d) = (s[k], s[k + 1], s[k + 2], s[k + 3]);
                        if horizontal {
                            self.curve(a, 0.0, b, c, last, d)
                        } else {
                            self.curve(0.0, a, b, c, d, last)
                        }
                        horizontal = !horizontal;
                        k += 4;
                    }
                    self.sp = 0;
                }
                10 => match self.call(self.private.subrs, depth)? {
                    Flow::Return => {}
                    Flow::End => return Some(Flow::End),
                },
                29 => match self.call(self.cff.gsubrs, depth)? {
                    Flow::Return => {}
                    Flow::End => return Some(Flow::End),
                },
                11 => return Some(Flow::Return),
                12 => {
                    let b1 = *cs.get(i)?;
                    i += 1;
                    self.escape(b1)?;
                }
                // Reserved operators: drop their operands.
                _ => self.sp = 0,
            }
        }
        // A charstring that runs off its end returns (subroutines) or ends.
        Some(Flow::Return)
    }

    fn escape(&mut self, b1: u8) -> Option<()> {
        let bool_f = |b: bool| if b { 1.0 } else { 0.0 };
        match b1 {
            // dotsection (deprecated): nothing to draw.
            0 => self.sp = 0,
            3 => {
                let (b, a) = (self.pop()?, self.pop()?);
                self.push(bool_f(a != 0.0 && b != 0.0))?;
            }
            4 => {
                let (b, a) = (self.pop()?, self.pop()?);
                self.push(bool_f(a != 0.0 || b != 0.0))?;
            }
            5 => {
                let a = self.pop()?;
                self.push(bool_f(a == 0.0))?;
            }
            9 => {
                let a = self.pop()?;
                self.push(a.abs())?;
            }
            10 => {
                let (b, a) = (self.pop()?, self.pop()?);
                self.push(a + b)?;
            }
            11 => {
                let (b, a) = (self.pop()?, self.pop()?);
                self.push(a - b)?;
            }
            12 => {
                let (b, a) = (self.pop()?, self.pop()?);
                self.push(if b == 0.0 { 0.0 } else { a / b })?;
            }
            14 => {
                let a = self.pop()?;
                self.push(-a)?;
            }
            15 => {
                let (b, a) = (self.pop()?, self.pop()?);
                self.push(bool_f(a == b))?;
            }
            18 => {
                self.pop()?;
            }
            20 => {
                let (i, v) = (self.pop()?, self.pop()?);
                if (0.0..32.0).contains(&i) {
                    self.transient[i as usize] = v;
                }
            }
            21 => {
                let i = self.pop()?;
                let v = if (0.0..32.0).contains(&i) { self.transient[i as usize] } else { 0.0 };
                self.push(v)?;
            }
            22 => {
                let (v2, v1, s2, s1) = (self.pop()?, self.pop()?, self.pop()?, self.pop()?);
                self.push(if v1 <= v2 { s1 } else { s2 })?;
            }
            // random: any value in (0, 1]; a fixed one keeps output stable.
            23 => self.push(0.5)?,
            24 => {
                let (b, a) = (self.pop()?, self.pop()?);
                self.push(a * b)?;
            }
            26 => {
                let a = self.pop()?;
                self.push(a.max(0.0).sqrt())?;
            }
            27 => {
                let a = self.pop()?;
                self.push(a)?;
                self.push(a)?;
            }
            28 => {
                let (b, a) = (self.pop()?, self.pop()?);
                self.push(b)?;
                self.push(a)?;
            }
            29 => {
                let i = self.pop()?;
                let i = if i < 0.0 { 0 } else { i as usize };
                let v = self.stack[self.sp.checked_sub(i.checked_add(1)?)?];
                self.push(v)?;
            }
            30 => {
                let (j, n) = (self.pop()?, self.pop()?);
                if n < 0.0 || n as usize > self.sp {
                    return None;
                }
                let n = n as usize;
                if n > 0 {
                    let s = &mut self.stack[self.sp - n..self.sp];
                    let j = (j as i64).rem_euclid(n as i64) as usize;
                    s.rotate_right(j);
                }
            }
            34 => {
                // hflex: dx1 dx2 dy2 dx3 dx4 dx5 dx6
                let a: Vec<f64> = (0..7).map(|k| self.arg(k)).collect();
                self.curve(a[0], 0.0, a[1], a[2], a[3], 0.0);
                self.curve(a[4], 0.0, a[5], -a[2], a[6], 0.0);
                self.sp = 0;
            }
            35 => {
                // flex: two full curves and a flex depth.
                let a: Vec<f64> = (0..12).map(|k| self.arg(k)).collect();
                self.curve(a[0], a[1], a[2], a[3], a[4], a[5]);
                self.curve(a[6], a[7], a[8], a[9], a[10], a[11]);
                self.sp = 0;
            }
            36 => {
                // hflex1: dx1 dy1 dx2 dy2 dx3 dx4 dx5 dy5 dx6
                let a: Vec<f64> = (0..9).map(|k| self.arg(k)).collect();
                self.curve(a[0], a[1], a[2], a[3], a[4], 0.0);
                self.curve(a[5], 0.0, a[6], a[7], a[8], -(a[1] + a[3] + a[7]));
                self.sp = 0;
            }
            37 => {
                // flex1: the last delta runs along the larger total motion.
                let a: Vec<f64> = (0..11).map(|k| self.arg(k)).collect();
                let dx = a[0] + a[2] + a[4] + a[6] + a[8];
                let dy = a[1] + a[3] + a[5] + a[7] + a[9];
                let (dx6, dy6) = if dx.abs() > dy.abs() { (a[10], -dy) } else { (-dx, a[10]) };
                self.curve(a[0], a[1], a[2], a[3], a[4], a[5]);
                self.curve(a[6], a[7], a[8], a[9], dx6, dy6);
                self.sp = 0;
            }
            _ => self.sp = 0,
        }
        Some(())
    }
}

/// The 391 CFF standard strings (CFF spec Appendix A), SIDs 0..390.
#[rustfmt::skip]
static STANDARD_STRINGS: [&str; 391] = [
    ".notdef", "space", "exclam", "quotedbl", "numbersign", "dollar",
    "percent", "ampersand", "quoteright", "parenleft", "parenright", "asterisk",
    "plus", "comma", "hyphen", "period", "slash", "zero",
    "one", "two", "three", "four", "five", "six",
    "seven", "eight", "nine", "colon", "semicolon", "less",
    "equal", "greater", "question", "at", "A", "B",
    "C", "D", "E", "F", "G", "H",
    "I", "J", "K", "L", "M", "N",
    "O", "P", "Q", "R", "S", "T",
    "U", "V", "W", "X", "Y", "Z",
    "bracketleft", "backslash", "bracketright", "asciicircum", "underscore", "quoteleft",
    "a", "b", "c", "d", "e", "f",
    "g", "h", "i", "j", "k", "l",
    "m", "n", "o", "p", "q", "r",
    "s", "t", "u", "v", "w", "x",
    "y", "z", "braceleft", "bar", "braceright", "asciitilde",
    "exclamdown", "cent", "sterling", "fraction", "yen", "florin",
    "section", "currency", "quotesingle", "quotedblleft", "guillemotleft", "guilsinglleft",
    "guilsinglright", "fi", "fl", "endash", "dagger", "daggerdbl",
    "periodcentered", "paragraph", "bullet", "quotesinglbase", "quotedblbase", "quotedblright",
    "guillemotright", "ellipsis", "perthousand", "questiondown", "grave", "acute",
    "circumflex", "tilde", "macron", "breve", "dotaccent", "dieresis",
    "ring", "cedilla", "hungarumlaut", "ogonek", "caron", "emdash",
    "AE", "ordfeminine", "Lslash", "Oslash", "OE", "ordmasculine",
    "ae", "dotlessi", "lslash", "oslash", "oe", "germandbls",
    "onesuperior", "logicalnot", "mu", "trademark", "Eth", "onehalf",
    "plusminus", "Thorn", "onequarter", "divide", "brokenbar", "degree",
    "thorn", "threequarters", "twosuperior", "registered", "minus", "eth",
    "multiply", "threesuperior", "copyright", "Aacute", "Acircumflex", "Adieresis",
    "Agrave", "Aring", "Atilde", "Ccedilla", "Eacute", "Ecircumflex",
    "Edieresis", "Egrave", "Iacute", "Icircumflex", "Idieresis", "Igrave",
    "Ntilde", "Oacute", "Ocircumflex", "Odieresis", "Ograve", "Otilde",
    "Scaron", "Uacute", "Ucircumflex", "Udieresis", "Ugrave", "Yacute",
    "Ydieresis", "Zcaron", "aacute", "acircumflex", "adieresis", "agrave",
    "aring", "atilde", "ccedilla", "eacute", "ecircumflex", "edieresis",
    "egrave", "iacute", "icircumflex", "idieresis", "igrave", "ntilde",
    "oacute", "ocircumflex", "odieresis", "ograve", "otilde", "scaron",
    "uacute", "ucircumflex", "udieresis", "ugrave", "yacute", "ydieresis",
    "zcaron", "exclamsmall", "Hungarumlautsmall", "dollaroldstyle", "dollarsuperior", "ampersandsmall",
    "Acutesmall", "parenleftsuperior", "parenrightsuperior", "twodotenleader", "onedotenleader", "zerooldstyle",
    "oneoldstyle", "twooldstyle", "threeoldstyle", "fouroldstyle", "fiveoldstyle", "sixoldstyle",
    "sevenoldstyle", "eightoldstyle", "nineoldstyle", "commasuperior", "threequartersemdash", "periodsuperior",
    "questionsmall", "asuperior", "bsuperior", "centsuperior", "dsuperior", "esuperior",
    "isuperior", "lsuperior", "msuperior", "nsuperior", "osuperior", "rsuperior",
    "ssuperior", "tsuperior", "ff", "ffi", "ffl", "parenleftinferior",
    "parenrightinferior", "Circumflexsmall", "hyphensuperior", "Gravesmall", "Asmall", "Bsmall",
    "Csmall", "Dsmall", "Esmall", "Fsmall", "Gsmall", "Hsmall",
    "Ismall", "Jsmall", "Ksmall", "Lsmall", "Msmall", "Nsmall",
    "Osmall", "Psmall", "Qsmall", "Rsmall", "Ssmall", "Tsmall",
    "Usmall", "Vsmall", "Wsmall", "Xsmall", "Ysmall", "Zsmall",
    "colonmonetary", "onefitted", "rupiah", "Tildesmall", "exclamdownsmall", "centoldstyle",
    "Lslashsmall", "Scaronsmall", "Zcaronsmall", "Dieresissmall", "Brevesmall", "Caronsmall",
    "Dotaccentsmall", "Macronsmall", "figuredash", "hypheninferior", "Ogoneksmall", "Ringsmall",
    "Cedillasmall", "questiondownsmall", "oneeighth", "threeeighths", "fiveeighths", "seveneighths",
    "onethird", "twothirds", "zerosuperior", "foursuperior", "fivesuperior", "sixsuperior",
    "sevensuperior", "eightsuperior", "ninesuperior", "zeroinferior", "oneinferior", "twoinferior",
    "threeinferior", "fourinferior", "fiveinferior", "sixinferior", "seveninferior", "eightinferior",
    "nineinferior", "centinferior", "dollarinferior", "periodinferior", "commainferior", "Agravesmall",
    "Aacutesmall", "Acircumflexsmall", "Atildesmall", "Adieresissmall", "Aringsmall", "AEsmall",
    "Ccedillasmall", "Egravesmall", "Eacutesmall", "Ecircumflexsmall", "Edieresissmall", "Igravesmall",
    "Iacutesmall", "Icircumflexsmall", "Idieresissmall", "Ethsmall", "Ntildesmall", "Ogravesmall",
    "Oacutesmall", "Ocircumflexsmall", "Otildesmall", "Odieresissmall", "OEsmall", "Oslashsmall",
    "Ugravesmall", "Uacutesmall", "Ucircumflexsmall", "Udieresissmall", "Yacutesmall", "Thornsmall",
    "Ydieresissmall", "001.000", "001.001", "001.002", "001.003", "Black",
    "Bold", "Book", "Light", "Medium", "Regular", "Roman",
    "Semibold",
];

/// The predefined Expert charset (Appendix C) as SIDs, gid order.
#[rustfmt::skip]
static EXPERT_CHARSET: [u16; 166] = [
    0, 1, 229, 230, 231, 232, 233, 234, 235, 236, 237, 238, 13, 14, 15, 99,
    239, 240, 241, 242, 243, 244, 245, 246, 247, 248, 27, 28, 249, 250, 251, 252,
    253, 254, 255, 256, 257, 258, 259, 260, 261, 262, 263, 264, 265, 266, 109, 110,
    267, 268, 269, 270, 271, 272, 273, 274, 275, 276, 277, 278, 279, 280, 281, 282,
    283, 284, 285, 286, 287, 288, 289, 290, 291, 292, 293, 294, 295, 296, 297, 298,
    299, 300, 301, 302, 303, 304, 305, 306, 307, 308, 309, 310, 311, 312, 313, 314,
    315, 316, 317, 318, 158, 155, 163, 319, 320, 321, 322, 323, 324, 325, 326, 150,
    164, 169, 327, 328, 329, 330, 331, 332, 333, 334, 335, 336, 337, 338, 339, 340,
    341, 342, 343, 344, 345, 346, 347, 348, 349, 350, 351, 352, 353, 354, 355, 356,
    357, 358, 359, 360, 361, 362, 363, 364, 365, 366, 367, 368, 369, 370, 371, 372,
    373, 374, 375, 376, 377, 378,
];

/// The predefined ExpertSubset charset (Appendix C) as SIDs, gid order.
#[rustfmt::skip]
static EXPERT_SUBSET_CHARSET: [u16; 87] = [
    0, 1, 231, 232, 235, 236, 237, 238, 13, 14, 15, 99, 239, 240, 241, 242,
    243, 244, 245, 246, 247, 248, 27, 28, 249, 250, 251, 253, 254, 255, 256, 257,
    258, 259, 260, 261, 262, 263, 264, 265, 266, 109, 110, 267, 268, 269, 270, 272,
    300, 301, 302, 305, 314, 315, 158, 155, 163, 320, 321, 322, 323, 324, 325, 326,
    150, 164, 169, 327, 328, 329, 330, 331, 332, 333, 334, 335, 336, 337, 338, 339,
    340, 341, 342, 343, 344, 345, 346,
];

/// The predefined Expert encoding (Appendix B) as code -> SID.
#[rustfmt::skip]
static EXPERT_ENCODING: [u16; 256] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    1, 229, 230, 0, 231, 232, 233, 234, 235, 236, 237, 238, 13, 14, 15, 99,
    239, 240, 241, 242, 243, 244, 245, 246, 247, 248, 27, 28, 249, 250, 251, 252,
    0, 253, 254, 255, 256, 257, 0, 0, 0, 258, 0, 0, 259, 260, 261, 262,
    0, 0, 263, 264, 265, 0, 266, 109, 110, 267, 268, 269, 0, 270, 271, 272,
    273, 274, 275, 276, 277, 278, 279, 280, 281, 282, 283, 284, 285, 286, 287, 288,
    289, 290, 291, 292, 293, 294, 295, 296, 297, 298, 299, 300, 301, 302, 303, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 304, 305, 306, 0, 0, 307, 308, 309, 310, 311, 0, 312, 0, 0, 313,
    0, 0, 314, 315, 0, 0, 316, 317, 318, 0, 0, 0, 158, 155, 163, 319,
    320, 321, 322, 323, 324, 325, 0, 0, 326, 150, 164, 169, 327, 328, 329, 330,
    331, 332, 333, 334, 335, 336, 337, 338, 339, 340, 341, 342, 343, 344, 345, 346,
    347, 348, 349, 350, 351, 352, 353, 354, 355, 356, 357, 358, 359, 360, 361, 362,
    363, 364, 365, 366, 367, 368, 369, 370, 371, 372, 373, 374, 375, 376, 377, 378,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::truetype::TrueType;
    use crate::font::truetype::tests::{find_fonts, fuzz, l, m, sfnt, z};
    use crate::geom::{Point, Seg};

    fn c(x1: f64, y1: f64, x2: f64, y2: f64, x: f64, y: f64) -> Seg {
        Seg::Cubic(Point::new(x1, y1), Point::new(x2, y2), Point::new(x, y))
    }

    /// Assembles a Type 2 charstring from text: numbers, operator names
    /// and `#hex` raw bytes (hint masks).
    fn cs(src: &str) -> Vec<u8> {
        let mut out = Vec::new();
        for tok in src.split_whitespace() {
            if let Some(hex) = tok.strip_prefix('#') {
                out.push(u8::from_str_radix(hex, 16).unwrap());
                continue;
            }
            if let Ok(v) = tok.parse::<f64>() {
                if v.fract() != 0.0 {
                    out.push(255);
                    out.extend(((v * 65536.0) as i32).to_be_bytes());
                    continue;
                }
                let i = v as i32;
                match i {
                    -107..=107 => out.push((i + 139) as u8),
                    108..=1131 => out.extend([((i - 108) >> 8) as u8 + 247, ((i - 108) & 0xFF) as u8]),
                    -1131..=-108 => out.extend([((-i - 108) >> 8) as u8 + 251, ((-i - 108) & 0xFF) as u8]),
                    _ => {
                        out.push(28);
                        out.extend((i as i16).to_be_bytes());
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
                "callsubr" => &[10],
                "return" => &[11],
                "endchar" => &[14],
                "hstemhm" => &[18],
                "hintmask" => &[19],
                "cntrmask" => &[20],
                "rmoveto" => &[21],
                "hmoveto" => &[22],
                "vstemhm" => &[23],
                "rcurveline" => &[24],
                "rlinecurve" => &[25],
                "vvcurveto" => &[26],
                "hhcurveto" => &[27],
                "callgsubr" => &[29],
                "vhcurveto" => &[30],
                "hvcurveto" => &[31],
                "dotsection" => &[12, 0],
                "and" => &[12, 3],
                "or" => &[12, 4],
                "not" => &[12, 5],
                "abs" => &[12, 9],
                "add" => &[12, 10],
                "sub" => &[12, 11],
                "div" => &[12, 12],
                "neg" => &[12, 14],
                "eq" => &[12, 15],
                "drop" => &[12, 18],
                "put" => &[12, 20],
                "get" => &[12, 21],
                "ifelse" => &[12, 22],
                "random" => &[12, 23],
                "mul" => &[12, 24],
                "sqrt" => &[12, 26],
                "dup" => &[12, 27],
                "exch" => &[12, 28],
                "index" => &[12, 29],
                "roll" => &[12, 30],
                "hflex" => &[12, 34],
                "flex" => &[12, 35],
                "hflex1" => &[12, 36],
                "flex1" => &[12, 37],
                _ => panic!("unknown operator {tok}"),
            };
            out.extend(code);
        }
        out
    }

    fn index(items: &[Vec<u8>]) -> Vec<u8> {
        let mut v = (items.len() as u16).to_be_bytes().to_vec();
        if items.is_empty() {
            return v;
        }
        v.push(4);
        let mut off = 1u32;
        v.extend(off.to_be_bytes());
        for it in items {
            off += it.len() as u32;
            v.extend(off.to_be_bytes());
        }
        for it in items {
            v.extend(it);
        }
        v
    }

    /// A 5-byte DICT integer, so offsets do not change a DICT's size.
    fn dint(v: usize) -> Vec<u8> {
        let mut out = vec![29];
        out.extend((v as i32).to_be_bytes());
        out
    }

    fn dop(code: u16) -> Vec<u8> {
        if code >= 0x0C00 { vec![12, code as u8] } else { vec![code as u8] }
    }

    /// A DICT real from its decimal text.
    fn dreal(s: &str) -> Vec<u8> {
        let mut nibbles = Vec::new();
        let b = s.as_bytes();
        let mut i = 0;
        while i < b.len() {
            match b[i] {
                d @ b'0'..=b'9' => nibbles.push(d - b'0'),
                b'.' => nibbles.push(0xA),
                b'-' => nibbles.push(0xE),
                b'E' if b.get(i + 1) == Some(&b'-') => {
                    nibbles.push(0xC);
                    i += 1;
                }
                b'E' => nibbles.push(0xB),
                _ => panic!(),
            }
            i += 1;
        }
        nibbles.push(0xF);
        if nibbles.len() % 2 == 1 {
            nibbles.push(0xF);
        }
        let mut out = vec![30];
        out.extend(nibbles.chunks(2).map(|p| (p[0] << 4) | p[1]));
        out
    }

    fn dmatrix(vals: [&str; 6]) -> Vec<u8> {
        let mut out: Vec<u8> = vals.iter().flat_map(|v| dreal(v)).collect();
        out.extend(dop(op::FONT_MATRIX));
        out
    }

    struct Fd {
        dict: Vec<u8>,
        private: Vec<u8>,
        subrs: Vec<Vec<u8>>,
    }

    /// The Private DICT followed by its Subrs INDEX, and the DICT's size.
    fn private_block(fd: &Fd) -> (Vec<u8>, usize) {
        let mut dict = fd.private.clone();
        if !fd.subrs.is_empty() {
            let len = dict.len() + 6;
            dict.extend(dint(len));
            dict.extend(dop(op::SUBRS));
        }
        let len = dict.len();
        if !fd.subrs.is_empty() {
            dict.extend(index(&fd.subrs));
        }
        (dict, len)
    }

    struct Spec {
        top_extra: Vec<u8>,
        strings: Vec<Vec<u8>>,
        gsubrs: Vec<Vec<u8>>,
        charset: Vec<u8>,
        encoding: Vec<u8>,
        charstrings: Vec<Vec<u8>>,
        fds: Vec<Fd>,
        fd_select: Vec<u8>,
        cid: bool,
        /// Predefined charset and encoding ids, used when the data is empty.
        charset_id: usize,
        encoding_id: usize,
    }

    fn build(s: &Spec) -> Vec<u8> {
        let name = index(&[b"Test".to_vec()]);
        let strings = index(&s.strings);
        let gsubrs = index(&s.gsubrs);
        let charstrings = index(&s.charstrings);
        let blocks: Vec<_> = s.fds.iter().map(private_block).collect();
        let fd_dict = |fd: &Fd, size: usize, off: usize| {
            let mut d = fd.dict.clone();
            d.extend(dint(size));
            d.extend(dint(off));
            d.extend(dop(op::PRIVATE));
            d
        };
        let top =
            |charset: usize, encoding: usize, cs: usize, private: (usize, usize), fdarray: usize, fdsel: usize| {
                let mut t = s.top_extra.clone();
                for (v, code) in [(charset, op::CHARSET), (cs, op::CHAR_STRINGS)] {
                    t.extend(dint(v));
                    t.extend(dop(code));
                }
                if !s.encoding.is_empty() || s.encoding_id != 0 {
                    t.extend(dint(encoding));
                    t.extend(dop(op::ENCODING));
                }
                if s.cid {
                    t.extend(dint(fdarray));
                    t.extend(dop(op::FD_ARRAY));
                    t.extend(dint(fdsel));
                    t.extend(dop(op::FD_SELECT));
                } else {
                    t.extend(dint(private.0));
                    t.extend(dint(private.1));
                    t.extend(dop(op::PRIVATE));
                }
                t
            };
        let top_len = index(&[top(0, 0, 0, (0, 0), 0, 0)]).len();
        let mut pos = 4 + name.len() + top_len + strings.len() + gsubrs.len();
        let charset_off = if s.charset.is_empty() { s.charset_id } else { pos };
        pos += s.charset.len();
        let encoding_off = if s.encoding.is_empty() { s.encoding_id } else { pos };
        pos += s.encoding.len();
        let cs_off = pos;
        pos += charstrings.len();
        let fdsel_off = pos;
        pos += s.fd_select.len();
        let fdarray_off = pos;
        if s.cid {
            let dicts: Vec<_> = s.fds.iter().map(|fd| fd_dict(fd, 0, 0)).collect();
            pos += index(&dicts).len();
        }
        let mut private_offs = Vec::new();
        for (block, _) in &blocks {
            private_offs.push(pos);
            pos += block.len();
        }
        let fdarray: Vec<_> =
            s.fds.iter().zip(&blocks).zip(&private_offs).map(|((fd, b), &off)| fd_dict(fd, b.1, off)).collect();
        let mut out = vec![1, 0, 4, 4];
        out.extend(name);
        out.extend(index(&[top(
            charset_off,
            encoding_off,
            cs_off,
            (blocks[0].1, private_offs[0]),
            fdarray_off,
            fdsel_off,
        )]));
        out.extend(strings);
        out.extend(gsubrs);
        out.extend(&s.charset);
        out.extend(&s.encoding);
        out.extend(charstrings);
        out.extend(&s.fd_select);
        if s.cid {
            out.extend(index(&fdarray));
        }
        for (block, _) in &blocks {
            out.extend(block);
        }
        assert_eq!(out.len(), pos);
        out
    }

    fn dnum(v: i32) -> Vec<u8> {
        if (-107..=107).contains(&v) { vec![(v + 139) as u8] } else { dint(v as usize) }
    }

    /// The name-keyed test font. `alt` switches to charset format 0 and an
    /// encoding of format 1.
    fn simple_font(alt: bool) -> Vec<u8> {
        simple_font_with(alt, true)
    }

    fn simple_font_with(alt: bool, bomb: bool) -> Vec<u8> {
        let names = ["square", "curves", "subrs", "hinted", "flexes", "deep", "bomb"];
        let sids: [u16; 10] = [391, 392, 393, 394, 395, 34, 125, 171, 396, 397];
        let charset = if alt {
            let mut v = vec![0u8];
            for sid in sids {
                v.extend(sid.to_be_bytes());
            }
            v
        } else {
            let mut v = vec![1u8];
            for (first, left) in [(391u16, 4u8), (34, 0), (125, 0), (171, 0), (396, 1)] {
                v.extend(first.to_be_bytes());
                v.push(left);
            }
            v
        };
        let encoding =
            if alt { vec![0x81, 1, 0x61, 2, 1, 0x41, 0, 34] } else { vec![0x80, 3, 0x61, 0x62, 0x63, 1, 0x41, 0, 34] };
        let mut subrs = vec![cs("100 0 rlineto return"), cs("-106 callsubr return")];
        for k in 2..10i32 {
            subrs.push(cs(&format!("{} callsubr ", k + 1 - 107).repeat(50)));
        }
        subrs.push(cs("return"));
        let charstrings = vec![
            cs("endchar"),
            cs("50 10 20 rmoveto 100 200 -100 hlineto endchar"),
            cs("0 0 rmoveto 10 20 30 40 50 60 rrcurveto 5 10 20 30 40 hhcurveto 10 20 30 40 50 hvcurveto \
                1 2 3 4 5 6 7 8 vhcurveto 3 1 2 3 4 vvcurveto 1 1 2 2 3 3 10 0 rcurveline \
                5 0 0 5 1 1 2 2 3 3 rlinecurve endchar"),
            cs("10 20 rmoveto -107 callgsubr 3 4 add 2 mul 0 put 0 get 0 rlineto 1 2 exch sub 0 rlineto \
                5 dup mul neg 0 exch rlineto 7 8 9 3 1 roll 2 index drop drop rlineto \
                1 2 3 4 ifelse 0 rlineto 9 4 div 0 rlineto 16 sqrt abs 0 exch rlineto \
                1 0 eq 1 1 and or 0 rlineto 0 1 eq not 0 exch rlineto endchar"),
            cs("7 10 20 30 40 hstemhm 50 60 hintmask #0E 0 0 rmoveto 100 0 rlineto 0 100 rlineto \
                cntrmask #0E -100 0 rlineto endchar"),
            cs("0 0 rmoveto 10 0 20 10 30 10 40 -10 50 -10 60 0 50 flex 10 20 5 30 40 50 60 hflex \
                10 5 20 5 30 40 20 -5 10 hflex1 10 10 20 10 10 0 10 -5 20 -10 15 flex1 endchar"),
            cs("0 0 rmoveto 100 200 rlineto 100 -200 rlineto endchar"),
            cs("50 300 rmoveto 20 20 rlineto endchar"),
            cs("100 30 40 65 194 endchar"),
            cs("0 0 rmoveto -106 callsubr endchar"),
            cs(if bomb { "0 0 rmoveto -105 callsubr endchar" } else { "endchar" }),
        ];
        let mut private = dnum(600);
        private.extend(dop(op::DEFAULT_WIDTH_X));
        private.extend(dnum(500));
        private.extend(dop(op::NOMINAL_WIDTH_X));
        build(&Spec {
            top_extra: dmatrix(["5E-4", "0", "0", ".0005", "0", "0"]),
            strings: names.iter().map(|n| n.as_bytes().to_vec()).collect(),
            gsubrs: vec![cs("0 100 rlineto -107 callsubr return")],
            charset,
            encoding,
            charstrings,
            fds: vec![Fd { dict: Vec::new(), private, subrs }],
            fd_select: Vec::new(),
            cid: false,
            charset_id: 0,
            encoding_id: 0,
        })
    }

    fn cid_font() -> Vec<u8> {
        cid_font_with(false)
    }

    /// The CID-keyed test font; FDSelect format 0 or 3.
    fn cid_font_with(fd_select0: bool) -> Vec<u8> {
        let mut ros = dint(391);
        ros.extend(dint(392));
        ros.extend(dnum(0));
        ros.extend(dop(op::ROS));
        ros.extend(dmatrix(["1", "0", "0", "1", "0", "0"]));
        let fd = |matrix: &str, default: i32, nominal: i32, subr: &str| {
            let mut private = dnum(default);
            private.extend(dop(op::DEFAULT_WIDTH_X));
            private.extend(dnum(nominal));
            private.extend(dop(op::NOMINAL_WIDTH_X));
            Fd { dict: dmatrix([matrix, "0", "0", matrix, "0", "0"]), private, subrs: vec![cs(subr)] }
        };
        let mut charset = vec![2u8];
        for (first, left) in [(100u16, 1u16), (5000, 0)] {
            charset.extend(first.to_be_bytes());
            charset.extend(left.to_be_bytes());
        }
        let mut fd_select = vec![3u8, 0, 2];
        for (first, fd) in [(0u16, 0u8), (2, 1)] {
            fd_select.extend(first.to_be_bytes());
            fd_select.push(fd);
        }
        fd_select.extend(4u16.to_be_bytes());
        if fd_select0 {
            fd_select = vec![0, 0, 0, 1, 1];
        }
        build(&Spec {
            top_extra: ros,
            strings: vec![b"Adobe".to_vec(), b"Identity".to_vec()],
            gsubrs: Vec::new(),
            charset,
            encoding: Vec::new(),
            charstrings: vec![
                cs("endchar"),
                cs("-107 callsubr endchar"),
                cs("20 -107 callsubr endchar"),
                cs("-107 callsubr endchar"),
            ],
            fds: vec![
                fd("0.001", 1000, 0, "0 0 rmoveto 100 0 rlineto 0 100 rlineto return"),
                fd("0.0005", 800, 100, "0 0 rmoveto 50 50 rlineto return"),
            ],
            fd_select,
            cid: true,
            charset_id: 0,
            encoding_id: 0,
        })
    }

    fn segs(font: &Cff, gid: u16) -> Vec<Seg> {
        font.glyph_path(gid).unwrap_or_else(|| panic!("glyph {gid}")).segs
    }

    #[test]
    fn charstring_operators() {
        let font = Cff::parse(simple_font(false)).unwrap();
        assert!(!font.is_cid());
        assert_eq!(font.num_glyphs(), 11);
        assert_eq!(font.font_matrix(), [0.0005, 0.0, 0.0, 0.0005, 0.0, 0.0]);
        assert_eq!(font.glyph_path(0), Some(Path::new()));
        assert_eq!(segs(&font, 1), vec![m(10.0, 20.0), l(110.0, 20.0), l(110.0, 220.0), l(10.0, 220.0), z()]);
        assert_eq!(
            segs(&font, 2),
            vec![
                m(0.0, 0.0),
                c(10.0, 20.0, 40.0, 60.0, 90.0, 120.0),
                c(100.0, 125.0, 120.0, 155.0, 160.0, 155.0),
                c(170.0, 155.0, 190.0, 185.0, 240.0, 225.0),
                c(240.0, 226.0, 242.0, 229.0, 246.0, 229.0),
                c(251.0, 229.0, 257.0, 236.0, 257.0, 244.0),
                c(260.0, 245.0, 262.0, 248.0, 262.0, 252.0),
                c(263.0, 253.0, 265.0, 255.0, 268.0, 258.0),
                l(278.0, 258.0),
                l(283.0, 258.0),
                l(283.0, 263.0),
                c(284.0, 264.0, 286.0, 266.0, 289.0, 269.0),
                z(),
            ]
        );
    }

    #[test]
    fn subroutines_and_arithmetic() {
        let font = Cff::parse(simple_font(false)).unwrap();
        assert_eq!(
            segs(&font, 3),
            vec![
                m(10.0, 20.0),
                l(10.0, 120.0),
                l(110.0, 120.0),
                l(124.0, 120.0),
                l(125.0, 120.0),
                l(125.0, 95.0),
                l(134.0, 102.0),
                l(135.0, 102.0),
                l(137.25, 102.0),
                l(137.25, 106.0),
                l(138.25, 106.0),
                l(138.25, 107.0),
                z(),
            ]
        );
        // A subroutine calling itself hits the depth limit; one that fans
        // out exponentially hits the operator budget.
        assert_eq!(font.glyph_path(9), None);
        assert_eq!(font.glyph_path(10), None);
    }

    #[test]
    fn hints_flex_and_seac() {
        let font = Cff::parse(simple_font(false)).unwrap();
        assert_eq!(segs(&font, 4), vec![m(0.0, 0.0), l(100.0, 0.0), l(100.0, 100.0), l(0.0, 100.0), z()]);
        assert_eq!(
            segs(&font, 5),
            vec![
                m(0.0, 0.0),
                c(10.0, 0.0, 30.0, 10.0, 60.0, 20.0),
                c(100.0, 10.0, 150.0, 0.0, 210.0, 0.0),
                c(220.0, 0.0, 240.0, 5.0, 270.0, 5.0),
                c(310.0, 5.0, 360.0, 0.0, 420.0, 0.0),
                c(430.0, 5.0, 450.0, 10.0, 480.0, 10.0),
                c(520.0, 10.0, 540.0, 5.0, 550.0, 0.0),
                c(560.0, 10.0, 580.0, 20.0, 590.0, 20.0),
                c(600.0, 15.0, 620.0, 5.0, 635.0, 0.0),
                z(),
            ]
        );
        assert_eq!(
            segs(&font, 8),
            vec![m(0.0, 0.0), l(100.0, 200.0), l(200.0, 0.0), z(), m(80.0, 340.0), l(100.0, 360.0), z()]
        );
    }

    #[test]
    fn widths_names_and_encodings() {
        for alt in [false, true] {
            let font = Cff::parse(simple_font(alt)).unwrap();
            assert_eq!(font.advance_width(0), Some(600.0));
            assert_eq!(font.advance_width(1), Some(550.0));
            assert_eq!(font.advance_width(4), Some(507.0));
            assert_eq!(font.advance_width(8), Some(600.0));
            assert_eq!(font.advance_width(11), None);
            assert_eq!(font.glyph_name(0).as_deref(), Some(".notdef"));
            assert_eq!(font.glyph_name(1).as_deref(), Some("square"));
            assert_eq!(font.glyph_name(7).as_deref(), Some("acute"));
            assert_eq!(font.glyph_name(10).as_deref(), Some("bomb"));
            assert_eq!(font.gid_for_name("Aacute"), Some(8));
            assert_eq!(font.gid_for_name("flexes"), Some(5));
            assert_eq!(font.gid_for_name("B"), None);
            assert_eq!(font.gid_for_cid(3), Some(3));
            assert_eq!(font.gid_for_cid(11), None);
            let enc = font.encoding();
            assert_eq!(enc.len(), 256);
            assert_eq!(enc[0x61].as_deref(), Some("square"));
            assert_eq!(enc[0x62].as_deref(), Some("curves"));
            assert_eq!(enc[0x63].as_deref(), Some("subrs"));
            assert_eq!(enc[0x64], None);
            assert_eq!(enc[0x41].as_deref(), Some("A"));
            assert_eq!(enc[0x20], None);
        }
    }

    #[test]
    fn cid_keyed_fonts() {
        let font = Cff::parse(cid_font()).unwrap();
        assert!(font.is_cid());
        assert_eq!(font.font_matrix(), [0.001, 0.0, 0.0, 0.001, 0.0, 0.0]);
        assert_eq!(font.gid_for_cid(0), Some(0));
        assert_eq!(font.gid_for_cid(100), Some(1));
        assert_eq!(font.gid_for_cid(101), Some(2));
        assert_eq!(font.gid_for_cid(5000), Some(3));
        assert_eq!(font.gid_for_cid(7), None);
        assert_eq!(font.glyph_name(1), None);
        assert_eq!(font.gid_for_name("A"), None);
        assert!(font.encoding().iter().all(|e| e.is_none()));
        assert_eq!(segs(&font, 1), vec![m(0.0, 0.0), l(100.0, 0.0), l(100.0, 100.0), z()]);
        // Font DICT 1's matrix is half of Font DICT 0's: its glyphs come back
        // in FD 0's space, local subrs and widths from its own Private DICT.
        let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
        for gid in [2, 3] {
            let got = segs(&font, gid);
            assert_eq!(got.len(), 3);
            assert_eq!((got[0], got[2]), (m(0.0, 0.0), z()));
            let Seg::Line(p) = got[1] else { panic!("{got:?}") };
            assert!(near(p.x, 25.0) && near(p.y, 25.0), "{got:?}");
        }
        assert_eq!(font.advance_width(1), Some(1000.0));
        assert!(near(font.advance_width(2).unwrap(), 60.0));
        assert!(near(font.advance_width(3).unwrap(), 400.0));
    }

    #[test]
    fn fd_select_format_0() {
        let font = Cff::parse(cid_font_with(true)).unwrap();
        assert_eq!(segs(&font, 1), vec![m(0.0, 0.0), l(100.0, 0.0), l(100.0, 100.0), z()]);
        assert_eq!(segs(&font, 3).len(), 3);
        assert_eq!(font.advance_width(3).map(|w| w.round()), Some(400.0));
    }

    #[test]
    fn predefined_charsets_and_encodings() {
        let font = |charset_id, encoding_id| {
            Cff::parse(build(&Spec {
                top_extra: Vec::new(),
                strings: Vec::new(),
                gsubrs: Vec::new(),
                charset: Vec::new(),
                encoding: Vec::new(),
                charstrings: vec![cs("endchar"); 3],
                fds: vec![Fd { dict: Vec::new(), private: Vec::new(), subrs: Vec::new() }],
                fd_select: Vec::new(),
                cid: false,
                charset_id,
                encoding_id,
            }))
            .unwrap()
        };
        let iso = font(0, 0);
        assert_eq!(iso.font_matrix(), DEFAULT_MATRIX);
        assert_eq!(iso.glyph_name(1).as_deref(), Some("space"));
        assert_eq!(iso.gid_for_name("exclam"), Some(2));
        assert_eq!(iso.glyph_name(3), None);
        assert_eq!(iso.encoding()[0x41].as_deref(), Some("A"));
        assert_eq!(iso.encoding()[0x27].as_deref(), Some("quoteright"));
        assert_eq!(iso.advance_width(1), Some(0.0));
        let expert = font(1, 1);
        assert_eq!(expert.glyph_name(2).as_deref(), Some("exclamsmall"));
        assert_eq!(expert.encoding()[0x21].as_deref(), Some("exclamsmall"));
        assert_eq!(expert.encoding()[0x56].as_deref(), Some("ff"));
        assert_eq!(expert.encoding()[0x23], None);
        assert_eq!(font(2, 0).glyph_name(2).as_deref(), Some("dollaroldstyle"));
    }

    #[test]
    fn opentype_wrapper() {
        let cff = simple_font(false);
        let otto = sfnt(u32::from_be_bytes(*b"OTTO"), &[(b"CFF ", cff), (b"maxp", vec![0, 0, 0x50, 0, 0, 11])]);
        let tt = TrueType::parse(otto.clone()).unwrap();
        assert!(tt.is_cff());
        assert_eq!(tt.num_glyphs(), 11);
        assert_eq!(tt.glyph_path(1), None);
        let font = Cff::from_opentype(&otto).unwrap();
        assert_eq!(segs(&font, 1).len(), 5);
        assert!(Cff::from_opentype(&sfnt(0x0001_0000, &[(b"maxp", vec![0; 6])])).is_err());
    }

    fn exercise(font: &Cff) {
        for gid in 0..font.num_glyphs().min(64) {
            let _ = font.glyph_path(gid);
            let _ = font.advance_width(gid);
            let _ = font.glyph_name(gid);
        }
        let _ = font.gid_for_name("A");
        let _ = font.gid_for_cid(100);
        let _ = font.encoding();
        let _ = format!("{font:?}");
    }

    #[test]
    fn hostile_inputs_never_panic() {
        // Without the exponential glyph: each variant would spend the whole
        // operator budget on it.
        for data in [simple_font_with(false, false), simple_font_with(true, false), cid_font(), cid_font_with(true)] {
            fuzz(&data, 10_000, |d| {
                if let Ok(font) = Cff::parse(d) {
                    exercise(&font);
                }
            });
            for cut in 0..data.len() {
                if let Ok(font) = Cff::parse(data[..cut].to_vec()) {
                    exercise(&font);
                }
            }
            for i in 0..data.len() {
                for v in [0x00, 0xFF, 0x80, 0x0C, 0x1D, 0x0A] {
                    let mut d = data.clone();
                    d[i] = v;
                    if let Ok(font) = Cff::parse(d) {
                        exercise(&font);
                    }
                }
            }
        }
        assert!(Cff::parse(Vec::new()).is_err());
        assert!(Cff::parse(vec![2, 0, 5, 4, 0, 0]).is_err());
    }

    #[test]
    fn system_opentype_fonts() {
        let mut files = find_fonts(&["/usr/share/fonts"], &["otf"], 4);
        files.extend(
            find_fonts(&["/usr/share/fonts"], &["ttc"], 64)
                .into_iter()
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.contains("CJK-Regular") || n.contains("CJK-Bold"))
                })
                .take(1),
        );
        for (n, path) in files.into_iter().enumerate() {
            let Ok(data) = std::fs::read(&path) else {
                continue;
            };
            let Ok(tt) = TrueType::parse(data.clone()) else {
                continue;
            };
            if n == 0
                && let Some(table) = tt.table(b"CFF ")
            {
                for k in 1..8 {
                    if let Ok(font) = Cff::parse(table[..table.len() * k / 8].to_vec()) {
                        (0..font.num_glyphs()).for_each(|g| drop(font.glyph_path(g)));
                    }
                }
            }
            if !tt.is_cff() {
                continue;
            }
            let font = Cff::from_opentype(&data).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let mut drawn = 0;
            for gid in 0..font.num_glyphs() {
                if font.glyph_path(gid).is_some_and(|p| !p.is_empty()) {
                    drawn += 1;
                }
                assert!(font.advance_width(gid).is_some(), "{} gid {gid}", path.display());
            }
            assert!(drawn > font.num_glyphs() as usize / 2, "{}: {drawn}", path.display());
            let bbox = font.font_bbox().unwrap_or([-1000.0, -1000.0, 2000.0, 2000.0]);
            for ch in ['A', 'o', 'g'] {
                let Some(gid) = tt.cmap_lookup(3, 1, ch as u32).or_else(|| tt.cmap_lookup(3, 10, ch as u32)) else {
                    continue;
                };
                let (x0, y0, x1, y1) = font.glyph_path(gid).unwrap().bounds().unwrap();
                assert!(x0 >= bbox[0] - 50.0 && y0 >= bbox[1] - 50.0, "{} {ch}", path.display());
                assert!(x1 <= bbox[2] + 50.0 && y1 <= bbox[3] + 50.0, "{} {ch}", path.display());
                assert!(x1 > x0 && y1 > y0);
                if !font.is_cid() {
                    let name = font.glyph_name(gid).unwrap();
                    assert_eq!(font.gid_for_name(&name), Some(gid));
                }
            }
            if font.is_cid() {
                for gid in (0..font.num_glyphs()).step_by(97) {
                    let cid = font.charset[gid as usize];
                    assert_eq!(font.gid_for_cid(cid), Some(gid));
                }
            }
        }
    }
}
