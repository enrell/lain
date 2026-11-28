//! PDF font dictionaries (§9.6–§9.9): simple fonts (Type 1, TrueType,
//! Type 3) and composite ones (Type 0 over CIDFontType0/2), their
//! encodings, widths and how a character code finds its glyph in the
//! embedded program, or in a system substitute when none is embedded.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::cff::Cff;
use super::cmap::CMap;
use super::system;
use super::truetype::TrueType;
use super::type1::Type1;
use super::{encoding, glyphlist};
use crate::document::Document;
use crate::geom::{Matrix, Path};
use crate::object::{Dict, Object, Stream};

/// Glyph outlines kept per font.
const MAX_GLYPHS_CACHED: usize = 8192;

pub enum Program {
    TrueType(TrueType),
    /// An OpenType font with CFF outlines: the sfnt for its cmap, the CFF
    /// for its glyphs.
    OpenType(TrueType, Cff),
    Cff(Cff),
    Type1(Type1),
    None,
}

impl Program {
    fn parse(data: Vec<u8>, hint: &str) -> Program {
        let sfnt = data.len() >= 4 && matches!(&data[..4], [0, 1, 0, 0] | b"true" | b"OTTO" | b"ttcf");
        if sfnt && let Ok(tt) = TrueType::parse(data.clone()) {
            if tt.is_cff()
                && let Ok(c) = Cff::from_opentype(&data)
            {
                return Program::OpenType(tt, c);
            }
            return Program::TrueType(tt);
        }
        // A bare CFF starts with major version 1 and a header size.
        if hint != "type1"
            && data.len() > 4
            && data[0] == 1
            && data[2] >= 4
            && let Ok(c) = Cff::parse(data.clone())
        {
            return Program::Cff(c);
        }
        if let Ok(t) = Type1::parse(&data) {
            return Program::Type1(t);
        }
        if let Ok(c) = Cff::parse(data) {
            return Program::Cff(c);
        }
        Program::None
    }

    fn matrix(&self) -> Matrix {
        match self {
            Program::TrueType(t) => {
                let s = 1.0 / f64::from(t.units_per_em().max(16));
                Matrix::scale(s, s)
            }
            Program::OpenType(_, c) | Program::Cff(c) => Matrix::from_array(c.font_matrix()),
            Program::Type1(t) => Matrix::from_array(t.font_matrix()),
            Program::None => Matrix::scale(0.001, 0.001),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Simple,
    Composite,
    Type3,
}

/// A Type 3 glyph: a content stream drawn in glyph space.
pub struct Type3 {
    pub matrix: Matrix,
    pub resources: Option<Arc<Dict>>,
    pub procs: HashMap<String, Arc<Stream>>,
}

pub struct PdfFont {
    pub kind: Kind,
    pub name: String,
    program: Program,
    /// Glyph space to text space (the program's matrix, or Type 3's).
    pub matrix: Matrix,
    /// Simple fonts: glyph name per code.
    names: Vec<Option<String>>,
    /// Simple fonts: the code was named by the PDF (an encoding or
    /// `/Differences`), not just the program's built-in encoding.
    symbolic: bool,
    /// Widths in thousandths of text space, by code (simple) or CID.
    widths: HashMap<u32, f64>,
    default_width: f64,
    /// Composite fonts.
    cmap: Option<CMap>,
    unicode_codes: bool,
    cid_to_gid: Option<Vec<u16>>,
    pub vertical: bool,
    pub type3: Option<Type3>,
    /// Text for codes (ToUnicode), kept for text extraction.
    pub to_unicode: Option<CMap>,
    /// The program is a system substitute.
    pub substituted: bool,
    glyphs: Mutex<HashMap<u32, Option<Arc<Path>>>>,
}

impl PdfFont {
    pub fn load(doc: &Document, d: &Dict) -> PdfFont {
        let subtype = d.name("Subtype").unwrap_or("Type1").to_string();
        let name = doc.get_in(d, "BaseFont").as_name().unwrap_or("").to_string();
        let to_unicode = match doc.get_in(d, "ToUnicode") {
            Object::Stream(s) => doc.decode_stream(&s).ok().map(|x| CMap::parse(&x.data)),
            _ => None,
        };
        match subtype.as_str() {
            "Type0" => PdfFont::composite(doc, d, name, to_unicode),
            "Type3" => PdfFont::type3(doc, d, name, to_unicode),
            _ => PdfFont::simple(doc, d, &subtype, name, to_unicode),
        }
    }

    fn base(kind: Kind, name: String, program: Program, to_unicode: Option<CMap>) -> PdfFont {
        let matrix = program.matrix();
        PdfFont {
            kind,
            name,
            program,
            matrix,
            names: vec![None; 256],
            symbolic: false,
            widths: HashMap::new(),
            default_width: 0.0,
            cmap: None,
            unicode_codes: false,
            cid_to_gid: None,
            vertical: false,
            type3: None,
            to_unicode,
            substituted: false,
            glyphs: Mutex::new(HashMap::new()),
        }
    }

    /// The embedded program, if any, from a font descriptor.
    fn embedded(doc: &Document, desc: Option<&Dict>) -> Option<(Vec<u8>, &'static str)> {
        let desc = desc?;
        for (key, hint) in [("FontFile", "type1"), ("FontFile2", "truetype"), ("FontFile3", "cff")] {
            if let Object::Stream(s) = doc.get_in(desc, key)
                && let Ok(d) = doc.decode_stream(&s)
                && !d.data.is_empty()
            {
                return Some((d.data, hint));
            }
        }
        None
    }

    fn substitute(name: &str, desc: Option<&Dict>, doc: &Document) -> Option<Program> {
        let flags = desc.and_then(|d| doc.get_in(d, "Flags").as_i64()).unwrap_or(0);
        let weight = desc.and_then(|d| doc.f64_in(d, "FontWeight")).unwrap_or(0.0);
        let style = system::style_for(name, flags, weight);
        let path = system::find(style)?;
        let data = system::load(&path)?;
        let p = Program::parse(data.to_vec(), "");
        (!matches!(p, Program::None)).then_some(p)
    }

    fn simple(doc: &Document, d: &Dict, subtype: &str, name: String, to_unicode: Option<CMap>) -> PdfFont {
        let desc = doc.dict_in(d, "FontDescriptor");
        let flags = desc.as_ref().and_then(|x| doc.get_in(x, "Flags").as_i64()).unwrap_or(0);
        let (program, substituted) = match PdfFont::embedded(doc, desc.as_deref()) {
            Some((data, hint)) => (Program::parse(data, hint), false),
            None => match PdfFont::substitute(&name, desc.as_deref(), doc) {
                Some(p) => (p, true),
                None => (Program::None, true),
            },
        };
        let mut f = PdfFont::base(Kind::Simple, name.clone(), program, to_unicode);
        f.substituted = substituted;
        // Encoding (§9.6.6): base, then differences.
        let enc = doc.get_in(d, "Encoding");
        let std_name = name.split_once('+').map_or(name.as_str(), |(_, r)| r);
        let is_symbol_font = std_name.starts_with("Symbol") || std_name.starts_with("ZapfDingbats");
        let builtin: Vec<Option<String>> = match &f.program {
            Program::Type1(t) => t.encoding(),
            Program::Cff(c) | Program::OpenType(_, c) => c.encoding(),
            _ => Vec::new(),
        };
        let table = |n: &str| -> Option<&'static [Option<&'static str>; 256]> {
            match n {
                "WinAnsiEncoding" => Some(&encoding::WIN_ANSI),
                "MacRomanEncoding" => Some(&encoding::MAC_ROMAN),
                "MacExpertEncoding" => Some(&encoding::MAC_EXPERT),
                "StandardEncoding" => Some(&encoding::STANDARD),
                _ => None,
            }
        };
        let mut names: Vec<Option<String>> = vec![None; 256];
        let fill = |names: &mut Vec<Option<String>>, t: &[Option<&str>; 256]| {
            for (i, n) in t.iter().enumerate() {
                names[i] = n.map(str::to_string);
            }
        };
        let builtin_usable = builtin.iter().any(Option::is_some);
        let default_base = |names: &mut Vec<Option<String>>| {
            if std_name.starts_with("Symbol") && !builtin_usable {
                fill(names, &encoding::SYMBOL);
            } else if std_name.starts_with("ZapfDingbats") && !builtin_usable {
                fill(names, &encoding::ZAPF_DINGBATS);
            } else if builtin_usable && (subtype != "TrueType") {
                *names = builtin.clone();
            } else if subtype != "TrueType" {
                fill(names, &encoding::STANDARD);
            }
        };
        match &enc {
            Object::Name(n) => match table(n.as_str()) {
                Some(t) => fill(&mut names, t),
                None => default_base(&mut names),
            },
            Object::Dict(ed) => {
                match ed.name("BaseEncoding").and_then(table) {
                    Some(t) => fill(&mut names, t),
                    // A symbolic TrueType font keeps its own codes.
                    None if subtype == "TrueType" && flags & 4 != 0 => {}
                    None if subtype == "TrueType" => fill(&mut names, &encoding::STANDARD),
                    None => default_base(&mut names),
                }
                let diffs = doc.array_in(ed, "Differences");
                let mut code = 0usize;
                for o in diffs {
                    match o {
                        Object::Int(i) => code = i.clamp(0, 255) as usize,
                        Object::Name(n) => {
                            if code < 256 {
                                names[code] = Some(n.as_str().to_string());
                            }
                            code += 1;
                        }
                        _ => {}
                    }
                }
            }
            _ => default_base(&mut names),
        }
        f.symbolic = (flags & 4 != 0 && !matches!(enc, Object::Name(_) | Object::Dict(_))) || is_symbol_font;
        f.names = names;
        // Widths (§9.6.2.1).
        let first = doc.get_in(d, "FirstChar").as_i64().unwrap_or(0);
        for (i, w) in doc.array_in(d, "Widths").iter().enumerate() {
            if let Some(w) = w.as_f64() {
                f.widths.insert((first + i as i64).clamp(0, 255) as u32, w);
            }
        }
        f.default_width = desc.as_ref().and_then(|x| doc.f64_in(x, "MissingWidth")).unwrap_or(0.0);
        f
    }

    fn type3(doc: &Document, d: &Dict, name: String, to_unicode: Option<CMap>) -> PdfFont {
        let mut f = PdfFont::base(Kind::Type3, name, Program::None, to_unicode);
        let m = doc.numbers_in(d, "FontMatrix");
        f.matrix =
            if m.len() == 6 { Matrix::new(m[0], m[1], m[2], m[3], m[4], m[5]) } else { Matrix::scale(0.001, 0.001) };
        let mut procs = HashMap::new();
        if let Some(cp) = doc.dict_in(d, "CharProcs") {
            for (k, v) in &cp.entries {
                if let Object::Stream(s) = doc.resolve(v) {
                    procs.insert(k.as_str().to_string(), s);
                }
            }
        }
        if let Object::Dict(ed) = doc.get_in(d, "Encoding") {
            let mut code = 0usize;
            for o in doc.array_in(&ed, "Differences") {
                match o {
                    Object::Int(i) => code = i.clamp(0, 255) as usize,
                    Object::Name(n) => {
                        if code < 256 {
                            f.names[code] = Some(n.as_str().to_string());
                        }
                        code += 1;
                    }
                    _ => {}
                }
            }
        }
        let first = doc.get_in(d, "FirstChar").as_i64().unwrap_or(0);
        for (i, w) in doc.array_in(d, "Widths").iter().enumerate() {
            if let Some(w) = w.as_f64() {
                f.widths.insert((first + i as i64).clamp(0, 255) as u32, w);
            }
        }
        f.type3 = Some(Type3 { matrix: f.matrix, resources: doc.dict_in(d, "Resources"), procs });
        f
    }

    fn composite(doc: &Document, d: &Dict, name: String, to_unicode: Option<CMap>) -> PdfFont {
        let (mut cmap, unicode_codes) = match doc.get_in(d, "Encoding") {
            Object::Name(n) => (CMap::predefined(n.as_str()), CMap::unicode_named(n.as_str())),
            Object::Stream(s) => match doc.decode_stream(&s) {
                Ok(x) => {
                    let mut c = CMap::parse(&x.data);
                    if let Some(v) = s.dict.get("WMode").and_then(Object::as_i64) {
                        c.vertical = v == 1;
                    }
                    (c, false)
                }
                Err(_) => (CMap::identity(false), false),
            },
            _ => (CMap::identity(false), false),
        };
        cmap.resolve_uses();
        let desc_font = doc
            .array_in(d, "DescendantFonts")
            .into_iter()
            .next()
            .and_then(|o| o.as_dict().cloned())
            .unwrap_or_default();
        let desc = doc.dict_in(&desc_font, "FontDescriptor");
        let (program, substituted) = match PdfFont::embedded(doc, desc.as_deref()) {
            Some((data, hint)) => (Program::parse(data, hint), false),
            None => match PdfFont::substitute(&name, desc.as_deref(), doc) {
                Some(p) => (p, true),
                None => (Program::None, true),
            },
        };
        let mut f = PdfFont::base(Kind::Composite, name, program, to_unicode);
        f.substituted = substituted;
        f.vertical = cmap.vertical;
        f.unicode_codes = unicode_codes;
        f.cmap = Some(cmap);
        if let Object::Stream(s) = doc.get_in(&desc_font, "CIDToGIDMap")
            && let Ok(x) = doc.decode_stream(&s)
        {
            f.cid_to_gid = Some(x.data.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes([c[0], c[1]])).collect());
        }
        f.default_width = doc.f64_in(&desc_font, "DW").unwrap_or(1000.0);
        // `/W`: `c [w1 w2 …]` or `cfirst clast w`.
        let w = doc.array_in(&desc_font, "W");
        let mut i = 0;
        let mut total = 0usize;
        while i < w.len() && total < 1 << 20 {
            let Some(c0) = w[i].as_i64() else { break };
            match w.get(i + 1) {
                Some(Object::Array(list)) => {
                    for (k, x) in list.iter().enumerate() {
                        if let Some(v) = doc.resolve(x).as_f64() {
                            f.widths.insert((c0.max(0) as u32).saturating_add(k as u32), v);
                            total += 1;
                        }
                    }
                    i += 2;
                }
                Some(o) => {
                    let (Some(c1), Some(v)) = (o.as_i64(), w.get(i + 2).and_then(Object::as_f64)) else { break };
                    let (c0, c1) = (c0.max(0) as u32, c1.max(0) as u32);
                    if c1 >= c0 && c1 - c0 < 1 << 16 {
                        for c in c0..=c1 {
                            f.widths.insert(c, v);
                        }
                        total += (c1 - c0) as usize + 1;
                    }
                    i += 3;
                }
                None => break,
            }
        }
        f
    }

    /// Splits a string into `(code, byte length)` pairs.
    pub fn codes(&self, bytes: &[u8]) -> Vec<(u32, usize)> {
        let mut out = Vec::with_capacity(bytes.len());
        match &self.cmap {
            None => out.extend(bytes.iter().map(|&b| (u32::from(b), 1))),
            Some(c) => {
                let mut i = 0;
                while i < bytes.len() {
                    let (code, n) = c.next_code(&bytes[i..]);
                    out.push((code, n));
                    i += n.max(1);
                }
            }
        }
        out
    }

    /// The horizontal advance of `code`, in text space units per unit of
    /// font size (a width of 500 is 0.5).
    pub fn advance(&self, code: u32) -> f64 {
        let key = match &self.cmap {
            Some(c) => c.cid(code),
            None => code,
        };
        if let Some(w) = self.widths.get(&key) {
            return if self.kind == Kind::Type3 { self.type3_advance(*w) } else { w / 1000.0 };
        }
        if self.kind == Kind::Simple && self.widths.is_empty() {
            // No `/Widths` (a standard 14 font): the program's own advance.
            if let Some(w) = self.program_advance(code) {
                return w;
            }
        }
        if self.kind == Kind::Type3 { self.type3_advance(self.default_width) } else { self.default_width / 1000.0 }
    }

    fn type3_advance(&self, w: f64) -> f64 {
        let p = self.matrix.apply_vector(w, 0.0);
        p.x
    }

    fn program_advance(&self, code: u32) -> Option<f64> {
        let name = self.names.get(code as usize).cloned().flatten();
        let m = self.program.matrix();
        let units = match &self.program {
            Program::Type1(t) => t.advance_width(name.as_deref()?)?,
            Program::Cff(c) | Program::OpenType(_, c) => c.advance_width(self.simple_gid(code)?)?,
            Program::TrueType(t) => f64::from(t.advance_width(self.simple_gid(code)?)?),
            Program::None => return None,
        };
        Some(m.apply_vector(units, 0.0).x)
    }

    /// Whether the code is the single-byte space word spacing applies to.
    pub fn is_space(&self, code: u32, len: usize) -> bool {
        code == 32 && len == 1
    }

    /// The glyph of `code` in text space (the font matrix applied); for
    /// Type 3 fonts use [`PdfFont::type3_proc`] instead.
    pub fn glyph(&self, code: u32) -> Option<Arc<Path>> {
        if let Some(g) = self.glyphs.lock().unwrap_or_else(|e| e.into_inner()).get(&code) {
            return g.clone();
        }
        let g = self.make_glyph(code).map(|p| Arc::new(p.transformed(&self.matrix)));
        let mut cache = self.glyphs.lock().unwrap_or_else(|e| e.into_inner());
        if cache.len() >= MAX_GLYPHS_CACHED {
            cache.clear();
        }
        cache.insert(code, g.clone());
        g
    }

    pub fn type3_proc(&self, code: u32) -> Option<Arc<Stream>> {
        let t = self.type3.as_ref()?;
        let name = self.names.get(code as usize)?.as_ref()?;
        t.procs.get(name).cloned()
    }

    fn make_glyph(&self, code: u32) -> Option<Path> {
        match self.kind {
            Kind::Type3 => None,
            Kind::Composite => {
                let cmap = self.cmap.as_ref()?;
                let cid = cmap.cid(code);
                match &self.program {
                    Program::TrueType(t) => {
                        let gid = if self.unicode_codes && self.cid_to_gid.is_none() {
                            t.cmap_lookup(3, 1, code).or_else(|| t.cmap_lookup(0, 3, code)).unwrap_or(cid as u16)
                        } else {
                            match &self.cid_to_gid {
                                Some(map) => *map.get(cid as usize)?,
                                None => cid.min(u32::from(u16::MAX)) as u16,
                            }
                        };
                        t.glyph_path(gid)
                    }
                    Program::Cff(c) | Program::OpenType(_, c) => {
                        let cid16 = cid.min(u32::from(u16::MAX)) as u16;
                        let gid = if c.is_cid() { c.gid_for_cid(cid16)? } else { cid16 };
                        c.glyph_path(gid)
                    }
                    Program::Type1(_) | Program::None => None,
                }
            }
            Kind::Simple => {
                let name = self.names.get(code as usize).cloned().flatten();
                match &self.program {
                    Program::Type1(t) => {
                        let n = name.or_else(|| t.encoding().get(code as usize).cloned().flatten())?;
                        t.glyph_path(&n)
                    }
                    Program::Cff(c) | Program::OpenType(_, c) => c.glyph_path(self.simple_gid(code)?),
                    Program::TrueType(t) => t.glyph_path(self.simple_gid(code)?),
                    Program::None => None,
                }
            }
        }
    }

    /// A simple font's glyph id for `code` (CFF and TrueType programs).
    fn simple_gid(&self, code: u32) -> Option<u16> {
        let name = self.names.get(code as usize).cloned().flatten();
        match &self.program {
            Program::Cff(c) => {
                if let Some(n) = &name
                    && let Some(g) = c.gid_for_name(n)
                {
                    return Some(g);
                }
                if let Some(n) = c.encoding().get(code as usize).cloned().flatten()
                    && let Some(g) = c.gid_for_name(&n)
                {
                    return Some(g);
                }
                // A CID-keyed CFF used as a simple font: code = CID.
                if c.is_cid() {
                    return c.gid_for_cid(code as u16);
                }
                None
            }
            Program::OpenType(t, c) => name
                .as_deref()
                .and_then(|n| c.gid_for_name(n))
                .or_else(|| truetype_gid(t, code, name.as_deref(), self.symbolic)),
            Program::TrueType(t) => truetype_gid(t, code, name.as_deref(), self.symbolic),
            _ => None,
        }
    }

    /// The text a code stands for, when the font says (ToUnicode) or the
    /// glyph name does.
    pub fn text(&self, code: u32) -> Option<String> {
        if let Some(t) = self.to_unicode.as_ref().and_then(|c| c.unicode.get(&code)) {
            return Some(t.clone());
        }
        if self.kind != Kind::Composite {
            let n = self.names.get(code as usize)?.as_ref()?;
            return glyphlist::to_unicode(n);
        }
        if self.unicode_codes {
            return char::from_u32(code).map(String::from);
        }
        None
    }
}

/// How a simple TrueType font maps a code (pdf.js's order, §9.6.6.4).
fn truetype_gid(t: &TrueType, code: u32, name: Option<&str>, symbolic: bool) -> Option<u16> {
    let has = |p: u16, e: u16| t.cmap_subtables().contains(&(p, e));
    if !symbolic && let Some(n) = name {
        if has(3, 1)
            && let Some(c) = glyphlist::to_char(n)
            && let Some(g) = t.cmap_lookup(3, 1, c as u32)
        {
            return Some(g);
        }
        if has(1, 0)
            && let Some(mac) = encoding::MAC_ROMAN.iter().position(|x| *x == Some(n))
            && let Some(g) = t.cmap_lookup(1, 0, mac as u32)
        {
            return Some(g);
        }
        if let Some(g) = t.glyph_by_name(n) {
            return Some(g);
        }
    }
    for base in [0xF000u32, 0xF100, 0xF200, 0] {
        if has(3, 0)
            && let Some(g) = t.cmap_lookup(3, 0, base + code)
        {
            return Some(g);
        }
    }
    if has(1, 0)
        && let Some(g) = t.cmap_lookup(1, 0, code)
    {
        return Some(g);
    }
    if has(3, 1)
        && let Some(g) = t.cmap_lookup(3, 1, code)
    {
        return Some(g);
    }
    if let Some(n) = name
        && let Some(g) = t.glyph_by_name(n)
    {
        return Some(g);
    }
    // No cmap at all: subset fonts often number glyphs by code.
    if t.cmap_subtables().is_empty() && code < u32::from(t.num_glyphs()) {
        return Some(code as u16);
    }
    None
}
