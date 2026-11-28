//! TrueType and OpenType (sfnt) font programs: the PDF `FontFile2` and
//! `FontFile3 /OpenType` streams, and TrueType collections.
//!
//! Parsing only reads the table directory and a few header fields; a
//! glyph is decoded when it is asked for. Fonts embedded in PDFs are
//! often subset and broken, so every table is optional and a glyph that
//! cannot be read is `None` without failing the font.

use super::FontError;
use crate::geom::Path;
use std::sync::OnceLock;

/// Nesting limit for composite glyphs (also what stops reference cycles).
const MAX_COMPOSITE_DEPTH: u32 = 8;
/// Path segments one glyph may produce before it is refused.
const MAX_SEGMENTS: usize = 20_000;
/// Points one glyph (composites included) may accumulate.
const MAX_POINTS: usize = 100_000;
/// Components one glyph may load in all: without it, nested composites
/// of empty glyphs could cost exponential time without adding a point.
const MAX_COMPONENTS: usize = 4_096;

const TAG_TTCF: u32 = u32::from_be_bytes(*b"ttcf");
const TAG_TRUE: u32 = u32::from_be_bytes(*b"true");
const TAG_OTTO: u32 = u32::from_be_bytes(*b"OTTO");
const VERSION_1: u32 = 0x0001_0000;

/// Big-endian reads that fail instead of panicking.
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8], pos: usize) -> Reader<'a> {
        Reader { data, pos }
    }

    fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let s = self.data.get(self.pos..end)?;
        self.pos = end;
        Some(s)
    }

    fn skip(&mut self, n: usize) -> Option<()> {
        self.bytes(n).map(|_| ())
    }

    fn u8(&mut self) -> Option<u8> {
        let b = *self.data.get(self.pos)?;
        self.pos += 1;
        Some(b)
    }

    fn i8(&mut self) -> Option<i8> {
        self.u8().map(|b| b as i8)
    }

    fn u16(&mut self) -> Option<u16> {
        self.bytes(2).map(|b| u16::from_be_bytes([b[0], b[1]]))
    }

    fn i16(&mut self) -> Option<i16> {
        self.u16().map(|v| v as i16)
    }
}

fn be_u16(d: &[u8], off: usize) -> Option<u16> {
    let b = d.get(off..off.checked_add(2)?)?;
    Some(u16::from_be_bytes([b[0], b[1]]))
}

fn be_i16(d: &[u8], off: usize) -> Option<i16> {
    be_u16(d, off).map(|v| v as i16)
}

fn be_u32(d: &[u8], off: usize) -> Option<u32> {
    let b = d.get(off..off.checked_add(4)?)?;
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn err(msg: &str) -> FontError {
    FontError(format!("sfnt: {msg}"))
}

/// One table, its byte range already clamped to the data.
#[derive(Debug, Clone, Copy)]
struct Table {
    tag: [u8; 4],
    start: usize,
    end: usize,
}

/// Reads the table directory at `dir`. Records past the data are skipped
/// and lengths past its end are clamped: truncated fonts keep what they have.
fn read_directory(data: &[u8], dir: usize) -> Result<(u32, Vec<Table>), FontError> {
    if dir >= data.len() {
        return Err(err("table directory past the end"));
    }
    let version = be_u32(data, dir).ok_or_else(|| err("no table directory"))?;
    let num_tables = be_u16(data, dir + 4).ok_or_else(|| err("no table directory"))?;
    let mut tables = Vec::with_capacity(num_tables as usize);
    for i in 0..num_tables as usize {
        let rec = dir + 12 + 16 * i;
        let (Some(tag), Some(offset), Some(length)) =
            (data.get(rec..rec + 4), be_u32(data, rec + 8), be_u32(data, rec + 12))
        else {
            break;
        };
        let start = offset as usize;
        if start >= data.len() {
            continue;
        }
        let end = start.saturating_add(length as usize).min(data.len());
        tables.push(Table { tag: [tag[0], tag[1], tag[2], tag[3]], start, end });
    }
    Ok((version, tables))
}

/// A table of an sfnt (the first face of a collection), for modules that
/// only need one table, such as the `CFF ` table of an OpenType font.
pub(crate) fn sfnt_table<'a>(data: &'a [u8], tag: &[u8; 4]) -> Option<&'a [u8]> {
    let dir = if be_u32(data, 0)? == TAG_TTCF { be_u32(data, 12)? as usize } else { 0 };
    let (_, tables) = read_directory(data, dir).ok()?;
    let t = tables.iter().find(|t| &t.tag == tag)?;
    data.get(t.start..t.end)
}

/// One `cmap` subtable this module can read.
#[derive(Debug, Clone, Copy)]
struct CmapSub {
    platform: u16,
    encoding: u16,
    /// Absolute offset of the subtable in the font data.
    start: usize,
    format: u16,
    /// Segments (format 4) or groups (12, 13) are in increasing order, so
    /// a binary search is valid; broken fonts get a linear scan.
    sorted: bool,
}

/// A TrueType or OpenType font. It owns the font data.
pub struct TrueType {
    data: Vec<u8>,
    tables: Vec<Table>,
    cff: bool,
    units_per_em: u16,
    num_glyphs: u16,
    loca_long: bool,
    loca: Option<(usize, usize)>,
    glyf: Option<(usize, usize)>,
    hmtx: Option<(usize, usize)>,
    num_hmetrics: u16,
    cmaps: Vec<CmapSub>,
    /// `post` glyph names sorted by name, built on the first lookup.
    names: OnceLock<Vec<(String, u16)>>,
}

impl std::fmt::Debug for TrueType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrueType")
            .field("bytes", &self.data.len())
            .field("cff", &self.cff)
            .field("units_per_em", &self.units_per_em)
            .field("num_glyphs", &self.num_glyphs)
            .field("cmaps", &self.cmap_subtables())
            .finish()
    }
}

/// A glyph's points while it is decoded: `ends` holds the exclusive end
/// index of each contour, so composites can append and match points.
#[derive(Default)]
struct Outline {
    points: Vec<(f64, f64, bool)>,
    ends: Vec<usize>,
    components: usize,
}

impl TrueType {
    /// An sfnt with version 0x00010000, `true` or `OTTO` (for `OTTO` only
    /// the tables are read: outlines then come from
    /// [`Cff::from_opentype`](super::cff::Cff::from_opentype)), or a
    /// TrueType collection (`ttcf`, its first face).
    pub fn parse(data: Vec<u8>) -> Result<TrueType, FontError> {
        TrueType::parse_collection(data, 0)
    }

    /// Face `index` of a TrueType collection. A plain sfnt is accepted as
    /// a collection of one face.
    pub fn parse_collection(data: Vec<u8>, index: u32) -> Result<TrueType, FontError> {
        let tag = be_u32(&data, 0).ok_or_else(|| err("too short"))?;
        let dir = if tag == TAG_TTCF {
            let count = be_u32(&data, 8).ok_or_else(|| err("truncated collection header"))?;
            if index >= count {
                return Err(err("collection face index out of range"));
            }
            let off = (index as usize).checked_mul(4).and_then(|o| o.checked_add(12)).ok_or_else(|| err("bad face"))?;
            be_u32(&data, off).ok_or_else(|| err("truncated collection header"))? as usize
        } else if index == 0 {
            0
        } else {
            return Err(err("not a collection"));
        };
        TrueType::parse_face(data, dir)
    }

    fn parse_face(data: Vec<u8>, dir: usize) -> Result<TrueType, FontError> {
        let (version, tables) = read_directory(&data, dir)?;
        let has = |tag: &[u8; 4]| tables.iter().any(|t| &t.tag == tag);
        let known = matches!(version, VERSION_1 | TAG_TRUE | TAG_OTTO);
        // Broken producers write odd versions; accept them when the
        // directory still names the tables that make a font.
        if !known && !(has(b"glyf") || has(b"CFF ") || has(b"head")) {
            return Err(err("not a TrueType or OpenType font"));
        }
        if tables.is_empty() {
            return Err(err("no tables"));
        }
        let mut font = TrueType {
            cff: version == TAG_OTTO || has(b"CFF "),
            data,
            tables,
            units_per_em: 1000,
            num_glyphs: 0,
            loca_long: false,
            loca: None,
            glyf: None,
            hmtx: None,
            num_hmetrics: 0,
            cmaps: Vec::new(),
            names: OnceLock::new(),
        };
        let range = |font: &TrueType, tag: &[u8; 4]| font.find(tag).map(|t| (t.start, t.end));
        let head = font.table(b"head");
        let upem = head.and_then(|h| be_u16(h, 18)).filter(|&u| u != 0);
        let loc_format = head.and_then(|h| be_i16(h, 50));
        font.units_per_em = upem.unwrap_or(1000);
        let maxp_glyphs = font.table(b"maxp").and_then(|m| be_u16(m, 4));
        font.loca = range(&font, b"loca");
        font.glyf = range(&font, b"glyf");
        font.hmtx = range(&font, b"hmtx");
        font.num_hmetrics = font.table(b"hhea").and_then(|h| be_u16(h, 34)).unwrap_or(0);
        if let Some((ls, le)) = font.loca {
            let len = le - ls;
            let declared_long =
                loc_format.map(|f| f != 0).unwrap_or(len >= 4 * (maxp_glyphs.unwrap_or(0) as usize + 1));
            let entries = if declared_long { len / 4 } else { len / 2 };
            let n = maxp_glyphs.map(|n| n as usize).unwrap_or(entries.saturating_sub(1));
            // A wrong indexToLocFormat is common in subset fonts: keep the
            // declared format unless only the other one reads sensibly.
            font.loca_long = declared_long;
            if !font.loca_plausible(declared_long, n) && font.loca_plausible(!declared_long, n) {
                font.loca_long = !declared_long;
            }
        }
        // Subset fonts sometimes understate maxp: glyphs that loca still
        // locates stay reachable (every read is bounds-checked anyway).
        let loca_glyphs = font.loca.map(|(ls, le)| {
            let entries = (le - ls) / if font.loca_long { 4 } else { 2 };
            entries.saturating_sub(1).min(u16::MAX as usize) as u16
        });
        font.num_glyphs = maxp_glyphs.unwrap_or(0).max(loca_glyphs.unwrap_or(0));
        font.cmaps = font.read_cmaps();
        Ok(font)
    }

    fn find(&self, tag: &[u8; 4]) -> Option<Table> {
        self.tables.iter().find(|t| &t.tag == tag).copied()
    }

    /// `true` when the outlines are CFF (an `OTTO` font, or a `CFF ` table).
    pub fn is_cff(&self) -> bool {
        self.cff
    }

    /// The bytes of a table, clamped to the font data.
    pub fn table(&self, tag: &[u8; 4]) -> Option<&[u8]> {
        let t = self.find(tag)?;
        self.data.get(t.start..t.end)
    }

    /// Units per em from `head` (1000 when `head` is missing or says 0).
    pub fn units_per_em(&self) -> u16 {
        self.units_per_em
    }

    /// The glyph count: the larger of `maxp`'s and what `loca` holds.
    pub fn num_glyphs(&self) -> u16 {
        self.num_glyphs
    }

    /// The offsets in `loca` read as one format: increasing, inside `glyf`.
    fn loca_plausible(&self, long: bool, n: usize) -> bool {
        let (Some((ls, le)), Some((gs, ge))) = (self.loca, self.glyf) else {
            return true;
        };
        let size = if long { 4 } else { 2 };
        let entries = ((le - ls) / size).min(n.saturating_add(1));
        if entries < 2 {
            return false;
        }
        let glyf_len = ge - gs;
        let mut prev = 0usize;
        for i in 0..entries {
            let Some(off) = self.loca_entry(long, i) else {
                return false;
            };
            if off < prev || off > glyf_len {
                return false;
            }
            prev = off;
        }
        true
    }

    fn loca_entry(&self, long: bool, i: usize) -> Option<usize> {
        let (ls, le) = self.loca?;
        let loca = self.data.get(ls..le)?;
        if long {
            be_u32(loca, i.checked_mul(4)?).map(|v| v as usize)
        } else {
            be_u16(loca, i.checked_mul(2)?).map(|v| v as usize * 2)
        }
    }

    /// The `glyf` bytes of one glyph; empty for a glyph without outline.
    fn glyph_data(&self, gid: u16) -> Option<&[u8]> {
        let (gs, ge) = self.glyf?;
        let start = self.loca_entry(self.loca_long, gid as usize)?;
        let end = self.loca_entry(self.loca_long, gid as usize + 1)?;
        if end < start {
            return None;
        }
        let glyf = self.data.get(gs..ge)?;
        if start == end {
            return Some(&[]);
        }
        // An entry past the end of `glyf` is clamped; a glyph cut short
        // then fails to decode on its own.
        glyf.get(start..end.min(glyf.len()))
    }

    /// The outline of a glyph in font units, y up, with quadratic curves;
    /// every contour is closed. Simple and composite glyphs (offsets, point
    /// matching, scales and 2x2 transforms). A glyph without outline (a
    /// space) is an empty path; `None` means out of range or unreadable.
    pub fn glyph_path(&self, gid: u16) -> Option<Path> {
        if gid >= self.num_glyphs {
            return None;
        }
        let mut outline = Outline::default();
        self.load_glyph(gid, 0, &mut outline)?;
        let mut path = Path::new();
        let mut start = 0;
        for &end in &outline.ends {
            contour_to_path(outline.points.get(start..end)?, &mut path);
            start = end;
            if path.segs.len() > MAX_SEGMENTS {
                return None;
            }
        }
        Some(path)
    }

    fn load_glyph(&self, gid: u16, depth: u32, out: &mut Outline) -> Option<()> {
        let data = self.glyph_data(gid)?;
        if data.is_empty() {
            return Some(());
        }
        let contours = be_i16(data, 0)?;
        if contours >= 0 { simple_glyph(data, contours as usize, out) } else { self.composite_glyph(data, depth, out) }
    }

    fn composite_glyph(&self, data: &[u8], depth: u32, out: &mut Outline) -> Option<()> {
        const ARG_1_AND_2_ARE_WORDS: u16 = 0x0001;
        const ARGS_ARE_XY_VALUES: u16 = 0x0002;
        const WE_HAVE_A_SCALE: u16 = 0x0008;
        const MORE_COMPONENTS: u16 = 0x0020;
        const WE_HAVE_AN_X_AND_Y_SCALE: u16 = 0x0040;
        const WE_HAVE_A_TWO_BY_TWO: u16 = 0x0080;
        const SCALED_COMPONENT_OFFSET: u16 = 0x0800;
        const UNSCALED_COMPONENT_OFFSET: u16 = 0x1000;
        if depth >= MAX_COMPOSITE_DEPTH {
            return None;
        }
        let base = out.points.len();
        let mut r = Reader::new(data, 10);
        loop {
            out.components += 1;
            if out.components > MAX_COMPONENTS {
                return None;
            }
            let flags = r.u16()?;
            let glyph = r.u16()?;
            let (arg1, arg2) = match (flags & ARG_1_AND_2_ARE_WORDS != 0, flags & ARGS_ARE_XY_VALUES != 0) {
                (true, true) => (r.i16()? as i32, r.i16()? as i32),
                (true, false) => (r.u16()? as i32, r.u16()? as i32),
                (false, true) => (r.i8()? as i32, r.i8()? as i32),
                (false, false) => (r.u8()? as i32, r.u8()? as i32),
            };
            let f2dot14 = |v: i16| v as f64 / 16384.0;
            let (mut a, mut b, mut c, mut d) = (1.0, 0.0, 0.0, 1.0);
            if flags & WE_HAVE_A_SCALE != 0 {
                a = f2dot14(r.i16()?);
                d = a;
            } else if flags & WE_HAVE_AN_X_AND_Y_SCALE != 0 {
                a = f2dot14(r.i16()?);
                d = f2dot14(r.i16()?);
            } else if flags & WE_HAVE_A_TWO_BY_TWO != 0 {
                a = f2dot14(r.i16()?);
                b = f2dot14(r.i16()?);
                c = f2dot14(r.i16()?);
                d = f2dot14(r.i16()?);
            }
            let start = out.points.len();
            self.load_glyph(glyph, depth + 1, out)?;
            if out.points.len() > MAX_POINTS {
                return None;
            }
            for p in &mut out.points[start..] {
                (p.0, p.1) = (a * p.0 + c * p.1, b * p.0 + d * p.1);
            }
            let (dx, dy) = if flags & ARGS_ARE_XY_VALUES != 0 {
                let (x, y) = (arg1 as f64, arg2 as f64);
                // Microsoft's rule: the offset is not scaled unless asked.
                if flags & SCALED_COMPONENT_OFFSET != 0 && flags & UNSCALED_COMPONENT_OFFSET == 0 {
                    (a * x + c * y, b * x + d * y)
                } else {
                    (x, y)
                }
            } else {
                // Point matching: the parent's point `arg1` (points so far in
                // this composite) meets the component's point `arg2`.
                let parent = base.checked_add(arg1 as usize).filter(|&i| i < start)?;
                let child = start.checked_add(arg2 as usize).filter(|&i| i < out.points.len())?;
                let (p, q) = (out.points[parent], out.points[child]);
                (p.0 - q.0, p.1 - q.1)
            };
            if dx != 0.0 || dy != 0.0 {
                for p in &mut out.points[start..] {
                    p.0 += dx;
                    p.1 += dy;
                }
            }
            if flags & MORE_COMPONENTS == 0 {
                return Some(());
            }
        }
    }

    /// The advance width of a glyph from `hmtx`, in font units. Glyphs past
    /// `numberOfHMetrics` take the last advance, as the format says.
    pub fn advance_width(&self, gid: u16) -> Option<u16> {
        if self.num_glyphs != 0 && gid >= self.num_glyphs {
            return None;
        }
        let (hs, he) = self.hmtx?;
        let n = (self.num_hmetrics as usize).min((he - hs) / 4);
        if n == 0 {
            return None;
        }
        let i = (gid as usize).min(n - 1);
        be_u16(&self.data, hs + i * 4)
    }

    /// The (platform, encoding) pairs of the `cmap` subtables this module
    /// can read, in table order.
    pub fn cmap_subtables(&self) -> Vec<(u16, u16)> {
        let mut out: Vec<(u16, u16)> = Vec::new();
        for s in &self.cmaps {
            if !out.contains(&(s.platform, s.encoding)) {
                out.push((s.platform, s.encoding));
            }
        }
        out
    }

    fn read_cmaps(&self) -> Vec<CmapSub> {
        let Some(t) = self.find(b"cmap") else {
            return Vec::new();
        };
        let d = &self.data;
        let count = be_u16(d, t.start + 2).unwrap_or(0) as usize;
        let mut out = Vec::new();
        for i in 0..count {
            let rec = t.start + 4 + 8 * i;
            let (Some(platform), Some(encoding), Some(offset)) =
                (be_u16(d, rec), be_u16(d, rec + 2), be_u32(d, rec + 4))
            else {
                break;
            };
            let Some(start) = t.start.checked_add(offset as usize).filter(|&s| s < t.end) else {
                continue;
            };
            let Some(format) = be_u16(d, start) else {
                continue;
            };
            if !matches!(format, 0 | 2 | 4 | 6 | 10 | 12 | 13) {
                continue;
            }
            let mut sub = CmapSub { platform, encoding, start, format, sorted: true };
            sub.sorted = self.cmap_sorted(&sub);
            out.push(sub);
        }
        out
    }

    fn cmap_sorted(&self, s: &CmapSub) -> bool {
        let d = &self.data;
        match s.format {
            4 => {
                let segs = be_u16(d, s.start + 6).unwrap_or(0) as usize / 2;
                let mut prev = None;
                for i in 0..segs {
                    let Some(end) = be_u16(d, s.start + 14 + 2 * i) else {
                        return false;
                    };
                    if prev.is_some_and(|p| end <= p) {
                        return false;
                    }
                    prev = Some(end);
                }
                true
            }
            12 | 13 => {
                let groups = self.cmap_groups(s);
                let mut prev = None;
                for i in 0..groups {
                    let g = s.start + 16 + 12 * i;
                    let (Some(first), Some(last)) = (be_u32(d, g), be_u32(d, g + 4)) else {
                        return false;
                    };
                    if last < first || prev.is_some_and(|p| first <= p) {
                        return false;
                    }
                    prev = Some(last);
                }
                true
            }
            _ => true,
        }
    }

    fn cmap_groups(&self, s: &CmapSub) -> usize {
        let n = be_u32(&self.data, s.start + 12).unwrap_or(0) as usize;
        n.min(self.data.len().saturating_sub(s.start + 16) / 12)
    }

    /// The glyph for a character code in the first readable subtable of
    /// (platform, encoding): formats 0, 2, 4, 6, 10, 12 and 13. `None`
    /// when the code maps to glyph 0 or past the glyph count.
    pub fn cmap_lookup(&self, platform: u16, encoding: u16, code: u32) -> Option<u16> {
        self.cmaps
            .iter()
            .filter(|s| s.platform == platform && s.encoding == encoding)
            .filter_map(|s| self.cmap_sub_lookup(s, code))
            .find(|&g| g != 0 && (self.num_glyphs == 0 || g < self.num_glyphs))
    }

    fn cmap_sub_lookup(&self, s: &CmapSub, code: u32) -> Option<u16> {
        let d = &self.data;
        let st = s.start;
        match s.format {
            0 => {
                let c = usize::try_from(code).ok().filter(|&c| c < 256)?;
                d.get(st + 6 + c).map(|&g| g as u16)
            }
            2 => self.cmap2(st, code),
            4 => self.cmap4(s, code),
            6 => {
                let first = be_u16(d, st + 6)? as u32;
                let count = be_u16(d, st + 8)? as u32;
                let i = code.checked_sub(first).filter(|&i| i < count)?;
                be_u16(d, st + 10 + 2 * i as usize)
            }
            10 => {
                let first = be_u32(d, st + 12)?;
                let count = be_u32(d, st + 16)?;
                let i = code.checked_sub(first).filter(|&i| i < count)?;
                be_u16(d, st.checked_add(20)?.checked_add((i as usize).checked_mul(2)?)?)
            }
            12 | 13 => {
                let groups = self.cmap_groups(s);
                let group = |i: usize| {
                    let g = st + 16 + 12 * i;
                    Some((be_u32(d, g)?, be_u32(d, g + 4)?, be_u32(d, g + 8)?))
                };
                let found = if s.sorted {
                    let (mut lo, mut hi) = (0, groups);
                    let mut hit = None;
                    while lo < hi {
                        let mid = (lo + hi) / 2;
                        let (first, last, gid) = group(mid)?;
                        if code < first {
                            hi = mid;
                        } else if code > last {
                            lo = mid + 1;
                        } else {
                            hit = Some((first, gid));
                            break;
                        }
                    }
                    hit
                } else {
                    (0..groups).filter_map(group).find(|g| g.0 <= code && code <= g.1).map(|g| (g.0, g.2))
                };
                let (first, gid) = found?;
                let gid = if s.format == 12 { gid.checked_add(code - first)? } else { gid };
                u16::try_from(gid).ok()
            }
            _ => None,
        }
    }

    /// Format 2, the high-byte mapping of the CJK code pages: a code whose
    /// first byte has no subheader is a one-byte code.
    fn cmap2(&self, st: usize, code: u32) -> Option<u16> {
        let d = &self.data;
        if code > 0xFFFF {
            return None;
        }
        let (high, low) = ((code >> 8) as usize, (code & 0xFF) as usize);
        let (key_byte, byte) = if code < 0x100 { (low, low) } else { (high, low) };
        let key = be_u16(d, st + 6 + 2 * key_byte)? as usize / 8;
        // A one-byte code must use subheader 0; a two-byte code must not.
        if (code < 0x100) != (key == 0) {
            return None;
        }
        let sh = st + 6 + 512 + key * 8;
        let first = be_u16(d, sh)? as usize;
        let count = be_u16(d, sh + 2)? as usize;
        let delta = be_i16(d, sh + 4)?;
        let range_offset = be_u16(d, sh + 6)? as usize;
        let i = byte.checked_sub(first).filter(|&i| i < count)?;
        let addr = (sh + 6).checked_add(range_offset)?.checked_add(2 * i)?;
        let g = be_u16(d, addr)?;
        if g == 0 { Some(0) } else { Some(g.wrapping_add(delta as u16)) }
    }

    fn cmap4(&self, s: &CmapSub, code: u32) -> Option<u16> {
        let d = &self.data;
        let code = u16::try_from(code).ok()?;
        let st = s.start;
        let seg_x2 = be_u16(d, st + 6)? as usize;
        let segs = seg_x2 / 2;
        let ends = st + 14;
        let starts = ends + seg_x2 + 2;
        let deltas = starts + seg_x2;
        let ranges = deltas + seg_x2;
        let seg = |i: usize| -> Option<u16> {
            let start = be_u16(d, starts + 2 * i)?;
            if code < start {
                return None;
            }
            let delta = be_u16(d, deltas + 2 * i)?;
            let ro = be_u16(d, ranges + 2 * i)? as usize;
            if ro == 0 {
                return Some(code.wrapping_add(delta));
            }
            let addr = (ranges + 2 * i).checked_add(ro)?.checked_add(2 * (code - start) as usize)?;
            let g = be_u16(d, addr)?;
            if g == 0 { Some(0) } else { Some(g.wrapping_add(delta)) }
        };
        if s.sorted {
            let (mut lo, mut hi) = (0, segs);
            while lo < hi {
                let mid = (lo + hi) / 2;
                if be_u16(d, ends + 2 * mid)? < code {
                    lo = mid + 1;
                } else {
                    hi = mid;
                }
            }
            if lo < segs { seg(lo) } else { None }
        } else {
            (0..segs).find(|&i| be_u16(d, ends + 2 * i).is_some_and(|e| code <= e)).and_then(seg)
        }
    }

    /// The glyph with this `post` name: formats 1, 2 and 2.5.
    pub fn glyph_by_name(&self, name: &str) -> Option<u16> {
        let names = self.names.get_or_init(|| self.post_names());
        let i = names.binary_search_by(|e| e.0.as_str().cmp(name)).ok()?;
        Some(names[i].1)
    }

    fn post_names(&self) -> Vec<(String, u16)> {
        let mut out = Vec::new();
        let Some(post) = self.table(b"post") else {
            return out;
        };
        let limit = |n: usize| if self.num_glyphs == 0 { n } else { n.min(self.num_glyphs as usize) };
        match be_u32(post, 0) {
            Some(0x0001_0000) => {
                for (gid, name) in MAC_GLYPHS.iter().enumerate().take(limit(MAC_GLYPHS.len())) {
                    out.push((name.to_string(), gid as u16));
                }
            }
            Some(0x0002_0000) => {
                let n = limit(be_u16(post, 32).unwrap_or(0) as usize);
                let count = be_u16(post, 32).unwrap_or(0) as usize;
                // The Pascal strings after the index array, in order.
                let mut strings: Vec<&[u8]> = Vec::new();
                let mut r = Reader::new(post, 34 + 2 * count);
                while let Some(len) = r.u8() {
                    match r.bytes(len as usize) {
                        Some(s) => strings.push(s),
                        None => break,
                    }
                }
                for gid in 0..n {
                    let Some(idx) = be_u16(post, 34 + 2 * gid) else {
                        break;
                    };
                    let idx = idx as usize;
                    let name = if idx < MAC_GLYPHS.len() {
                        MAC_GLYPHS[idx].to_string()
                    } else {
                        match strings.get(idx - MAC_GLYPHS.len()) {
                            Some(s) => String::from_utf8_lossy(s).into_owned(),
                            None => continue,
                        }
                    };
                    out.push((name, gid as u16));
                }
            }
            Some(0x0002_5000) => {
                let n = limit(be_u16(post, 32).unwrap_or(0) as usize);
                for gid in 0..n {
                    let Some(&off) = post.get(34 + gid) else {
                        break;
                    };
                    let idx = gid as isize + off as i8 as isize;
                    if let Some(name) = usize::try_from(idx).ok().and_then(|i| MAC_GLYPHS.get(i)) {
                        out.push((name.to_string(), gid as u16));
                    }
                }
            }
            _ => {}
        }
        // Sorted for binary search; a repeated name keeps its lowest glyph.
        out.sort();
        out.dedup_by(|b, a| a.0 == b.0);
        out
    }

    /// Ascender and descender in font units: `hhea`, else the typographic
    /// values of `OS/2`.
    pub fn ascent_descent(&self) -> Option<(i16, i16)> {
        if let Some(h) = self.table(b"hhea")
            && let (Some(a), Some(d)) = (be_i16(h, 4), be_i16(h, 6))
            && (a != 0 || d != 0)
        {
            return Some((a, d));
        }
        let os2 = self.table(b"OS/2")?;
        let (a, d) = (be_i16(os2, 68)?, be_i16(os2, 70)?);
        if a != 0 || d != 0 {
            return Some((a, d));
        }
        // Last resort: the Windows clipping values (descent is positive there).
        let (wa, wd) = (be_u16(os2, 74)?, be_u16(os2, 76)?);
        if wa == 0 && wd == 0 {
            return None;
        }
        Some((i16::try_from(wa).unwrap_or(i16::MAX), (-(wd as i32)).max(i16::MIN as i32) as i16))
    }
}

/// Decodes a simple glyph's contours into `out`.
fn simple_glyph(data: &[u8], contours: usize, out: &mut Outline) -> Option<()> {
    const ON_CURVE: u8 = 0x01;
    const X_SHORT: u8 = 0x02;
    const Y_SHORT: u8 = 0x04;
    const REPEAT: u8 = 0x08;
    const X_SAME_OR_POSITIVE: u8 = 0x10;
    const Y_SAME_OR_POSITIVE: u8 = 0x20;
    let mut r = Reader::new(data, 10);
    let mut ends = Vec::with_capacity(contours);
    let mut last: i64 = -1;
    for _ in 0..contours {
        let e = r.u16()? as i64;
        if e < last {
            return None;
        }
        ends.push(e);
        last = e;
    }
    let n = (last + 1) as usize;
    if out.points.len() + n > MAX_POINTS {
        return None;
    }
    let ins_len = r.u16()? as usize;
    r.skip(ins_len)?;
    let mut flags = Vec::with_capacity(n);
    while flags.len() < n {
        let f = r.u8()?;
        flags.push(f);
        if f & REPEAT != 0 {
            let count = r.u8()? as usize;
            for _ in 0..count.min(n - flags.len()) {
                flags.push(f);
            }
        }
    }
    let mut xs = Vec::with_capacity(n);
    // i64: 65 536 points of 16-bit deltas cannot overflow it.
    let mut x: i64 = 0;
    for &f in &flags {
        if f & X_SHORT != 0 {
            let dx = r.u8()? as i64;
            x += if f & X_SAME_OR_POSITIVE != 0 { dx } else { -dx };
        } else if f & X_SAME_OR_POSITIVE == 0 {
            x += r.i16()? as i64;
        }
        xs.push(x);
    }
    let base = out.points.len();
    let mut y: i64 = 0;
    for (&f, &x) in flags.iter().zip(&xs) {
        if f & Y_SHORT != 0 {
            let dy = r.u8()? as i64;
            y += if f & Y_SAME_OR_POSITIVE != 0 { dy } else { -dy };
        } else if f & Y_SAME_OR_POSITIVE == 0 {
            y += r.i16()? as i64;
        }
        out.points.push((x as f64, y as f64, f & ON_CURVE != 0));
    }
    let mut prev = None;
    for &e in &ends {
        // A repeated end point is an empty contour: nothing to draw.
        if prev != Some(e) {
            out.ends.push(base + e as usize + 1);
        }
        prev = Some(e);
    }
    Some(())
}

/// One closed contour of on- and off-curve points as lines and
/// quadratic curves; two off-curve points in a row imply the on-curve
/// point midway.
fn contour_to_path(pts: &[(f64, f64, bool)], path: &mut Path) {
    let n = pts.len();
    if n == 0 {
        return;
    }
    let mid = |a: (f64, f64, bool), b: (f64, f64, bool)| ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    let first_on = pts.iter().position(|p| p.2);
    let (start, first) = match first_on {
        Some(i) => ((pts[i].0, pts[i].1), i + 1),
        // All points off-curve: start midway between the last and first.
        None => (mid(pts[n - 1], pts[0]), 0),
    };
    path.move_to(start.0, start.1);
    let count = if first_on.is_some() { n - 1 } else { n };
    let mut pending: Option<(f64, f64, bool)> = None;
    for k in 0..count {
        let p = pts[(first + k) % n];
        if p.2 {
            match pending.take() {
                Some(c) => path.quad_to(c.0, c.1, p.0, p.1),
                None => path.line_to(p.0, p.1),
            }
        } else {
            if let Some(c) = pending {
                let m = mid(c, p);
                path.quad_to(c.0, c.1, m.0, m.1);
            }
            pending = Some(p);
        }
    }
    if let Some(c) = pending {
        path.quad_to(c.0, c.1, start.0, start.1);
    }
    path.close();
}

/// The 258 standard Macintosh glyph names `post` formats 1, 2 and 2.5 index.
#[rustfmt::skip]
static MAC_GLYPHS: [&str; 258] = [
    ".notdef", ".null", "nonmarkingreturn", "space", "exclam", "quotedbl",
    "numbersign", "dollar", "percent", "ampersand", "quotesingle", "parenleft",
    "parenright", "asterisk", "plus", "comma", "hyphen", "period",
    "slash", "zero", "one", "two", "three", "four",
    "five", "six", "seven", "eight", "nine", "colon",
    "semicolon", "less", "equal", "greater", "question", "at",
    "A", "B", "C", "D", "E", "F",
    "G", "H", "I", "J", "K", "L",
    "M", "N", "O", "P", "Q", "R",
    "S", "T", "U", "V", "W", "X",
    "Y", "Z", "bracketleft", "backslash", "bracketright", "asciicircum",
    "underscore", "grave", "a", "b", "c", "d",
    "e", "f", "g", "h", "i", "j",
    "k", "l", "m", "n", "o", "p",
    "q", "r", "s", "t", "u", "v",
    "w", "x", "y", "z", "braceleft", "bar",
    "braceright", "asciitilde", "Adieresis", "Aring", "Ccedilla", "Eacute",
    "Ntilde", "Odieresis", "Udieresis", "aacute", "agrave", "acircumflex",
    "adieresis", "atilde", "aring", "ccedilla", "eacute", "egrave",
    "ecircumflex", "edieresis", "iacute", "igrave", "icircumflex", "idieresis",
    "ntilde", "oacute", "ograve", "ocircumflex", "odieresis", "otilde",
    "uacute", "ugrave", "ucircumflex", "udieresis", "dagger", "degree",
    "cent", "sterling", "section", "bullet", "paragraph", "germandbls",
    "registered", "copyright", "trademark", "acute", "dieresis", "notequal",
    "AE", "Oslash", "infinity", "plusminus", "lessequal", "greaterequal",
    "yen", "mu", "partialdiff", "summation", "product", "pi",
    "integral", "ordfeminine", "ordmasculine", "Omega", "ae", "oslash",
    "questiondown", "exclamdown", "logicalnot", "radical", "florin", "approxequal",
    "Delta", "guillemotleft", "guillemotright", "ellipsis", "nonbreakingspace", "Agrave",
    "Atilde", "Otilde", "OE", "oe", "endash", "emdash",
    "quotedblleft", "quotedblright", "quoteleft", "quoteright", "divide", "lozenge",
    "ydieresis", "Ydieresis", "fraction", "currency", "guilsinglleft", "guilsinglright",
    "fi", "fl", "daggerdbl", "periodcentered", "quotesinglbase", "quotedblbase",
    "perthousand", "Acircumflex", "Ecircumflex", "Aacute", "Edieresis", "Egrave",
    "Iacute", "Icircumflex", "Idieresis", "Igrave", "Oacute", "Ocircumflex",
    "apple", "Ograve", "Uacute", "Ucircumflex", "Ugrave", "dotlessi",
    "circumflex", "tilde", "macron", "breve", "dotaccent", "ring",
    "cedilla", "hungarumlaut", "ogonek", "caron", "Lslash", "lslash",
    "Scaron", "scaron", "Zcaron", "zcaron", "brokenbar", "Eth",
    "eth", "Yacute", "yacute", "Thorn", "thorn", "minus",
    "multiply", "onesuperior", "twosuperior", "threesuperior", "onehalf", "onequarter",
    "threequarters", "franc", "Gbreve", "gbreve", "Idotaccent", "Scedilla",
    "scedilla", "Cacute", "cacute", "Ccaron", "ccaron", "dcroat",
];

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::geom::{Point, Seg};

    pub(crate) fn m(x: f64, y: f64) -> Seg {
        Seg::Move(Point::new(x, y))
    }
    pub(crate) fn l(x: f64, y: f64) -> Seg {
        Seg::Line(Point::new(x, y))
    }
    pub(crate) fn q(cx: f64, cy: f64, x: f64, y: f64) -> Seg {
        Seg::Quad(Point::new(cx, cy), Point::new(x, y))
    }
    pub(crate) fn z() -> Seg {
        Seg::Close
    }

    /// An sfnt from (tag, bytes) pairs, tables 4-byte aligned.
    pub(crate) fn sfnt(version: u32, tables: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend(version.to_be_bytes());
        out.extend((tables.len() as u16).to_be_bytes());
        out.extend([0u8; 6]);
        let mut offset = 12 + 16 * tables.len();
        let mut body = Vec::new();
        for (tag, data) in tables {
            out.extend(*tag);
            out.extend(0u32.to_be_bytes());
            out.extend((offset as u32).to_be_bytes());
            out.extend((data.len() as u32).to_be_bytes());
            body.extend(data);
            while body.len() % 4 != 0 {
                body.push(0);
            }
            offset = 12 + 16 * tables.len() + body.len();
        }
        out.extend(body);
        out
    }

    fn be16(v: &mut Vec<u8>, x: i32) {
        v.extend((x as u16).to_be_bytes());
    }

    /// A simple glyph from contours of (x, y, on-curve) points, using
    /// short, long, same-as-previous and repeated-flag encodings.
    fn simple(contours: &[&[(i16, i16, bool)]]) -> Vec<u8> {
        let mut v = Vec::new();
        be16(&mut v, contours.len() as i32);
        let all: Vec<_> = contours.iter().flat_map(|c| c.iter().copied()).collect();
        let (x0, x1) = (all.iter().map(|p| p.0).min().unwrap(), all.iter().map(|p| p.0).max().unwrap());
        let (y0, y1) = (all.iter().map(|p| p.1).min().unwrap(), all.iter().map(|p| p.1).max().unwrap());
        for b in [x0, y0, x1, y1] {
            be16(&mut v, b as i32);
        }
        let mut end = -1i32;
        for c in contours {
            end += c.len() as i32;
            be16(&mut v, end);
        }
        be16(&mut v, 2);
        v.extend([0xB0, 0x01]); // two bytes of instructions to skip
        let (mut flags, mut xs, mut ys) = (Vec::new(), Vec::new(), Vec::new());
        let (mut px, mut py) = (0i32, 0i32);
        for p in &all {
            let (dx, dy) = (p.0 as i32 - px, p.1 as i32 - py);
            (px, py) = (p.0 as i32, p.1 as i32);
            let mut f = p.2 as u8;
            if dx == 0 {
                f |= 0x10;
            } else if dx.abs() < 256 {
                f |= 0x02 | if dx > 0 { 0x10 } else { 0 };
                xs.push(dx.unsigned_abs() as u8);
            } else {
                xs.extend((dx as i16).to_be_bytes());
            }
            if dy == 0 {
                f |= 0x20;
            } else if dy.abs() < 256 {
                f |= 0x04 | if dy > 0 { 0x20 } else { 0 };
                ys.push(dy.unsigned_abs() as u8);
            } else {
                ys.extend((dy as i16).to_be_bytes());
            }
            flags.push(f);
        }
        // Run-length encode the flags.
        let mut i = 0;
        while i < flags.len() {
            let mut run = 1;
            while i + run < flags.len() && flags[i + run] == flags[i] && run < 255 {
                run += 1;
            }
            if run > 1 {
                v.extend([flags[i] | 0x08, (run - 1) as u8]);
            } else {
                v.push(flags[i]);
            }
            i += run;
        }
        v.extend(xs);
        v.extend(ys);
        v
    }

    fn composite(parts: &[(u16, u16, Vec<u8>)]) -> Vec<u8> {
        let mut v = Vec::new();
        be16(&mut v, -1);
        v.extend([0u8; 8]);
        for (i, (flags, gid, args)) in parts.iter().enumerate() {
            let more = if i + 1 < parts.len() { 0x20 } else { 0 };
            be16(&mut v, (flags | more) as i32);
            be16(&mut v, *gid as i32);
            v.extend(args);
        }
        v
    }

    const ON: bool = true;
    const OFF: bool = false;

    pub(crate) fn glyphs() -> Vec<Vec<u8>> {
        let square: &[(i16, i16, bool)] = &[(0, 0, ON), (0, 100, ON), (100, 100, ON), (100, 0, ON)];
        let mut words = Vec::new();
        for x in [1000i16, -300, 0x2000] {
            words.extend(x.to_be_bytes());
        }
        let mut rot = vec![10u8, (-10i8) as u8];
        for x in [0i16, 0x4000, -0x4000, 0] {
            rot.extend(x.to_be_bytes());
        }
        vec![
            Vec::new(),
            simple(&[square]),
            simple(&[
                &[(0, 0, ON), (0, 300, ON), (300, 300, ON), (300, 0, ON)],
                &[(100, 100, ON), (100, 200, OFF), (200, 200, OFF), (200, 100, ON)],
                &[(400, 0, OFF), (500, 0, ON), (500, 1000, ON)],
            ]),
            composite(&[(0x0001 | 0x0002 | 0x0008, 1, words), (0x0002 | 0x0080, 1, rot)]),
            composite(&[(0x0002, 4, vec![0, 0])]),
            composite(&[(0x0002, 1, vec![0, 0]), (0, 1, vec![2, 0])]),
            simple(&[&[(0, 0, OFF), (0, 100, OFF), (100, 100, OFF), (100, 0, OFF)]]),
        ]
    }

    fn cmap() -> Vec<u8> {
        // (3,1) format 4: A-C by delta, a-b through the glyph array.
        let mut f4 = Vec::new();
        let segs: [(u16, u16, i16, u16); 3] = [(0x41, 0x43, -0x40, 0), (0x61, 0x62, 0, 1), (0xFFFF, 0xFFFF, 1, 0)];
        be16(&mut f4, 4);
        be16(&mut f4, 0);
        be16(&mut f4, 0);
        be16(&mut f4, 6);
        f4.extend([0u8; 6]);
        for s in segs {
            be16(&mut f4, s.1 as i32);
        }
        be16(&mut f4, 0);
        for s in segs {
            be16(&mut f4, s.0 as i32);
        }
        for s in segs {
            be16(&mut f4, s.2 as i32);
        }
        // Segment 1's range offset points just past the offsets array.
        be16(&mut f4, 0);
        be16(&mut f4, 4);
        be16(&mut f4, 0);
        be16(&mut f4, 5);
        be16(&mut f4, 6);
        // (3,10) format 12.
        let mut f12 = Vec::new();
        be16(&mut f12, 12);
        be16(&mut f12, 0);
        f12.extend(28u32.to_be_bytes());
        f12.extend(0u32.to_be_bytes());
        f12.extend(1u32.to_be_bytes());
        for x in [0x1F600u32, 0x1F601, 3] {
            f12.extend(x.to_be_bytes());
        }
        // (1,0) format 6: 0x20 -> 0 (absent), 0x21 -> 1.
        let mut f6 = Vec::new();
        for x in [6, 0, 0, 0x20, 2, 0, 1] {
            be16(&mut f6, x);
        }
        // (3,2) format 2: one-byte 0x41 -> 3, two-byte 0x8140..0x8141 -> 1, 2.
        let mut f2 = Vec::new();
        for x in [2, 0, 0] {
            be16(&mut f2, x);
        }
        for b in 0..256 {
            be16(&mut f2, if b == 0x81 { 8 } else { 0 });
        }
        for x in [0, 256, 0, 10, 0x40, 2, 0, 514] {
            be16(&mut f2, x);
        }
        for c in 0..256 {
            be16(&mut f2, if c == 0x41 { 3 } else { 0 });
        }
        be16(&mut f2, 1);
        be16(&mut f2, 2);
        let subs: [(u16, u16, &Vec<u8>); 4] = [(3, 1, &f4), (3, 10, &f12), (1, 0, &f6), (3, 2, &f2)];
        let mut v = Vec::new();
        be16(&mut v, 0);
        be16(&mut v, subs.len() as i32);
        let mut off = 4 + 8 * subs.len();
        for (p, e, d) in subs {
            be16(&mut v, p as i32);
            be16(&mut v, e as i32);
            v.extend((off as u32).to_be_bytes());
            off += d.len();
        }
        for (_, _, d) in subs {
            v.extend(d);
        }
        v
    }

    fn post() -> Vec<u8> {
        let mut v = Vec::new();
        v.extend(0x0002_0000u32.to_be_bytes());
        v.extend([0u8; 28]);
        let idx = [0u16, 258, 259, 36, 260, 261, 36];
        be16(&mut v, idx.len() as i32);
        for i in idx {
            be16(&mut v, i as i32);
        }
        for name in ["square", "ring", "loop", "matched"] {
            v.push(name.len() as u8);
            v.extend(name.as_bytes());
        }
        v
    }

    /// The test font; `long` picks the loca format, `declared` the format
    /// `head` claims.
    pub(crate) fn font_bytes(long: bool, declared_long: bool) -> Vec<u8> {
        let glyphs = glyphs();
        let (mut glyf, mut loca) = (Vec::new(), Vec::new());
        for g in &glyphs {
            if long {
                loca.extend((glyf.len() as u32).to_be_bytes());
            } else {
                be16(&mut loca, glyf.len() as i32 / 2);
            }
            glyf.extend(g);
            if glyf.len() % 2 == 1 {
                glyf.push(0);
            }
        }
        if long {
            loca.extend((glyf.len() as u32).to_be_bytes());
        } else {
            be16(&mut loca, glyf.len() as i32 / 2);
        }
        let mut head = vec![0u8; 54];
        head[0..4].copy_from_slice(&VERSION_1.to_be_bytes());
        head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
        head[18..20].copy_from_slice(&2048u16.to_be_bytes());
        for (i, b) in [-100i16, -400, 1200, 1100].iter().enumerate() {
            head[36 + 2 * i..38 + 2 * i].copy_from_slice(&b.to_be_bytes());
        }
        head[50..52].copy_from_slice(&(declared_long as i16).to_be_bytes());
        let mut maxp = Vec::new();
        maxp.extend(0x0000_5000u32.to_be_bytes());
        be16(&mut maxp, glyphs.len() as i32);
        let mut hhea = vec![0u8; 36];
        hhea[4..6].copy_from_slice(&1800i16.to_be_bytes());
        hhea[6..8].copy_from_slice(&(-400i16).to_be_bytes());
        hhea[34..36].copy_from_slice(&3u16.to_be_bytes());
        let mut hmtx = Vec::new();
        for (adv, lsb) in [(500, 0), (600, 0), (700, 0)] {
            be16(&mut hmtx, adv);
            be16(&mut hmtx, lsb);
        }
        for _ in 3..glyphs.len() {
            be16(&mut hmtx, 0);
        }
        sfnt(
            VERSION_1,
            &[
                (b"cmap", cmap()),
                (b"glyf", glyf),
                (b"head", head),
                (b"hhea", hhea),
                (b"hmtx", hmtx),
                (b"loca", loca),
                (b"maxp", maxp),
                (b"post", post()),
            ],
        )
    }

    fn segs(font: &TrueType, gid: u16) -> Vec<Seg> {
        font.glyph_path(gid).unwrap().segs
    }

    #[test]
    fn simple_glyphs() {
        let font = TrueType::parse(font_bytes(false, false)).unwrap();
        assert!(!font.is_cff());
        assert_eq!(font.units_per_em(), 2048);
        assert_eq!(font.num_glyphs(), 7);
        assert_eq!(font.glyph_path(0), Some(Path::new()));
        assert_eq!(segs(&font, 1), vec![m(0.0, 0.0), l(0.0, 100.0), l(100.0, 100.0), l(100.0, 0.0), z()]);
        assert_eq!(
            segs(&font, 2),
            vec![
                m(0.0, 0.0),
                l(0.0, 300.0),
                l(300.0, 300.0),
                l(300.0, 0.0),
                z(),
                m(100.0, 100.0),
                q(100.0, 200.0, 150.0, 200.0),
                q(200.0, 200.0, 200.0, 100.0),
                z(),
                m(500.0, 0.0),
                l(500.0, 1000.0),
                q(400.0, 0.0, 500.0, 0.0),
                z(),
            ]
        );
        assert_eq!(
            segs(&font, 6),
            vec![
                m(50.0, 0.0),
                q(0.0, 0.0, 0.0, 50.0),
                q(0.0, 100.0, 50.0, 100.0),
                q(100.0, 100.0, 100.0, 50.0),
                q(100.0, 0.0, 50.0, 0.0),
                z(),
            ]
        );
        assert_eq!(font.glyph_path(7), None);
    }

    #[test]
    fn composite_glyphs() {
        let font = TrueType::parse(font_bytes(true, true)).unwrap();
        assert_eq!(
            segs(&font, 3),
            vec![
                m(1000.0, -300.0),
                l(1000.0, -250.0),
                l(1050.0, -250.0),
                l(1050.0, -300.0),
                z(),
                m(10.0, -10.0),
                l(-90.0, -10.0),
                l(-90.0, 90.0),
                l(10.0, 90.0),
                z(),
            ]
        );
        // A composite that contains itself stops at the depth limit.
        assert_eq!(font.glyph_path(4), None);
        // Point matching: the second square's point 0 meets the first's point 2.
        assert_eq!(&segs(&font, 5)[5..], &[m(100.0, 100.0), l(100.0, 200.0), l(200.0, 200.0), l(200.0, 100.0), z()]);
    }

    #[test]
    fn composite_fan_out_is_bounded() {
        // Glyph k is 200 copies of glyph k + 1; the last one is empty.
        let mut glyphs = vec![Vec::new()];
        for k in 1..8u16 {
            glyphs.push(composite(&vec![(0x0002, k + 1, vec![0, 0]); 200]));
        }
        glyphs.push(Vec::new());
        let (mut glyf, mut loca) = (Vec::new(), Vec::new());
        for g in &glyphs {
            loca.extend((glyf.len() as u32).to_be_bytes());
            glyf.extend(g);
        }
        loca.extend((glyf.len() as u32).to_be_bytes());
        let mut head = vec![0u8; 54];
        head[50..52].copy_from_slice(&1i16.to_be_bytes());
        let font = TrueType::parse(sfnt(VERSION_1, &[(b"glyf", glyf), (b"head", head), (b"loca", loca)])).unwrap();
        assert_eq!(font.num_glyphs(), 9);
        assert_eq!(font.glyph_path(7), Some(Path::new()));
        assert_eq!(font.glyph_path(1), None);
    }

    #[test]
    fn wrong_loca_format_is_detected() {
        for (long, declared) in [(true, false), (false, true)] {
            let font = TrueType::parse(font_bytes(long, declared)).unwrap();
            assert_eq!(segs(&font, 1).len(), 5, "long {long} declared {declared}");
            assert_eq!(segs(&font, 2).len(), 13);
        }
    }

    #[test]
    fn cmaps_metrics_and_names() {
        let font = TrueType::parse(font_bytes(false, false)).unwrap();
        assert_eq!(font.cmap_subtables(), vec![(3, 1), (3, 10), (1, 0), (3, 2)]);
        assert_eq!(font.cmap_lookup(3, 1, 0x41), Some(1));
        assert_eq!(font.cmap_lookup(3, 1, 0x43), Some(3));
        assert_eq!(font.cmap_lookup(3, 1, 0x61), Some(5));
        assert_eq!(font.cmap_lookup(3, 1, 0x62), Some(6));
        assert_eq!(font.cmap_lookup(3, 1, 0x44), None);
        assert_eq!(font.cmap_lookup(3, 1, 0x1F600), None);
        assert_eq!(font.cmap_lookup(3, 10, 0x1F600), Some(3));
        assert_eq!(font.cmap_lookup(3, 10, 0x1F601), Some(4));
        assert_eq!(font.cmap_lookup(3, 10, 0x1F602), None);
        assert_eq!(font.cmap_lookup(1, 0, 0x21), Some(1));
        assert_eq!(font.cmap_lookup(1, 0, 0x20), None);
        assert_eq!(font.cmap_lookup(3, 2, 0x41), Some(3));
        assert_eq!(font.cmap_lookup(3, 2, 0x8140), Some(1));
        assert_eq!(font.cmap_lookup(3, 2, 0x8141), Some(2));
        assert_eq!(font.cmap_lookup(3, 2, 0x81), None);
        assert_eq!(font.cmap_lookup(9, 9, 0x41), None);
        assert_eq!(font.advance_width(0), Some(500));
        assert_eq!(font.advance_width(2), Some(700));
        assert_eq!(font.advance_width(6), Some(700));
        assert_eq!(font.advance_width(7), None);
        assert_eq!(font.glyph_by_name("square"), Some(1));
        assert_eq!(font.glyph_by_name("A"), Some(3));
        assert_eq!(font.glyph_by_name("matched"), Some(5));
        assert_eq!(font.glyph_by_name("B"), None);
        assert_eq!(font.ascent_descent(), Some((1800, -400)));
        assert!(font.table(b"glyf").is_some());
        assert!(font.table(b"OS/2").is_none());
    }

    #[test]
    fn collections_select_a_face() {
        let face = font_bytes(false, false);
        // Two faces sharing nothing: the second face's directory is rebased.
        let mut ttc = Vec::new();
        ttc.extend(*b"ttcf");
        ttc.extend(0x0001_0000u32.to_be_bytes());
        ttc.extend(2u32.to_be_bytes());
        let first = 20u32;
        let second = first + face.len() as u32;
        ttc.extend(first.to_be_bytes());
        ttc.extend(second.to_be_bytes());
        for base in [first, second] {
            let mut f = face.clone();
            let n = u16::from_be_bytes([f[4], f[5]]) as usize;
            for i in 0..n {
                let o = 12 + 16 * i + 8;
                let v = u32::from_be_bytes([f[o], f[o + 1], f[o + 2], f[o + 3]]) + base;
                f[o..o + 4].copy_from_slice(&v.to_be_bytes());
            }
            ttc.extend(f);
        }
        for index in 0..2 {
            let font = TrueType::parse_collection(ttc.clone(), index).unwrap();
            assert_eq!(font.num_glyphs(), 7);
            assert_eq!(segs(&font, 1).len(), 5);
        }
        assert!(TrueType::parse_collection(ttc.clone(), 2).is_err());
        assert_eq!(TrueType::parse(ttc).unwrap().cmap_lookup(3, 1, 0x41), Some(1));
    }

    #[test]
    fn missing_tables_degrade() {
        // glyf and loca alone: no cmap, post, hmtx, head or maxp.
        let full = TrueType::parse(font_bytes(false, false)).unwrap();
        let glyf = full.table(b"glyf").unwrap().to_vec();
        let loca = full.table(b"loca").unwrap().to_vec();
        let font = TrueType::parse(sfnt(VERSION_1, &[(b"glyf", glyf), (b"loca", loca)])).unwrap();
        assert_eq!(font.units_per_em(), 1000);
        assert_eq!(font.num_glyphs(), 7);
        assert_eq!(segs(&font, 1).len(), 5);
        assert_eq!(font.cmap_lookup(3, 1, 0x41), None);
        assert_eq!(font.glyph_by_name("A"), None);
        assert_eq!(font.advance_width(1), None);
        assert_eq!(font.ascent_descent(), None);
        assert!(TrueType::parse(Vec::new()).is_err());
        assert!(TrueType::parse(b"not a font at all".to_vec()).is_err());
    }

    fn exercise(font: &TrueType) {
        for gid in 0..font.num_glyphs().min(64) {
            let _ = font.glyph_path(gid);
            let _ = font.advance_width(gid);
        }
        for (p, e) in font.cmap_subtables() {
            for code in [0, 0x20, 0x41, 0x61, 0x8140, 0xFFFF, 0x1F600] {
                let _ = font.cmap_lookup(p, e, code);
            }
        }
        let _ = font.glyph_by_name("A");
        let _ = font.ascent_descent();
        let _ = format!("{font:?}");
    }

    /// Deterministic random mutations of `data`: a few bytes overwritten,
    /// sometimes truncated as well.
    pub(crate) fn fuzz(data: &[u8], rounds: usize, mut f: impl FnMut(Vec<u8>)) {
        let mut s: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = move || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        for _ in 0..rounds {
            let mut d = data.to_vec();
            for _ in 0..1 + next() % 8 {
                let i = (next() % d.len() as u64) as usize;
                d[i] = next() as u8;
            }
            if next() % 4 == 0 {
                d.truncate((next() % d.len() as u64) as usize);
            }
            f(d);
        }
    }

    #[test]
    fn hostile_inputs_never_panic() {
        let data = font_bytes(false, false);
        fuzz(&data, 20_000, |d| {
            if let Ok(font) = TrueType::parse(d) {
                exercise(&font);
            }
        });
        for cut in 0..data.len() {
            if let Ok(font) = TrueType::parse(data[..cut].to_vec()) {
                exercise(&font);
            }
        }
        for i in 0..data.len() {
            for v in [0x00, 0xFF, 0x80, 0x7F] {
                let mut d = data.clone();
                d[i] = v;
                if let Ok(font) = TrueType::parse(d) {
                    exercise(&font);
                }
            }
        }
    }

    /// Font files under `dir` with one of `exts`, sorted, at most `max`.
    pub(crate) fn find_fonts(dirs: &[&str], exts: &[&str], max: usize) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        let mut stack: Vec<std::path::PathBuf> = dirs.iter().map(std::path::PathBuf::from).collect();
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p
                    .extension()
                    .and_then(|x| x.to_str())
                    .is_some_and(|x| exts.iter().any(|e| e.eq_ignore_ascii_case(x)))
                {
                    out.push(p);
                }
            }
        }
        out.sort();
        out.truncate(max);
        out
    }

    #[test]
    fn system_truetype_fonts() {
        for (n, path) in find_fonts(&["/usr/share/fonts"], &["ttf"], 6).into_iter().enumerate() {
            let Ok(data) = std::fs::read(&path) else {
                continue;
            };
            if n == 0 {
                // A real font cut short anywhere still never panics.
                for k in 1..8 {
                    if let Ok(font) = TrueType::parse(data[..data.len() * k / 8].to_vec()) {
                        (0..font.num_glyphs()).for_each(|g| drop(font.glyph_path(g)));
                    }
                }
            }
            let font = TrueType::parse(data).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            if font.is_cff() {
                continue;
            }
            let head = font.table(b"head").unwrap();
            let bbox: Vec<f64> = (0..4).map(|i| be_i16(head, 36 + 2 * i).unwrap() as f64).collect();
            let mut drawn = 0;
            for gid in 0..font.num_glyphs() {
                if let Some(p) = font.glyph_path(gid) {
                    drawn += !p.is_empty() as usize;
                }
            }
            assert!(drawn > 0, "{}", path.display());
            for ch in ['A', 'o', 'g'] {
                let Some(gid) = font.cmap_lookup(3, 1, ch as u32).or_else(|| font.cmap_lookup(0, 3, ch as u32)) else {
                    continue;
                };
                let (x0, y0, x1, y1) = font.glyph_path(gid).unwrap().bounds().unwrap();
                let slack = font.units_per_em() as f64 * 0.05;
                assert!(x0 >= bbox[0] - slack && y0 >= bbox[1] - slack, "{} {ch}", path.display());
                assert!(x1 <= bbox[2] + slack && y1 <= bbox[3] + slack, "{} {ch}", path.display());
                assert!(x1 > x0 && y1 > y0);
                assert!(font.advance_width(gid).unwrap() > 0);
            }
        }
    }
}
