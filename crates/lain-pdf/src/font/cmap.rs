//! CMaps (§9.7.5, §9.10.3): how a composite font's string bytes split
//! into codes and map to CIDs, and how `/ToUnicode` maps codes to text.
//! Embedded CMap streams are parsed; of the predefined ones only
//! `Identity-H`/`-V` are known (no CMap data files ship with Lain).

use std::collections::HashMap;

use crate::lexer::{Lexer, Token};

/// Ranges beyond this many entries are refused (a hostile CMap).
const MAX_RANGE: u32 = 1 << 20;

#[derive(Debug, Clone, Default)]
pub struct CMap {
    /// `(byte length, low, high)` of each codespace range.
    pub codespace: Vec<(usize, u32, u32)>,
    /// `(low, high, first cid)`.
    pub ranges: Vec<(u32, u32, u32)>,
    pub singles: HashMap<u32, u32>,
    pub vertical: bool,
    /// Codes read as CIDs unchanged (Identity, or an unknown predefined
    /// CMap whose fonts are usually Identity-ordered).
    pub identity: bool,
    /// The CMap names one this one extends.
    pub uses: Option<String>,
    /// Text for ToUnicode CMaps.
    pub unicode: HashMap<u32, String>,
}

impl CMap {
    pub fn identity(vertical: bool) -> CMap {
        CMap { codespace: vec![(2, 0, 0xffff)], identity: true, vertical, ..CMap::default() }
    }

    /// A predefined CMap by name. Unknown names read two-byte codes as
    /// CIDs, the most useful guess without CMap data.
    pub fn predefined(name: &str) -> CMap {
        let vertical = name.ends_with("-V");
        CMap::identity(vertical)
    }

    /// Whether the codes are Unicode (the `Uni…-UCS2`/`-UTF16` CMaps), so
    /// a TrueType font's own Unicode cmap can stand in for the CMap.
    pub fn unicode_named(name: &str) -> bool {
        name.starts_with("Uni") && (name.contains("UCS2") || name.contains("UTF16"))
    }

    pub fn parse(data: &[u8]) -> CMap {
        let mut c = CMap::default();
        let mut l = Lexer::new(data);
        let mut operands: Vec<Token> = Vec::new();
        let mut last_name: Option<String> = None;
        while let Some(t) = l.next_token() {
            match t {
                Token::Keyword(b"begincodespacerange") => {
                    operands.clear();
                    while let Some(t) = l.next_token() {
                        if t == Token::Keyword(b"endcodespacerange") {
                            break;
                        }
                        operands.push(t);
                    }
                    for pair in operands.chunks(2) {
                        if let [Token::String(lo), Token::String(hi)] = pair
                            && !lo.is_empty()
                            && lo.len() <= 4
                            && lo.len() == hi.len()
                        {
                            c.codespace.push((lo.len(), be(lo), be(hi)));
                        }
                    }
                    operands.clear();
                }
                Token::Keyword(b"begincidrange") => {
                    let items = collect(&mut l, b"endcidrange");
                    for tri in items.chunks(3) {
                        if let [Token::String(lo), Token::String(hi), Token::Int(cid)] = tri {
                            let (lo, hi) = (be(lo), be(hi));
                            if hi >= lo && hi - lo < MAX_RANGE && *cid >= 0 {
                                c.ranges.push((lo, hi, *cid as u32));
                            }
                        }
                    }
                }
                Token::Keyword(b"begincidchar") => {
                    let items = collect(&mut l, b"endcidchar");
                    for pair in items.chunks(2) {
                        if let [Token::String(code), Token::Int(cid)] = pair
                            && *cid >= 0
                        {
                            c.singles.insert(be(code), *cid as u32);
                        }
                    }
                }
                Token::Keyword(b"beginbfchar") => {
                    let items = collect(&mut l, b"endbfchar");
                    for pair in items.chunks(2) {
                        if let [Token::String(code), dst] = pair
                            && let Some(text) = bf_text(dst, 0)
                        {
                            c.unicode.insert(be(code), text);
                        }
                    }
                }
                Token::Keyword(b"beginbfrange") => {
                    let items = collect_bfrange(&mut l);
                    for (lo, hi, dst) in items {
                        if hi < lo || hi - lo >= MAX_RANGE {
                            continue;
                        }
                        match dst {
                            BfDst::Base(base) => {
                                for (i, code) in (lo..=hi).enumerate() {
                                    if let Some(t) = bump(&base, i as u32) {
                                        c.unicode.insert(code, t);
                                    }
                                }
                            }
                            BfDst::List(list) => {
                                for (i, code) in (lo..=hi).enumerate() {
                                    if let Some(t) = list.get(i) {
                                        c.unicode.insert(code, t.clone());
                                    }
                                }
                            }
                        }
                    }
                }
                Token::Name(n) => {
                    if n.as_str() == "WMode" {
                        if let Some(Token::Int(v)) = l.next_token() {
                            c.vertical = v == 1;
                        }
                    } else {
                        last_name = Some(n.as_str().to_string());
                    }
                }
                Token::Keyword(b"usecmap") => {
                    c.uses = last_name.clone();
                }
                _ => {}
            }
        }
        c
    }

    /// Folds in the CMap this one names with `usecmap` (only predefined
    /// Identity ones can be known).
    pub fn resolve_uses(&mut self) {
        if let Some(name) = self.uses.take()
            && name.starts_with("Identity")
        {
            if self.codespace.is_empty() {
                self.codespace.push((2, 0, 0xffff));
            }
            if self.ranges.is_empty() && self.singles.is_empty() {
                self.identity = true;
            }
        }
    }

    /// The next code in `bytes`: its value and length in bytes.
    pub fn next_code(&self, bytes: &[u8]) -> (u32, usize) {
        if self.codespace.is_empty() {
            let n = 2.min(bytes.len()).max(1);
            return (be(&bytes[..n.min(bytes.len())]), n);
        }
        for len in 1..=4usize {
            if bytes.len() < len {
                break;
            }
            let v = be(&bytes[..len]);
            if self.codespace.iter().any(|&(l, lo, hi)| l == len && in_range(v, lo, hi, len)) {
                return (v, len);
            }
        }
        // No range matches: take the shortest range's length (§9.7.6.3).
        let n = self.codespace.iter().map(|c| c.0).min().unwrap_or(1).min(bytes.len()).max(1);
        (be(&bytes[..n]), n)
    }

    pub fn cid(&self, code: u32) -> u32 {
        if let Some(&c) = self.singles.get(&code) {
            return c;
        }
        for &(lo, hi, first) in &self.ranges {
            if code >= lo && code <= hi {
                return first + (code - lo);
            }
        }
        if self.identity || (self.ranges.is_empty() && self.singles.is_empty()) { code } else { 0 }
    }
}

/// A codespace range test, byte by byte (a range is a box, §9.7.6.2).
fn in_range(v: u32, lo: u32, hi: u32, len: usize) -> bool {
    (0..len).all(|i| {
        let shift = 8 * (len - 1 - i);
        let (b, l, h) = ((v >> shift) & 0xff, (lo >> shift) & 0xff, (hi >> shift) & 0xff);
        b >= l && b <= h
    })
}

fn be(b: &[u8]) -> u32 {
    b.iter().take(4).fold(0u32, |acc, &x| acc << 8 | u32::from(x))
}

fn collect<'a>(l: &mut Lexer<'a>, end: &[u8]) -> Vec<Token<'a>> {
    let mut out = Vec::new();
    while let Some(t) = l.next_token() {
        if matches!(&t, Token::Keyword(k) if *k == end) {
            break;
        }
        out.push(t);
        if out.len() > 3 * 10_000 {
            break;
        }
    }
    out
}

enum BfDst {
    Base(Vec<u8>),
    List(Vec<String>),
}

fn collect_bfrange(l: &mut Lexer) -> Vec<(u32, u32, BfDst)> {
    let mut out = Vec::new();
    while let Some(t) = l.next_token() {
        let lo = match t {
            Token::Keyword(b"endbfrange") => break,
            Token::String(s) => be(&s),
            _ => continue,
        };
        let Some(Token::String(hi)) = l.next_token() else { break };
        let hi = be(&hi);
        match l.next_token() {
            Some(Token::String(base)) => out.push((lo, hi, BfDst::Base(base))),
            Some(Token::ArrayStart) => {
                let mut list = Vec::new();
                while let Some(t) = l.next_token() {
                    if t == Token::ArrayEnd {
                        break;
                    }
                    if let Some(s) = bf_text(&t, 0) {
                        list.push(s);
                    }
                }
                out.push((lo, hi, BfDst::List(list)));
            }
            _ => break,
        }
        if out.len() > 10_000 {
            break;
        }
    }
    out
}

/// A bf destination: UTF-16BE bytes, or a glyph name.
fn bf_text(t: &Token, add: u32) -> Option<String> {
    match t {
        Token::String(s) => bump(s, add),
        Token::Name(n) => super::glyphlist::to_unicode(n.as_str()),
        _ => None,
    }
}

/// UTF-16BE `base` with its last unit increased by `add`.
fn bump(base: &[u8], add: u32) -> Option<String> {
    let mut units: Vec<u16> = base.chunks(2).map(|c| u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)])).collect();
    if let Some(last) = units.last_mut() {
        *last = last.wrapping_add(add as u16);
    }
    let s = String::from_utf16_lossy(&units);
    (!s.is_empty()).then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_cmap() {
        let src = b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /Test def
            2 begincodespacerange <00> <80> <8140> <9FFC> endcodespacerange
            1 begincidrange <8140> <817E> 633 endcidrange
            2 begincidchar <20> 1 <41> 34 endcidchar
            /WMode 1 def endcmap";
        let c = CMap::parse(src);
        assert!(c.vertical);
        assert_eq!(c.next_code(&[0x41, 0x81, 0x40]), (0x41, 1));
        assert_eq!(c.next_code(&[0x81, 0x41]), (0x8141, 2));
        assert_eq!(c.cid(0x8141), 634);
        assert_eq!(c.cid(0x41), 34);
        assert_eq!(c.cid(0x99), 0);
        assert_eq!(CMap::identity(false).next_code(&[1, 2, 3]), (0x0102, 2));
    }

    #[test]
    fn to_unicode() {
        let src = b"1 begincodespacerange <0000> <FFFF> endcodespacerange
            2 beginbfchar <0003> <0020> <0004> <D83DDE00> endbfchar
            2 beginbfrange <0010> <0012> <0041> <0020> <0021> [<0066006C> <00E9>] endbfrange";
        let c = CMap::parse(src);
        assert_eq!(c.unicode.get(&3).map(String::as_str), Some(" "));
        assert_eq!(c.unicode.get(&4).map(String::as_str), Some("😀"));
        assert_eq!(c.unicode.get(&0x12).map(String::as_str), Some("C"));
        assert_eq!(c.unicode.get(&0x20).map(String::as_str), Some("fl"));
        assert_eq!(c.unicode.get(&0x21).map(String::as_str), Some("é"));
        let _ = CMap::parse(b"1 begincidrange <0000> <FFFFFFFF> 0 endcidrange beginbfrange <00> endbfrange");
    }
}
