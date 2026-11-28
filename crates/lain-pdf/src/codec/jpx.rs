//! JPEG 2000 (JPXDecode): a Part 1 decoder for JP2 files and raw J2K codestreams.
//!
//! The codestream is parsed up front into tiles: tile-part bodies are concatenated per tile and packed packet
//! headers (PPM, PPT) are collected per tile. Each tile then runs tier 2 (packet headers with tag trees, in the
//! tile's progression order or its POC list), tier 1 (EBCOT: the MQ decoder and the three coding passes per
//! code-block), dequantisation, the inverse 5-3 or 9-7 wavelet, the inverse component transform and the DC level
//! shift, into one plane per component. The JP2 boxes then decide how planes become output channels: palette
//! (pclr/cmap), channel definitions (cdef) and sYCC; subsampled components are upsampled to the full grid.
//!
//! Damage degrades like OpenJPEG: a truncated codestream decodes with the packets that are there (missing
//! coefficients stay zero), and a packet that cannot be read ends its tile rather than failing the image.

use super::CodecError;
use std::collections::{BTreeMap, HashMap};

const MAX_DIM: u32 = 65535;
const MAX_PIXELS: u64 = 1 << 28;
/// Samples over all components (each held as an `i32` plane until the end).
const MAX_SAMPLES: u64 = 1 << 29;
const MAX_COMPONENTS: usize = 256;
const MAX_TILES: u64 = 65535;
/// Code-blocks and precincts are only built when a packet reaches them; these bound what a tile may build.
const MAX_CODEBLOCKS: usize = 1 << 21;
const MAX_PRECINCTS: u64 = 1 << 20;
/// Packet slots visited per tile (decoded or skipped): stops files built to spin on empty progressions.
const MAX_VISITS: u64 = 1 << 26;

const SOC: u16 = 0xff4f;
const SIZ: u16 = 0xff51;
const COD: u16 = 0xff52;
const COC: u16 = 0xff53;
const QCD: u16 = 0xff5c;
const QCC: u16 = 0xff5d;
const RGN: u16 = 0xff5e;
const POC: u16 = 0xff5f;
const PPM: u16 = 0xff60;
const PPT: u16 = 0xff61;
const SOT: u16 = 0xff90;
const SOD: u16 = 0xff93;

// Code-block style flags (COD/COC SPcod).
const CBLK_BYPASS: u8 = 1;
const CBLK_RESET: u8 = 2;
const CBLK_TERMALL: u8 = 4;
const CBLK_VSC: u8 = 8;
const CBLK_SEGSYM: u8 = 32;

/// A decoded image.
#[derive(Debug, Clone)]
pub struct Decoded {
    pub width: u32,
    pub height: u32,
    /// Colour components in `data`.
    pub components: u8,
    /// Interleaved 8-bit samples (deeper samples scaled to 8 bits, signed shifted to unsigned).
    pub data: Vec<u8>,
    /// The last component is alpha (JP2 cdef).
    pub has_alpha: bool,
    /// JP2 colr enumerated colour space (16 sRGB, 17 grey, 18 sYCC, 12 CMYK), 0 unknown/ICC. sYCC data is
    /// converted to RGB and reported as 16.
    pub colorspace: u32,
}

fn err(msg: impl Into<String>) -> CodecError {
    CodecError(format!("jpx: {}", msg.into()))
}

/// Decodes a JP2 file (boxes) or a raw J2K codestream (FF 4F FF 51).
pub fn decode(data: &[u8]) -> Result<Decoded, CodecError> {
    let (cs, jp2) = parse_file(data)?;
    let cs = parse_codestream(cs)?;
    let (chans, has_alpha) = layout(&cs.siz, &jp2)?;
    let (planes, mct_used) = cs.decode_planes()?;
    Ok(render(&cs.siz, &planes, &jp2, &chans, has_alpha, mct_used))
}

/// Header only: (width, height, components) as [`decode`] would return them.
pub fn info(data: &[u8]) -> Result<(u32, u32, u8), CodecError> {
    let (cs, jp2) = parse_file(data)?;
    let siz = read_siz(cs)?;
    let (chans, _) = layout(&siz, &jp2)?;
    Ok((siz.x1 - siz.x0, siz.y1 - siz.y0, chans.len() as u8))
}

fn be16(d: &[u8], i: usize) -> Option<u16> {
    let b = d.get(i..i.checked_add(2)?)?;
    Some(u16::from_be_bytes([b[0], b[1]]))
}

fn be32(d: &[u8], i: usize) -> Option<u32> {
    let b = d.get(i..i.checked_add(4)?)?;
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn be64(d: &[u8], i: usize) -> Option<u64> {
    Some((u64::from(be32(d, i)?) << 32) | u64::from(be32(d, i + 4)?))
}

// ---------------------------------------------------------------------------------------------------------------
// JP2 boxes
// ---------------------------------------------------------------------------------------------------------------

#[derive(Default)]
struct Jp2 {
    colorspace: u32,
    palette: Option<Palette>,
    /// (component, mapping type, palette column)
    cmap: Vec<(u16, u8, u8)>,
    /// (channel, type, association)
    cdef: Vec<(u16, u16, u16)>,
}

struct Palette {
    entries: usize,
    /// (bit depth, signed) per column.
    depth: Vec<(u32, bool)>,
    /// `entries` rows of `depth.len()` values.
    values: Vec<i32>,
}

/// The boxes of one level; a box running past the end is cut short (truncated files keep their codestream).
fn boxes(d: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while let (Some(l), Some(t)) = (be32(d, pos), d.get(pos + 4..pos + 8)) {
        let (hdr, len) = match l {
            0 => (8, (d.len() - pos) as u64),
            1 => match be64(d, pos + 8) {
                Some(xl) => (16, xl),
                None => break,
            },
            l => (8, u64::from(l)),
        };
        if len < hdr as u64 {
            break;
        }
        let end = (pos as u64).saturating_add(len).min(d.len() as u64) as usize;
        let Some(body) = d.get(pos + hdr..end) else { break };
        out.push(([t[0], t[1], t[2], t[3]], body));
        if end <= pos || l == 0 {
            break;
        }
        pos = end;
    }
    out
}

fn parse_file(d: &[u8]) -> Result<(&[u8], Jp2), CodecError> {
    if d.starts_with(&[0xff, 0x4f, 0xff, 0x51]) {
        return Ok((d, Jp2::default()));
    }
    if d.get(4..8) != Some(b"jP  ") {
        return Err(err("neither a JP2 file nor a J2K codestream"));
    }
    let mut jp2 = Jp2::default();
    let mut cs = None;
    for (t, body) in boxes(d) {
        match &t {
            b"jp2h" => parse_jp2h(body, &mut jp2),
            b"jp2c" if cs.is_none() => cs = Some(body),
            _ => {}
        }
    }
    let cs = cs.ok_or_else(|| err("no codestream box"))?;
    Ok((cs, jp2))
}

fn parse_jp2h(d: &[u8], jp2: &mut Jp2) {
    let mut have_colr = false;
    for (t, b) in boxes(d) {
        match &t {
            b"colr" if !have_colr => {
                have_colr = true;
                if b.first() == Some(&1) {
                    jp2.colorspace = be32(b, 3).unwrap_or(0);
                }
            }
            b"pclr" => jp2.palette = parse_pclr(b),
            b"cmap" => {
                jp2.cmap =
                    b.as_chunks::<4>().0.iter().map(|c| (u16::from_be_bytes([c[0], c[1]]), c[2], c[3])).collect();
            }
            b"cdef" => {
                let n = be16(b, 0).unwrap_or(0) as usize;
                jp2.cdef = b
                    .get(2..)
                    .unwrap_or(&[])
                    .as_chunks::<6>()
                    .0
                    .iter()
                    .take(n)
                    .map(|c| {
                        (
                            u16::from_be_bytes([c[0], c[1]]),
                            u16::from_be_bytes([c[2], c[3]]),
                            u16::from_be_bytes([c[4], c[5]]),
                        )
                    })
                    .collect();
            }
            _ => {}
        }
    }
}

fn parse_pclr(b: &[u8]) -> Option<Palette> {
    let entries = be16(b, 0)? as usize;
    let npc = *b.get(2)? as usize;
    if entries == 0 || entries > 1024 || npc == 0 {
        return None;
    }
    let depth: Vec<(u32, bool)> =
        b.get(3..3 + npc)?.iter().map(|&v| (u32::from(v & 0x7f) + 1, v & 0x80 != 0)).collect();
    if depth.iter().any(|&(d, _)| d > 16) {
        return None;
    }
    let mut pos = 3 + npc;
    let mut values = Vec::with_capacity(entries * npc);
    for _ in 0..entries {
        for &(dp, signed) in &depth {
            let nb = dp.div_ceil(8) as usize;
            let mut v: u32 = 0;
            for &byte in b.get(pos..pos + nb)? {
                v = (v << 8) | u32::from(byte);
            }
            pos += nb;
            let v = v & ((1u32 << dp) - 1);
            let v = if signed && v >> (dp - 1) != 0 { v as i32 - (1i32 << dp) } else { v as i32 };
            values.push(v);
        }
    }
    Some(Palette { entries, depth, values })
}

/// One output channel: a component, optionally through a palette column.
#[derive(Clone, Copy)]
struct Chan {
    comp: usize,
    pcol: Option<usize>,
}

/// Output channels in order (colours by association, then others, then one alpha) and whether alpha is last.
fn layout(siz: &Siz, jp2: &Jp2) -> Result<(Vec<Chan>, bool), CodecError> {
    let nc = siz.comps.len();
    let mut chans: Vec<Chan> = match (&jp2.palette, jp2.cmap.is_empty()) {
        (Some(pal), false) => jp2
            .cmap
            .iter()
            .map(|&(comp, mtyp, pcol)| {
                let comp = comp as usize;
                let pcol = if mtyp == 1 { Some(pcol as usize) } else { None };
                if comp >= nc || pcol.is_some_and(|p| p >= pal.depth.len()) {
                    return Err(err("bad component mapping"));
                }
                Ok(Chan { comp, pcol })
            })
            .collect::<Result<_, _>>()?,
        _ => (0..nc).map(|comp| Chan { comp, pcol: None }).collect(),
    };
    let mut has_alpha = false;
    if !jp2.cdef.is_empty() {
        let mut info = vec![(0xffffu16, 0xffffu16); chans.len()];
        for &(cn, typ, asoc) in &jp2.cdef {
            if let Some(slot) = info.get_mut(cn as usize) {
                *slot = (typ, asoc);
            }
        }
        let mut colour: Vec<usize> = (0..chans.len()).filter(|&i| info[i].0 == 0).collect();
        colour.sort_by_key(|&i| info[i].1);
        let other = (0..chans.len()).filter(|&i| info[i].0 > 2);
        let alpha = (0..chans.len()).find(|&i| info[i].0 == 1 || info[i].0 == 2);
        has_alpha = alpha.is_some();
        let order: Vec<usize> = colour.into_iter().chain(other).chain(alpha).collect();
        chans = order.into_iter().map(|i| chans[i]).collect();
    }
    if chans.is_empty() || chans.len() > 255 {
        return Err(err("unsupported channel count"));
    }
    Ok((chans, has_alpha))
}

// ---------------------------------------------------------------------------------------------------------------
// Codestream headers
// ---------------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct CompSiz {
    prec: u32,
    signed: bool,
    dx: u32,
    dy: u32,
}

struct Siz {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    tw: u32,
    th: u32,
    tx0: u32,
    ty0: u32,
    ntx: u32,
    nty: u32,
    comps: Vec<CompSiz>,
}

impl Siz {
    /// The component's sample rectangle on its own grid.
    fn comp_rect(&self, c: usize) -> (u32, u32, u32, u32) {
        let cs = self.comps[c];
        (self.x0.div_ceil(cs.dx), self.y0.div_ceil(cs.dy), self.x1.div_ceil(cs.dx), self.y1.div_ceil(cs.dy))
    }
}

fn parse_siz(s: &[u8]) -> Result<Siz, CodecError> {
    let g = |i| be32(s, i).ok_or_else(|| err("short SIZ"));
    let (x1, y1, x0, y0, tw, th, tx0, ty0) = (g(2)?, g(6)?, g(10)?, g(14)?, g(18)?, g(22)?, g(26)?, g(30)?);
    let nc = be16(s, 34).ok_or_else(|| err("short SIZ"))? as usize;
    if nc == 0 || nc > MAX_COMPONENTS {
        return Err(err(format!("unsupported component count {nc}")));
    }
    let mut comps = Vec::with_capacity(nc);
    for i in 0..nc {
        let b = s.get(36 + 3 * i..39 + 3 * i).ok_or_else(|| err("short SIZ"))?;
        let prec = u32::from(b[0] & 0x7f) + 1;
        if prec > 16 {
            return Err(err(format!("unsupported bit depth {prec}")));
        }
        if b[1] == 0 || b[2] == 0 {
            return Err(err("zero subsampling"));
        }
        comps.push(CompSiz { prec, signed: b[0] & 0x80 != 0, dx: u32::from(b[1]), dy: u32::from(b[2]) });
    }
    if x1 <= x0 || y1 <= y0 || x1 - x0 > MAX_DIM || y1 - y0 > MAX_DIM {
        return Err(err("bad image size"));
    }
    if u64::from(x1 - x0) * u64::from(y1 - y0) > MAX_PIXELS {
        return Err(err("image too large"));
    }
    if tw == 0 || th == 0 || tx0 > x0 || ty0 > y0 || u64::from(tx0) + u64::from(tw) <= u64::from(x0) {
        return Err(err("bad tile grid"));
    }
    if u64::from(ty0) + u64::from(th) <= u64::from(y0) {
        return Err(err("bad tile grid"));
    }
    let ntx = (x1 - tx0).div_ceil(tw);
    let nty = (y1 - ty0).div_ceil(th);
    if u64::from(ntx) * u64::from(nty) > MAX_TILES {
        return Err(err("too many tiles"));
    }
    let siz = Siz { x0, y0, x1, y1, tw, th, tx0, ty0, ntx, nty, comps };
    let mut samples = 0u64;
    for c in 0..nc {
        let (a, b, cx, d) = siz.comp_rect(c);
        if cx <= a || d <= b {
            return Err(err("component without samples"));
        }
        samples += u64::from(cx - a) * u64::from(d - b);
    }
    if samples > MAX_SAMPLES {
        return Err(err("image too large"));
    }
    Ok(siz)
}

fn read_siz(d: &[u8]) -> Result<Siz, CodecError> {
    if be16(d, 0) != Some(SOC) || be16(d, 2) != Some(SIZ) {
        return Err(err("missing SOC/SIZ"));
    }
    let len = be16(d, 4).ok_or_else(|| err("short SIZ"))? as usize;
    parse_siz(d.get(6..4 + len.max(2)).ok_or_else(|| err("short SIZ"))?)
}

#[derive(Clone)]
struct Coding {
    levels: u32,
    /// Code-block width and height exponents.
    cbw: u32,
    cbh: u32,
    style: u8,
    reversible: bool,
    /// Precinct exponents (PPx, PPy) per resolution.
    prec: Vec<(u32, u32)>,
}

#[derive(Clone, Copy)]
struct Global {
    sop: bool,
    eph: bool,
    order: u8,
    layers: u32,
    mct: u8,
}

#[derive(Clone)]
struct Quant {
    style: u8,
    guard: u32,
    /// (exponent, mantissa) per subband, or one entry for derived quantisation.
    steps: Vec<(u32, u32)>,
}

#[derive(Clone, Copy)]
struct Poc {
    rs: u32,
    cs: u32,
    lye: u32,
    re: u32,
    ce: u32,
    order: u8,
}

#[derive(Clone)]
struct Params {
    cod: Option<(Global, Coding)>,
    coc: Vec<Option<Coding>>,
    qcd: Option<Quant>,
    qcc: Vec<Option<Quant>>,
    rgn: Vec<Option<u8>>,
    poc: Vec<Poc>,
}

impl Params {
    fn new(nc: usize) -> Params {
        Params { cod: None, coc: vec![None; nc], qcd: None, qcc: vec![None; nc], rgn: vec![None; nc], poc: Vec::new() }
    }

    fn marker(&mut self, m: u16, s: &[u8]) -> Result<(), CodecError> {
        let nc = self.coc.len();
        match m {
            COD => self.cod = Some(parse_cod(s)?),
            COC => {
                let (c, rest) = comp_index(s, nc)?;
                let scoc = *rest.first().ok_or_else(|| err("short COC"))?;
                self.coc[c] = Some(parse_coding(&rest[1..], scoc & 1 != 0)?);
            }
            QCD => self.qcd = Some(parse_quant(s)?),
            QCC => {
                let (c, rest) = comp_index(s, nc)?;
                self.qcc[c] = Some(parse_quant(rest)?);
            }
            RGN => {
                let (c, rest) = comp_index(s, nc)?;
                if let [0, shift, ..] = rest {
                    self.rgn[c] = Some(*shift);
                }
            }
            POC => self.poc.extend(parse_poc(s, nc)?),
            _ => {}
        }
        Ok(())
    }
}

fn comp_index(s: &[u8], nc: usize) -> Result<(usize, &[u8]), CodecError> {
    let (c, rest) = if nc < 257 {
        (*s.first().ok_or_else(|| err("short segment"))? as usize, &s[1.min(s.len())..])
    } else {
        (be16(s, 0).ok_or_else(|| err("short segment"))? as usize, &s[2.min(s.len())..])
    };
    if c >= nc {
        return Err(err("component index out of range"));
    }
    Ok((c, rest))
}

fn parse_coding(b: &[u8], explicit_precincts: bool) -> Result<Coding, CodecError> {
    let [levels, cbw, cbh, style, transform, rest @ ..] = b else { return Err(err("short coding style")) };
    let levels = u32::from(*levels);
    if levels > 32 || *cbw > 8 || *cbh > 8 || cbw + cbh > 8 || *transform > 1 {
        return Err(err("bad coding style"));
    }
    let prec = if explicit_precincts {
        let p = rest.get(..levels as usize + 1).ok_or_else(|| err("short precinct sizes"))?;
        p.iter().map(|&v| (u32::from(v & 15), u32::from(v >> 4))).collect()
    } else {
        vec![(15, 15); levels as usize + 1]
    };
    Ok(Coding {
        levels,
        cbw: u32::from(*cbw) + 2,
        cbh: u32::from(*cbh) + 2,
        style: *style,
        reversible: *transform == 1,
        prec,
    })
}

fn parse_cod(s: &[u8]) -> Result<(Global, Coding), CodecError> {
    let [scod, order, l0, l1, mct, rest @ ..] = s else { return Err(err("short COD")) };
    let layers = u32::from(u16::from_be_bytes([*l0, *l1]));
    if *order > 4 || layers == 0 {
        return Err(err("bad COD"));
    }
    let g = Global { sop: scod & 2 != 0, eph: scod & 4 != 0, order: *order, layers, mct: *mct };
    Ok((g, parse_coding(rest, scod & 1 != 0)?))
}

fn parse_quant(s: &[u8]) -> Result<Quant, CodecError> {
    let (&sq, rest) = s.split_first().ok_or_else(|| err("short QCD"))?;
    let style = sq & 0x1f;
    let pair = |c: &[u8; 2]| {
        let v = u32::from(u16::from_be_bytes([c[0], c[1]]));
        (v >> 11, v & 0x7ff)
    };
    let steps: Vec<(u32, u32)> = match style {
        0 => rest.iter().map(|&b| (u32::from(b >> 3), 0)).collect(),
        1 => rest.as_chunks::<2>().0.iter().take(1).map(pair).collect(),
        2 => rest.as_chunks::<2>().0.iter().map(pair).collect(),
        _ => return Err(err("bad quantization style")),
    };
    if steps.is_empty() {
        return Err(err("no quantization steps"));
    }
    Ok(Quant { style, guard: u32::from(sq >> 5), steps })
}

fn parse_poc(s: &[u8], nc: usize) -> Result<Vec<Poc>, CodecError> {
    let wide = nc >= 257;
    let size = if wide { 9 } else { 7 };
    let mut out = Vec::new();
    for e in s.chunks_exact(size) {
        let comp = |i: usize| if wide { u32::from(u16::from_be_bytes([e[i], e[i + 1]])) } else { u32::from(e[i]) };
        let cs = comp(1);
        let o = if wide { 3 } else { 2 };
        let lye = u32::from(u16::from_be_bytes([e[o], e[o + 1]]));
        let re = u32::from(e[o + 2]);
        let mut ce = comp(o + 3);
        if ce == 0 {
            ce = if wide { 16384 } else { 256 };
        }
        let order = e[size - 1];
        if order > 4 {
            return Err(err("bad POC"));
        }
        out.push(Poc { rs: u32::from(e[0]), cs, lye, re, ce, order });
    }
    Ok(out)
}

struct Tile {
    params: Params,
    data: Vec<u8>,
    /// Packed packet headers (PPT, or this tile's PPM chunks), if the codestream packs them.
    headers: Option<Vec<u8>>,
}

struct Codestream {
    siz: Siz,
    main: Params,
    tiles: BTreeMap<usize, Tile>,
}

fn parse_codestream(d: &[u8]) -> Result<Codestream, CodecError> {
    if be16(d, 0) != Some(SOC) {
        return Err(err("missing SOC"));
    }
    let siz = read_siz(d)?;
    let nc = siz.comps.len();
    let mut main = Params::new(nc);
    let mut ppm: Option<Vec<u8>> = None;
    let mut pos = 2;
    loop {
        let m = be16(d, pos).ok_or_else(|| err("truncated main header"))?;
        if m == SOT {
            break;
        }
        if m < 0xff00 {
            return Err(err("bad marker in main header"));
        }
        if (0xff30..=0xff3f).contains(&m) {
            pos += 2;
            continue;
        }
        let len = be16(d, pos + 2).ok_or_else(|| err("truncated main header"))? as usize;
        let s = d.get(pos + 4..pos + 2 + len.max(2)).ok_or_else(|| err("truncated main header"))?;
        match m {
            SIZ => {}
            PPM => ppm.get_or_insert_with(Vec::new).extend_from_slice(s.get(1..).unwrap_or(&[])),
            _ => main.marker(m, s)?,
        }
        pos += 2 + len.max(2);
    }
    // PPM data is a run of (Nppm, headers) chunks, one per tile-part in codestream order.
    let chunks: Option<Vec<&[u8]>> = ppm.as_ref().map(|raw| {
        let mut v = Vec::new();
        let mut i = 0usize;
        while let Some(n) = be32(raw, i) {
            let start = i + 4;
            let end = start.saturating_add(n as usize).min(raw.len());
            v.push(&raw[start..end]);
            i = end;
        }
        v
    });
    let ntiles = siz.ntx as usize * siz.nty as usize;
    let mut tiles: BTreeMap<usize, Tile> = BTreeMap::new();
    let mut tp_index = 0usize;
    while be16(d, pos) == Some(SOT) {
        let (Some(10), Some(isot), Some(psot)) = (be16(d, pos + 2), be16(d, pos + 4), be32(d, pos + 6)) else {
            break;
        };
        let start = pos;
        let end = if psot == 0 { d.len() } else { (start as u64 + u64::from(psot)).min(d.len() as u64) as usize };
        pos += 12;
        if end < pos {
            break;
        }
        let isot = isot as usize;
        let mut scratch = None;
        let tile = if isot < ntiles {
            tiles.entry(isot).or_insert_with(|| Tile { params: Params::new(nc), data: Vec::new(), headers: None })
        } else {
            // A tile index outside the grid: parse past it without keeping anything.
            scratch.insert(Tile { params: Params::new(nc), data: Vec::new(), headers: None })
        };
        let mut body_start = None;
        while let Some(m) = be16(d, pos).filter(|_| pos + 2 <= end) {
            if m == SOD {
                body_start = Some(pos + 2);
                break;
            }
            if m < 0xff00 {
                break;
            }
            if (0xff30..=0xff3f).contains(&m) {
                pos += 2;
                continue;
            }
            let Some(len) = be16(d, pos + 2).map(|l| l as usize).filter(|&l| l >= 2 && pos + 2 + l <= end) else {
                break;
            };
            let s = &d[pos + 4..pos + 2 + len];
            if m == PPT {
                tile.headers.get_or_insert_with(Vec::new).extend_from_slice(s.get(1..).unwrap_or(&[]));
            } else if tile.params.marker(m, s).is_err() {
                break;
            }
            pos += 2 + len;
        }
        if let Some(b) = body_start {
            let mut body = &d[b..end];
            if psot == 0 && body.ends_with(&[0xff, 0xd9]) {
                body = &body[..body.len() - 2];
            }
            tile.data.extend_from_slice(body);
            if let Some(chunks) = &chunks {
                let h = tile.headers.get_or_insert_with(Vec::new);
                h.extend_from_slice(chunks.get(tp_index).copied().unwrap_or(&[]));
            }
        }
        tp_index += 1;
        if psot == 0 || end >= d.len() {
            break;
        }
        pos = end;
    }
    if tiles.is_empty() {
        return Err(err("no tile data"));
    }
    Ok(Codestream { siz, main, tiles })
}

impl Codestream {
    /// One plane per component on its own grid, and whether a tile applied the inverse component transform.
    fn decode_planes(&self) -> Result<(Vec<Vec<i32>>, bool), CodecError> {
        let mut planes = Vec::with_capacity(self.siz.comps.len());
        for c in 0..self.siz.comps.len() {
            let (x0, y0, x1, y1) = self.siz.comp_rect(c);
            planes.push(vec![0i32; (x1 - x0) as usize * (y1 - y0) as usize]);
        }
        let mut mct_used = false;
        let mut first_err = None;
        let mut decoded = 0;
        for (&t, tile) in &self.tiles {
            match self.decode_tile(t, tile, &mut planes) {
                Ok(m) => {
                    mct_used |= m;
                    decoded += 1;
                }
                Err(e) => {
                    first_err.get_or_insert(e);
                }
            }
        }
        if decoded == 0 {
            return Err(first_err.unwrap_or_else(|| err("no tile data")));
        }
        Ok((planes, mct_used))
    }

    fn decode_tile(&self, t: usize, tile: &Tile, planes: &mut [Vec<i32>]) -> Result<bool, CodecError> {
        let siz = &self.siz;
        let (p, q) = (t as u64 % u64::from(siz.ntx), t as u64 / u64::from(siz.ntx));
        let tx0 = (u64::from(siz.tx0) + p * u64::from(siz.tw)).max(u64::from(siz.x0)) as u32;
        let ty0 = (u64::from(siz.ty0) + q * u64::from(siz.th)).max(u64::from(siz.y0)) as u32;
        let tx1 = (u64::from(siz.tx0) + (p + 1) * u64::from(siz.tw)).min(u64::from(siz.x1)) as u32;
        let ty1 = (u64::from(siz.ty0) + (q + 1) * u64::from(siz.th)).min(u64::from(siz.y1)) as u32;
        if tx1 <= tx0 || ty1 <= ty0 {
            return Ok(false);
        }
        let (tp, mp) = (&tile.params, &self.main);
        let g = tp.cod.as_ref().or(mp.cod.as_ref()).map(|c| c.0).ok_or_else(|| err("no COD"))?;
        let mut tcs = Vec::with_capacity(siz.comps.len());
        let mut nprec = 0u64;
        for (c, cs) in siz.comps.iter().enumerate() {
            let coding = tp.coc[c]
                .as_ref()
                .or(tp.cod.as_ref().map(|x| &x.1))
                .or(mp.coc[c].as_ref())
                .or(mp.cod.as_ref().map(|x| &x.1))
                .ok_or_else(|| err("no COD"))?;
            let quant = tp.qcc[c]
                .as_ref()
                .or(tp.qcd.as_ref())
                .or(mp.qcc[c].as_ref())
                .or(mp.qcd.as_ref())
                .ok_or_else(|| err("no QCD"))?;
            let roi = tp.rgn[c].or(mp.rgn[c]).unwrap_or(0);
            let rect = (tx0.div_ceil(cs.dx), ty0.div_ceil(cs.dy), tx1.div_ceil(cs.dx), ty1.div_ceil(cs.dy));
            let tc = Tc::new(rect, coding, quant, roi, cs.prec);
            nprec += tc.res.iter().map(|r| u64::from(r.pw) * u64::from(r.ph)).sum::<u64>();
            tcs.push(tc);
        }
        if nprec > MAX_PRECINCTS {
            return Err(err("too many precincts"));
        }
        let pocs = if tp.poc.is_empty() { &mp.poc } else { &tp.poc };
        let mut td = TileDec {
            tcs,
            comps: &siz.comps,
            g,
            origin: (tx0, ty0),
            precincts: HashMap::new(),
            cblks: 0,
            visits: 0,
            st: Stream { body: &tile.data, bpos: 0, hdr: tile.headers.as_deref(), hpos: 0 },
        };
        td.packets(pocs)?;
        let mut coefs = td.tier1();
        for (tc, cf) in td.tcs.iter().zip(coefs.iter_mut()) {
            idwt(tc, cf);
        }
        let mct = g.mct == 1
            && td.tcs.len() >= 3
            && td.tcs[..3]
                .iter()
                .all(|tc| (tc.x0, tc.y0, tc.x1, tc.y1) == (td.tcs[0].x0, td.tcs[0].y0, td.tcs[0].x1, td.tcs[0].y1));
        let mct = mct && inverse_mct(&mut coefs);
        for (c, (tc, cf)) in td.tcs.iter().zip(&coefs).enumerate() {
            let (cx0, cy0, cx1, _) = siz.comp_rect(c);
            store(tc, cf, &siz.comps[c], &mut planes[c], (cx1 - cx0) as usize, (cx0, cy0));
        }
        Ok(mct)
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Tile geometry
// ---------------------------------------------------------------------------------------------------------------

fn ceil_shr(v: u32, s: u32) -> u32 {
    ((u64::from(v) + (1u64 << s) - 1) >> s) as u32
}

fn floor_shr(v: u32, s: u32) -> u32 {
    (u64::from(v) >> s) as u32
}

struct Band {
    /// 0 LL, 1 HL, 2 LH, 3 HH.
    orient: u8,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    /// Where the band sits in the tile-component's coefficient buffer.
    offx: usize,
    offy: usize,
    /// Mb: magnitude bit-planes before zero-bit-plane removal.
    mb: i32,
    step: f32,
    cbw: u32,
    cbh: u32,
}

struct Res {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    ppx: u32,
    ppy: u32,
    pw: u32,
    ph: u32,
    bands: Vec<Band>,
}

struct Tc {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    style: u8,
    reversible: bool,
    roi: u8,
    res: Vec<Res>,
}

impl Tc {
    fn new(rect: (u32, u32, u32, u32), coding: &Coding, quant: &Quant, roi: u8, prec: u32) -> Tc {
        let (x0, y0, x1, y1) = rect;
        let nl = coding.levels;
        let mut res: Vec<Res> = Vec::with_capacity(nl as usize + 1);
        for r in 0..=nl {
            let s = nl - r;
            let (rx0, ry0, rx1, ry1) = (ceil_shr(x0, s), ceil_shr(y0, s), ceil_shr(x1, s), ceil_shr(y1, s));
            let (ppx, ppy) = coding.prec[r as usize];
            let pw = if rx1 > rx0 { ceil_shr(rx1, ppx) - floor_shr(rx0, ppx) } else { 0 };
            let ph = if ry1 > ry0 { ceil_shr(ry1, ppy) - floor_shr(ry0, ppy) } else { 0 };
            let (cpx, cpy) = if r == 0 { (ppx, ppy) } else { (ppx.saturating_sub(1), ppy.saturating_sub(1)) };
            let (cbw, cbh) = (coding.cbw.min(cpx), coding.cbh.min(cpy));
            let mut bands = Vec::new();
            let orients: &[u8] = if r == 0 { &[0] } else { &[1, 2, 3] };
            for &orient in orients {
                let (xo, yo) = (u32::from(orient & 1), u32::from(orient >> 1));
                let (bx0, by0, bx1, by1, offx, offy) = if r == 0 {
                    (rx0, ry0, rx1, ry1, 0, 0)
                } else {
                    let n = s + 1;
                    let low = &res[r as usize - 1];
                    let band_edge = |v: u32, o: u32| {
                        let num = i64::from(v) - (i64::from(o) << (n - 1));
                        (-((-num) >> n)).max(0) as u32
                    };
                    (
                        band_edge(x0, xo),
                        band_edge(y0, yo),
                        band_edge(x1, xo),
                        band_edge(y1, yo),
                        if xo == 1 { (low.x1 - low.x0) as usize } else { 0 },
                        if yo == 1 { (low.y1 - low.y0) as usize } else { 0 },
                    )
                };
                let idx = if r == 0 { 0 } else { 3 * (r as usize - 1) + orient as usize };
                let (eps, mu) = if quant.style == 1 {
                    let (e0, m0) = quant.steps[0];
                    (e0.saturating_sub(if r == 0 { 0 } else { r - 1 }), m0)
                } else {
                    quant.steps.get(idx).or(quant.steps.last()).copied().unwrap_or((0, 0))
                };
                let gain = [0, 1, 1, 2][orient as usize];
                let step = (2f64.powi(prec as i32 + gain - eps as i32) * (1.0 + f64::from(mu) / 2048.0)) as f32;
                bands.push(Band {
                    orient,
                    x0: bx0,
                    y0: by0,
                    x1: bx1.max(bx0),
                    y1: by1.max(by0),
                    offx,
                    offy,
                    mb: quant.guard as i32 + eps as i32 - 1,
                    step,
                    cbw,
                    cbh,
                });
            }
            res.push(Res { x0: rx0, y0: ry0, x1: rx1, y1: ry1, ppx, ppy, pw, ph, bands });
        }
        Tc { x0, y0, x1, y1, style: coding.style, reversible: coding.reversible, roi, res }
    }

    fn levels(&self) -> u32 {
        self.res.len() as u32 - 1
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Tier 2: packets
// ---------------------------------------------------------------------------------------------------------------

/// Packet-header bit reader: a 0xFF byte is followed by a byte with a stuffed zero MSB.
struct Bio<'a> {
    d: &'a [u8],
    pos: usize,
    buf: u32,
    ct: u32,
    overrun: bool,
}

impl<'a> Bio<'a> {
    fn new(d: &'a [u8]) -> Bio<'a> {
        Bio { d, pos: 0, buf: 0, ct: 0, overrun: false }
    }

    fn bytein(&mut self, strict: bool) {
        self.buf = (self.buf << 8) & 0xffff;
        self.ct = if self.buf == 0xff00 { 7 } else { 8 };
        if let Some(&b) = self.d.get(self.pos) {
            self.buf |= u32::from(b);
            self.pos += 1;
        } else if strict {
            self.overrun = true;
        }
    }

    fn bit(&mut self) -> u32 {
        if self.ct == 0 {
            self.bytein(true);
        }
        self.ct -= 1;
        (self.buf >> self.ct) & 1
    }

    fn bits(&mut self, n: u32) -> u64 {
        let mut v = 0u64;
        for _ in 0..n {
            v = (v << 1) | u64::from(self.bit());
        }
        v
    }

    fn align(&mut self) {
        if self.buf & 0xff == 0xff {
            self.bytein(false);
        }
        self.ct = 0;
    }
}

struct TagTree {
    /// (parent, value, low) per node; leaves first.
    nodes: Vec<(u32, i32, i32)>,
}

impl TagTree {
    fn new(w: u32, h: u32) -> TagTree {
        let mut dims = vec![(w, h)];
        while let Some(&(lw, lh)) = dims.last() {
            if lw <= 1 && lh <= 1 {
                break;
            }
            dims.push((lw.div_ceil(2), lh.div_ceil(2)));
        }
        let mut nodes = Vec::new();
        let mut off = 0u32;
        for (l, &(lw, lh)) in dims.iter().enumerate() {
            let next = off + lw * lh;
            for y in 0..lh {
                for x in 0..lw {
                    let parent = match dims.get(l + 1) {
                        Some(&(pw, _)) => next + (y / 2) * pw + x / 2,
                        None => u32::MAX,
                    };
                    nodes.push((parent, i32::MAX, 0));
                }
            }
            off = next;
        }
        TagTree { nodes }
    }

    /// Whether the leaf's value is below `threshold`, reading the bits that decides it.
    fn decode(&mut self, bio: &mut Bio, leaf: usize, threshold: i32) -> bool {
        let mut stack = [0u32; 48];
        let mut sp = 0;
        let mut node = leaf as u32;
        while self.nodes[node as usize].0 != u32::MAX && sp < stack.len() {
            stack[sp] = node;
            sp += 1;
            node = self.nodes[node as usize].0;
        }
        let mut low = 0;
        loop {
            let n = &mut self.nodes[node as usize];
            if low > n.2 {
                n.2 = low;
            } else {
                low = n.2;
            }
            while low < threshold && low < n.1 {
                if bio.bit() == 1 {
                    n.1 = low;
                } else {
                    low += 1;
                }
            }
            n.2 = low;
            if sp == 0 {
                break;
            }
            sp -= 1;
            node = stack[sp];
        }
        self.nodes[leaf].1 < threshold
    }
}

/// A codeword segment of a code-block: bytes and passes so far, and the passes it may hold.
#[derive(Clone, Copy)]
struct Seg {
    len: u32,
    passes: u32,
    max: u32,
}

struct Cblk {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    included: bool,
    numbps: i32,
    lblock: u32,
    data: Vec<u8>,
    segs: Vec<Seg>,
}

struct PrecBand {
    incl: TagTree,
    imsb: TagTree,
    cblks: Vec<Cblk>,
}

struct Precinct {
    next_layer: u32,
    bands: Vec<PrecBand>,
}

struct Stream<'a> {
    body: &'a [u8],
    bpos: usize,
    hdr: Option<&'a [u8]>,
    hpos: usize,
}

struct TileDec<'a> {
    tcs: Vec<Tc>,
    comps: &'a [CompSiz],
    g: Global,
    origin: (u32, u32),
    precincts: HashMap<u64, Precinct>,
    cblks: usize,
    visits: u64,
    st: Stream<'a>,
}

fn seg_max(style: u8, prev: Option<&Seg>) -> u32 {
    if style & CBLK_TERMALL != 0 {
        1
    } else if style & CBLK_BYPASS != 0 {
        match prev {
            None => 10,
            Some(s) if s.max == 1 || s.max == 10 => 2,
            Some(_) => 1,
        }
    } else {
        109
    }
}

fn read_pass_count(bio: &mut Bio) -> u32 {
    if bio.bit() == 0 {
        return 1;
    }
    if bio.bit() == 0 {
        return 2;
    }
    let n = bio.bits(2) as u32;
    if n < 3 {
        return 3 + n;
    }
    let n = bio.bits(5) as u32;
    if n < 31 {
        return 6 + n;
    }
    37 + bio.bits(7) as u32
}

impl TileDec<'_> {
    fn build_precinct(&mut self, c: usize, r: usize, p: u32) -> Result<Precinct, CodecError> {
        let res = &self.tcs[c].res[r];
        let (i, j) = (p % res.pw, p / res.pw);
        let mut bands = Vec::with_capacity(res.bands.len());
        for b in &res.bands {
            let (sx, sy) =
                if r == 0 { (res.ppx, res.ppy) } else { (res.ppx.saturating_sub(1), res.ppy.saturating_sub(1)) };
            // Precinct origin: the resolution's precinct grid, expressed on the band.
            let gx = u64::from(floor_shr(res.x0, res.ppx) + i) << sx;
            let gy = u64::from(floor_shr(res.y0, res.ppy) + j) << sy;
            let px0 = gx.max(u64::from(b.x0)).min(u64::from(b.x1)) as u32;
            let py0 = gy.max(u64::from(b.y0)).min(u64::from(b.y1)) as u32;
            let px1 = (gx + (1u64 << sx)).min(u64::from(b.x1)).max(u64::from(px0)) as u32;
            let py1 = (gy + (1u64 << sy)).min(u64::from(b.y1)).max(u64::from(py0)) as u32;
            let (cw, ch) = if px1 > px0 && py1 > py0 {
                (ceil_shr(px1, b.cbw) - floor_shr(px0, b.cbw), ceil_shr(py1, b.cbh) - floor_shr(py0, b.cbh))
            } else {
                (0, 0)
            };
            let n = cw as usize * ch as usize;
            self.cblks += n;
            if self.cblks > MAX_CODEBLOCKS {
                return Err(err("too many code-blocks"));
            }
            let mut cblks = Vec::with_capacity(n);
            for y in 0..ch {
                for x in 0..cw {
                    let cx = (floor_shr(px0, b.cbw) + x) << b.cbw;
                    let cy = (floor_shr(py0, b.cbh) + y) << b.cbh;
                    cblks.push(Cblk {
                        x0: cx.max(px0),
                        y0: cy.max(py0),
                        x1: (cx + (1 << b.cbw)).min(px1),
                        y1: (cy + (1 << b.cbh)).min(py1),
                        included: false,
                        numbps: 0,
                        lblock: 3,
                        data: Vec::new(),
                        segs: Vec::new(),
                    });
                }
            }
            bands.push(PrecBand { incl: TagTree::new(cw, ch), imsb: TagTree::new(cw, ch), cblks });
        }
        Ok(Precinct { next_layer: 0, bands })
    }

    /// Runs the progression; stops quietly at the end of the data.
    fn packets(&mut self, pocs: &[Poc]) -> Result<(), CodecError> {
        let nc = self.tcs.len() as u32;
        let maxres = self.tcs.iter().map(|t| t.res.len() as u32).max().unwrap_or(0);
        let layers = self.g.layers;
        let default = [Poc { rs: 0, cs: 0, lye: layers, re: maxres, ce: nc, order: self.g.order }];
        let entries = if pocs.is_empty() { &default[..] } else { pocs };
        for e in entries {
            let (lye, re, ce) = (e.lye.min(layers), e.re.min(maxres), e.ce.min(nc));
            let (rs, cs) = (e.rs, e.cs);
            let go = match e.order {
                0 => 'o: {
                    for l in 0..lye {
                        for r in rs..re {
                            for c in cs..ce {
                                if !self.precincts_of(l, r, c)? {
                                    break 'o false;
                                }
                            }
                        }
                    }
                    true
                }
                1 => 'o: {
                    for r in rs..re {
                        for l in 0..lye {
                            for c in cs..ce {
                                if !self.precincts_of(l, r, c)? {
                                    break 'o false;
                                }
                            }
                        }
                    }
                    true
                }
                order => self.position_order(order, (rs, re), (cs, ce), lye)?,
            };
            if !go {
                break;
            }
        }
        Ok(())
    }

    fn precincts_of(&mut self, l: u32, r: u32, c: u32) -> Result<bool, CodecError> {
        self.visits += 1;
        let tc = &self.tcs[c as usize];
        let Some(res) = tc.res.get(r as usize) else { return Ok(self.visits < MAX_VISITS) };
        let np = res.pw * res.ph;
        for p in 0..np {
            if !self.visit(l, r, c, p)? {
                return Ok(false);
            }
        }
        Ok(self.visits < MAX_VISITS)
    }

    fn position_order(&mut self, order: u8, rr: (u32, u32), cr: (u32, u32), lye: u32) -> Result<bool, CodecError> {
        let mut items: Vec<([u64; 4], u32, u32, u32)> = Vec::new();
        for c in cr.0..cr.1 {
            let tc = &self.tcs[c as usize];
            let cs = self.comps[c as usize];
            let nl = tc.levels();
            for r in rr.0..rr.1.min(nl + 1) {
                let res = &tc.res[r as usize];
                let n = nl - r;
                for p in 0..res.pw * res.ph {
                    let (i, j) = (p % res.pw, p / res.pw);
                    let px = u128::from(floor_shr(res.x0, res.ppx) + i) << res.ppx;
                    let py = u128::from(floor_shr(res.y0, res.ppy) + j) << res.ppy;
                    let x =
                        ((px << n) * u128::from(cs.dx)).max(u128::from(self.origin.0)).min(u128::from(u64::MAX)) as u64;
                    let y =
                        ((py << n) * u128::from(cs.dy)).max(u128::from(self.origin.1)).min(u128::from(u64::MAX)) as u64;
                    let (c64, r64) = (u64::from(c), u64::from(r));
                    let key = match order {
                        2 => [r64, y, x, c64],
                        3 => [y, x, c64, r64],
                        _ => [c64, y, x, r64],
                    };
                    items.push((key, c, r, p));
                }
            }
        }
        items.sort_unstable_by_key(|it| it.0);
        for (_, c, r, p) in items {
            for l in 0..lye {
                if !self.visit(l, r, c, p)? {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    /// Decodes the packet (l, r, c, p) unless it was decoded already; false once the data has run out.
    fn visit(&mut self, l: u32, r: u32, c: u32, p: u32) -> Result<bool, CodecError> {
        self.visits += 1;
        if self.visits > MAX_VISITS {
            return Ok(false);
        }
        let key = (u64::from(c) << 40) | (u64::from(r) << 32) | u64::from(p);
        if !self.precincts.contains_key(&key) {
            let pr = self.build_precinct(c as usize, r as usize, p)?;
            self.precincts.insert(key, pr);
        }
        let prec = self.precincts.get_mut(&key).expect("inserted above");
        if l < prec.next_layer {
            return Ok(true);
        }
        prec.next_layer = l + 1;
        let tc = &self.tcs[c as usize];
        Ok(read_packet(tc, r as usize, l, prec, &mut self.st, self.g))
    }
}

/// Reads one packet header and its body into the precinct's code-blocks; false when the data ran out.
fn read_packet(tc: &Tc, r: usize, layer: u32, prec: &mut Precinct, st: &mut Stream, g: Global) -> bool {
    if g.sop && st.body.get(st.bpos..st.bpos + 2) == Some(&[0xff, 0x91]) {
        st.bpos = (st.bpos + 6).min(st.body.len());
    }
    let (hbuf, hpos) = match st.hdr {
        Some(h) => (h, st.hpos),
        None => (st.body, st.bpos),
    };
    if hpos >= hbuf.len() {
        return false;
    }
    let mut bio = Bio::new(&hbuf[hpos..]);
    // (band, code-block, segment, passes, bytes)
    let mut pending: Vec<(usize, usize, usize, u32, u64)> = Vec::new();
    if bio.bit() == 1 {
        for (bi, band) in prec.bands.iter_mut().enumerate() {
            let mb = tc.res[r].bands[bi].mb;
            for k in 0..band.cblks.len() {
                let cb = &mut band.cblks[k];
                let included =
                    if cb.included { bio.bit() == 1 } else { band.incl.decode(&mut bio, k, layer as i32 + 1) };
                if bio.overrun {
                    return false;
                }
                if !included {
                    continue;
                }
                if !cb.included {
                    let mut n = 1;
                    while !band.imsb.decode(&mut bio, k, n) {
                        n += 1;
                        if n > 74 || bio.overrun {
                            return false;
                        }
                    }
                    cb.numbps = mb - (n - 1);
                    cb.included = true;
                }
                let mut left = read_pass_count(&mut bio);
                let mut inc = 0;
                while bio.bit() == 1 {
                    inc += 1;
                    if inc > 32 || bio.overrun {
                        return false;
                    }
                }
                cb.lblock = (cb.lblock + inc).min(32);
                if cb.segs.last().is_none_or(|s| s.passes >= s.max) {
                    let max = seg_max(tc.style, cb.segs.last());
                    cb.segs.push(Seg { len: 0, passes: 0, max });
                }
                let mut si = cb.segs.len() - 1;
                loop {
                    let s = cb.segs[si];
                    let take = (s.max - s.passes).min(left).max(1);
                    let len = bio.bits(cb.lblock + (31 - take.leading_zeros()));
                    pending.push((bi, k, si, take, len));
                    left = left.saturating_sub(take);
                    if left == 0 {
                        break;
                    }
                    let max = seg_max(tc.style, cb.segs.last());
                    cb.segs.push(Seg { len: 0, passes: 0, max });
                    si += 1;
                }
                if bio.overrun {
                    return false;
                }
            }
        }
    }
    bio.align();
    if bio.overrun {
        return false;
    }
    let mut hpos = hpos + bio.pos;
    if g.eph && hbuf.get(hpos..hpos + 2) == Some(&[0xff, 0x92]) {
        hpos += 2;
    }
    match st.hdr {
        Some(_) => st.hpos = hpos,
        None => st.bpos = hpos,
    }
    let mut complete = true;
    for (bi, k, si, take, len) in pending {
        let avail = st.body.len().saturating_sub(st.bpos);
        let n = (len.min(avail as u64)) as usize;
        let cb = &mut prec.bands[bi].cblks[k];
        cb.data.extend_from_slice(&st.body[st.bpos..st.bpos + n]);
        let seg = &mut cb.segs[si];
        seg.len += n as u32;
        if n > 0 || len == 0 {
            seg.passes += take;
        }
        st.bpos += n;
        if (n as u64) < len {
            complete = false;
        }
    }
    complete
}

// ---------------------------------------------------------------------------------------------------------------
// Tier 1: EBCOT
// ---------------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct MqState {
    qe: u32,
    nmps: u8,
    nlps: u8,
}

#[rustfmt::skip]
const MQ_QE: [u16; 47] = [
    0x5601, 0x3401, 0x1801, 0x0ac1, 0x0521, 0x0221, 0x5601, 0x5401, 0x4801, 0x3801, 0x3001, 0x2401, 0x1c01, 0x1601,
    0x5601, 0x5401, 0x5101, 0x4801, 0x3801, 0x3401, 0x3001, 0x2801, 0x2401, 0x2201, 0x1c01, 0x1801, 0x1601, 0x1401,
    0x1201, 0x1101, 0x0ac1, 0x09c1, 0x08a1, 0x0521, 0x0441, 0x02a1, 0x0221, 0x0141, 0x0111, 0x0085, 0x0049, 0x0025,
    0x0015, 0x0009, 0x0005, 0x0001, 0x5601,
];
#[rustfmt::skip]
const MQ_NMPS: [u8; 47] = [
    1, 2, 3, 4, 5, 38, 7, 8, 9, 10, 11, 12, 13, 29, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30,
    31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 45, 46,
];
#[rustfmt::skip]
const MQ_NLPS: [u8; 47] = [
    1, 6, 9, 12, 29, 33, 6, 14, 14, 14, 17, 18, 20, 21, 14, 14, 15, 16, 17, 18, 19, 19, 20, 21, 22, 23, 24, 25, 26, 27,
    28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 46,
];

/// Context states as `state * 2 + mps`, with both transitions resolved.
const MQ: [MqState; 94] = {
    let mut t = [MqState { qe: 0, nmps: 0, nlps: 0 }; 94];
    let mut i = 0;
    while i < 94 {
        let s = i / 2;
        let mps = (i & 1) as u8;
        let switch = (s == 0 || s == 6 || s == 14) as u8;
        t[i] = MqState { qe: MQ_QE[s] as u32, nmps: MQ_NMPS[s] * 2 + mps, nlps: MQ_NLPS[s] * 2 + (mps ^ switch) };
        i += 1;
    }
    t
};

trait Bits {
    const RAW: bool;
    fn bit(&mut self, cx: &mut u8) -> u32;
}

struct Mq<'a> {
    d: &'a [u8],
    pos: usize,
    a: u32,
    c: u32,
    ct: u32,
}

impl<'a> Mq<'a> {
    fn new(d: &'a [u8]) -> Mq<'a> {
        let mut m = Mq { d, pos: 0, a: 0x8000, c: u32::from(*d.first().unwrap_or(&0xff)) << 16, ct: 0 };
        m.bytein();
        m.c <<= 7;
        m.ct -= 7;
        m
    }

    #[inline(always)]
    fn byte(&self, i: usize) -> u32 {
        u32::from(*self.d.get(i).unwrap_or(&0xff))
    }

    #[inline(always)]
    fn bytein(&mut self) {
        if self.byte(self.pos) == 0xff {
            let b1 = self.byte(self.pos + 1);
            if b1 > 0x8f {
                self.c = self.c.wrapping_add(0xff00);
                self.ct = 8;
            } else {
                self.pos += 1;
                self.c = self.c.wrapping_add(b1 << 9);
                self.ct = 7;
            }
        } else {
            self.pos += 1;
            self.c = self.c.wrapping_add(self.byte(self.pos) << 8);
            self.ct = 8;
        }
    }

    #[inline(always)]
    fn renorm(&mut self) {
        loop {
            if self.ct == 0 {
                self.bytein();
            }
            self.a <<= 1;
            self.c <<= 1;
            self.ct -= 1;
            if self.a & 0x8000 != 0 {
                break;
            }
        }
    }
}

impl Bits for Mq<'_> {
    const RAW: bool = false;

    #[inline(always)]
    fn bit(&mut self, cx: &mut u8) -> u32 {
        let st = MQ[*cx as usize];
        let mps = u32::from(*cx & 1);
        self.a -= st.qe;
        if (self.c >> 16) < st.qe {
            let d = if self.a < st.qe {
                *cx = st.nmps;
                mps
            } else {
                *cx = st.nlps;
                mps ^ 1
            };
            self.a = st.qe;
            self.renorm();
            d
        } else {
            self.c -= st.qe << 16;
            if self.a & 0x8000 == 0 {
                let d = if self.a < st.qe {
                    *cx = st.nlps;
                    mps ^ 1
                } else {
                    *cx = st.nmps;
                    mps
                };
                self.renorm();
                d
            } else {
                mps
            }
        }
    }
}

/// Raw (bypass) bits, with the same 0xFF stuffing.
struct Raw<'a> {
    d: &'a [u8],
    pos: usize,
    c: u32,
    ct: u32,
}

impl Bits for Raw<'_> {
    const RAW: bool = true;

    #[inline(always)]
    fn bit(&mut self, _: &mut u8) -> u32 {
        if self.ct == 0 {
            let b = u32::from(*self.d.get(self.pos).unwrap_or(&0xff));
            if self.c == 0xff {
                if b > 0x8f {
                    self.c = 0xff;
                    self.ct = 8;
                } else {
                    self.c = b;
                    self.pos += 1;
                    self.ct = 7;
                }
            } else {
                self.c = b;
                self.pos += 1;
                self.ct = 8;
            }
        }
        self.ct -= 1;
        (self.c >> self.ct) & 1
    }
}

// Per-coefficient flags: neighbour significance (8 bits), neighbour signs (4), own state (4).
const F_SIG_N: u16 = 1;
const F_SIG_S: u16 = 2;
const F_SIG_W: u16 = 4;
const F_SIG_E: u16 = 8;
const F_SIG_NW: u16 = 16;
const F_SIG_NE: u16 = 32;
const F_SIG_SW: u16 = 64;
const F_SIG_SE: u16 = 128;
const F_NEG_S: u16 = 512;
const F_SIG: u16 = 1 << 12;
const F_VISIT: u16 = 1 << 13;
const F_REFINE: u16 = 1 << 14;
const F_NEG: u16 = 1 << 15;
/// Vertically causal mode: the next stripe's coefficients count as insignificant.
const F_VSC_MASK: u16 = !(F_SIG_S | F_SIG_SW | F_SIG_SE | F_NEG_S);

const CX_RL: usize = 17;
const CX_UNI: usize = 18;

/// Zero-coding contexts from the 8 neighbour bits: LL/LH, HL (horizontal and vertical swapped), HH.
const ZC_LUT: [[u8; 256]; 3] = {
    let mut t = [[0u8; 256]; 3];
    let mut class = 0;
    while class < 3 {
        let mut f = 0;
        while f < 256 {
            let mut h = ((f >> 2) & 1) + ((f >> 3) & 1);
            let mut v = (f & 1) + ((f >> 1) & 1);
            let d = ((f >> 4) & 1) + ((f >> 5) & 1) + ((f >> 6) & 1) + ((f >> 7) & 1);
            if class == 1 {
                (h, v) = (v, h);
            }
            t[class][f] = if class < 2 {
                match (h, v, d) {
                    (2, _, _) => 8,
                    (1, 1.., _) => 7,
                    (1, 0, 1..) => 6,
                    (1, 0, 0) => 5,
                    (0, 2, _) => 4,
                    (0, 1, _) => 3,
                    (0, 0, 2..) => 2,
                    (0, 0, 1) => 1,
                    _ => 0,
                }
            } else {
                match (h + v, d) {
                    (_, 3..) => 8,
                    (1.., 2) => 7,
                    (0, 2) => 6,
                    (2.., 1) => 5,
                    (1, 1) => 4,
                    (0, 1) => 3,
                    (2.., 0) => 2,
                    (1, 0) => 1,
                    _ => 0,
                }
            };
            f += 1;
        }
        class += 1;
    }
    t
};

const fn sc_contrib(i: usize, sig: usize, neg: usize) -> i32 {
    if (i >> sig) & 1 == 0 {
        0
    } else if (i >> neg) & 1 == 1 {
        -1
    } else {
        1
    }
}

const fn sc_clamp(v: i32) -> i32 {
    if v > 1 {
        1
    } else if v < -1 {
        -1
    } else {
        v
    }
}

/// Sign contexts (bits 0-6) and the XOR bit (bit 7) from N,S,W,E significance (bits 0-3) and signs (bits 4-7).
const SC_LUT: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        let v = sc_clamp(sc_contrib(i, 0, 4) + sc_contrib(i, 1, 5));
        let h = sc_clamp(sc_contrib(i, 2, 6) + sc_contrib(i, 3, 7));
        let (cx, xor) = match (h, v) {
            (1, 1) => (13, 0),
            (1, 0) => (12, 0),
            (1, _) => (11, 0),
            (0, 1) => (10, 0),
            (0, 0) => (9, 0),
            (0, _) => (10, 1),
            (_, 1) => (11, 1),
            (_, 0) => (12, 1),
            _ => (13, 1),
        };
        t[i] = cx | (xor << 7);
        i += 1;
    }
    t
};

struct T1 {
    flags: Vec<u16>,
    /// Magnitudes with one extra fractional bit: bit-plane p lands at bit p+1, bit p holds the midpoint.
    mag: Vec<u32>,
    ctx: [u8; 19],
    w: usize,
    h: usize,
    fw: usize,
    vsc: bool,
}

struct BlockJob<'a> {
    w: usize,
    h: usize,
    orient: u8,
    planes: u32,
    style: u8,
    data: &'a [u8],
    segs: &'a [Seg],
}

impl T1 {
    fn new() -> T1 {
        T1 { flags: Vec::new(), mag: Vec::new(), ctx: [0; 19], w: 0, h: 0, fw: 0, vsc: false }
    }

    fn reset_ctx(&mut self) {
        self.ctx = [0; 19];
        self.ctx[0] = 4 * 2;
        self.ctx[CX_RL] = 3 * 2;
        self.ctx[CX_UNI] = 46 * 2;
    }

    fn decode(&mut self, job: &BlockJob) {
        let (w, h) = (job.w, job.h);
        self.w = w;
        self.h = h;
        self.fw = w + 2;
        self.vsc = job.style & CBLK_VSC != 0;
        self.flags.clear();
        self.flags.resize((w + 2) * (h + 2), 0);
        self.mag.clear();
        self.mag.resize(w * h, 0);
        self.reset_ctx();
        let zc = &ZC_LUT[match job.orient {
            1 => 1,
            3 => 2,
            _ => 0,
        }];
        let bypass = job.style & CBLK_BYPASS != 0;
        let mut pass = 0u32;
        let mut pos = 0usize;
        'segs: for seg in job.segs {
            let end = (pos + seg.len as usize).min(job.data.len());
            let bytes = &job.data[pos.min(end)..end];
            pos = end;
            let mut mq: Option<Mq> = None;
            let mut raw: Option<Raw> = None;
            for _ in 0..seg.passes {
                let kind = (pass + 2) % 3;
                let Some(p) = job.planes.checked_sub(1 + pass.div_ceil(3)) else { break 'segs };
                if bypass && pass >= 10 && kind != 2 {
                    let dec = raw.get_or_insert(Raw { d: bytes, pos: 0, c: 0, ct: 0 });
                    if kind == 0 {
                        self.sig_pass(dec, p, zc);
                    } else {
                        self.ref_pass(dec, p);
                    }
                } else {
                    let dec = mq.get_or_insert_with(|| Mq::new(bytes));
                    match kind {
                        0 => self.sig_pass(dec, p, zc),
                        1 => self.ref_pass(dec, p),
                        _ => {
                            self.cleanup_pass(dec, p, zc);
                            if job.style & CBLK_SEGSYM != 0 {
                                for _ in 0..4 {
                                    dec.bit(&mut self.ctx[CX_UNI]);
                                }
                            }
                        }
                    }
                }
                if job.style & CBLK_RESET != 0 {
                    self.reset_ctx();
                }
                pass += 1;
            }
        }
    }

    #[inline(always)]
    fn masked(&self, f: u16, row_in_stripe: usize) -> u16 {
        if self.vsc && row_in_stripe == 3 { f & F_VSC_MASK } else { f }
    }

    #[inline(always)]
    fn set_sig(&mut self, fi: usize, neg: bool) {
        let fw = self.fw;
        let n = u16::from(neg);
        let f = &mut self.flags;
        f[fi] |= F_SIG | (n << 15);
        f[fi - fw] |= F_SIG_S | (n << 9);
        f[fi + fw] |= F_SIG_N | (n << 8);
        f[fi - 1] |= F_SIG_E | (n << 11);
        f[fi + 1] |= F_SIG_W | (n << 10);
        f[fi - fw - 1] |= F_SIG_SE;
        f[fi - fw + 1] |= F_SIG_SW;
        f[fi + fw - 1] |= F_SIG_NE;
        f[fi + fw + 1] |= F_SIG_NW;
    }

    #[inline(always)]
    fn sign<B: Bits>(&mut self, dec: &mut B, fm: u16) -> bool {
        let s = SC_LUT[((fm & 0xf) | ((fm >> 4) & 0xf0)) as usize];
        let bit = dec.bit(&mut self.ctx[(s & 0x7f) as usize]);
        if B::RAW { bit != 0 } else { bit ^ u32::from(s >> 7) != 0 }
    }

    fn sig_pass<B: Bits>(&mut self, dec: &mut B, p: u32, zc: &[u8; 256]) {
        let one = 3u32 << p;
        let (w, h, fw) = (self.w, self.h, self.fw);
        for y0 in (0..h).step_by(4) {
            let ye = (y0 + 4).min(h);
            for x in 0..w {
                for y in y0..ye {
                    let fi = (y + 1) * fw + x + 1;
                    let f = self.flags[fi];
                    if f & F_SIG != 0 || f & 0xff == 0 {
                        continue;
                    }
                    let fm = self.masked(f, y - y0);
                    if fm & 0xff == 0 {
                        continue;
                    }
                    if dec.bit(&mut self.ctx[zc[(fm & 0xff) as usize] as usize]) != 0 {
                        let neg = self.sign(dec, fm);
                        self.mag[y * w + x] = one;
                        self.set_sig(fi, neg);
                    }
                    self.flags[fi] |= F_VISIT;
                }
            }
        }
    }

    fn ref_pass<B: Bits>(&mut self, dec: &mut B, p: u32) {
        let half = 1u32 << p;
        let (w, h, fw) = (self.w, self.h, self.fw);
        for y0 in (0..h).step_by(4) {
            let ye = (y0 + 4).min(h);
            for x in 0..w {
                for y in y0..ye {
                    let fi = (y + 1) * fw + x + 1;
                    let f = self.flags[fi];
                    if f & (F_SIG | F_VISIT) != F_SIG {
                        continue;
                    }
                    let cx = if f & F_REFINE != 0 {
                        16
                    } else if self.masked(f, y - y0) & 0xff != 0 {
                        15
                    } else {
                        14
                    };
                    let m = &mut self.mag[y * w + x];
                    *m = if dec.bit(&mut self.ctx[cx]) != 0 { m.wrapping_add(half) } else { m.wrapping_sub(half) };
                    self.flags[fi] = f | F_REFINE;
                }
            }
        }
    }

    fn cleanup_pass(&mut self, dec: &mut Mq, p: u32, zc: &[u8; 256]) {
        let one = 3u32 << p;
        let (w, h, fw) = (self.w, self.h, self.fw);
        const BUSY: u16 = F_SIG | F_VISIT | 0xff;
        for y0 in (0..h).step_by(4) {
            let ye = (y0 + 4).min(h);
            for x in 0..w {
                let fi0 = (y0 + 1) * fw + x + 1;
                let mut ystart = y0;
                if ye - y0 == 4 {
                    let f = &self.flags;
                    let agg = (f[fi0] | f[fi0 + fw] | f[fi0 + 2 * fw]) & BUSY | self.masked(f[fi0 + 3 * fw], 3) & BUSY;
                    if agg == 0 {
                        if dec.bit(&mut self.ctx[CX_RL]) == 0 {
                            continue;
                        }
                        let r = ((dec.bit(&mut self.ctx[CX_UNI]) << 1) | dec.bit(&mut self.ctx[CX_UNI])) as usize;
                        let fi = fi0 + r * fw;
                        let fm = self.masked(self.flags[fi], r);
                        let neg = self.sign(dec, fm);
                        self.mag[(y0 + r) * w + x] = one;
                        self.set_sig(fi, neg);
                        ystart = y0 + r + 1;
                    }
                }
                for y in ystart..ye {
                    let fi = (y + 1) * fw + x + 1;
                    let f = self.flags[fi];
                    if f & F_VISIT != 0 {
                        self.flags[fi] = f & !F_VISIT;
                        continue;
                    }
                    if f & F_SIG != 0 {
                        continue;
                    }
                    let fm = self.masked(f, y - y0);
                    if dec.bit(&mut self.ctx[zc[(fm & 0xff) as usize] as usize]) != 0 {
                        let neg = self.sign(dec, fm);
                        self.mag[y * w + x] = one;
                        self.set_sig(fi, neg);
                    }
                }
            }
        }
    }
}

/// Coefficients of one tile-component: integers for 5-3, floats for 9-7.
enum Coefs {
    Int(Vec<i32>),
    Float(Vec<f32>),
}

impl TileDec<'_> {
    fn tier1(&self) -> Vec<Coefs> {
        let mut out: Vec<Coefs> = self
            .tcs
            .iter()
            .map(|tc| {
                let n = (tc.x1 - tc.x0) as usize * (tc.y1 - tc.y0) as usize;
                if tc.reversible { Coefs::Int(vec![0; n]) } else { Coefs::Float(vec![0.0; n]) }
            })
            .collect();
        let mut t1 = T1::new();
        for (&key, prec) in &self.precincts {
            let (c, r) = ((key >> 40) as usize, ((key >> 32) & 0xff) as usize);
            let tc = &self.tcs[c];
            let stride = (tc.x1 - tc.x0) as usize;
            for (pb, band) in prec.bands.iter().zip(&tc.res[r].bands) {
                for cb in &pb.cblks {
                    if cb.segs.iter().all(|s| s.passes == 0) {
                        continue;
                    }
                    let planes = cb.numbps + i32::from(tc.roi);
                    if planes <= 0 || planes > 31 {
                        continue;
                    }
                    let (w, h) = ((cb.x1 - cb.x0) as usize, (cb.y1 - cb.y0) as usize);
                    let job = BlockJob {
                        w,
                        h,
                        orient: band.orient,
                        planes: planes as u32,
                        style: tc.style,
                        data: &cb.data,
                        segs: &cb.segs,
                    };
                    t1.decode(&job);
                    let ox = band.offx + (cb.x0 - band.x0) as usize;
                    let oy = band.offy + (cb.y0 - band.y0) as usize;
                    let roi = u32::from(tc.roi);
                    let half_step = band.step * 0.5;
                    for y in 0..h {
                        let row = (oy + y) * stride + ox;
                        for x in 0..w {
                            let mut m = t1.mag[y * w + x];
                            if m == 0 {
                                continue;
                            }
                            if roi > 0 && m >= 1 << (roi + 1).min(31) {
                                m >>= roi;
                            }
                            let neg = t1.flags[(y + 1) * t1.fw + x + 1] & F_NEG != 0;
                            match &mut out[c] {
                                Coefs::Int(b) => {
                                    let v = (m >> 1) as i32;
                                    b[row + x] = if neg { -v } else { v };
                                }
                                Coefs::Float(b) => {
                                    let v = m as f32 * half_step;
                                    b[row + x] = if neg { -v } else { v };
                                }
                            }
                        }
                    }
                }
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Inverse wavelet transforms
// ---------------------------------------------------------------------------------------------------------------

const DWT_ALPHA: f32 = -1.586_134_3;
const DWT_BETA: f32 = -0.052_980_118;
const DWT_GAMMA: f32 = 0.882_911_1;
const DWT_DELTA: f32 = 0.443_506_87;
const DWT_K: f32 = 1.230_174_1;

trait Lift: Copy + Default {
    /// One line in place; low-pass samples at the indices with `i & 1 == cas`.
    fn line(s: &mut [Self], cas: usize);
    /// `rh` rows of `rw` samples in place, filtering vertically.
    fn rows(t: &mut [Self], rw: usize, rh: usize, cas: usize);
}

/// Row `i` mutably and its mirrored neighbours above and below.
fn rows3<T>(t: &mut [T], rw: usize, rh: usize, i: usize) -> (&mut [T], &[T], &[T]) {
    let pi = if i > 0 { i - 1 } else { 1 };
    let ni = if i + 1 < rh { i + 1 } else { i - 1 };
    let (left, right) = t.split_at_mut(i * rw);
    let (cur, rest) = right.split_at_mut(rw);
    let get = |k: usize| -> &[T] {
        if k < i {
            &left[k * rw..(k + 1) * rw]
        } else {
            let o = (k - i - 1) * rw;
            &rest[o..o + rw]
        }
    };
    let (a, b) = (get(pi), get(ni));
    (cur, a, b)
}

#[inline(always)]
fn nb<T: Copy>(s: &[T], i: usize) -> (T, T) {
    let n = s.len();
    (if i > 0 { s[i - 1] } else { s[1] }, if i + 1 < n { s[i + 1] } else { s[i - 1] })
}

impl Lift for i32 {
    fn line(s: &mut [i32], cas: usize) {
        if s.len() < 2 {
            if cas == 1 {
                for v in s.iter_mut() {
                    *v /= 2;
                }
            }
            return;
        }
        for i in (cas..s.len()).step_by(2) {
            let (a, b) = nb(s, i);
            s[i] = s[i].wrapping_sub(a.wrapping_add(b).wrapping_add(2) >> 2);
        }
        for i in (1 - cas..s.len()).step_by(2) {
            let (a, b) = nb(s, i);
            s[i] = s[i].wrapping_add(a.wrapping_add(b) >> 1);
        }
    }

    fn rows(t: &mut [i32], rw: usize, rh: usize, cas: usize) {
        if rh < 2 {
            if cas == 1 {
                for v in t[..rw].iter_mut() {
                    *v /= 2;
                }
            }
            return;
        }
        for i in (cas..rh).step_by(2) {
            let (cur, a, b) = rows3(t, rw, rh, i);
            for ((c, &a), &b) in cur.iter_mut().zip(a).zip(b) {
                *c = c.wrapping_sub(a.wrapping_add(b).wrapping_add(2) >> 2);
            }
        }
        for i in (1 - cas..rh).step_by(2) {
            let (cur, a, b) = rows3(t, rw, rh, i);
            for ((c, &a), &b) in cur.iter_mut().zip(a).zip(b) {
                *c = c.wrapping_add(a.wrapping_add(b) >> 1);
            }
        }
    }
}

impl Lift for f32 {
    fn line(s: &mut [f32], cas: usize) {
        if s.len() < 2 {
            if cas == 1 {
                for v in s.iter_mut() {
                    *v *= 0.5;
                }
            }
            return;
        }
        for (i, v) in s.iter_mut().enumerate() {
            *v *= if i & 1 == cas { DWT_K } else { 1.0 / DWT_K };
        }
        for (start, k) in [(cas, DWT_DELTA), (1 - cas, DWT_GAMMA), (cas, DWT_BETA), (1 - cas, DWT_ALPHA)] {
            for i in (start..s.len()).step_by(2) {
                let (a, b) = nb(s, i);
                s[i] -= k * (a + b);
            }
        }
    }

    fn rows(t: &mut [f32], rw: usize, rh: usize, cas: usize) {
        if rh < 2 {
            if cas == 1 {
                for v in t[..rw].iter_mut() {
                    *v *= 0.5;
                }
            }
            return;
        }
        for (i, row) in t[..rw * rh].chunks_exact_mut(rw).enumerate() {
            let k = if i & 1 == cas { DWT_K } else { 1.0 / DWT_K };
            for v in row {
                *v *= k;
            }
        }
        for (start, k) in [(cas, DWT_DELTA), (1 - cas, DWT_GAMMA), (cas, DWT_BETA), (1 - cas, DWT_ALPHA)] {
            for i in (start..rh).step_by(2) {
                let (cur, a, b) = rows3(t, rw, rh, i);
                for ((c, &a), &b) in cur.iter_mut().zip(a).zip(b) {
                    *c -= k * (a + b);
                }
            }
        }
    }
}

/// One synthesis level over the top-left `rw` x `rh` of a buffer holding [LL HL; LH HH].
struct Level {
    rw: usize,
    rh: usize,
    lw: usize,
    lh: usize,
    cx: usize,
    cy: usize,
}

fn idwt_level<T: Lift>(b: &mut [T], stride: usize, lv: &Level, tmp: &mut Vec<T>) {
    let (rw, rh) = (lv.rw, lv.rh);
    tmp.clear();
    tmp.resize(rw * rh, T::default());
    for y in 0..rh {
        let row = &mut b[y * stride..y * stride + rw];
        let line = &mut tmp[..rw];
        for (i, v) in line.iter_mut().enumerate() {
            *v = if i & 1 == lv.cx { row[i / 2] } else { row[lv.lw + i / 2] };
        }
        T::line(line, lv.cx);
        row.copy_from_slice(line);
    }
    for i in 0..rh {
        let src = if i & 1 == lv.cy { i / 2 } else { lv.lh + i / 2 };
        tmp[i * rw..(i + 1) * rw].copy_from_slice(&b[src * stride..src * stride + rw]);
    }
    T::rows(tmp, rw, rh, lv.cy);
    for (i, row) in tmp.chunks_exact(rw).enumerate() {
        b[i * stride..i * stride + rw].copy_from_slice(row);
    }
}

fn idwt(tc: &Tc, coefs: &mut Coefs) {
    let stride = (tc.x1 - tc.x0) as usize;
    for r in 1..tc.res.len() {
        let (res, low) = (&tc.res[r], &tc.res[r - 1]);
        let lv = Level {
            rw: (res.x1 - res.x0) as usize,
            rh: (res.y1 - res.y0) as usize,
            lw: (low.x1 - low.x0) as usize,
            lh: (low.y1 - low.y0) as usize,
            cx: (res.x0 & 1) as usize,
            cy: (res.y0 & 1) as usize,
        };
        if lv.rw == 0 || lv.rh == 0 {
            continue;
        }
        match coefs {
            Coefs::Int(b) => idwt_level(b, stride, &lv, &mut Vec::new()),
            Coefs::Float(b) => idwt_level(b, stride, &lv, &mut Vec::new()),
        }
    }
}

/// Inverse RCT or ICT on the first three components; false when their types do not match.
fn inverse_mct(coefs: &mut [Coefs]) -> bool {
    let [a, b, c, ..] = coefs else { return false };
    match (a, b, c) {
        (Coefs::Int(y), Coefs::Int(u), Coefs::Int(v)) => {
            for ((y, u), v) in y.iter_mut().zip(u.iter_mut()).zip(v.iter_mut()) {
                let g = y.wrapping_sub(u.wrapping_add(*v) >> 2);
                let r = v.wrapping_add(g);
                let bb = u.wrapping_add(g);
                (*y, *u, *v) = (r, g, bb);
            }
            true
        }
        (Coefs::Float(y), Coefs::Float(u), Coefs::Float(v)) => {
            for ((y, u), v) in y.iter_mut().zip(u.iter_mut()).zip(v.iter_mut()) {
                let (yy, cb, cr) = (*y, *u, *v);
                *y = yy + 1.402 * cr;
                *u = yy - 0.344_136 * cb - 0.714_136 * cr;
                *v = yy + 1.772 * cb;
            }
            true
        }
        _ => false,
    }
}

/// DC level shift, clamp and copy of a tile-component into its plane.
fn store(tc: &Tc, coefs: &Coefs, cs: &CompSiz, plane: &mut [i32], pw: usize, origin: (u32, u32)) {
    let w = (tc.x1 - tc.x0) as usize;
    let (ox, oy) = ((tc.x0 - origin.0) as usize, (tc.y0 - origin.1) as usize);
    let (lo, hi, shift) = if cs.signed {
        (-(1i32 << (cs.prec - 1)), (1i32 << (cs.prec - 1)) - 1, 0)
    } else {
        (0, (1i32 << cs.prec) - 1, 1i32 << (cs.prec - 1))
    };
    for y in 0..(tc.y1 - tc.y0) as usize {
        let dst = &mut plane[(oy + y) * pw + ox..(oy + y) * pw + ox + w];
        match coefs {
            Coefs::Int(b) => {
                for (d, &v) in dst.iter_mut().zip(&b[y * w..(y + 1) * w]) {
                    *d = v.wrapping_add(shift).clamp(lo, hi);
                }
            }
            Coefs::Float(b) => {
                for (d, &v) in dst.iter_mut().zip(&b[y * w..(y + 1) * w]) {
                    *d = ((v + shift as f32).round() as i32).clamp(lo, hi);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------------------------------------------

fn to8(v: i64, depth: u32, signed: bool) -> u8 {
    let u = if signed { v + (1i64 << (depth - 1)) } else { v };
    let max = (1i64 << depth) - 1;
    let u = u.clamp(0, max);
    if depth == 8 { u as u8 } else { ((u * 255 + max / 2) / max) as u8 }
}

fn render(siz: &Siz, planes: &[Vec<i32>], jp2: &Jp2, chans: &[Chan], has_alpha: bool, mct_used: bool) -> Decoded {
    let (w, h) = ((siz.x1 - siz.x0) as usize, (siz.y1 - siz.y0) as usize);
    let nch = chans.len();
    // Per channel: a lookup from (plane value - offset) to 8 bits, and the component's grid.
    struct Prep {
        lut: Vec<u8>,
        offset: i32,
        cols: Vec<u32>,
        comp: usize,
        pw: usize,
        dy: u32,
        cy0: u32,
        ch: u32,
    }
    let preps: Vec<Prep> = chans
        .iter()
        .map(|ch| {
            let cs = siz.comps[ch.comp];
            let (lut, offset) = match (ch.pcol, &jp2.palette) {
                (Some(col), Some(pal)) => {
                    let (d, s) = pal.depth[col];
                    let n = pal.depth.len();
                    ((0..pal.entries).map(|e| to8(i64::from(pal.values[e * n + col]), d, s)).collect(), 0)
                }
                _ => {
                    let lo = if cs.signed { -(1i32 << (cs.prec - 1)) } else { 0 };
                    ((0..1i64 << cs.prec).map(|v| to8(v + i64::from(lo), cs.prec, cs.signed)).collect(), lo)
                }
            };
            let (cx0, cy0, cx1, cy1) = siz.comp_rect(ch.comp);
            let cols = (0..w as u32).map(|x| ((siz.x0 + x) / cs.dx).clamp(cx0, cx1 - 1) - cx0).collect();
            Prep { lut, offset, cols, comp: ch.comp, pw: (cx1 - cx0) as usize, dy: cs.dy, cy0, ch: cy1 - cy0 }
        })
        .collect();
    let mut data = vec![0u8; w * h * nch];
    for y in 0..h {
        let out = &mut data[y * w * nch..(y + 1) * w * nch];
        for (k, pr) in preps.iter().enumerate() {
            let ry = (((siz.y0 + y as u32) / pr.dy).max(pr.cy0) - pr.cy0).min(pr.ch - 1) as usize;
            let row = &planes[pr.comp][ry * pr.pw..(ry + 1) * pr.pw];
            let last = pr.lut.len() as i64 - 1;
            for (x, &col) in pr.cols.iter().enumerate() {
                let v = (i64::from(row[col as usize]) - i64::from(pr.offset)).clamp(0, last);
                out[x * nch + k] = pr.lut[v as usize];
            }
        }
    }
    let mut colorspace = jp2.colorspace;
    let colours = nch - usize::from(has_alpha);
    if colorspace == 18 && !mct_used && colours >= 3 {
        for px in data.chunks_exact_mut(nch) {
            let (y, cb, cr) = (f32::from(px[0]), f32::from(px[1]) - 128.0, f32::from(px[2]) - 128.0);
            px[0] = (y + 1.402 * cr).round().clamp(0.0, 255.0) as u8;
            px[1] = (y - 0.344_136 * cb - 0.714_136 * cr).round().clamp(0.0, 255.0) as u8;
            px[2] = (y + 1.772 * cb).round().clamp(0.0, 255.0) as u8;
        }
        colorspace = 16;
    }
    Decoded { width: w as u32, height: h as u32, components: nch as u8, data, has_alpha, colorspace }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// Smooth gradients, a few hard edges and a little texture: close to photographic content.
    fn test_image(w: usize, h: usize, nc: usize, maxval: u32) -> Vec<u16> {
        let mut v = Vec::with_capacity(w * h * nc);
        let scale = maxval as f32 / 255.0;
        for y in 0..h {
            for x in 0..w {
                let (fx, fy) = (x as f32 / w.max(1) as f32, y as f32 / h.max(1) as f32);
                let ring = (((fx - 0.5).powi(2) + (fy - 0.4).powi(2)).sqrt() * 18.0).sin();
                let edge = if (x / 37 + y / 29) % 2 == 0 { 30.0 } else { -30.0 };
                let noise = ((x * 7919 + y * 104_729) % 13) as f32 - 6.0;
                for c in 0..nc {
                    let base = 128.0 + 70.0 * ((fx * 3.0 + c as f32).sin() * (fy * 2.0 - c as f32 * 0.7).cos());
                    let val = base + 25.0 * ring + edge * (c as f32 - 1.0).abs().min(1.0) + noise;
                    v.push((val.clamp(0.0, 255.0) * scale).round() as u16);
                }
            }
        }
        v
    }

    fn mae(a: &[u8], b: &[u8]) -> f64 {
        assert_eq!(a.len(), b.len());
        a.iter().zip(b).map(|(&x, &y)| (f64::from(x) - f64::from(y)).abs()).sum::<f64>() / a.len() as f64
    }

    fn lcg(seed: &mut u32) -> u32 {
        *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        *seed >> 8
    }

    // ----- tools (tests skip silently when they are missing) -----

    fn have_opj() -> bool {
        Command::new("opj_compress").arg("-h").output().is_ok()
            && Command::new("opj_decompress").arg("-h").output().is_ok()
    }

    fn have_magick_jp2() -> bool {
        Command::new("magick")
            .args(["-list", "format"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_ascii_lowercase().contains("jp2"))
            .unwrap_or(false)
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> TempDir {
            let p = std::env::temp_dir().join(format!("pdf-codecs-jpx-{}-{name}", std::process::id()));
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

    fn write_pnm(path: &Path, img: &[u16], w: usize, h: usize, nc: usize, maxval: u32) {
        let mut v = format!("{}\n{w} {h}\n{maxval}\n", if nc == 1 { "P5" } else { "P6" }).into_bytes();
        for &s in img {
            if maxval > 255 {
                v.extend_from_slice(&s.to_be_bytes());
            } else {
                v.push(s as u8);
            }
        }
        std::fs::write(path, v).unwrap();
    }

    /// (width, height, channels, maxval, samples)
    fn read_pnm(path: &Path) -> Option<(usize, usize, usize, u32, Vec<u16>)> {
        let d = std::fs::read(path).ok()?;
        let mut fields = Vec::new();
        let mut i = 0;
        while fields.len() < 4 && i < d.len() {
            while i < d.len() && (d[i].is_ascii_whitespace() || d[i] == b'#') {
                if d[i] == b'#' {
                    while i < d.len() && d[i] != b'\n' {
                        i += 1;
                    }
                }
                i += 1;
            }
            let s = i;
            while i < d.len() && !d[i].is_ascii_whitespace() {
                i += 1;
            }
            fields.push(String::from_utf8_lossy(&d[s..i]).to_string());
        }
        i += 1;
        let nc = match fields[0].as_str() {
            "P5" => 1,
            "P6" => 3,
            _ => return None,
        };
        let (w, h, maxval): (usize, usize, u32) =
            (fields[1].parse().ok()?, fields[2].parse().ok()?, fields[3].parse().ok()?);
        let body = &d[i..];
        let samples = if maxval > 255 {
            body.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes([c[0], c[1]])).collect()
        } else {
            body.iter().map(|&b| u16::from(b)).collect()
        };
        Some((w, h, nc, maxval, samples))
    }

    /// Encodes a PNM with opj_compress; `ext` picks J2K or JP2.
    fn opj_encode(dir: &TempDir, input: &Path, name: &str, args: &[&str]) -> Option<Vec<u8>> {
        let out = dir.path(name);
        let o = Command::new("opj_compress").arg("-i").arg(input).arg("-o").arg(&out).args(args).output().ok()?;
        if !o.status.success() {
            eprintln!("opj_compress {args:?} failed: {}", String::from_utf8_lossy(&o.stderr));
            return None;
        }
        std::fs::read(out).ok()
    }

    /// OpenJPEG's own decode as 8-bit samples (deeper samples scaled with `to8`).
    fn opj_reference(dir: &TempDir, file: &[u8], ext: &str, nc: usize) -> Option<(usize, usize, Vec<u8>)> {
        let input = dir.path(&format!("ref-in.{ext}"));
        std::fs::write(&input, file).unwrap();
        let out = dir.path(if nc == 1 { "ref-out.pgm" } else { "ref-out.ppm" });
        if !run(Command::new("opj_decompress").arg("-i").arg(&input).arg("-o").arg(&out)) {
            return None;
        }
        let (w, h, _, maxval, s) = read_pnm(&out)?;
        let depth = 32 - maxval.leading_zeros();
        Some((w, h, s.iter().map(|&v| to8(i64::from(v), depth, false)).collect()))
    }

    struct Case {
        name: &'static str,
        w: usize,
        h: usize,
        nc: usize,
        maxval: u32,
        ext: &'static str,
        args: &'static [&'static str],
    }

    const fn case(name: &'static str, w: usize, h: usize, nc: usize, args: &'static [&'static str]) -> Case {
        Case { name, w, h, nc, maxval: 255, ext: "j2k", args }
    }

    /// Encodes the case's test image; None when the tool is missing or refuses the options.
    fn encode_case(dir: &TempDir, c: &Case) -> Option<Vec<u8>> {
        let img = test_image(c.w, c.h, c.nc, c.maxval);
        let input = dir.path(if c.nc == 1 { "in.pgm" } else { "in.ppm" });
        write_pnm(&input, &img, c.w, c.h, c.nc, c.maxval);
        opj_encode(dir, &input, &format!("{}.{}", c.name, c.ext), c.args)
    }

    fn lossless_cases() -> Vec<Case> {
        vec![
            case("grey", 97, 61, 1, &[]),
            Case { ext: "jp2", ..case("rgb-jp2", 128, 96, 3, &[]) },
            case("rgb-raw", 77, 51, 3, &[]),
            case("tiles", 150, 110, 3, &["-t", "64,48"]),
            case("tiles-offsets", 131, 77, 1, &["-t", "50,40", "-d", "13,7", "-T", "5,3"]),
            case("odd-offset", 40, 31, 3, &["-d", "3,5", "-n", "4"]),
            case("levels0", 33, 20, 1, &["-n", "1"]),
            case("levels2", 64, 64, 1, &["-n", "3"]),
            case("levels7", 300, 200, 1, &["-n", "8"]),
            case("lrcp-layers", 120, 90, 3, &["-r", "40,10,1", "-p", "LRCP", "-c", "[64,64],[32,32]"]),
            case("rlcp", 120, 90, 3, &["-r", "40,10,1", "-p", "RLCP", "-c", "[64,64],[32,32]"]),
            case("rpcl", 120, 90, 3, &["-r", "40,10,1", "-p", "RPCL", "-c", "[64,64],[32,32]"]),
            case("pcrl", 120, 90, 3, &["-r", "40,10,1", "-p", "PCRL", "-c", "[32,32],[16,16]", "-t", "70,50"]),
            case("cprl", 120, 90, 3, &["-r", "40,10,1", "-p", "CPRL", "-c", "[64,64],[32,32]"]),
            case("sop-eph", 100, 80, 3, &["-SOP", "-EPH", "-r", "20,1"]),
            case("plt-tlm", 100, 80, 3, &["-PLT", "-TLM", "-t", "60,60"]),
            case("tileparts", 128, 128, 3, &["-t", "64,64", "-TP", "R", "-p", "RLCP"]),
            case("cblk-small", 90, 70, 1, &["-b", "16,8"]),
            case("bypass", 120, 90, 3, &["-M", "1"]),
            case("reset", 120, 90, 1, &["-M", "2"]),
            case("termall", 120, 90, 1, &["-M", "4"]),
            case("vsc", 120, 90, 1, &["-M", "8"]),
            case("erterm", 120, 90, 1, &["-M", "16"]),
            case("segsym", 120, 90, 1, &["-M", "32"]),
            case("all-modes", 120, 90, 3, &["-M", "63", "-r", "30,1"]),
            case("bypass-termall", 120, 90, 3, &["-M", "5", "-r", "30,1"]),
            case("no-mct", 64, 48, 3, &["-mct", "0"]),
            case("roi", 64, 48, 3, &["-ROI", "c=0,U=4"]),
            case("poc", 96, 64, 3, &["-r", "20,1", "-POC", "T1=0,0,1,4,3,CPRL/T1=0,0,2,6,3,LRCP"]),
            Case { maxval: 65535, ..case("grey16", 70, 50, 1, &[]) },
            Case { maxval: 65535, ext: "jp2", ..case("rgb16", 70, 50, 3, &[]) },
            Case { maxval: 4095, ..case("grey12", 70, 50, 1, &["-t", "32,32"]) },
        ]
    }

    #[test]
    fn lossless_matches_openjpeg() {
        if !have_opj() {
            return;
        }
        let dir = TempDir::new("lossless");
        for c in lossless_cases() {
            let Some(file) = encode_case(&dir, &c) else { continue };
            let (rw, rh, reference) = opj_reference(&dir, &file, c.ext, c.nc).expect("opj_decompress");
            let d = decode(&file).unwrap_or_else(|e| panic!("{}: {e}", c.name));
            assert_eq!((d.width as usize, d.height as usize), (rw, rh), "{}", c.name);
            assert_eq!(d.components as usize, c.nc, "{}", c.name);
            assert_eq!(info(&file).unwrap(), (d.width, d.height, d.components), "{}", c.name);
            if d.data != reference {
                let bad = d.data.iter().zip(&reference).filter(|(a, b)| a != b).count();
                panic!("{}: {bad} of {} samples differ (MAE {:.3})", c.name, d.data.len(), mae(&d.data, &reference));
            }
            // Lossless means the original back, too.
            let orig: Vec<u8> = test_image(c.w, c.h, c.nc, c.maxval)
                .iter()
                .map(|&v| to8(i64::from(v), 32 - c.maxval.leading_zeros(), false))
                .collect();
            // OpenJPEG's encoder leaves packets out with this POC list (its own decode matches ours).
            if c.args.iter().all(|a| !a.starts_with("-d")) && c.name != "poc" {
                assert_eq!(d.data, orig, "{}: not the original image", c.name);
            }
        }
    }

    /// Bit depths below 8 and signed samples, through OpenJPEG's raw input (PGM input is promoted to 8 bits).
    #[test]
    fn low_and_signed_depths() {
        if !have_opj() {
            return;
        }
        let dir = TempDir::new("depths");
        let (w, h) = (45usize, 33usize);
        for (depth, signed, nc, args) in [
            (2u32, false, 1usize, &["-n", "3"][..]),
            (4, false, 1, &[][..]),
            (6, false, 3, &["-t", "32,16", "-n", "3"][..]),
            (8, true, 1, &[][..]),
            (8, true, 3, &["-I"][..]),
        ] {
            let img = test_image(w, h, nc, 255);
            let vals: Vec<i32> = img
                .iter()
                .map(|&v| {
                    let v = i32::from(v) >> (8 - depth);
                    if signed { v - (1 << (depth - 1)) } else { v }
                })
                .collect();
            let planar: Vec<u8> = (0..nc).flat_map(|c| vals.iter().skip(c).step_by(nc).map(|&v| v as u8)).collect();
            let raw = dir.path("in.raw");
            std::fs::write(&raw, planar).unwrap();
            let f = format!("{w},{h},{nc},{depth},{}", if signed { "s" } else { "u" });
            let mut all = vec!["-F", f.as_str()];
            all.extend_from_slice(args);
            let Some(file) = opj_encode(&dir, &raw, "depth.j2k", &all) else { continue };
            assert_eq!(file[42], (depth - 1) as u8 | if signed { 0x80 } else { 0 }, "Ssiz");
            let d = decode(&file).unwrap();
            let want: Vec<u8> = vals.iter().map(|&v| to8(i64::from(v), depth, signed)).collect();
            let m = mae(&d.data, &want);
            if args.contains(&"-I") {
                assert!(m < 2.0, "depth {depth} signed {signed}: MAE {m}");
            } else {
                assert_eq!(d.data, want, "depth {depth} signed {signed}");
            }
        }
    }

    #[test]
    fn lossy_97_close_to_openjpeg() {
        if !have_opj() {
            return;
        }
        let dir = TempDir::new("lossy");
        let cases = [
            Case { ext: "jp2", ..case("rgb", 160, 120, 3, &["-I", "-r", "20"]) },
            case("grey", 133, 97, 1, &["-I", "-r", "10"]),
            case("tiles", 150, 110, 3, &["-I", "-r", "15", "-t", "64,48", "-d", "3,2"]),
            case("levels1", 64, 64, 3, &["-I", "-r", "8", "-n", "2"]),
            case("layers-rpcl", 140, 100, 3, &["-I", "-r", "50,20,5", "-p", "RPCL", "-c", "[64,64],[32,32]"]),
            case("modes", 140, 100, 3, &["-I", "-r", "12", "-M", "63", "-SOP", "-EPH"]),
            case("near-lossless", 90, 60, 3, &["-I"]),
            Case { maxval: 65535, ..case("grey16", 90, 60, 1, &["-I", "-r", "10"]) },
        ];
        for c in cases {
            let Some(file) = encode_case(&dir, &c) else { continue };
            let (rw, rh, reference) = opj_reference(&dir, &file, c.ext, c.nc).expect("opj_decompress");
            let d = decode(&file).unwrap_or_else(|e| panic!("{}: {e}", c.name));
            assert_eq!((d.width as usize, d.height as usize), (rw, rh), "{}", c.name);
            let m = mae(&d.data, &reference);
            let worst = d.data.iter().zip(&reference).map(|(&a, &b)| a.abs_diff(b)).max().unwrap();
            eprintln!("lossy {}: MAE {m:.4} vs OpenJPEG, max diff {worst}", c.name);
            assert!(m < 1.0, "{}: MAE {m}", c.name);
        }
    }

    #[test]
    fn imagemagick_jp2() {
        if !have_magick_jp2() {
            return;
        }
        let dir = TempDir::new("magick");
        let (w, h) = (90, 70);
        let img = test_image(w, h, 3, 255);
        let ppm = dir.path("in.ppm");
        write_pnm(&ppm, &img, w, h, 3, 255);
        let orig: Vec<u8> = img.iter().map(|&v| v as u8).collect();
        for (name, args) in [("lossless.jp2", &["-quality", "0"][..]), ("lossy.jp2", &["-quality", "40"][..])] {
            let out = dir.path(name);
            if !run(Command::new("magick").arg(&ppm).args(args).arg(&out)) {
                continue;
            }
            let file = std::fs::read(&out).unwrap();
            let d = decode(&file).unwrap();
            assert_eq!((d.width, d.height, d.components), (w as u32, h as u32, 3));
            let back = dir.path("back.ppm");
            assert!(run(Command::new("magick").arg(&out).args(["-depth", "8"]).arg(&back)));
            let (_, _, _, _, reference) = read_pnm(&back).unwrap();
            let reference: Vec<u8> = reference.iter().map(|&v| v as u8).collect();
            let m = mae(&d.data, &reference);
            eprintln!("magick {name}: MAE {m:.4} vs magick, {:.3} vs original", mae(&d.data, &orig));
            assert!(m < 1.0, "{name}: MAE {m}");
        }
    }

    // ----- packed packet headers: rewrite a SOP/EPH codestream into PPT or PPM form -----

    /// Splits a tile-part body written with SOP and EPH into (header incl. EPH, body incl. SOP) per packet.
    fn split_packets(body: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)> {
        let mut out = Vec::new();
        let mut i = 0;
        while i + 6 <= body.len() {
            assert_eq!(&body[i..i + 2], &[0xff, 0x91], "packet without SOP");
            let hs = i + 6;
            let eph = (hs..body.len() - 1).find(|&k| body[k] == 0xff && body[k + 1] == 0x92).unwrap();
            let next = (eph + 2..body.len().saturating_sub(1))
                .find(|&k| body[k] == 0xff && body[k + 1] == 0x91)
                .unwrap_or(body.len());
            let mut b = body[i..hs].to_vec();
            b.extend_from_slice(&body[eph + 2..next]);
            out.push((body[hs..eph + 2].to_vec(), b));
            i = next;
        }
        out
    }

    fn marker_segments(code: u16, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        for (z, chunk) in data.chunks(60000).enumerate() {
            out.extend_from_slice(&code.to_be_bytes());
            out.extend_from_slice(&((3 + chunk.len()) as u16).to_be_bytes());
            out.push(z as u8);
            out.extend_from_slice(chunk);
        }
        out
    }

    /// Moves every packet header into PPT markers (per tile-part) or one PPM run in the main header.
    fn pack_headers(cs: &[u8], use_ppm: bool) -> Vec<u8> {
        let sot = (0..cs.len() - 1).find(|&k| cs[k] == 0xff && cs[k + 1] == 0x90).unwrap();
        let mut main = cs[..sot].to_vec();
        let mut parts = Vec::new();
        let mut ppm = Vec::new();
        let mut pos = sot;
        while be16(cs, pos) == Some(SOT) {
            let psot = be32(cs, pos + 6).unwrap() as usize;
            let end = if psot == 0 { cs.len() - 2 } else { pos + psot };
            let sod = (pos + 12..end).find(|&k| cs[k] == 0xff && cs[k + 1] == 0x93).unwrap();
            let packets = split_packets(&cs[sod + 2..end]);
            let headers: Vec<u8> = packets.iter().flat_map(|p| p.0.clone()).collect();
            let bodies: Vec<u8> = packets.iter().flat_map(|p| p.1.clone()).collect();
            let mut hdr = cs[pos + 12..sod].to_vec();
            if use_ppm {
                ppm.extend_from_slice(&(headers.len() as u32).to_be_bytes());
                ppm.extend_from_slice(&headers);
            } else {
                hdr.extend(marker_segments(PPT, &headers));
            }
            let mut tp = cs[pos..pos + 12].to_vec();
            tp.extend_from_slice(&hdr);
            tp.extend_from_slice(&[0xff, 0x93]);
            tp.extend_from_slice(&bodies);
            let len = tp.len() as u32;
            tp[6..10].copy_from_slice(&len.to_be_bytes());
            parts.push(tp);
            pos = end;
        }
        if use_ppm {
            main.extend(marker_segments(PPM, &ppm));
        }
        let mut out = main;
        for p in parts {
            out.extend(p);
        }
        out.extend_from_slice(&[0xff, 0xd9]);
        out
    }

    #[test]
    fn packed_packet_headers() {
        if !have_opj() {
            return;
        }
        let dir = TempDir::new("packed");
        let cases = [
            case("single", 100, 80, 3, &["-SOP", "-EPH", "-r", "20,5,1"]),
            case("tiles", 130, 90, 3, &["-SOP", "-EPH", "-t", "64,64", "-r", "10,1", "-p", "RPCL"]),
            case("tileparts", 130, 90, 3, &["-SOP", "-EPH", "-t", "64,64", "-TP", "R", "-I", "-r", "10"]),
        ];
        for c in cases {
            let Some(file) = encode_case(&dir, &c) else { continue };
            let want = decode(&file).unwrap();
            for use_ppm in [false, true] {
                let packed = pack_headers(&file, use_ppm);
                let got = decode(&packed).unwrap_or_else(|e| panic!("{} ppm={use_ppm}: {e}", c.name));
                assert_eq!(got.data, want.data, "{} ppm={use_ppm}", c.name);
                // OpenJPEG 2.5 reads one Nppm chunk per tile, not per tile-part as A.7.4 says, so it only
                // agrees on single-part tiles.
                if let Some((_, _, r)) = opj_reference(&dir, &packed, "j2k", c.nc) {
                    if c.name == "tileparts" && use_ppm {
                        eprintln!("tile-parts with PPM: OpenJPEG agrees: {}", r == want.data);
                    } else {
                        assert_eq!(r, want.data, "{} ppm={use_ppm}: OpenJPEG disagrees", c.name);
                    }
                }
            }
        }
    }

    // ----- JP2 boxes built around OpenJPEG codestreams -----

    fn jp2_box(t: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(t);
        v.extend_from_slice(body);
        v
    }

    fn wrap_jp2(cs: &[u8], w: u32, h: u32, nc: u16, colr: u32, extra: &[Vec<u8>]) -> Vec<u8> {
        let mut out = jp2_box(b"jP  ", &[0x0d, 0x0a, 0x87, 0x0a]);
        out.extend(jp2_box(b"ftyp", b"jp2 \0\0\0\0jp2 "));
        let mut ihdr = h.to_be_bytes().to_vec();
        ihdr.extend_from_slice(&w.to_be_bytes());
        ihdr.extend_from_slice(&nc.to_be_bytes());
        ihdr.extend_from_slice(&[7, 7, 0, 0]);
        let mut hdr = jp2_box(b"ihdr", &ihdr);
        let mut c = vec![1, 0, 0];
        c.extend_from_slice(&colr.to_be_bytes());
        hdr.extend(jp2_box(b"colr", &c));
        for e in extra {
            hdr.extend_from_slice(e);
        }
        out.extend(jp2_box(b"jp2h", &hdr));
        out.extend(jp2_box(b"jp2c", cs));
        out
    }

    fn raw_encode(dir: &TempDir, samples: &[u8], w: usize, h: usize, nc: usize, args: &[&str]) -> Option<Vec<u8>> {
        let raw = dir.path("in.raw");
        // OpenJPEG's raw input is planar.
        let planar: Vec<u8> = (0..nc).flat_map(|c| samples.iter().skip(c).step_by(nc).copied()).collect();
        std::fs::write(&raw, planar).unwrap();
        let f = format!("{w},{h},{nc},8,u");
        let mut all = vec!["-F", &f];
        all.extend_from_slice(args);
        opj_encode(dir, &raw, "raw.j2k", &all)
    }

    #[test]
    fn palette_alpha_and_sycc() {
        if !have_opj() {
            return;
        }
        let dir = TempDir::new("boxes");
        let (w, h) = (61usize, 43usize);
        // Palette: 200 entries of RGB, 8-bit indices.
        let idx: Vec<u8> = (0..w * h).map(|i| ((i * 7 + i / w * 3) % 200) as u8).collect();
        let cs = raw_encode(&dir, &idx, w, h, 1, &[]).unwrap();
        let pal: Vec<[u8; 3]> = (0..200u32).map(|e| [e as u8, (255 - e) as u8, (e * 3 % 256) as u8]).collect();
        let mut pclr = 200u16.to_be_bytes().to_vec();
        pclr.extend_from_slice(&[3, 7, 7, 7]);
        for e in &pal {
            pclr.extend_from_slice(e);
        }
        let cmap = [0, 0, 1, 0, 0, 0, 1, 1, 0, 0, 1, 2];
        let file = wrap_jp2(&cs, w as u32, h as u32, 1, 16, &[jp2_box(b"pclr", &pclr), jp2_box(b"cmap", &cmap)]);
        let d = decode(&file).unwrap();
        assert_eq!((d.components, d.colorspace, d.has_alpha), (3, 16, false));
        assert_eq!(info(&file).unwrap(), (w as u32, h as u32, 3));
        let want: Vec<u8> = idx.iter().flat_map(|&i| pal[i as usize]).collect();
        assert_eq!(d.data, want);

        // Channel definitions: alpha stored first, colours associated out of order.
        let rgba = test_image(w, h, 4, 255).iter().map(|&v| v as u8).collect::<Vec<u8>>();
        let cs = raw_encode(&dir, &rgba, w, h, 4, &["-mct", "0"]).unwrap();
        let mut cdef = 4u16.to_be_bytes().to_vec();
        for (cn, typ, asoc) in [(0u16, 1u16, 0u16), (1, 0, 3), (2, 0, 1), (3, 0, 2)] {
            for v in [cn, typ, asoc] {
                cdef.extend_from_slice(&v.to_be_bytes());
            }
        }
        let file = wrap_jp2(&cs, w as u32, h as u32, 4, 16, &[jp2_box(b"cdef", &cdef)]);
        let d = decode(&file).unwrap();
        assert_eq!((d.components, d.has_alpha), (4, true));
        let want: Vec<u8> = rgba.as_chunks::<4>().0.iter().flat_map(|p| [p[2], p[3], p[1], p[0]]).collect();
        assert_eq!(d.data, want);

        // sYCC: three components without MCT, converted to RGB.
        let ycc = test_image(w, h, 3, 255).iter().map(|&v| v as u8).collect::<Vec<u8>>();
        let cs = raw_encode(&dir, &ycc, w, h, 3, &["-mct", "0"]).unwrap();
        let file = wrap_jp2(&cs, w as u32, h as u32, 3, 18, &[]);
        let d = decode(&file).unwrap();
        assert_eq!((d.components, d.colorspace), (3, 16));
        let want: Vec<u8> = ycc
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| {
                let (y, cb, cr) = (f32::from(p[0]), f32::from(p[1]) - 128.0, f32::from(p[2]) - 128.0);
                [y + 1.402 * cr, y - 0.344_136 * cb - 0.714_136 * cr, y + 1.772 * cb]
                    .map(|v| v.round().clamp(0.0, 255.0) as u8)
            })
            .collect();
        assert_eq!(d.data, want);
        if let Some((_, _, r)) = opj_reference(&dir, &file, "jp2", 3) {
            let m = mae(&d.data, &r);
            eprintln!("sYCC: MAE {m:.3} vs OpenJPEG");
            assert!(m < 1.5, "sYCC MAE {m}");
        }
    }

    #[test]
    fn subsampled_components_upsample() {
        if !have_opj() {
            return;
        }
        let dir = TempDir::new("subsampled");
        let (w, h) = (64usize, 48usize);
        let y: Vec<u8> = (0..w * h).map(|i| (i % w * 4) as u8).collect();
        let c: Vec<u8> = (0..w * h / 4).map(|i| (i % (w / 2) * 8) as u8).collect();
        let mut raw = y.clone();
        raw.extend_from_slice(&c);
        raw.extend_from_slice(&c);
        let rawp = dir.path("in.raw");
        std::fs::write(&rawp, &raw).unwrap();
        let f = format!("{w},{h},3,8,u@1x1:2x2:2x2");
        let Some(cs) = opj_encode(&dir, &rawp, "sub.j2k", &["-F", &f, "-mct", "0"]) else { return };
        let d = decode(&cs).unwrap();
        assert_eq!((d.width, d.height, d.components), (w as u32, h as u32, 3));
        for py in 0..h {
            for px in 0..w {
                let o = (py * w + px) * 3;
                assert_eq!(d.data[o], y[py * w + px]);
                assert_eq!(d.data[o + 1], c[(py / 2) * (w / 2) + px / 2]);
            }
        }
    }

    // ----- damage -----

    fn damage_inputs(dir: &TempDir) -> Vec<(String, Vec<u8>)> {
        let cases = [
            Case { ext: "jp2", ..case("rgb", 96, 72, 3, &["-r", "10,1", "-t", "48,48"]) },
            case("lossy", 96, 72, 3, &["-I", "-r", "8", "-SOP", "-EPH", "-p", "PCRL"]),
            case("modes", 80, 60, 1, &["-M", "63", "-b", "16,16"]),
        ];
        let mut v = Vec::new();
        for c in cases {
            if let Some(f) = encode_case(dir, &c) {
                v.push((c.name.to_string(), f));
            }
        }
        if let Some(f) = encode_case(dir, &case("ppm-src", 96, 72, 3, &["-SOP", "-EPH", "-t", "48,48"])) {
            v.push(("ppm".into(), pack_headers(&f, true)));
            v.push(("ppt".into(), pack_headers(&f, false)));
        }
        v
    }

    #[test]
    fn truncated_streams_decode_what_is_there() {
        if !have_opj() {
            return;
        }
        let dir = TempDir::new("truncate");
        for (name, file) in damage_inputs(&dir) {
            let full = decode(&file).unwrap();
            let step = (file.len() / 300).max(1);
            for cut in (0..file.len()).step_by(step).chain([file.len() * 7 / 10, file.len() - 1]) {
                let r = decode(&file[..cut]);
                if let Ok(d) = &r {
                    assert_eq!(d.data.len(), (d.width * d.height) as usize * d.components as usize);
                }
                let _ = info(&file[..cut]);
                if cut == file.len() * 7 / 10 {
                    let d = r.unwrap_or_else(|e| panic!("{name} at 70%: {e}"));
                    assert_eq!((d.width, d.height, d.components), (full.width, full.height, full.components));
                }
            }
        }
    }

    #[test]
    fn corrupt_streams_never_panic() {
        if !have_opj() {
            return;
        }
        let dir = TempDir::new("corrupt");
        let mut seed = 99;
        for (_, file) in damage_inputs(&dir) {
            for round in 0..400 {
                let mut bad = file.clone();
                for _ in 0..1 + lcg(&mut seed) % 6 {
                    // Half the rounds aim at the headers, where the structure lives.
                    let span = if round % 2 == 0 { bad.len().min(300) } else { bad.len() };
                    let i = lcg(&mut seed) as usize % span;
                    bad[i] = lcg(&mut seed) as u8;
                }
                if let Ok(d) = decode(&bad) {
                    assert_eq!(d.data.len(), (d.width * d.height) as usize * d.components as usize);
                }
                let _ = info(&bad);
            }
        }
    }

    #[test]
    fn hostile_headers() {
        let mut seed = 5;
        for len in [0usize, 1, 2, 3, 4, 10, 100, 1000] {
            let mut junk: Vec<u8> = (0..len).map(|_| lcg(&mut seed) as u8).collect();
            assert!(decode(&junk).is_err());
            junk.splice(0..0, [0xff, 0x4f, 0xff, 0x51]);
            let _ = decode(&junk);
        }
        let siz = |w: u32, h: u32, tw: u32, th: u32, nc: u16, prec: u8, sub: u8| {
            let mut v = vec![0xff, 0x4f, 0xff, 0x51];
            v.extend_from_slice(&(38 + 3 * nc).to_be_bytes());
            v.extend_from_slice(&[0, 0]);
            for x in [w, h, 0, 0, tw, th, 0, 0] {
                v.extend_from_slice(&x.to_be_bytes());
            }
            v.extend_from_slice(&nc.to_be_bytes());
            for _ in 0..nc {
                v.extend_from_slice(&[prec, sub, sub]);
            }
            // COD, QCD, one empty tile-part, EOC.
            v.extend_from_slice(&[0xff, 0x52, 0, 12, 0, 0, 0, 1, 0, 5, 4, 4, 0, 1]);
            v.extend_from_slice(&[0xff, 0x5c, 0, 4, 0x40, 0x48]);
            v.extend_from_slice(&[0xff, 0x90, 0, 10, 0, 0, 0, 0, 0, 14, 0, 1, 0xff, 0x93, 0xff, 0xd9]);
            v
        };
        assert!(decode(&siz(0, 10, 10, 10, 1, 7, 1)).is_err());
        assert!(decode(&siz(70000, 10, 70000, 10, 1, 7, 1)).is_err());
        assert!(decode(&siz(60000, 60000, 60000, 60000, 1, 7, 1)).is_err(), "over the pixel cap");
        assert!(decode(&siz(60000, 60000, 1, 1, 1, 7, 1)).is_err(), "too many tiles");
        assert!(decode(&siz(100, 100, 100, 100, 1, 40, 1)).is_err(), "bit depth");
        assert!(decode(&siz(100, 100, 100, 100, 1, 7, 0)).is_err(), "zero subsampling");
        assert!(decode(&siz(100, 100, 100, 100, 0, 7, 1)).is_err(), "no components");
        // Offsets that leave a subsampled component without samples.
        let mut v = siz(2, 2, 2, 2, 1, 7, 3);
        v[16..20].copy_from_slice(&1u32.to_be_bytes());
        v[20..24].copy_from_slice(&1u32.to_be_bytes());
        assert!(decode(&v).is_err() && info(&v).is_err());
        // A valid header with no packet data decodes to mid grey.
        let d = decode(&siz(20, 10, 20, 10, 1, 7, 1)).unwrap();
        assert_eq!((d.width, d.height, d.components), (20, 10, 1));
        assert!(d.data.iter().all(|&v| v == 128));
        assert_eq!(info(&siz(20, 10, 20, 10, 3, 7, 1)).unwrap(), (20, 10, 3));
        // Random bytes after a valid main header.
        for _ in 0..500 {
            let mut v = siz(37, 29, 16, 16, 3, 7, 1);
            v.truncate(v.len() - 2);
            let n = lcg(&mut seed) as usize % 200;
            v.extend((0..n).map(|_| lcg(&mut seed) as u8));
            let _ = decode(&v);
        }
    }

    /// A large lossy RGB page: the decode time is the number that matters.
    #[test]
    fn large_lossy_timing() {
        if !have_opj() {
            return;
        }
        let dir = TempDir::new("large");
        let (w, h) = (2500usize, 3500usize);
        let img = test_image(w, h, 3, 255);
        let input = dir.path("big.ppm");
        write_pnm(&input, &img, w, h, 3, 255);
        let Some(file) = opj_encode(&dir, &input, "big.jp2", &["-I", "-r", "15"]) else { return };
        let t = std::time::Instant::now();
        let d = decode(&file).unwrap();
        let secs = t.elapsed().as_secs_f64();
        let orig: Vec<u8> = img.iter().map(|&v| v as u8).collect();
        eprintln!(
            "2500x3500 RGB 9-7 ({} KB): {secs:.3} s, MAE {:.3} vs original",
            file.len() / 1024,
            mae(&d.data, &orig)
        );
        assert_eq!((d.width, d.height, d.components), (2500, 3500, 3));
        if !cfg!(debug_assertions) {
            assert!(secs < 4.0, "slow: {secs:.2} s");
        }
    }
}
