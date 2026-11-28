//! A parsed document (§7.5, §7.7): the cross-reference data (tables,
//! streams, hybrid files, incremental updates), object streams, lazy
//! object loading with a cache, the page tree, the document
//! information, the outline and the standard security handler. A file
//! whose cross-reference data is broken is rebuilt by scanning for
//! objects, as viewers do.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use crate::crypt::Decryptor;
use crate::error::{ErrorKind, PdfError, Result};
use crate::filter::{self, Decoded, Filter};
use crate::lexer::{self, Lexer, Token};
use crate::object::{Dict, ObjRef, Object, Stream};

/// The largest file opened.
pub const MAX_FILE: usize = 2 << 30;
/// Pages beyond this many are ignored.
const MAX_PAGES: usize = 100_000;
/// Reference chains and page tree depth.
const MAX_RESOLVE: usize = 32;
const MAX_OUTLINE: usize = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Entry {
    Offset(usize, u16),
    InStream(u32, u32),
}

/// One page, its inheritable attributes resolved.
#[derive(Debug, Clone)]
pub struct Page {
    pub dict: Arc<Dict>,
    pub id: Option<ObjRef>,
    pub resources: Option<Arc<Dict>>,
    /// `[x0 y0 x1 y1]`, normalised, in default user space units.
    pub media_box: [f64; 4],
    pub crop_box: [f64; 4],
    /// 0, 90, 180 or 270, clockwise.
    pub rotate: i32,
}

impl Page {
    /// The visible size in points, rotation applied.
    pub fn size(&self) -> (f64, f64) {
        let b = self.crop_box;
        let (w, h) = (b[2] - b[0], b[3] - b[1]);
        if self.rotate % 180 == 0 { (w, h) } else { (h, w) }
    }
}

/// The document information dictionary and catalog language, as text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Info {
    pub title: String,
    pub author: String,
    pub subject: String,
    pub keywords: String,
    pub creator: String,
    pub producer: String,
    pub language: String,
    /// `D:YYYY...`'s year, 0 when absent.
    pub year: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OutlineItem {
    pub title: String,
    /// The target page, when the destination resolves to one.
    pub page: Option<usize>,
    pub children: Vec<OutlineItem>,
}

pub struct Document {
    data: Arc<[u8]>,
    xref: HashMap<u32, Entry>,
    pub trailer: Dict,
    crypt: Option<Decryptor>,
    cache: Mutex<HashMap<u32, Object>>,
    objstms: Mutex<HashMap<u32, Arc<Vec<Object>>>>,
    pages: Vec<Page>,
    page_ids: HashMap<ObjRef, usize>,
    /// The cross-reference data had to be rebuilt.
    pub repaired: bool,
}

impl std::fmt::Debug for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Document").field("objects", &self.xref.len()).field("pages", &self.pages.len()).finish()
    }
}

impl Document {
    pub fn open(data: impl Into<Arc<[u8]>>) -> Result<Document> {
        let data: Arc<[u8]> = data.into();
        if data.len() > MAX_FILE {
            return Err(PdfError::too_large("file"));
        }
        let Some(header) = lexer::find(&data[..data.len().min(1024)], b"%PDF-", 0) else {
            return Err(PdfError::invalid("not a PDF file"));
        };
        let mut doc = Document {
            data: data.clone(),
            xref: HashMap::new(),
            trailer: Dict::new(),
            crypt: None,
            cache: Mutex::new(HashMap::new()),
            objstms: Mutex::new(HashMap::new()),
            pages: Vec::new(),
            page_ids: HashMap::new(),
            repaired: false,
        };
        let read = doc.read_xref(header);
        if read.is_err() || !doc.trailer.contains("Root") {
            doc.rebuild()?;
        }
        doc.setup_crypt()?;
        doc.load_pages();
        if doc.pages.is_empty() && !doc.repaired {
            // A tree that names no page: the file may still have pages.
            doc.rebuild()?;
            doc.setup_crypt()?;
            doc.load_pages();
        }
        if doc.pages.is_empty() {
            return Err(PdfError::invalid("no pages"));
        }
        Ok(doc)
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub fn page(&self, i: usize) -> Result<&Page> {
        self.pages.get(i).ok_or_else(|| PdfError::new(ErrorKind::NoSuchPage, format!("no page {}", i + 1)))
    }

    pub fn page_index(&self, r: ObjRef) -> Option<usize> {
        self.page_ids.get(&r).copied()
    }

    pub fn is_encrypted(&self) -> bool {
        self.crypt.is_some()
    }

    // ---- cross-reference data ------------------------------------------

    fn read_xref(&mut self, header: usize) -> Result<()> {
        let data = self.data.clone();
        let tail = data.len().saturating_sub(2048);
        let at = lexer::rfind(&data, b"startxref", data.len()).filter(|&p| p >= tail.saturating_sub(1 << 20));
        let Some(at) = at else { return Err(PdfError::invalid("no startxref")) };
        let mut l = Lexer::at(&data, at + 9);
        let Some(Token::Int(off)) = l.next_token() else { return Err(PdfError::invalid("bad startxref")) };
        let mut next = Some(off.max(0) as usize);
        let mut seen = HashSet::new();
        let mut first = true;
        while let Some(mut off) = next.take() {
            if !seen.insert(off) || seen.len() > 512 {
                break;
            }
            // Files with junk before `%PDF-` count offsets from the header.
            if !self.looks_like_xref(off) && self.looks_like_xref(off + header) {
                off += header;
            }
            let trailer = if data.get(off..off + 4) == Some(b"xref") {
                self.read_table(off)?
            } else {
                self.read_stream_section(off)?
            };
            if first {
                self.trailer = trailer.clone();
                first = false;
            } else {
                for (k, v) in trailer.entries.iter() {
                    if !self.trailer.contains(k.as_str()) && k.as_str() != "Prev" && k.as_str() != "XRefStm" {
                        self.trailer.entries.push((k.clone(), v.clone()));
                    }
                }
            }
            // A hybrid file's table names an extra stream section.
            if let Some(stm) = trailer.get("XRefStm").and_then(Object::as_i64) {
                let _ = self.read_stream_section(stm.max(0) as usize);
            }
            next = trailer.get("Prev").and_then(Object::as_i64).map(|p| p.max(0) as usize);
        }
        if self.xref.is_empty() {
            return Err(PdfError::invalid("empty cross-reference data"));
        }
        Ok(())
    }

    fn looks_like_xref(&self, off: usize) -> bool {
        let d = &self.data;
        if d.get(off..off + 4) == Some(b"xref") {
            return true;
        }
        let mut l = Lexer::at(d, off);
        matches!(
            (l.next_token(), l.next_token(), l.next_token()),
            (Some(Token::Int(_)), Some(Token::Int(_)), Some(Token::Keyword(b"obj")))
        )
    }

    /// A classic `xref` table and its trailer; earlier sections (later in
    /// the `Prev` chain) never override entries already read.
    fn read_table(&mut self, off: usize) -> Result<Dict> {
        let data = self.data.clone();
        let mut l = Lexer::at(&data, off + 4);
        loop {
            match l.next_token() {
                Some(Token::Int(start)) => {
                    let Some(Token::Int(count)) = l.next_token() else {
                        return Err(PdfError::invalid("bad xref subsection"));
                    };
                    if start < 0 || !(0..=10_000_000).contains(&count) {
                        return Err(PdfError::invalid("bad xref subsection"));
                    }
                    for i in 0..count {
                        let (Some(Token::Int(o)), Some(Token::Int(g)), Some(Token::Keyword(t))) =
                            (l.next_token(), l.next_token(), l.next_token())
                        else {
                            return Err(PdfError::invalid("bad xref entry"));
                        };
                        let num = (start + i) as u32;
                        if t == b"n" && o > 0 {
                            self.xref.entry(num).or_insert(Entry::Offset(o as usize, g.clamp(0, 65535) as u16));
                        }
                        // Free entries are skipped: a hybrid file marks
                        // its compressed objects free in the table.
                    }
                }
                Some(Token::Keyword(b"trailer")) => {
                    let Some(Object::Dict(d)) = l.object() else { return Err(PdfError::invalid("bad trailer")) };
                    return Ok((*d).clone());
                }
                _ => return Err(PdfError::invalid("bad xref table")),
            }
        }
    }

    /// A cross-reference stream (§7.5.8); its dictionary is the trailer.
    fn read_stream_section(&mut self, off: usize) -> Result<Dict> {
        let (_, obj) = self.parse_at(off, None)?;
        let Object::Stream(s) = obj else { return Err(PdfError::invalid("xref offset is not a stream")) };
        if s.dict.name("Type") != Some("XRef") {
            return Err(PdfError::invalid("not an xref stream"));
        }
        let data = self.decode_raw(&s)?.data;
        let w: Vec<usize> = s
            .dict
            .get("W")
            .and_then(Object::as_array)
            .map(|a| a.iter().map(|x| x.as_i64().unwrap_or(0).clamp(0, 8) as usize).collect())
            .unwrap_or_default();
        if w.len() < 3 {
            return Err(PdfError::invalid("xref stream without W"));
        }
        let size = s.dict.get("Size").and_then(Object::as_i64).unwrap_or(0).max(0);
        let index: Vec<i64> = s
            .dict
            .get("Index")
            .and_then(Object::as_array)
            .map(|a| a.iter().filter_map(Object::as_i64).collect())
            .unwrap_or_else(|| vec![0, size]);
        let row = w[0] + w[1] + w[2];
        if row == 0 {
            return Err(PdfError::invalid("xref stream with empty rows"));
        }
        let field = |b: &[u8]| b.iter().fold(0u64, |acc, &x| acc << 8 | u64::from(x));
        let mut pos = 0usize;
        for pair in index.chunks(2) {
            let [start, count] = [pair[0], *pair.get(1).unwrap_or(&0)];
            if start < 0 || !(0..=10_000_000).contains(&count) {
                break;
            }
            for i in 0..count {
                let Some(r) = data.get(pos..pos + row) else { break };
                pos += row;
                let t = if w[0] == 0 { 1 } else { field(&r[..w[0]]) };
                let a = field(&r[w[0]..w[0] + w[1]]);
                let b = field(&r[w[0] + w[1]..]);
                let num = (start + i) as u32;
                let e = match t {
                    1 => Entry::Offset(a as usize, b.min(65535) as u16),
                    2 => Entry::InStream(a as u32, b as u32),
                    _ => continue,
                };
                self.xref.entry(num).or_insert(e);
            }
        }
        Ok(s.dict.clone())
    }

    /// Rebuilds the cross-reference data by scanning for `n g obj`.
    fn rebuild(&mut self) -> Result<()> {
        self.repaired = true;
        self.xref.clear();
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).clear();
        let data = self.data.clone();
        let mut pos = 0;
        let mut trailers: Vec<Dict> = Vec::new();
        while let Some(at) = lexer::find(&data, b"obj", pos) {
            pos = at + 3;
            // `obj` must be a whole word preceded by `num generation`.
            if data.get(at + 3).is_some_and(|&c| !lexer::is_white(c) && !lexer::is_delim(c)) {
                continue;
            }
            let Some((num, generation, start)) = back_two_ints(&data, at) else { continue };
            self.xref.insert(num, Entry::Offset(start, generation));
        }
        let mut pos = 0;
        while let Some(at) = lexer::find(&data, b"trailer", pos) {
            pos = at + 7;
            let mut l = Lexer::at(&data, at + 7);
            if let Some(Object::Dict(d)) = l.object() {
                trailers.push((*d).clone());
            }
        }
        // Object streams and xref streams hold objects and trailers too.
        let nums: Vec<u32> = self.xref.keys().copied().collect();
        let mut catalog = None;
        for num in nums {
            let Ok(Object::Stream(s)) = self.load(num) else {
                if catalog.is_none()
                    && let Ok(Object::Dict(d)) = self.load(num)
                    && d.name("Type") == Some("Catalog")
                {
                    catalog = Some(num);
                }
                continue;
            };
            match s.dict.name("Type") {
                Some("XRef") => trailers.push(s.dict.clone()),
                Some("ObjStm") => {
                    if let Ok(objs) = self.objstm_numbers(&s) {
                        for (i, n) in objs.into_iter().enumerate() {
                            self.xref.entry(n).or_insert(Entry::InStream(num, i as u32));
                        }
                    }
                }
                _ => {}
            }
        }
        // The last trailer with a usable /Root wins.
        let mut trailer = Dict::new();
        for t in trailers.into_iter().rev() {
            if t.get("Root").and_then(Object::as_ref).is_some_and(|r| self.xref.contains_key(&r.num)) {
                trailer = t;
                break;
            }
        }
        if !trailer.contains("Root") {
            let num = match catalog {
                Some(n) => Some(n),
                None => self.find_catalog(),
            };
            let Some(n) = num else { return Err(PdfError::invalid("no document catalog")) };
            trailer.insert("Root", Object::Ref(ObjRef { num: n, generation: 0 }));
        }
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.trailer = trailer;
        Ok(())
    }

    fn find_catalog(&self) -> Option<u32> {
        let mut nums: Vec<u32> = self.xref.keys().copied().collect();
        nums.sort_unstable();
        nums.into_iter().find(|&n| matches!(self.load(n), Ok(Object::Dict(d)) if d.name("Type") == Some("Catalog")))
    }

    fn setup_crypt(&mut self) -> Result<()> {
        self.crypt = None;
        let Some(enc) = self.trailer.get("Encrypt").cloned() else { return Ok(()) };
        let enc_ref = enc.as_ref();
        let enc = self.resolve(&enc);
        let Some(d) = enc.as_dict() else { return Ok(()) };
        let id0 = self
            .trailer
            .get("ID")
            .map(|o| self.resolve(o))
            .and_then(|o| o.as_array().and_then(|a| a.first().cloned()))
            .and_then(|o| o.as_bytes().map(<[u8]>::to_vec))
            .unwrap_or_default();
        let crypt = Decryptor::open(d, &id0, enc_ref)?;
        self.crypt = Some(crypt);
        // Objects read before the key was known were not decrypted.
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.objstms.lock().unwrap_or_else(|e| e.into_inner()).clear();
        Ok(())
    }

    // ---- objects --------------------------------------------------------

    /// Parses the indirect object at `off`; `expect` checks its number.
    fn parse_at(&self, off: usize, expect: Option<u32>) -> Result<(ObjRef, Object)> {
        let data: &[u8] = &self.data;
        let mut l = Lexer::at(data, off);
        let (Some(Token::Int(n)), Some(Token::Int(g)), Some(Token::Keyword(b"obj"))) =
            (l.next_token(), l.next_token(), l.next_token())
        else {
            return Err(PdfError::invalid(format!("no object at {off}")));
        };
        if n < 0 || n > i64::from(u32::MAX) || expect.is_some_and(|e| e as i64 != n) {
            return Err(PdfError::invalid(format!("object {n} where {expect:?} was expected")));
        }
        let id = ObjRef { num: n as u32, generation: g.clamp(0, 65535) as u16 };
        let obj = l.object().unwrap_or(Object::Null);
        let save = l.pos;
        if let Object::Dict(d) = &obj
            && let Some(Token::Keyword(b"stream")) = l.next_token()
        {
            let mut start = l.pos;
            // `stream` is followed by CRLF or LF (a lone CR is tolerated).
            if data.get(start) == Some(&b'\r') {
                start += 1;
            }
            if data.get(start) == Some(&b'\n') {
                start += 1;
            }
            let declared = d.get("Length").map(|o| self.resolve(o)).and_then(|o| o.as_i64()).unwrap_or(-1);
            let len = self.stream_len(start, declared);
            let s = Stream { dict: (**d).clone(), start, len, id: Some(id) };
            return Ok((id, Object::Stream(Arc::new(s))));
        }
        let _ = save;
        Ok((id, obj))
    }

    /// A stream's length: the declared one when `endstream` follows it,
    /// else up to the next `endstream`.
    fn stream_len(&self, start: usize, declared: i64) -> usize {
        let data: &[u8] = &self.data;
        if declared >= 0 {
            let end = start.saturating_add(declared as usize);
            if end <= data.len() {
                let mut l = Lexer::at(data, end);
                l.skip_ws();
                if data.get(l.pos..l.pos + 9) == Some(b"endstream") {
                    return declared as usize;
                }
            }
        }
        match lexer::find(data, b"endstream", start) {
            Some(mut e) => {
                // The end of line before `endstream` is not data.
                if e > start && data[e - 1] == b'\n' {
                    e -= 1;
                }
                if e > start && data[e - 1] == b'\r' {
                    e -= 1;
                }
                e - start
            }
            None => data.len().saturating_sub(start),
        }
    }

    /// Loads object `num` (uncached).
    fn load(&self, num: u32) -> Result<Object> {
        match self.xref.get(&num).copied() {
            None | Some(Entry::Offset(0, _)) => Ok(Object::Null),
            Some(Entry::Offset(off, _)) => {
                let (id, obj) = match self.parse_at(off, Some(num)) {
                    Ok(x) => x,
                    // Offsets a few bytes off: look for the header nearby.
                    Err(_) => self.parse_near(off, num)?,
                };
                Ok(match &self.crypt {
                    Some(c) => c.object(id, obj),
                    None => obj,
                })
            }
            Some(Entry::InStream(stm, idx)) => {
                let objs = self.objstm(stm)?;
                Ok(objs.get(idx as usize).cloned().unwrap_or(Object::Null))
            }
        }
    }

    fn parse_near(&self, off: usize, num: u32) -> Result<(ObjRef, Object)> {
        let data: &[u8] = &self.data;
        let from = off.saturating_sub(64);
        let to = (off + 64).min(data.len());
        let needle = format!("{num} ");
        let mut pos = from;
        while let Some(at) = lexer::find(&data[..to], needle.as_bytes(), pos) {
            pos = at + 1;
            if (at == 0 || !data[at - 1].is_ascii_digit())
                && let Ok(x) = self.parse_at(at, Some(num))
            {
                return Ok(x);
            }
        }
        Err(PdfError::invalid(format!("object {num} not found")))
    }

    fn objstm_numbers(&self, s: &Stream) -> Result<Vec<u32>> {
        let data = self.decode_raw(s)?.data;
        let n = s.dict.get("N").and_then(Object::as_i64).unwrap_or(0).clamp(0, 1_000_000);
        let mut l = Lexer::new(&data);
        let mut out = Vec::new();
        for _ in 0..n {
            let (Some(Token::Int(num)), Some(Token::Int(_))) = (l.next_token(), l.next_token()) else { break };
            out.push(num.max(0) as u32);
        }
        Ok(out)
    }

    fn objstm(&self, num: u32) -> Result<Arc<Vec<Object>>> {
        if let Some(v) = self.objstms.lock().unwrap_or_else(|e| e.into_inner()).get(&num) {
            return Ok(v.clone());
        }
        let Object::Stream(s) = self.get(num)? else { return Err(PdfError::invalid("object stream missing")) };
        let data = self.decode_stream(&s)?.data;
        let n = s.dict.get("N").and_then(Object::as_i64).unwrap_or(0).clamp(0, 1_000_000) as usize;
        let first = s.dict.get("First").and_then(Object::as_i64).unwrap_or(0).max(0) as usize;
        let mut l = Lexer::new(&data);
        let mut offs = Vec::with_capacity(n);
        for _ in 0..n {
            let (Some(Token::Int(_)), Some(Token::Int(o))) = (l.next_token(), l.next_token()) else { break };
            offs.push(o.max(0) as usize);
        }
        let objs: Vec<Object> = offs
            .iter()
            .map(|&o| {
                let mut l = Lexer::at(&data, first.saturating_add(o));
                l.object().unwrap_or(Object::Null)
            })
            .collect();
        let objs = Arc::new(objs);
        self.objstms.lock().unwrap_or_else(|e| e.into_inner()).insert(num, objs.clone());
        Ok(objs)
    }

    /// Object `num`, cached.
    pub fn get(&self, num: u32) -> Result<Object> {
        if let Some(o) = self.cache.lock().unwrap_or_else(|e| e.into_inner()).get(&num) {
            return Ok(o.clone());
        }
        let o = self.load(num)?;
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).insert(num, o.clone());
        Ok(o)
    }

    /// Follows references; a dangling one reads as null (§7.3.10).
    pub fn resolve(&self, o: &Object) -> Object {
        let mut cur = o.clone();
        for _ in 0..MAX_RESOLVE {
            match cur {
                Object::Ref(r) => cur = self.get(r.num).unwrap_or(Object::Null),
                other => return other,
            }
        }
        Object::Null
    }

    /// `d[key]`, resolved.
    pub fn get_in(&self, d: &Dict, key: &str) -> Object {
        d.get(key).map(|o| self.resolve(o)).unwrap_or(Object::Null)
    }

    pub fn dict_in(&self, d: &Dict, key: &str) -> Option<Arc<Dict>> {
        match self.get_in(d, key) {
            Object::Dict(d) => Some(d),
            Object::Stream(s) => Some(Arc::new(s.dict.clone())),
            _ => None,
        }
    }

    pub fn f64_in(&self, d: &Dict, key: &str) -> Option<f64> {
        self.get_in(d, key).as_f64()
    }

    pub fn array_in(&self, d: &Dict, key: &str) -> Vec<Object> {
        match self.get_in(d, key) {
            Object::Array(a) => a.iter().map(|o| self.resolve(o)).collect(),
            _ => Vec::new(),
        }
    }

    /// A number array (`/MediaBox`, `/Matrix`, `/Decode`).
    pub fn numbers_in(&self, d: &Dict, key: &str) -> Vec<f64> {
        self.array_in(d, key).iter().filter_map(Object::as_f64).collect()
    }

    // ---- streams --------------------------------------------------------

    fn raw_bytes(&self, s: &Stream) -> Vec<u8> {
        let end = s.start.saturating_add(s.len).min(self.data.len());
        let raw = &self.data[s.start.min(end)..end];
        match (&self.crypt, s.id) {
            (Some(c), Some(id)) if self.stream_encrypted(s) && Some(id) != c.encrypt_ref => c.stream(id, raw),
            _ => raw.to_vec(),
        }
    }

    fn stream_encrypted(&self, s: &Stream) -> bool {
        let Some(c) = &self.crypt else { return false };
        match s.dict.name("Type") {
            Some("XRef") => return false,
            Some("Metadata") if !c.encrypt_metadata => return false,
            _ => {}
        }
        // A `/Crypt` filter naming `Identity` opts the stream out.
        let filters = self.filters(&s.dict);
        !filters.iter().any(|f| {
            f.name == "Crypt" && f.params.as_ref().and_then(|p| p.name("Name")).is_none_or(|n| n == "Identity")
        })
    }

    /// The filter chain of a stream dictionary (`/F`/`/DP` in inline images).
    pub fn filters(&self, d: &Dict) -> Vec<Filter> {
        let f = d.get_any(&["Filter", "F"]).map(|o| self.resolve(o)).unwrap_or(Object::Null);
        let p = d.get_any(&["DecodeParms", "DP"]).map(|o| self.resolve(o)).unwrap_or(Object::Null);
        let names: Vec<String> = match &f {
            Object::Name(n) => vec![n.as_str().to_string()],
            Object::Array(a) => a.iter().filter_map(|o| self.resolve(o).as_name().map(str::to_string)).collect(),
            _ => Vec::new(),
        };
        let params: Vec<Option<Dict>> = match &p {
            Object::Dict(d) => vec![Some((**d).clone())],
            Object::Array(a) => a.iter().map(|o| self.resolve(o).as_dict().cloned()).collect(),
            _ => Vec::new(),
        };
        names
            .into_iter()
            .enumerate()
            .map(|(i, name)| Filter { name, params: params.get(i).cloned().flatten() })
            .collect()
    }

    /// Stream bytes without the image filters' decoding: what an xref
    /// stream or object stream needs, before the key may be known.
    fn decode_raw(&self, s: &Stream) -> Result<Decoded> {
        let raw = self.raw_bytes(s);
        filter::decode(&raw, &self.filters(&s.dict), None)
    }

    /// The decoded bytes of a stream, every filter applied.
    pub fn decode_stream(&self, s: &Stream) -> Result<Decoded> {
        let raw = self.raw_bytes(s);
        let filters = self.filters(&s.dict);
        let globals = if filters.iter().any(|f| f.name == "JBIG2Decode") {
            filters
                .iter()
                .find_map(|f| f.params.as_ref().and_then(|p| p.get("JBIG2Globals")).map(|o| self.resolve(o)))
                .and_then(|o| o.as_stream().cloned())
                .and_then(|g| self.decode_stream(&g).ok())
                .map(|d| d.data)
        } else {
            None
        };
        filter::decode(&raw, &filters, globals.as_deref())
    }

    /// The bytes as stored, decrypted but not decoded (a JPEG to pass on).
    pub fn stream_bytes_encoded(&self, s: &Stream) -> Vec<u8> {
        self.raw_bytes(s)
    }

    /// A page's content: one stream or the concatenation of an array.
    pub fn page_content(&self, page: &Page) -> Vec<u8> {
        let mut out = Vec::new();
        let streams: Vec<Object> = match self.get_in(&page.dict, "Contents") {
            Object::Array(a) => a.iter().map(|o| self.resolve(o)).collect(),
            o @ Object::Stream(_) => vec![o],
            _ => Vec::new(),
        };
        for s in streams {
            if let Object::Stream(s) = s
                && let Ok(d) = self.decode_stream(&s)
            {
                out.extend_from_slice(&d.data);
                // Streams may split a token: a separator keeps them apart.
                out.push(b'\n');
            }
        }
        out
    }

    // ---- page tree -------------------------------------------------------

    fn load_pages(&mut self) {
        self.pages.clear();
        self.page_ids.clear();
        let root = self.resolve(self.trailer.get("Root").unwrap_or(&Object::Null));
        let Some(cat) = root.as_dict() else { return };
        let top = cat.get("Pages").cloned().unwrap_or(Object::Null);
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        self.walk_pages(&top, Inherited::default(), &mut seen, &mut out, 0);
        if out.is_empty() && self.repaired {
            // A broken tree: every page object, in file order.
            let mut nums: Vec<(usize, u32)> = self
                .xref
                .iter()
                .filter_map(|(&n, e)| match e {
                    Entry::Offset(o, _) => Some((*o, n)),
                    Entry::InStream(..) => None,
                })
                .collect();
            nums.sort_unstable();
            for (_, n) in nums {
                if let Ok(Object::Dict(d)) = self.get(n)
                    && d.name("Type") == Some("Page")
                {
                    let r = Object::Ref(ObjRef { num: n, generation: 0 });
                    self.walk_pages(&r, Inherited::default(), &mut seen, &mut out, 0);
                }
            }
        }
        for (i, p) in out.iter().enumerate() {
            if let Some(id) = p.id {
                self.page_ids.entry(id).or_insert(i);
            }
        }
        self.pages = out;
    }

    fn walk_pages(&self, node: &Object, inh: Inherited, seen: &mut HashSet<ObjRef>, out: &mut Vec<Page>, depth: usize) {
        if depth > MAX_RESOLVE || out.len() >= MAX_PAGES {
            return;
        }
        let id = node.as_ref();
        if let Some(r) = id
            && !seen.insert(r)
        {
            return;
        }
        let obj = self.resolve(node);
        let Some(d) = obj.as_dict() else { return };
        let mut inh = inh;
        if let Some(r) = self.dict_in(d, "Resources") {
            inh.resources = Some(r);
        }
        if let Some(b) = rect(&self.numbers_in(d, "MediaBox")) {
            inh.media_box = Some(b);
        }
        if let Some(b) = rect(&self.numbers_in(d, "CropBox")) {
            inh.crop_box = Some(b);
        }
        if let Some(r) = self.get_in(d, "Rotate").as_i64() {
            inh.rotate = Some(r);
        }
        let kids = self.get_in(d, "Kids");
        let is_page = d.name("Type") == Some("Page") || (d.name("Type").is_none() && !matches!(kids, Object::Array(_)));
        if is_page && !matches!(kids, Object::Array(_)) {
            // US Letter when no box is given anywhere.
            let media = inh.media_box.unwrap_or([0.0, 0.0, 612.0, 792.0]);
            let crop = inh.crop_box.map(|c| intersect(c, media)).filter(|c| c[2] - c[0] >= 1.0 && c[3] - c[1] >= 1.0);
            let rotate = (inh.rotate.unwrap_or(0).rem_euclid(360) / 90 * 90) as i32;
            out.push(Page {
                dict: Arc::new(d.clone()),
                id,
                resources: inh.resources.clone(),
                media_box: media,
                crop_box: crop.unwrap_or(media),
                rotate,
            });
            return;
        }
        if let Object::Array(k) = kids {
            for kid in k.iter() {
                self.walk_pages(kid, inh.clone(), seen, out, depth + 1);
            }
        }
    }

    // ---- information ------------------------------------------------------

    pub fn catalog(&self) -> Option<Arc<Dict>> {
        match self.resolve(self.trailer.get("Root")?) {
            Object::Dict(d) => Some(d),
            _ => None,
        }
    }

    pub fn info(&self) -> Info {
        let mut out = Info::default();
        if let Some(Object::Dict(d)) = self.trailer.get("Info").map(|o| self.resolve(o)) {
            let text = |k: &str| self.get_in(&d, k).as_bytes().map(text_string).unwrap_or_default();
            out.title = text("Title");
            out.author = text("Author");
            out.subject = text("Subject");
            out.keywords = text("Keywords");
            out.creator = text("Creator");
            out.producer = text("Producer");
            out.year = year_of(&text("CreationDate"));
        }
        if let Some(cat) = self.catalog() {
            out.language = self.get_in(&cat, "Lang").as_bytes().map(text_string).unwrap_or_default();
            // XMP fills what the info dictionary leaves empty.
            if (out.title.is_empty() || out.author.is_empty())
                && let Object::Stream(s) = self.get_in(&cat, "Metadata")
                && let Ok(d) = self.decode_stream(&s)
            {
                let xmp = String::from_utf8_lossy(&d.data);
                if out.title.is_empty() {
                    out.title = xmp_first(&xmp, "dc:title").unwrap_or_default();
                }
                if out.author.is_empty() {
                    out.author = xmp_first(&xmp, "dc:creator").unwrap_or_default();
                }
            }
        }
        for s in [&mut out.title, &mut out.author, &mut out.subject, &mut out.keywords, &mut out.language] {
            *s = clean_text(s, 2000);
        }
        out
    }

    /// The outline (bookmarks), nested, each entry with its page.
    pub fn outline(&self) -> Vec<OutlineItem> {
        let Some(cat) = self.catalog() else { return Vec::new() };
        let Some(root) = self.dict_in(&cat, "Outlines") else { return Vec::new() };
        let mut seen = HashSet::new();
        let mut budget = MAX_OUTLINE;
        self.outline_level(root.get("First"), &cat, &mut seen, &mut budget, 0)
    }

    fn outline_level(
        &self,
        first: Option<&Object>,
        cat: &Dict,
        seen: &mut HashSet<ObjRef>,
        budget: &mut usize,
        depth: usize,
    ) -> Vec<OutlineItem> {
        let mut out = Vec::new();
        let mut cur = first.cloned();
        while let Some(node) = cur.take() {
            if depth > 16 || *budget == 0 {
                break;
            }
            if let Some(r) = node.as_ref()
                && !seen.insert(r)
            {
                break;
            }
            *budget -= 1;
            let obj = self.resolve(&node);
            let Some(d) = obj.as_dict() else { break };
            let title = clean_text(&self.get_in(d, "Title").as_bytes().map(text_string).unwrap_or_default(), 500);
            let mut dest = self.get_in(d, "Dest");
            if dest.is_null()
                && let Some(a) = self.dict_in(d, "A")
                && a.name("S") == Some("GoTo")
            {
                dest = self.get_in(&a, "D");
            }
            let page = self.dest_page(&dest, cat, 0);
            let children = self.outline_level(d.get("First"), cat, seen, budget, depth + 1);
            if !title.is_empty() || !children.is_empty() {
                out.push(OutlineItem { title, page, children });
            }
            cur = d.get("Next").cloned();
        }
        out
    }

    /// The page a destination points at (§12.3.2).
    pub fn dest_page(&self, dest: &Object, cat: &Dict, depth: usize) -> Option<usize> {
        if depth > 4 {
            return None;
        }
        match dest {
            Object::Array(a) => match a.first()? {
                Object::Ref(r) => self.page_index(*r),
                // Remote-style destinations number pages from 0.
                Object::Int(i) => usize::try_from(*i).ok().filter(|&i| i < self.pages.len()),
                _ => None,
            },
            Object::Dict(d) => self.dest_page(&self.get_in(d, "D"), cat, depth + 1),
            Object::Name(n) => {
                let named = self.dict_in(cat, "Dests").map(|d| self.get_in(&d, n.as_str()))?;
                self.dest_page(&named, cat, depth + 1)
            }
            Object::String(s) => {
                let names = self.dict_in(cat, "Names")?;
                let tree = self.dict_in(&names, "Dests")?;
                let found = self.name_tree_lookup(&tree, s, 0)?;
                self.dest_page(&found, cat, depth + 1)
            }
            _ => None,
        }
    }

    fn name_tree_lookup(&self, node: &Dict, key: &[u8], depth: usize) -> Option<Object> {
        if depth > 32 {
            return None;
        }
        let names = self.array_in(node, "Names");
        for pair in names.chunks(2) {
            if pair.first().and_then(Object::as_bytes) == Some(key) {
                return pair.get(1).map(|o| self.resolve(o));
            }
        }
        for kid in self.array_in(node, "Kids") {
            let Some(k) = kid.as_dict() else { continue };
            let limits = self.array_in(k, "Limits");
            if let (Some(lo), Some(hi)) =
                (limits.first().and_then(Object::as_bytes), limits.get(1).and_then(Object::as_bytes))
                && (key < lo || key > hi)
            {
                continue;
            }
            if let Some(v) = self.name_tree_lookup(k, key, depth + 1) {
                return Some(v);
            }
        }
        None
    }
}

#[derive(Clone, Default)]
struct Inherited {
    resources: Option<Arc<Dict>>,
    media_box: Option<[f64; 4]>,
    crop_box: Option<[f64; 4]>,
    rotate: Option<i64>,
}

/// The numbers `num generation` that end right before `at` (the `obj` keyword),
/// and where they start.
fn back_two_ints(data: &[u8], at: usize) -> Option<(u32, u16, usize)> {
    let mut i = at;
    let skip_ws = |i: &mut usize| {
        while *i > 0 && lexer::is_white(data[*i - 1]) {
            *i -= 1;
        }
    };
    let digits = |i: &mut usize| -> Option<(u64, usize)> {
        let end = *i;
        while *i > 0 && data[*i - 1].is_ascii_digit() && end - *i < 10 {
            *i -= 1;
        }
        if *i == end {
            return None;
        }
        let v = std::str::from_utf8(&data[*i..end]).ok()?.parse::<u64>().ok()?;
        Some((v, *i))
    };
    skip_ws(&mut i);
    let (generation, _) = digits(&mut i)?;
    let before = i;
    skip_ws(&mut i);
    if i == before {
        return None;
    }
    let (num, start) = digits(&mut i)?;
    if i > 0 && (data[i - 1].is_ascii_digit() || !(lexer::is_white(data[i - 1]) || lexer::is_delim(data[i - 1]))) {
        return None;
    }
    if num > u64::from(u32::MAX) || generation > 65535 {
        return None;
    }
    Some((num as u32, generation as u16, start))
}

fn rect(v: &[f64]) -> Option<[f64; 4]> {
    if v.len() < 4 {
        return None;
    }
    let r = [v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])];
    (r[2] - r[0] > 0.0 && r[3] - r[1] > 0.0 && r.iter().all(|x| x.is_finite() && x.abs() < 1e7)).then_some(r)
}

fn intersect(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].min(b[3])]
}

/// A text string (§7.9.2.2): UTF-16BE with a BOM, UTF-8 with a BOM
/// (PDF 2.0), else PDFDocEncoding.
pub fn text_string(b: &[u8]) -> String {
    if b.len() >= 2 && b[0] == 0xfe && b[1] == 0xff {
        let units: Vec<u16> = b[2..].as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        return String::from_utf16_lossy(&units);
    }
    if b.len() >= 2 && b[0] == 0xff && b[1] == 0xfe {
        let units: Vec<u16> = b[2..].as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        return String::from_utf16_lossy(&units);
    }
    if b.starts_with(&[0xef, 0xbb, 0xbf]) {
        return String::from_utf8_lossy(&b[3..]).into_owned();
    }
    b.iter().filter_map(|&c| pdf_doc_char(c)).collect()
}

/// PDFDocEncoding (Annex D.2): Latin-1 with its own 0x18–0x1F and
/// 0x80–0x9F rows.
pub fn pdf_doc_char(c: u8) -> Option<char> {
    const LOW: [char; 8] =
        ['\u{02D8}', '\u{02C7}', '\u{02C6}', '\u{02D9}', '\u{02DD}', '\u{02DB}', '\u{02DA}', '\u{02DC}'];
    const HIGH: [char; 32] = [
        '\u{2022}', '\u{2020}', '\u{2021}', '\u{2026}', '\u{2014}', '\u{2013}', '\u{0192}', '\u{2044}', '\u{2039}',
        '\u{203A}', '\u{2212}', '\u{2030}', '\u{201E}', '\u{201C}', '\u{201D}', '\u{2018}', '\u{2019}', '\u{201A}',
        '\u{2122}', '\u{FB01}', '\u{FB02}', '\u{0141}', '\u{0152}', '\u{0160}', '\u{0178}', '\u{017D}', '\u{0131}',
        '\u{0142}', '\u{0153}', '\u{0161}', '\u{017E}', '\u{FFFD}',
    ];
    match c {
        0x18..=0x1f => Some(LOW[(c - 0x18) as usize]),
        0x80..=0x9f => Some(HIGH[(c - 0x80) as usize]),
        0xa0 => Some('\u{20AC}'),
        0xad => None,
        0..=0x08 | 0x0b | 0x0e..=0x17 | 0x7f => None,
        _ => Some(c as char),
    }
}

fn clean_text(s: &str, max: usize) -> String {
    let t: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    t.chars().take(max).collect()
}

fn year_of(date: &str) -> i64 {
    let d = date.strip_prefix("D:").unwrap_or(date);
    let y: String = d.chars().take(4).collect();
    match y.parse::<i64>() {
        Ok(v) if (1000..=9999).contains(&v) => v,
        _ => 0,
    }
}

/// The first `<rdf:li>` text inside `<tag>` of an XMP packet.
fn xmp_first(xmp: &str, tag: &str) -> Option<String> {
    let start = xmp.find(&format!("<{tag}"))?;
    let end = xmp[start..].find(&format!("</{tag}>")).map(|e| start + e)?;
    let inner = &xmp[start..end];
    let li = inner.find("<rdf:li")?;
    let open = inner[li..].find('>')? + li + 1;
    let close = inner[open..].find("</rdf:li>")? + open;
    let raw = &inner[open..close];
    let text = raw
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'");
    let t = text.trim().to_string();
    (!t.is_empty()).then_some(t)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Builds a small PDF: `objects` are the bodies of objects 1..n
    /// (object 1 must be the catalog), with a correct xref table.
    pub fn build(objects: &[&[u8]]) -> Vec<u8> {
        let mut out = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
        for o in &offsets {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!("trailer\n<< /Size {} /Root 1 0 R /Info 3 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1)
                .as_bytes(),
        );
        out
    }

    pub fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
        let mut v = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
        v.extend_from_slice(data);
        v.extend_from_slice(b"\nendstream");
        v
    }

    pub fn simple(content: &[u8]) -> Vec<u8> {
        let s = stream("", content);
        build(&[
            b"<< /Type /Catalog /Pages 2 0 R >>",
            b"<< /Type /Pages /Kids [4 0 R] /Count 1 /MediaBox [0 0 200 100] >>",
            b"<< /Title (Test \\(book\\)) /Author <FEFF00C9006D0069006C0065> /CreationDate (D:20210304) >>",
            b"<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Rotate 90 >>",
            &s,
        ])
    }

    #[test]
    fn opens_a_simple_file() {
        let pdf = simple(b"0 0 m 10 10 l S");
        let d = Document::open(pdf).unwrap();
        assert!(!d.repaired);
        assert_eq!(d.page_count(), 1);
        let p = d.page(0).unwrap();
        assert_eq!(p.media_box, [0.0, 0.0, 200.0, 100.0]);
        assert_eq!((p.rotate, p.size()), (90, (100.0, 200.0)));
        assert_eq!(d.page_content(p), b"0 0 m 10 10 l S\n");
        let i = d.info();
        assert_eq!((i.title.as_str(), i.author.as_str(), i.year), ("Test (book)", "Émile", 2021));
        assert!(d.page(1).is_err());
    }

    #[test]
    fn repairs_broken_offsets() {
        let mut pdf = simple(b"BT ET");
        // Point startxref at garbage.
        let at = lexer::rfind(&pdf, b"startxref", pdf.len()).unwrap();
        pdf.truncate(at);
        pdf.extend_from_slice(b"startxref\n9\n%%EOF\n");
        let d = Document::open(pdf.clone()).unwrap();
        assert!(d.repaired);
        assert_eq!(d.page_count(), 1);
        // No xref or trailer at all.
        let cut = lexer::find(&pdf, b"xref\n", 0).unwrap();
        let d = Document::open(pdf[..cut].to_vec()).unwrap();
        assert_eq!(d.page_count(), 1);
    }

    #[test]
    fn xref_streams_and_object_streams() {
        // Objects 1–3 live in an object stream (object 5), the page
        // content is object 4, the xref stream is object 6.
        let objs = "<< /Type /Catalog /Pages 2 0 R >> << /Type /Pages /Kids [3 0 R] /Count 1 >> << /Type /Page /Parent 2 0 R /MediaBox [0 0 50 50] /Contents 4 0 R >>";
        let o1 = 0;
        let o2 = objs.find("<< /Type /Pages").unwrap();
        let o3 = objs.find("<< /Type /Page ").unwrap();
        let header = format!("1 {o1} 2 {o2} 3 {o3} ");
        let body = format!("{header}{objs}");
        let packed = miniz_oxide::deflate::compress_to_vec_zlib(body.as_bytes(), 6);
        let mut out = b"%PDF-1.5\n".to_vec();
        let off4 = out.len();
        out.extend_from_slice(&[b"4 0 obj\n".as_slice(), &stream("", b"0 0 1 1 re f"), b"\nendobj\n"].concat());
        let off5 = out.len();
        let mut s5 = format!(
            "5 0 obj\n<< /Type /ObjStm /N 3 /First {} /Filter /FlateDecode /Length {} >>\nstream\n",
            header.len(),
            packed.len()
        )
        .into_bytes();
        s5.extend_from_slice(&packed);
        s5.extend_from_slice(b"\nendstream\nendobj\n");
        out.extend_from_slice(&s5);
        let off6 = out.len();
        let mut rows = Vec::new();
        let mut row = |t: u8, a: u32, b: u16| {
            rows.push(t);
            rows.extend_from_slice(&a.to_be_bytes());
            rows.extend_from_slice(&b.to_be_bytes());
        };
        row(0, 0, 65535);
        row(2, 5, 0);
        row(2, 5, 1);
        row(2, 5, 2);
        row(1, off4 as u32, 0);
        row(1, off5 as u32, 0);
        row(1, off6 as u32, 0);
        let mut s6 =
            format!("6 0 obj\n<< /Type /XRef /Size 7 /W [1 4 2] /Root 1 0 R /Length {} >>\nstream\n", rows.len())
                .into_bytes();
        s6.extend_from_slice(&rows);
        s6.extend_from_slice(b"\nendstream\nendobj\n");
        out.extend_from_slice(&s6);
        out.extend_from_slice(format!("startxref\n{off6}\n%%EOF\n").as_bytes());
        let d = Document::open(out).unwrap();
        assert!(!d.repaired);
        assert_eq!(d.page_count(), 1);
        assert_eq!(d.page(0).unwrap().media_box, [0.0, 0.0, 50.0, 50.0]);
        assert_eq!(d.page_content(d.page(0).unwrap()), b"0 0 1 1 re f\n");
    }

    #[test]
    fn outline_and_named_destinations() {
        let pdf = build(&[
            b"<< /Type /Catalog /Pages 2 0 R /Outlines 6 0 R /Dests << /intro [5 0 R /Fit] >> >>",
            b"<< /Type /Pages /Kids [4 0 R 5 0 R] /Count 2 >>",
            b"<< >>",
            b"<< /Type /Page /Parent 2 0 R >>",
            b"<< /Type /Page /Parent 2 0 R >>",
            b"<< /First 7 0 R >>",
            b"<< /Title (One) /Dest [4 0 R /XYZ 0 0 0] /Next 8 0 R /First 9 0 R >>",
            b"<< /Title (Two) /A << /S /GoTo /D /intro >> /Next 7 0 R >>",
            b"<< /Title (One.a) /Dest [5 0 R /Fit] >>",
        ]);
        let d = Document::open(pdf).unwrap();
        let o = d.outline();
        assert_eq!(o.len(), 2, "the Next loop back to 7 stops");
        assert_eq!((o[0].title.as_str(), o[0].page), ("One", Some(0)));
        assert_eq!((o[0].children[0].title.as_str(), o[0].children[0].page), ("One.a", Some(1)));
        assert_eq!((o[1].title.as_str(), o[1].page), ("Two", Some(1)));
    }

    #[test]
    fn hostile_files_never_panic() {
        let pdf = simple(b"0 0 m 10 10 l S");
        for cut in (0..pdf.len()).step_by(7) {
            let _ = Document::open(pdf[..cut].to_vec()).map(|d| (d.page_count(), d.info(), d.outline()));
        }
        let mut seed = 7u32;
        for _ in 0..300 {
            let mut v = pdf.clone();
            for _ in 0..4 {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                let i = (seed as usize >> 4) % v.len();
                v[i] = (seed >> 20) as u8;
            }
            if let Ok(d) = Document::open(v) {
                for i in 0..d.page_count() {
                    let _ = d.page_content(d.page(i).unwrap());
                }
            }
        }
        // Self-referencing pages and a reference loop.
        let pdf = build(&[
            b"<< /Type /Catalog /Pages 2 0 R >>",
            b"<< /Type /Pages /Kids [2 0 R 4 0 R] >>",
            b"3 0 R",
            b"<< /Type /Page /Contents 3 0 R >>",
        ]);
        let d = Document::open(pdf).unwrap();
        assert_eq!(d.page_count(), 1);
        assert!(d.page_content(d.page(0).unwrap()).is_empty());
    }

    #[test]
    fn text_strings() {
        assert_eq!(text_string(b"\xfe\xff\x00A\x00B"), "AB");
        assert_eq!(text_string(b"\xef\xbb\xbfh\xc3\xa9"), "hé");
        assert_eq!(text_string(b"\x8dquoted\x8e \xa0\x93"), "\u{201C}quoted\u{201D} €\u{FB01}");
    }
}
