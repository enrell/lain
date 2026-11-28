//! A tolerant HTML tokenizer and a streaming tree balancer that turns
//! the tag soup of MOBI books into well-formed XHTML.
//!
//! The tokenizer works on raw bytes (UTF-8 or Windows-1252, both ASCII
//! compatible) so token positions are the byte offsets MOBI links use.
//! The balancer keeps a stack of open elements, applies the implicit
//! closes of HTML (`p`, `li`, table rows, ...), drops unknown and
//! namespaced tags but keeps their text, drops scripts, rewrites URLs
//! through [`Hooks`], and can split the output into several documents
//! (at MOBI page breaks or by size), closing the open elements at the
//! split and reopening them in the next document. Everything is linear
//! in the input size.

use std::collections::{HashMap, HashSet};

use super::encoding::{cp1252, push_decoded};
use super::entities;

pub(crate) const TEXT: u8 = 0;
pub(crate) const RAW: u8 = 1;
pub(crate) const START: u8 = 2;
pub(crate) const END: u8 = 3;
pub(crate) const OTHER: u8 = 4;
pub(crate) const CDATA: u8 = 5;

/// Attributes kept per tag; the rest are ignored.
const MAX_ATTRS: usize = 64;
/// Open elements beyond this depth are dropped (their text is kept).
const MAX_DEPTH: usize = 256;
/// Output documents are split at a top-level block past this size...
const SOFT_SPLIT: usize = 256 << 10;
/// ...and at any block past this one.
const HARD_SPLIT: usize = 1 << 20;
/// Characters kept of a heading or link label.
const MAX_LABEL: usize = 512;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Token {
    pub kind: u8,
    /// A start tag written `<x/>`.
    pub closed: bool,
    pub start: u32,
    pub end: u32,
    /// The tag name, or the content of a CDATA section.
    pub name: (u32, u32),
    /// The token's attributes: a range of [`Tokens::attrs`].
    pub attrs: (u32, u32),
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Attr {
    pub name: (u32, u32),
    pub value: Option<(u32, u32)>,
}

/// A token stream. Tokens cover the input contiguously.
pub(crate) struct Tokens {
    pub toks: Vec<Token>,
    pub attrs: Vec<Attr>,
}

fn span(src: &[u8], r: (u32, u32)) -> &[u8] {
    src.get(r.0 as usize..r.1 as usize).unwrap_or(&[])
}

impl Tokens {
    pub fn name<'s>(&self, src: &'s [u8], t: &Token) -> &'s [u8] {
        span(src, t.name)
    }

    pub fn attrs_of(&self, t: &Token) -> &[Attr] {
        self.attrs.get(t.attrs.0 as usize..t.attrs.1 as usize).unwrap_or(&[])
    }

    /// The raw value of attribute `name` (ASCII, case-insensitive).
    pub fn attr<'s>(&self, src: &'s [u8], t: &Token, name: &[u8]) -> Option<&'s [u8]> {
        self.attrs_of(t)
            .iter()
            .find(|a| span(src, a.name).eq_ignore_ascii_case(name))
            .map(|a| a.value.map(|v| span(src, v)).unwrap_or(b""))
    }

    /// The index of the token containing byte `pos`.
    pub fn at(&self, pos: u32) -> Option<usize> {
        let i = self.toks.partition_point(|t| t.start <= pos);
        let t = self.toks.get(i.checked_sub(1)?)?;
        (pos < t.end).then_some(i - 1)
    }
}

fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c')
}

fn find(hay: &[u8], from: usize, b: u8) -> Option<usize> {
    hay.get(from..)?.iter().position(|&x| x == b).map(|p| from + p)
}

fn find_seq(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    let first = *needle.first()?;
    let mut i = from;
    while let Some(p) = find(hay, i, first) {
        if hay.get(p..p + needle.len()) == Some(needle) {
            return Some(p);
        }
        i = p + 1;
    }
    None
}

/// Elements whose content is raw text up to the matching end tag.
fn raw_element(name: &[u8]) -> bool {
    const RAW_ELEMENTS: [&[u8]; 8] =
        [b"script", b"style", b"title", b"textarea", b"xmp", b"iframe", b"noembed", b"noframes"];
    RAW_ELEMENTS.iter().any(|r| r.eq_ignore_ascii_case(name))
}

/// Splits `src` into tokens. Inputs longer than 4 GB are cut (callers
/// cap them far lower).
pub(crate) fn tokenize(src: &[u8]) -> Tokens {
    let n = src.len().min(u32::MAX as usize - 1);
    let src = &src[..n];
    let mut toks = Vec::with_capacity(n / 24 + 1);
    let mut attrs = Vec::new();
    let mut i = 0usize;
    let mut text_start = 0usize;
    let text = |a: usize, b: usize| Token {
        kind: TEXT,
        closed: false,
        start: a as u32,
        end: b as u32,
        name: (0, 0),
        attrs: (0, 0),
    };
    while let Some(lt) = find(src, i, b'<') {
        let parsed = match src.get(lt + 1) {
            Some(c) if c.is_ascii_alphabetic() => Some(start_tag(src, lt, &mut attrs)),
            Some(b'/') => end_tag(src, lt),
            Some(b'!') => Some(bang(src, lt)),
            Some(b'?') => Some(other(lt, find(src, lt, b'>').map_or(n, |p| p + 1))),
            _ => None,
        };
        let Some(tok) = parsed else {
            i = lt + 1;
            continue;
        };
        if text_start < lt {
            toks.push(text(text_start, lt));
        }
        toks.push(tok);
        i = tok.end as usize;
        text_start = i;
        if tok.kind == START && !tok.closed && raw_element(span(src, tok.name)) {
            let close = find_close(src, i, span(src, tok.name)).unwrap_or(n);
            if close > i {
                toks.push(Token { kind: RAW, ..text(i, close) });
            }
            i = close;
            text_start = close;
        }
    }
    if text_start < n {
        toks.push(text(text_start, n));
    }
    Tokens { toks, attrs }
}

fn other(lt: usize, end: usize) -> Token {
    Token { kind: OTHER, closed: false, start: lt as u32, end: end as u32, name: (0, 0), attrs: (0, 0) }
}

fn start_tag(src: &[u8], lt: usize, attrs: &mut Vec<Attr>) -> Token {
    let n = src.len();
    let mut j = lt + 1;
    while j < n && !is_ws(src[j]) && src[j] != b'/' && src[j] != b'>' {
        j += 1;
    }
    let name = (lt as u32 + 1, j as u32);
    let a0 = attrs.len();
    let mut closed = false;
    loop {
        while j < n && (is_ws(src[j]) || src[j] == b'/') {
            closed = src[j] == b'/' && src.get(j + 1) == Some(&b'>');
            j += 1;
        }
        if j >= n {
            // End of input inside a tag: the tag is dropped.
            attrs.truncate(a0);
            return other(lt, n);
        }
        if src[j] == b'>' {
            j += 1;
            break;
        }
        closed = false;
        let ns = j;
        j += 1;
        while j < n && !is_ws(src[j]) && !matches!(src[j], b'/' | b'>' | b'=') {
            j += 1;
        }
        let name = (ns as u32, j as u32);
        let mut k = j;
        while k < n && is_ws(src[k]) {
            k += 1;
        }
        let mut value = None;
        if k < n && src[k] == b'=' {
            k += 1;
            while k < n && is_ws(src[k]) {
                k += 1;
            }
            if k < n && (src[k] == b'"' || src[k] == b'\'') {
                let vs = k + 1;
                let Some(ve) = find(src, vs, src[k]) else {
                    attrs.truncate(a0);
                    return other(lt, n);
                };
                value = Some((vs as u32, ve as u32));
                j = ve + 1;
            } else {
                let vs = k;
                while k < n && !is_ws(src[k]) && src[k] != b'>' {
                    k += 1;
                }
                value = Some((vs as u32, k as u32));
                j = k;
            }
        }
        if attrs.len() - a0 < MAX_ATTRS {
            attrs.push(Attr { name, value });
        }
    }
    Token { kind: START, closed, start: lt as u32, end: j as u32, name, attrs: (a0 as u32, attrs.len() as u32) }
}

fn end_tag(src: &[u8], lt: usize) -> Option<Token> {
    let n = src.len();
    let c = *src.get(lt + 2)?;
    if !c.is_ascii_alphabetic() {
        // `</>` and `</3...>` are bogus comments up to the next '>'.
        return Some(other(lt, find(src, lt, b'>').map_or(n, |p| p + 1)));
    }
    let mut j = lt + 2;
    while j < n && !is_ws(src[j]) && src[j] != b'/' && src[j] != b'>' {
        j += 1;
    }
    let name = (lt as u32 + 2, j as u32);
    let Some(gt) = find(src, j, b'>') else { return Some(other(lt, n)) };
    Some(Token { kind: END, closed: false, start: lt as u32, end: gt as u32 + 1, name, attrs: (0, 0) })
}

fn bang(src: &[u8], lt: usize) -> Token {
    let n = src.len();
    if src[lt..].starts_with(b"<!--") {
        let end = find_seq(src, lt + 4, b"-->").map_or(n, |p| p + 3);
        return other(lt, end);
    }
    if src.get(lt..lt + 9).is_some_and(|s| s.eq_ignore_ascii_case(b"<![CDATA[")) {
        let close = find_seq(src, lt + 9, b"]]>");
        let (content_end, end) = close.map_or((n, n), |p| (p, p + 3));
        return Token {
            kind: CDATA,
            closed: false,
            start: lt as u32,
            end: end as u32,
            name: (lt as u32 + 9, content_end as u32),
            attrs: (0, 0),
        };
    }
    other(lt, find(src, lt, b'>').map_or(n, |p| p + 1))
}

/// The position of `</name` (case-insensitive, then a delimiter).
fn find_close(src: &[u8], from: usize, name: &[u8]) -> Option<usize> {
    let mut i = from;
    while let Some(p) = find(src, i, b'<') {
        if src.get(p + 1) == Some(&b'/')
            && src.get(p + 2..p + 2 + name.len()).is_some_and(|s| s.eq_ignore_ascii_case(name))
            && src.get(p + 2 + name.len()).is_none_or(|&c| c == b'>' || c == b'/' || is_ws(c))
        {
            return Some(p);
        }
        i = p + 1;
    }
    None
}

/// Appends `raw` decoded to `out`; `refs` also decodes character
/// references (`&amp;`, `&#233;`, ...).
pub(crate) fn decode_into(out: &mut String, raw: &[u8], utf8: bool, refs: bool) {
    if !refs {
        push_decoded(out, raw, utf8);
        return;
    }
    let mut i = 0;
    while i < raw.len() {
        let Some(amp) = find(raw, i, b'&') else {
            push_decoded(out, &raw[i..], utf8);
            break;
        };
        push_decoded(out, &raw[i..amp], utf8);
        match char_ref(&raw[amp..]) {
            Some((c, used)) => {
                out.push(c);
                i = amp + used;
            }
            None => {
                out.push('&');
                i = amp + 1;
            }
        }
    }
}

/// A character reference at the start of `s` (which starts with '&'):
/// (character, bytes used). Named references need their ';'.
fn char_ref(s: &[u8]) -> Option<(char, usize)> {
    if s.get(1) == Some(&b'#') {
        let hex = matches!(s.get(2), Some(b'x' | b'X'));
        let (radix, at) = if hex { (16, 3) } else { (10, 2) };
        let mut j = at;
        let mut v: u32 = 0;
        while let Some(d) = s.get(j).and_then(|&c| char::from(c).to_digit(radix)) {
            v = v.saturating_mul(radix).saturating_add(d);
            j += 1;
        }
        if j == at {
            return None;
        }
        let used = if s.get(j) == Some(&b';') { j + 1 } else { j };
        let c = match v {
            0 => '\u{fffd}',
            0x80..=0x9f => cp1252(v as u8),
            _ => char::from_u32(v).unwrap_or('\u{fffd}'),
        };
        return Some((c, used));
    }
    let mut j = 1;
    while j < s.len() && j <= 32 && s[j].is_ascii_alphanumeric() {
        j += 1;
    }
    if j == 1 || s.get(j) != Some(&b';') {
        return None;
    }
    let c = entities::lookup(std::str::from_utf8(&s[1..j]).ok()?)?;
    Some((c, j + 1))
}

/// Appends `s` XML-escaped, dropping characters XML 1.0 forbids (and
/// the C1 controls).
pub(crate) fn escape(out: &mut String, s: &str, attr: bool) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attr => out.push_str("&quot;"),
            '\t' | '\n' | '\r' => out.push(if attr { ' ' } else { c }),
            '\0'..='\u{1f}' | '\u{7f}'..='\u{9f}' | '\u{fffe}' | '\u{ffff}' => {}
            _ => out.push(c),
        }
    }
}

/// `s` with runs of whitespace collapsed to one space, trimmed.
pub(crate) fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for w in s.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(w);
    }
    out
}

/// The text of an HTML fragment (a description), one line per block.
pub(crate) fn plain_text(src: &[u8], utf8: bool) -> String {
    let t = tokenize(src);
    let mut raw = String::new();
    let mut buf = [0u8; 32];
    for tok in &t.toks {
        match tok.kind {
            TEXT => decode_into(&mut raw, span(src, (tok.start, tok.end)), utf8, true),
            CDATA => decode_into(&mut raw, span(src, tok.name), utf8, false),
            START | END => {
                if matches!(
                    lower(t.name(src, tok), &mut buf),
                    "p" | "br" | "div" | "li" | "tr" | "blockquote" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
                ) {
                    raw.push('\n');
                }
            }
            _ => {}
        }
    }
    let mut out = String::new();
    for line in raw.lines() {
        let line = collapse_ws(line);
        if !line.is_empty() {
            if !out.is_empty() {
                out.push('\n');
            }
            let mut clean = String::new();
            strip_controls(&mut clean, &line);
            out.push_str(&clean);
        }
    }
    out
}

/// Appends `s` without the characters XML forbids (no escaping).
fn strip_controls(out: &mut String, s: &str) {
    out.extend(s.chars().filter(|&c| !matches!(c, '\0'..='\u{8}' | '\u{b}'..='\u{1f}' | '\u{7f}'..='\u{9f}')));
}

/// The ASCII-lowercased tag or attribute name, or "" when too long.
fn lower<'b>(raw: &[u8], buf: &'b mut [u8; 32]) -> &'b str {
    if raw.len() > buf.len() {
        return "";
    }
    for (d, s) in buf.iter_mut().zip(raw) {
        *d = s.to_ascii_lowercase();
    }
    std::str::from_utf8(&buf[..raw.len()]).unwrap_or("")
}

// ---------------------------------------------------------------------
// The balancer.

/// What a converter does with a URL.
pub(crate) enum Url {
    /// Write this value.
    Keep(String),
    /// A link to target `key`, resolved after every document is built.
    Link(usize),
    /// Drop the attribute (an `img` without a source is dropped).
    Drop,
}

/// Converter-specific URL and CSS rewriting.
pub(crate) trait Hooks {
    /// `attr` of `tag` (lowercase): `href`, `src`, `filepos`,
    /// `recindex`, `hirecindex`, `lorecindex`; `image`'s `xlink:href`;
    /// `link`'s `href`.
    fn url(&mut self, tag: &str, attr: &str, value: &str) -> Url;
    /// CSS of a `style` element or attribute, rewritten and sanitized
    /// (see [`sanitize_css`]); by default only fragment URLs are kept.
    fn css(&mut self, css: &str) -> String {
        sanitize_css(css, &|u| u.starts_with('#'))
    }
}

/// A link target to place in the output. `attach` targets a start tag
/// (its id is used, or one is given to it); otherwise an empty span is
/// placed at byte `off` (inside text, or at the token's start).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Anchor {
    pub tok: u32,
    pub off: u32,
    pub attach: bool,
    pub key: usize,
}

pub(crate) struct Options {
    /// Start a new document at `<mbp:pagebreak/>`.
    pub split_pagebreaks: bool,
    /// Start a new document when one grows too large.
    pub split_size: bool,
    /// Drop a last document with no content and no anchors.
    pub drop_empty_tail: bool,
}

#[derive(Default, Clone)]
pub(crate) struct OutDoc {
    pub title: Option<String>,
    /// `link` and `style` elements for the head.
    pub head: String,
    pub body_attrs: String,
    pub body: String,
    /// Has inline SVG.
    pub svg: bool,
}

pub(crate) struct Heading {
    pub level: u8,
    pub doc: usize,
    pub id: String,
    pub text: String,
}

pub(crate) struct LinkRec {
    pub key: usize,
    pub doc: usize,
    pub text: String,
}

/// State shared by every source of one book.
pub(crate) struct Ctx {
    pub docs: Vec<OutDoc>,
    /// Per key: (document, id) once placed.
    pub targets: Vec<Option<(usize, Option<String>)>>,
    /// Per key: the id to give an anchor that has none.
    pub gen_ids: Vec<String>,
    pub headings: Vec<Heading>,
    pub links: Vec<LinkRec>,
    heading_seq: usize,
}

impl Ctx {
    pub fn new(gen_ids: Vec<String>) -> Ctx {
        Ctx {
            docs: Vec::new(),
            targets: vec![None; gen_ids.len()],
            gen_ids,
            headings: Vec::new(),
            links: Vec::new(),
            heading_seq: 0,
        }
    }
}

const VOID: u16 = 1;
const BLOCK: u16 = 2;
const CLOSES_P: u16 = 4;
const INLINE: u16 = 8;
const TABLE_PART: u16 = 16;

#[derive(Clone, Copy)]
struct Spec {
    /// The source name (end tags match it).
    src: &'static str,
    out: &'static str,
    flags: u16,
    /// Style implied by a presentational element.
    style: &'static str,
}

const INLINES: &[&str] = &[
    "a", "abbr", "b", "bdi", "bdo", "cite", "code", "del", "dfn", "em", "i", "ins", "kbd", "mark", "q", "s", "samp",
    "small", "span", "strong", "sub", "sup", "time", "u", "var", "ruby", "rb", "rt", "rp", "img", "br", "wbr",
];
const BLOCKS: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "details",
    "div",
    "dl",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "header",
    "hgroup",
    "main",
    "nav",
    "ol",
    "p",
    "pre",
    "section",
    "summary",
    "ul",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "li",
    "dd",
    "dt",
    "legend",
];
const TABLES: &[&str] = &["table", "caption", "colgroup", "col", "thead", "tbody", "tfoot", "tr", "td", "th"];

fn spec(name: &str) -> Option<Spec> {
    let s = |src, out, flags, style| Some(Spec { src, out, flags, style });
    match name {
        "acronym" => return s("acronym", "abbr", INLINE, ""),
        "big" => return s("big", "span", INLINE, "font-size: larger"),
        "font" => return s("font", "span", INLINE, ""),
        "strike" => return s("strike", "s", INLINE, ""),
        "tt" => return s("tt", "span", INLINE, "font-family: monospace"),
        "nobr" => return s("nobr", "span", INLINE, "white-space: nowrap"),
        "blink" => return s("blink", "span", INLINE, ""),
        "center" => return s("center", "div", BLOCK | CLOSES_P, "text-align: center"),
        "dir" => return s("dir", "ul", BLOCK | CLOSES_P, ""),
        "menu" => return s("menu", "ul", BLOCK | CLOSES_P, ""),
        "listing" => return s("listing", "pre", BLOCK | CLOSES_P, ""),
        "plaintext" => return s("plaintext", "pre", BLOCK | CLOSES_P, ""),
        "marquee" => return s("marquee", "div", BLOCK, ""),
        "hr" => return s("hr", "hr", VOID | BLOCK | CLOSES_P, ""),
        "body" => return s("body", "body", 0, ""),
        _ => {}
    }
    if let Some(&n) = INLINES.iter().find(|&&x| x == name) {
        let void = if matches!(n, "img" | "br" | "wbr") { VOID } else { 0 };
        return s(n, n, INLINE | void, "");
    }
    if let Some(&n) = BLOCKS.iter().find(|&&x| x == name) {
        return s(n, n, BLOCK | CLOSES_P, "");
    }
    if let Some(&n) = TABLES.iter().find(|&&x| x == name) {
        let extra = match n {
            "table" => CLOSES_P,
            "col" => VOID,
            _ => 0,
        };
        return s(n, n, BLOCK | TABLE_PART | extra, "");
    }
    None
}

const SVG_ELEMENTS: &[(&str, &str)] = &[
    ("svg", "svg"),
    ("g", "g"),
    ("defs", "defs"),
    ("symbol", "symbol"),
    ("use", "use"),
    ("image", "image"),
    ("rect", "rect"),
    ("circle", "circle"),
    ("ellipse", "ellipse"),
    ("line", "line"),
    ("polyline", "polyline"),
    ("polygon", "polygon"),
    ("path", "path"),
    ("text", "text"),
    ("tspan", "tspan"),
    ("textpath", "textPath"),
    ("lineargradient", "linearGradient"),
    ("radialgradient", "radialGradient"),
    ("stop", "stop"),
    ("clippath", "clipPath"),
    ("mask", "mask"),
    ("pattern", "pattern"),
    ("switch", "switch"),
];

const SVG_ATTRS: &[(&str, &str)] = &[
    ("class", "class"),
    ("width", "width"),
    ("height", "height"),
    ("x", "x"),
    ("y", "y"),
    ("x1", "x1"),
    ("y1", "y1"),
    ("x2", "x2"),
    ("y2", "y2"),
    ("cx", "cx"),
    ("cy", "cy"),
    ("fx", "fx"),
    ("fy", "fy"),
    ("r", "r"),
    ("rx", "rx"),
    ("ry", "ry"),
    ("d", "d"),
    ("points", "points"),
    ("transform", "transform"),
    ("fill", "fill"),
    ("fill-opacity", "fill-opacity"),
    ("fill-rule", "fill-rule"),
    ("stroke", "stroke"),
    ("stroke-width", "stroke-width"),
    ("stroke-opacity", "stroke-opacity"),
    ("stroke-linecap", "stroke-linecap"),
    ("stroke-linejoin", "stroke-linejoin"),
    ("stroke-dasharray", "stroke-dasharray"),
    ("stroke-miterlimit", "stroke-miterlimit"),
    ("opacity", "opacity"),
    ("font-family", "font-family"),
    ("font-size", "font-size"),
    ("font-weight", "font-weight"),
    ("font-style", "font-style"),
    ("text-anchor", "text-anchor"),
    ("dx", "dx"),
    ("dy", "dy"),
    ("offset", "offset"),
    ("stop-color", "stop-color"),
    ("stop-opacity", "stop-opacity"),
    ("viewbox", "viewBox"),
    ("preserveaspectratio", "preserveAspectRatio"),
    ("gradientunits", "gradientUnits"),
    ("gradienttransform", "gradientTransform"),
    ("spreadmethod", "spreadMethod"),
    ("patternunits", "patternUnits"),
    ("patterncontentunits", "patternContentUnits"),
    ("patterntransform", "patternTransform"),
    ("clippathunits", "clipPathUnits"),
    ("maskunits", "maskUnits"),
    ("maskcontentunits", "maskContentUnits"),
    ("clip-path", "clip-path"),
    ("clip-rule", "clip-rule"),
    ("mask", "mask"),
    ("version", "version"),
    ("visibility", "visibility"),
    ("display", "display"),
];

fn svg_name(lname: &str) -> Option<(&'static str, &'static str)> {
    SVG_ELEMENTS.iter().find(|e| e.0 == lname).copied()
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Head,
    Body,
}

#[derive(Clone, Copy, PartialEq)]
enum Raw {
    None,
    Title,
    Style,
    Pre,
}

struct Open {
    src: &'static str,
    out: &'static str,
    svg: bool,
    block: bool,
    /// The start tag without its id, to reopen after a split.
    reopen: String,
}

enum CapKind {
    Heading { level: u8, id: String },
    Link { key: usize },
}

struct Capture {
    depth: usize,
    doc: usize,
    kind: CapKind,
    text: String,
}

/// A start tag's attributes, serialized.
struct El {
    id: Option<String>,
    attrs: String,
    link: Option<usize>,
}

/// Emits the documents of one source into `ctx.docs`.
pub(crate) fn emit(
    src: &[u8],
    utf8: bool,
    t: &Tokens,
    anchors: &[Anchor],
    hooks: &mut dyn Hooks,
    opts: &Options,
    ctx: &mut Ctx,
) {
    let first_doc = ctx.docs.len();
    let mut e = Emitter {
        src,
        utf8,
        t,
        hooks,
        opts,
        ctx,
        cur: OutDoc::default(),
        mode: Mode::Head,
        raw: Raw::None,
        stack: Vec::new(),
        ids: HashSet::new(),
        suffix: HashMap::new(),
        positions: HashMap::new(),
        blocks: Vec::new(),
        pending: Vec::new(),
        attach: Vec::new(),
        splits: Vec::new(),
        has_content: false,
        anchors_here: 0,
        svg_depth: 0,
        captures: Vec::new(),
        first_doc,
        body_attrs_set: false,
        head_links: HashSet::new(),
        scratch: String::new(),
    };
    let mut ai = 0;
    for (ti, tok) in t.toks.iter().enumerate() {
        while let Some(a) = anchors.get(ai).filter(|a| a.tok as usize <= ti) {
            ai += 1;
            if (a.tok as usize) < ti || (!a.attach && a.off <= tok.start) {
                e.pending.push(a.key);
            } else if a.attach {
                e.attach.push(a.key);
            } else {
                e.splits.push((a.off, a.key));
            }
        }
        e.token(*tok);
    }
    e.pending.extend(anchors[ai.min(anchors.len())..].iter().map(|a| a.key));
    e.finish();
}

struct Emitter<'e> {
    src: &'e [u8],
    utf8: bool,
    t: &'e Tokens,
    hooks: &'e mut dyn Hooks,
    opts: &'e Options,
    ctx: &'e mut Ctx,
    cur: OutDoc,
    mode: Mode,
    raw: Raw,
    stack: Vec<Open>,
    ids: HashSet<String>,
    suffix: HashMap<String, usize>,
    /// Stack indices of the open elements, by source name.
    positions: HashMap<&'static str, Vec<usize>>,
    /// Stack indices of the open block elements.
    blocks: Vec<usize>,
    /// Keys waiting for the next emitted content.
    pending: Vec<usize>,
    /// Keys for the current start tag.
    attach: Vec<usize>,
    /// (offset, key) inside the current text token.
    splits: Vec<(u32, usize)>,
    has_content: bool,
    anchors_here: usize,
    svg_depth: usize,
    captures: Vec<Capture>,
    first_doc: usize,
    body_attrs_set: bool,
    head_links: HashSet<String>,
    scratch: String,
}

impl Emitter<'_> {
    fn doc(&self) -> usize {
        self.ctx.docs.len()
    }

    fn token(&mut self, tok: Token) {
        let raw = std::mem::replace(&mut self.raw, Raw::None);
        match tok.kind {
            RAW => self.raw_text(tok, raw),
            TEXT => {
                let splits = std::mem::take(&mut self.splits);
                let mut pos = tok.start;
                for &(off, key) in &splits {
                    let off = off.clamp(pos, tok.end);
                    self.text_run(pos, off, true);
                    pos = off;
                    self.pending.push(key);
                }
                self.splits = splits;
                self.splits.clear();
                self.text_run(pos, tok.end, true);
            }
            CDATA => self.text_run(tok.name.0, tok.name.1, false),
            START => self.start(tok),
            END => self.end(tok),
            _ => {}
        }
        // Splits outside a text token and keys of a start tag that was not
        // emitted wait for the next content.
        self.pending.extend(self.splits.drain(..).map(|s| s.1));
        self.pending.append(&mut self.attach);
    }

    fn text_run(&mut self, a: u32, b: u32, refs: bool) {
        if a >= b {
            return;
        }
        let raw = span(self.src, (a, b));
        let ws = raw.iter().all(|&c| is_ws(c));
        if self.mode != Mode::Body {
            if ws {
                return;
            }
            self.mode = Mode::Body;
        }
        let mut s = std::mem::take(&mut self.scratch);
        s.clear();
        decode_into(&mut s, raw, self.utf8, refs);
        if !ws {
            self.has_content = true;
            self.flush_pending();
        }
        escape(&mut self.cur.body, &s, false);
        for c in &mut self.captures {
            if c.text.len() < MAX_LABEL {
                c.text.push_str(&s);
            }
        }
        self.scratch = s;
    }

    fn raw_text(&mut self, tok: Token, raw: Raw) {
        let bytes = span(self.src, (tok.start, tok.end));
        match raw {
            Raw::Title => {
                if self.cur.title.is_none() {
                    let mut s = String::new();
                    decode_into(&mut s, bytes, self.utf8, true);
                    let s = collapse_ws(&s);
                    if !s.is_empty() {
                        self.cur.title = Some(s);
                    }
                }
            }
            Raw::Style => {
                let mut s = String::new();
                decode_into(&mut s, bytes, self.utf8, false);
                let s = s.replace("<!--", "").replace("-->", "").replace("<![CDATA[", "").replace("]]>", "");
                let css = self.hooks.css(&s);
                if !css.trim().is_empty() {
                    self.cur.head.push_str("<style type=\"text/css\">\n");
                    escape(&mut self.cur.head, &css, false);
                    self.cur.head.push_str("\n</style>\n");
                }
            }
            Raw::Pre => {
                self.mode = Mode::Body;
                self.flush_pending();
                self.cur.body.push_str("<pre>");
                let mut s = String::new();
                decode_into(&mut s, bytes, self.utf8, false);
                escape(&mut self.cur.body, &s, false);
                self.cur.body.push_str("</pre>");
                self.has_content = true;
            }
            Raw::None => {}
        }
    }

    fn start(&mut self, tok: Token) {
        let mut buf = [0u8; 32];
        let lname = lower(self.t.name(self.src, &tok), &mut buf);
        if lname.is_empty() {
            return;
        }
        if lname.contains(':') {
            if lname == "mbp:pagebreak" && self.opts.split_pagebreaks && self.mode == Mode::Body && self.svg_depth == 0
            {
                self.page_break();
            }
            return;
        }
        if self.svg_depth > 0 {
            if matches!(lname, "title" | "style" | "script") {
                return;
            }
            self.svg_start(tok, lname);
            return;
        }
        match lname {
            "html" | "head" => return,
            "body" => {
                if !self.body_attrs_set {
                    self.body_attrs_set = true;
                    if let Some(spec) = spec("body")
                        && let Some(el) = self.attributes(tok, &spec)
                    {
                        self.cur.body_attrs = el.attrs;
                    }
                }
                self.mode = Mode::Body;
                return;
            }
            "title" => {
                if self.mode == Mode::Head {
                    self.raw = Raw::Title;
                }
                return;
            }
            "style" => {
                self.raw = Raw::Style;
                return;
            }
            "xmp" => {
                self.raw = Raw::Pre;
                return;
            }
            "link" => {
                self.link(tok);
                return;
            }
            "script" | "textarea" | "iframe" | "noembed" | "noframes" | "meta" | "base" => return,
            "svg" => {
                self.mode = Mode::Body;
                self.svg_start(tok, lname);
                return;
            }
            _ => {}
        }
        let Some(spec) = spec(lname) else { return };
        self.mode = Mode::Body;
        self.element(tok, spec);
    }

    fn element(&mut self, tok: Token, spec: Spec) {
        self.auto_close(&spec);
        if spec.flags & BLOCK != 0 && self.opts.split_size {
            self.size_split();
        }
        let void = spec.flags & VOID != 0;
        if !void && self.stack.len() >= MAX_DEPTH {
            return;
        }
        let Some(el) = self.attributes(tok, &spec) else { return };
        let level = heading_level(spec.src);
        let mut id = el.id.filter(|i| !self.ids.contains(i));
        if !self.attach.is_empty() {
            let keys = std::mem::take(&mut self.attach);
            let chosen = match id.take() {
                Some(i) => i,
                None => self.unique(&self.gen_id(keys[0])),
            };
            let doc = self.doc();
            for &k in &keys {
                if let Some(slot) = self.ctx.targets.get_mut(k) {
                    *slot = Some((doc, Some(chosen.clone())));
                }
            }
            self.anchors_here += 1;
            id = Some(chosen);
        }
        if (1..=3).contains(&level) && id.is_none() {
            self.ctx.heading_seq += 1;
            id = Some(self.unique(&format!("lain-h{}", self.ctx.heading_seq)));
        }
        if let Some(i) = &id {
            self.ids.insert(i.clone());
        }
        self.flush_pending();
        let mut tag = String::with_capacity(16 + el.attrs.len());
        tag.push('<');
        tag.push_str(spec.out);
        tag.push_str(&el.attrs);
        if let Some(k) = el.link {
            push_placeholder(&mut tag, k);
        }
        let body = &mut self.cur.body;
        body.push('<');
        body.push_str(spec.out);
        if let Some(i) = &id {
            body.push_str(" id=\"");
            escape(body, i, true);
            body.push('"');
        }
        body.push_str(&tag[1 + spec.out.len()..]);
        if void {
            body.push_str("/>");
            if matches!(spec.src, "img" | "hr") {
                self.has_content = true;
            }
            return;
        }
        if tok.closed {
            body.push_str("></");
            body.push_str(spec.out);
            body.push('>');
            return;
        }
        body.push('>');
        tag.push('>');
        self.push_open(Open { src: spec.src, out: spec.out, svg: false, block: spec.flags & BLOCK != 0, reopen: tag });
        let (depth, doc) = (self.stack.len(), self.doc());
        if (1..=3).contains(&level)
            && self.captures.len() < 8
            && let Some(id) = id
        {
            self.captures.push(Capture { depth, doc, kind: CapKind::Heading { level, id }, text: String::new() });
        }
        if let Some(key) = el.link
            && self.captures.len() < 8
        {
            self.captures.push(Capture { depth, doc, kind: CapKind::Link { key }, text: String::new() });
        }
    }

    fn gen_id(&self, key: usize) -> String {
        self.ctx.gen_ids.get(key).cloned().unwrap_or_else(|| format!("lain-a{key}"))
    }

    fn unique(&mut self, base: &str) -> String {
        if !self.ids.contains(base) {
            return base.to_string();
        }
        let mut n = self.suffix.get(base).copied().unwrap_or(2);
        loop {
            let c = format!("{base}-{n}");
            n += 1;
            if !self.ids.contains(&c) {
                self.suffix.insert(base.to_string(), n);
                return c;
            }
        }
    }

    /// Places the waiting anchors as one empty span.
    fn flush_pending(&mut self) {
        if self.pending.is_empty() || self.mode != Mode::Body {
            return;
        }
        let keys = std::mem::take(&mut self.pending);
        let id = self.unique(&self.gen_id(keys[0]));
        self.cur.body.push_str("<span id=\"");
        escape(&mut self.cur.body, &id, true);
        self.cur.body.push_str("\"></span>");
        let doc = self.doc();
        for k in keys {
            if let Some(slot) = self.ctx.targets.get_mut(k) {
                *slot = Some((doc, Some(id.clone())));
            }
        }
        self.ids.insert(id);
        self.anchors_here += 1;
    }

    fn attributes(&mut self, tok: Token, spec: &Spec) -> Option<El> {
        let tag = spec.src;
        let mut out = String::new();
        let mut style = String::from(spec.style);
        let mut seen: Vec<String> = Vec::new();
        let (mut id, mut name_id, mut lang, mut dir, mut alt) = (None, None, None, None, None);
        let mut src_style = None;
        let mut urls: Vec<(String, String)> = Vec::new();
        let mut buf = [0u8; 32];
        for a in self.t.attrs_of(&tok) {
            let lname = lower(span(self.src, a.name), &mut buf);
            if lname.is_empty() || seen.iter().any(|s| s == lname) {
                continue;
            }
            seen.push(lname.to_string());
            let mut v = String::new();
            if let Some(r) = a.value {
                decode_into(&mut v, span(self.src, r), self.utf8, true);
            }
            match lname {
                "id" => id = valid_id(&v),
                "name" if tag == "a" => name_id = valid_id(&v),
                "class" => {
                    let c = collapse_ws(&v);
                    if !c.is_empty() {
                        push_attr(&mut out, "class", &c);
                    }
                }
                "style" => src_style = Some(v),
                "title" => push_attr(&mut out, "title", &v),
                "lang" | "xml:lang" => {
                    if lang.is_none() {
                        lang = valid_lang(&v);
                    }
                }
                "dir" => {
                    let d = v.trim().to_ascii_lowercase();
                    if matches!(d.as_str(), "ltr" | "rtl" | "auto") {
                        dir = Some(d);
                    }
                }
                "epub:type" => {
                    let c = collapse_ws(&v);
                    if !c.is_empty() {
                        push_attr(&mut out, "epub:type", &c);
                    }
                }
                "alt" if tag == "img" => alt = Some(v),
                "width" | "height" => dimension(tag, lname, &v, &mut out, &mut style),
                "colspan" | "rowspan" if matches!(tag, "td" | "th") => push_number(&mut out, lname, &v, false),
                "span" if matches!(tag, "col" | "colgroup") => push_number(&mut out, lname, &v, false),
                "start" if tag == "ol" => push_number(&mut out, lname, &v, true),
                "value" if tag == "li" => push_number(&mut out, lname, &v, true),
                "reversed" if tag == "ol" => push_attr(&mut out, "reversed", "reversed"),
                "type" if tag == "ol" && matches!(v.trim(), "1" | "a" | "A" | "i" | "I") => {
                    push_attr(&mut out, "type", v.trim())
                }
                "datetime" if matches!(tag, "time" | "del" | "ins") => push_attr(&mut out, "datetime", &v),
                "align" => align(tag, &v, &mut style),
                "valign" => {
                    let v = v.trim().to_ascii_lowercase();
                    if matches!(v.as_str(), "top" | "middle" | "bottom" | "baseline") {
                        add_style(&mut style, &format!("vertical-align: {v}"));
                    }
                }
                "bgcolor" => {
                    if let Some(c) = css_value(&v) {
                        add_style(&mut style, &format!("background-color: {c}"));
                    }
                }
                "color" if tag == "font" => {
                    if let Some(c) = css_value(&v) {
                        add_style(&mut style, &format!("color: {c}"));
                    }
                }
                "face" if tag == "font" => {
                    if let Some(c) = css_value(&v) {
                        add_style(&mut style, &format!("font-family: {c}"));
                    }
                }
                "size" if tag == "font" => {
                    if let Some(c) = font_size(&v) {
                        add_style(&mut style, &format!("font-size: {c}"));
                    }
                }
                "href" | "src" | "filepos" | "recindex" | "hirecindex" | "lorecindex" => {
                    urls.push((lname.to_string(), v));
                }
                _ => {}
            }
        }
        let mut link = None;
        let find = |urls: &[(String, String)], name: &str| urls.iter().find(|u| u.0 == name).map(|u| u.1.clone());
        match tag {
            "a" => {
                for want in ["filepos", "href"] {
                    let Some(v) = find(&urls, want) else { continue };
                    match self.hooks.url("a", want, &v) {
                        Url::Keep(s) => {
                            push_attr(&mut out, "href", &s);
                            break;
                        }
                        Url::Link(k) => {
                            link = Some(k);
                            break;
                        }
                        Url::Drop => {}
                    }
                }
            }
            "img" => {
                let mut found = None;
                for want in ["src", "recindex", "hirecindex", "lorecindex"] {
                    let Some(v) = find(&urls, want) else { continue };
                    if let Url::Keep(s) = self.hooks.url("img", want, &v) {
                        found = Some(s);
                        break;
                    }
                }
                let src = found?;
                push_attr(&mut out, "src", &src);
                push_attr(&mut out, "alt", alt.as_deref().unwrap_or(""));
            }
            _ => {}
        }
        if let Some(l) = lang {
            push_attr(&mut out, "lang", &l);
            push_attr(&mut out, "xml:lang", &l);
        }
        if let Some(d) = dir {
            push_attr(&mut out, "dir", &d);
        }
        if let Some(s) = src_style {
            let s = self.hooks.css(&s);
            let s = s.trim().trim_end_matches(';').trim();
            if !s.is_empty() && safe_css(s) {
                add_style(&mut style, s);
            }
        }
        if !style.is_empty() {
            push_attr(&mut out, "style", &style);
        }
        Some(El { id: id.or(name_id), attrs: out, link })
    }

    fn link(&mut self, tok: Token) {
        let rel = self.t.attr(self.src, &tok, b"rel").unwrap_or(b"");
        if !rel.split(|&b| is_ws(b)).any(|r| r.eq_ignore_ascii_case(b"stylesheet")) {
            return;
        }
        let Some(href) = self.t.attr(self.src, &tok, b"href") else { return };
        let mut v = String::new();
        decode_into(&mut v, href, self.utf8, true);
        if let Url::Keep(p) = self.hooks.url("link", "href", v.trim())
            && self.head_links.insert(p.clone())
        {
            self.cur.head.push_str("<link rel=\"stylesheet\" type=\"text/css\" href=\"");
            escape(&mut self.cur.head, &p, true);
            self.cur.head.push_str("\"/>\n");
        }
    }

    fn svg_start(&mut self, tok: Token, lname: &str) {
        let Some((lower_name, canon)) = svg_name(lname) else { return };
        let root = self.svg_depth == 0;
        if (root && lower_name != "svg") || self.stack.len() >= MAX_DEPTH {
            return;
        }
        let mut out = String::new();
        let mut id = None;
        let mut href = None;
        let mut seen: Vec<String> = Vec::new();
        let mut buf = [0u8; 32];
        for a in self.t.attrs_of(&tok) {
            let an = lower(span(self.src, a.name), &mut buf);
            if an.is_empty() || seen.iter().any(|s| s == an) {
                continue;
            }
            seen.push(an.to_string());
            let mut v = String::new();
            if let Some(r) = a.value {
                decode_into(&mut v, span(self.src, r), self.utf8, true);
            }
            match an {
                "id" => id = valid_id(&v),
                "href" | "xlink:href" => href = Some(v),
                "style" => {
                    let s = self.hooks.css(&v);
                    if safe_css(&s) && !s.trim().is_empty() {
                        push_attr(&mut out, "style", s.trim());
                    }
                }
                _ => {
                    if let Some(&(_, c)) = SVG_ATTRS.iter().find(|x| x.0 == an)
                        && safe_svg_value(&v)
                    {
                        push_attr(&mut out, c, &v);
                    }
                }
            }
        }
        match lower_name {
            "image" => {
                let Some(Url::Keep(p)) = href.map(|h| self.hooks.url("image", "xlink:href", &h)) else { return };
                push_attr(&mut out, "xlink:href", &p);
            }
            "use" => match href {
                Some(h) if h.starts_with('#') && valid_id(&h[1..]).is_some() => push_attr(&mut out, "xlink:href", &h),
                _ => return,
            },
            _ => {}
        }
        self.flush_pending();
        let mut id = id.filter(|i| !self.ids.contains(i));
        if !self.attach.is_empty() {
            let keys = std::mem::take(&mut self.attach);
            let chosen = match id.take() {
                Some(i) => i,
                None => self.unique(&self.gen_id(keys[0])),
            };
            let doc = self.doc();
            for &k in &keys {
                if let Some(slot) = self.ctx.targets.get_mut(k) {
                    *slot = Some((doc, Some(chosen.clone())));
                }
            }
            self.anchors_here += 1;
            id = Some(chosen);
        }
        let body = &mut self.cur.body;
        body.push('<');
        body.push_str(canon);
        if root {
            body.push_str(" xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\"");
        }
        if let Some(i) = &id {
            body.push_str(" id=\"");
            escape(body, i, true);
            body.push('"');
            self.ids.insert(i.clone());
        }
        body.push_str(&out);
        self.cur.svg = true;
        self.has_content = true;
        if tok.closed {
            body.push_str("/>");
            return;
        }
        body.push('>');
        self.push_open(Open { src: lower_name, out: canon, svg: true, block: false, reopen: String::new() });
        self.svg_depth += 1;
    }

    fn end(&mut self, tok: Token) {
        let mut buf = [0u8; 32];
        let lname = lower(self.t.name(self.src, &tok), &mut buf);
        if lname.is_empty() || lname.contains(':') {
            return;
        }
        if self.svg_depth > 0
            && let Some((lower_name, _)) = svg_name(lname)
        {
            if let Some(k) = self.top(lower_name).filter(|&k| self.stack[k].svg) {
                self.pop_to(k);
            }
            return;
        }
        match lname {
            "body" | "html" | "head" => return,
            "br" => {
                if self.mode == Mode::Body {
                    self.cur.body.push_str("<br/>");
                }
                return;
            }
            _ => {}
        }
        let Some(spec) = spec(lname) else { return };
        if spec.flags & VOID != 0 {
            return;
        }
        // The nearest open element of that name, unless a boundary is open
        // above it: a stray inline end tag never closes a block, and only
        // table end tags reach out of a table cell.
        let Some(t) = self.top(spec.src) else { return };
        let above = |names: &[&str]| names.iter().filter_map(|n| self.top(n)).any(|i| i > t);
        let inline = spec.flags & INLINE != 0;
        let table_part = spec.flags & TABLE_PART != 0;
        if (inline && self.blocks.last().is_some_and(|&b| b > t))
            || (!table_part && above(&["table", "td", "th", "caption"]))
            || (spec.src == "p" && above(&["marquee"]))
            || (table_part && spec.src != "table" && above(&["table"]))
        {
            return;
        }
        self.pop_to(t);
    }

    /// The stack index of the topmost open element named `src`.
    fn top(&self, src: &str) -> Option<usize> {
        self.positions.get(src).and_then(|v| v.last().copied())
    }

    fn push_open(&mut self, o: Open) {
        let i = self.stack.len();
        self.positions.entry(o.src).or_default().push(i);
        if o.block {
            self.blocks.push(i);
        }
        self.stack.push(o);
    }

    /// The implicit closes HTML applies before `spec` opens.
    fn auto_close(&mut self, spec: &Spec) {
        const CELL: &[&str] = &["table", "td", "th", "caption", "svg"];
        if spec.flags & CLOSES_P != 0 {
            self.close_in_scope(&["p"], &["table", "td", "th", "caption", "button", "object", "marquee", "svg"]);
        }
        match spec.src {
            "li" => self.close_in_scope(&["li"], &["ul", "ol", "table", "td", "th", "caption", "svg"]),
            "dt" | "dd" => self.close_in_scope(&["dt", "dd"], &["dl", "table", "td", "th", "caption", "svg"]),
            "tr" => self.close_in_scope(&["tr"], &["table", "thead", "tbody", "tfoot", "svg"]),
            "td" | "th" => self.close_in_scope(&["td", "th"], &["tr", "table", "svg"]),
            "thead" | "tbody" | "tfoot" | "caption" | "colgroup" => {
                self.close_in_scope(&["thead", "tbody", "tfoot", "caption", "colgroup"], &["table", "svg"])
            }
            "a" => self.close_in_scope(&["a"], CELL),
            "rt" | "rp" => self.close_in_scope(&["rt", "rp"], &["ruby", "svg"]),
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" if self.stack.last().is_some_and(|o| heading_level(o.src) > 0) => {
                let k = self.stack.len() - 1;
                self.pop_to(k);
            }
            _ => {}
        }
    }

    /// Closes the nearest open element named in `targets` (and every
    /// element above it), unless one of `stops` comes first.
    fn close_in_scope(&mut self, targets: &[&str], stops: &[&str]) {
        let Some(t) = targets.iter().filter_map(|n| self.top(n)).max() else { return };
        if stops.iter().filter_map(|n| self.top(n)).any(|s| s > t) {
            return;
        }
        self.pop_to(t);
    }

    /// Closes every open element above index `k` and `k` itself.
    fn pop_to(&mut self, k: usize) {
        while self.stack.len() > k {
            let Some(o) = self.stack.pop() else { break };
            if let Some(v) = self.positions.get_mut(o.src) {
                v.pop();
            }
            if o.block {
                self.blocks.pop();
            }
            self.cur.body.push_str("</");
            self.cur.body.push_str(o.out);
            self.cur.body.push('>');
            if o.svg {
                self.svg_depth = self.svg_depth.saturating_sub(1);
            }
            let depth = self.stack.len() + 1;
            while self.captures.last().is_some_and(|c| c.depth >= depth) {
                if let Some(c) = self.captures.pop() {
                    self.finish_capture(c);
                }
            }
        }
    }

    fn finish_capture(&mut self, c: Capture) {
        let mut text = collapse_ws(&c.text);
        if text.is_empty() {
            return;
        }
        if text.len() > 300 {
            let mut cut = 300;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
        }
        match c.kind {
            CapKind::Heading { level, id } => self.ctx.headings.push(Heading { level, doc: c.doc, id, text }),
            CapKind::Link { key } => self.ctx.links.push(LinkRec { key, doc: c.doc, text }),
        }
    }

    fn page_break(&mut self) {
        if self.has_content {
            self.split();
        }
    }

    fn size_split(&mut self) {
        if !self.has_content || self.svg_depth > 0 {
            return;
        }
        let len = self.cur.body.len();
        let soft = len >= SOFT_SPLIT
            && self.stack.len() <= 3
            && !self.stack.iter().any(|o| matches!(o.src, "table" | "pre" | "ul" | "ol" | "dl"));
        if soft || len >= HARD_SPLIT {
            self.split();
        }
    }

    /// Ends the current document and starts the next with the same open
    /// elements.
    fn split(&mut self) {
        for o in self.stack.iter().rev() {
            self.cur.body.push_str("</");
            self.cur.body.push_str(o.out);
            self.cur.body.push('>');
        }
        let next = OutDoc {
            title: self.cur.title.clone(),
            head: self.cur.head.clone(),
            body_attrs: self.cur.body_attrs.clone(),
            body: String::new(),
            svg: false,
        };
        let done = std::mem::replace(&mut self.cur, next);
        self.ctx.docs.push(done);
        for o in &self.stack {
            self.cur.body.push_str(&o.reopen);
        }
        self.ids.clear();
        self.suffix.clear();
        self.has_content = false;
        self.anchors_here = 0;
    }

    fn finish(mut self) {
        if !self.pending.is_empty() {
            self.mode = Mode::Body;
            self.flush_pending();
        }
        self.pop_to(0);
        let index = self.doc();
        let drop_tail =
            self.opts.drop_empty_tail && !self.has_content && self.anchors_here == 0 && index > self.first_doc;
        if drop_tail {
            self.ctx.headings.retain(|h| h.doc != index);
            self.ctx.links.retain(|l| l.doc != index);
        } else {
            let doc = std::mem::take(&mut self.cur);
            self.ctx.docs.push(doc);
        }
    }
}

fn heading_level(src: &str) -> u8 {
    match src {
        "h1" => 1,
        "h2" => 2,
        "h3" => 3,
        "h4" => 4,
        "h5" => 5,
        "h6" => 6,
        _ => 0,
    }
}

fn push_attr(out: &mut String, name: &str, value: &str) {
    out.push(' ');
    out.push_str(name);
    out.push_str("=\"");
    escape(out, value, true);
    out.push('"');
}

/// The marker a link leaves in the body until [`resolve_links`].
fn push_placeholder(out: &mut String, key: usize) {
    out.push('\u{1}');
    out.push_str(&key.to_string());
    out.push('\u{2}');
}

/// Replaces link markers with ` href="..."` (or nothing when `f` has
/// no target for the key).
pub(crate) fn resolve_links(body: String, mut f: impl FnMut(usize) -> Option<String>) -> String {
    if !body.contains('\u{1}') {
        return body;
    }
    let mut out = String::with_capacity(body.len() + body.len() / 16);
    let mut rest = body.as_str();
    while let Some(p) = rest.find('\u{1}') {
        out.push_str(&rest[..p]);
        let after = &rest[p + 1..];
        let Some(q) = after.find('\u{2}') else {
            rest = after;
            break;
        };
        if let Ok(k) = after[..q].parse::<usize>()
            && let Some(h) = f(k)
        {
            push_attr(&mut out, "href", &h);
        }
        rest = &after[q + 1..];
    }
    out.push_str(rest);
    out
}

fn add_style(style: &mut String, decl: &str) {
    if !style.is_empty() {
        style.push_str("; ");
    }
    style.push_str(decl);
}

fn valid_id(v: &str) -> Option<String> {
    let v = v.trim();
    if v.is_empty() || v.len() > 200 || v.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    Some(v.to_string())
}

fn valid_lang(v: &str) -> Option<String> {
    let v = v.trim();
    let ok = !v.is_empty()
        && v.len() <= 35
        && v.as_bytes()[0].is_ascii_alphabetic()
        && v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    ok.then(|| v.replace('_', "-"))
}

fn push_number(out: &mut String, name: &str, v: &str, signed: bool) {
    let v = v.trim();
    let digits = v.strip_prefix('-').filter(|_| signed).unwrap_or(v);
    if !digits.is_empty() && digits.len() <= 9 && digits.bytes().all(|b| b.is_ascii_digit()) {
        push_attr(out, name, v);
    }
}

/// A CSS value from an HTML attribute (colors, lengths, font names).
fn css_value(v: &str) -> Option<String> {
    let v = v.trim();
    let ok = !v.is_empty()
        && v.len() <= 64
        && v.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'#' | b'%' | b'.' | b',' | b'-' | b' '));
    ok.then(|| v.to_string())
}

/// A length with a unit (a bare number is in pixels).
fn length(v: &str) -> Option<String> {
    let v = css_value(v)?;
    let bare = v.bytes().all(|b| b.is_ascii_digit() || b == b'.' || b == b'-');
    Some(if bare { format!("{v}px") } else { v })
}

fn dimension(tag: &str, name: &str, v: &str, out: &mut String, style: &mut String) {
    let v = v.trim();
    match tag {
        "img" => {
            let digits = v.strip_suffix("px").unwrap_or(v);
            if !digits.is_empty() && digits.len() <= 6 && digits.bytes().all(|b| b.is_ascii_digit()) {
                push_attr(out, name, digits);
            } else if v.ends_with('%')
                && let Some(l) = css_value(v)
            {
                add_style(style, &format!("{name}: {l}"));
            }
        }
        "td" | "th" | "table" | "col" | "colgroup" | "hr" => {
            if let Some(l) = length(v) {
                add_style(style, &format!("{name}: {l}"));
            }
        }
        // MOBI's spacing attributes on blocks: height is the space above,
        // width the first-line indent.
        "p" | "div" | "blockquote" | "li" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "center" => {
            if let Some(l) = length(v) {
                let prop = if name == "height" { "margin-top" } else { "text-indent" };
                add_style(style, &format!("{prop}: {l}"));
            }
        }
        _ => {}
    }
}

fn align(tag: &str, v: &str, style: &mut String) {
    let v = v.trim().to_ascii_lowercase();
    match (tag, v.as_str()) {
        ("img", "left" | "right") => add_style(style, &format!("float: {v}")),
        ("img", "top" | "middle" | "bottom") => add_style(style, &format!("vertical-align: {v}")),
        ("table", "center") => add_style(style, "margin-left: auto; margin-right: auto"),
        ("table", "left" | "right") => add_style(style, &format!("float: {v}")),
        (_, "left" | "right" | "center" | "justify") => add_style(style, &format!("text-align: {v}")),
        _ => {}
    }
}

fn font_size(v: &str) -> Option<&'static str> {
    let v = v.trim();
    if v.starts_with('+') {
        return Some("larger");
    }
    if v.starts_with('-') {
        return Some("smaller");
    }
    Some(match v {
        "1" => "x-small",
        "2" => "small",
        "3" => "medium",
        "4" => "large",
        "5" => "x-large",
        "6" | "7" => "xx-large",
        _ => return None,
    })
}

/// CSS that cannot run code or fetch remote resources.
pub(crate) fn safe_css(css: &str) -> bool {
    let l = css.to_ascii_lowercase();
    if ["expression", "javascript:", "vbscript:", "-moz-binding", "behavior:", "@import"].iter().any(|b| l.contains(b))
    {
        return false;
    }
    let mut rest = l.as_str();
    while let Some(p) = rest.find("url(") {
        let target = rest[p + 4..].trim_start().trim_start_matches(['"', '\'']);
        if ["http:", "https:", "ftp:", "//", "data:", "file:"].iter().any(|s| target.starts_with(s)) {
            return false;
        }
        rest = &rest[p + 4..];
    }
    true
}

/// A stylesheet with what could run code or fetch resources outside
/// the book neutralized: `@import` rules removed, and every `url()`
/// whose target `keep` refuses emptied.
pub(crate) fn sanitize_css(css: &str, keep: &dyn Fn(&str) -> bool) -> String {
    const NEUTER: [&str; 5] = ["expression(", "javascript:", "vbscript:", "-moz-binding", "behavior:"];
    let lower = css.to_ascii_lowercase();
    let mut out = String::with_capacity(css.len());
    let mut i = 0;
    while i < css.len() {
        let rest = &lower[i..];
        if rest.starts_with("@import") {
            i += rest.find(';').map_or(rest.len(), |p| p + 1);
            continue;
        }
        if rest.starts_with("url(") {
            let (inner_end, close) = match rest.find(')') {
                Some(p) => (i + p, p + 1),
                None => (css.len(), rest.len()),
            };
            let inner = css.get(i + 4..inner_end).unwrap_or("");
            let target = inner.trim().trim_matches(['"', '\'']).trim();
            if keep(target) {
                out.push_str(&css[i..i + close]);
            } else {
                out.push_str("url()");
            }
            i += close;
            continue;
        }
        if NEUTER.iter().any(|n| rest.starts_with(n)) {
            out.push_str("x-");
        }
        let Some(c) = css[i..].chars().next() else { break };
        out.push(c);
        i += c.len_utf8();
    }
    out
}

fn safe_svg_value(v: &str) -> bool {
    let l = v.to_ascii_lowercase();
    !(l.contains("javascript:") || l.contains("vbscript:") || l.contains("data:"))
        && l.match_indices("url(")
            .all(|(p, _)| l[p + 4..].trim_start().trim_start_matches(['"', '\'']).starts_with('#'))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoHooks;

    impl Hooks for NoHooks {
        fn url(&mut self, tag: &str, attr: &str, value: &str) -> Url {
            match (tag, attr) {
                ("a", "href") if value.starts_with("http") => Url::Keep(value.to_string()),
                ("img", "src") => Url::Keep(value.to_string()),
                _ => Url::Drop,
            }
        }
    }

    fn balance(src: &str) -> Vec<String> {
        let t = tokenize(src.as_bytes());
        let mut ctx = Ctx::new(Vec::new());
        let opts = Options { split_pagebreaks: true, split_size: false, drop_empty_tail: true };
        emit(src.as_bytes(), true, &t, &[], &mut NoHooks, &opts, &mut ctx);
        ctx.docs.into_iter().map(|d| d.body).collect()
    }

    #[test]
    fn tag_soup_is_balanced() {
        assert_eq!(balance("<p>one<p>two"), ["<p>one</p><p>two</p>"]);
        assert_eq!(balance("<ul><li>a<li>b</ul>"), ["<ul><li>a</li><li>b</li></ul>"]);
        assert_eq!(balance("<b><i>x</b>y</i>"), ["<b><i>x</i></b>y"]);
        assert_eq!(balance("<div><span>x</div>"), ["<div><span>x</span></div>"]);
        assert_eq!(balance("a<br>b<hr>c"), ["a<br/>b<hr/>c"]);
        assert_eq!(balance("<p>x</span></p>"), ["<p>x</p>"]);
        assert_eq!(
            balance("<table><tr><td>1<td>2<tr><td>3</table>"),
            ["<table><tr><td>1</td><td>2</td></tr><tr><td>3</td></tr></table>"]
        );
        assert_eq!(
            balance("<p>a &amp; b &lt; c &copy; &#233; &#x41; &bogus; & x</p>"),
            ["<p>a &amp; b &lt; c \u{a9} \u{e9} A &amp;bogus; &amp; x</p>"]
        );
        assert_eq!(balance("<script>alert(1)</script><p onclick=\"x\">ok</p>"), ["<p>ok</p>"]);
        assert_eq!(balance("<foo:bar>kept</foo:bar><blink>b</blink><unknown>u</unknown>"), ["kept<span>b</span>u"]);
        assert_eq!(balance("<p align=center x=1 x=2>c</p>"), ["<p style=\"text-align: center\">c</p>"]);
        assert_eq!(
            balance("<a href=\"javascript:x\">j</a><a href=\"http://e/\">e</a>"),
            ["<a>j</a><a href=\"http://e/\">e</a>"]
        );
        assert_eq!(balance("<img src=\"a.png\"><img>"), ["<img src=\"a.png\" alt=\"\"/>"]);
        assert_eq!(balance("<p>\x01bad\u{1}</p>"), ["<p>bad</p>"]);
    }

    #[test]
    fn page_breaks_split_and_reopen() {
        let docs = balance("<div class=\"c\" id=\"d\"><p>one</p><mbp:pagebreak/><p>two</p></div><mbp:pagebreak/>");
        assert_eq!(docs, ["<div id=\"d\" class=\"c\"><p>one</p></div>", "<div class=\"c\"><p>two</p></div>"]);
        // A break before any content does not split.
        assert_eq!(balance("<mbp:pagebreak/><p>x</p>").len(), 1);
    }

    #[test]
    fn head_and_title() {
        let src = "<html><head><title>T &amp; U</title><style>p{color:red}</style><guide><reference type=\"toc\"/></guide></head><body class=\"b\"><p>x</p></body></html>";
        let t = tokenize(src.as_bytes());
        let mut ctx = Ctx::new(Vec::new());
        let opts = Options { split_pagebreaks: false, split_size: false, drop_empty_tail: false };
        emit(src.as_bytes(), true, &t, &[], &mut NoHooks, &opts, &mut ctx);
        let d = &ctx.docs[0];
        assert_eq!(d.title.as_deref(), Some("T & U"));
        assert!(d.head.contains("p{color:red}"));
        assert_eq!(d.body_attrs, " class=\"b\"");
        assert_eq!(d.body, "<p>x</p>");
    }

    #[test]
    fn tokenizer_edge_cases() {
        for src in ["<", "<a", "<a href=\"x", "</", "</a", "<!--", "<![CDATA[x", "<?x", "a < b", "<a b=c/>", "<p/>"] {
            let t = tokenize(src.as_bytes());
            let mut pos = 0;
            for tok in &t.toks {
                assert_eq!(tok.start, pos, "{src}");
                pos = tok.end;
            }
            assert_eq!(pos as usize, src.len(), "{src}");
            let _ = balance(src);
        }
    }

    #[test]
    fn svg_is_kept_with_namespaces() {
        let docs = balance(
            "<div><svg viewbox=\"0 0 1 1\" onload=\"x\"><image xlink:href=\"c.jpg\" width=\"1\"/><script>x</script></svg></div>",
        );
        assert_eq!(
            docs,
            [
                "<div><svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" viewBox=\"0 0 1 1\"></svg></div>"
            ]
        );
    }

    #[test]
    fn plain_text_of_a_description() {
        assert_eq!(plain_text(b"<p>A <b>bold</b>   claim &amp; more</p><p>Two</p>", true), "A bold claim & more\nTwo");
    }
}
