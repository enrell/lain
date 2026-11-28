//! Reflowable books (EPUB, MOBI, AZW3; D-303..D-309) for the native
//! reader: the server's sanitized chapter HTML turned into a flat list of
//! blocks the app can lay out, and the progress mapping between a place
//! in the book and the shared progress record (D-307).
//!
//! The parser is a small tolerant tokenizer, not an HTML engine: the
//! server already removed scripts, forms and foreign URLs (D-305) and
//! balanced the markup, so what is left is structure (headings,
//! paragraphs, lists, quotes, tables, images) and inline emphasis. Styles
//! are ignored, except a centered alignment. Bad markup never fails:
//! stray end tags are skipped and unclosed elements end with the chapter.

use std::collections::HashMap;

/// Characters in one virtual page (`BOOK_PAGE_CHARS` on the server).
pub const PAGE_CHARS: f64 = 1024.;

// Inline style bits of a span.
pub const BOLD: u8 = 1;
pub const ITALIC: u8 = 2;
pub const UNDERLINE: u8 = 4;
pub const STRIKE: u8 = 8;
pub const SUP: u8 = 16;
pub const SUB: u8 = 32;
pub const SMALL: u8 = 64;
pub const CODE: u8 = 128;

/// Where a link goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    /// Another chapter (`data-lain-chapter`), at an anchor or its start.
    Chapter(i64, String),
    /// An element of this chapter (`#id`).
    Anchor(String),
    /// A web address outside the book.
    External(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub style: u8,
    pub link: Option<Link>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// `h1`..`h6`.
    Heading(u8),
    Para,
    /// A list item's first block, with its marker (`•`, `3.`).
    Item(String),
    /// Preformatted text: whitespace kept.
    Pre,
    Rule,
    /// An image member of the book (decoded path) and its alt text.
    Image {
        src: String,
        alt: String,
    },
    /// A table row, cells separated in the text.
    Row,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub kind: Kind,
    pub spans: Vec<Span>,
    /// Nesting of quotes and lists.
    pub indent: u8,
    pub center: bool,
    /// Non-whitespace characters of the block's text (the server's rule).
    pub chars: usize,
}

impl Block {
    #[cfg(test)]
    pub fn text(&self) -> String {
        self.spans.iter().map(|s| s.text.as_str()).collect()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Doc {
    pub blocks: Vec<Block>,
    /// Element ids → the block they start at (link targets).
    pub anchors: HashMap<String, usize>,
    /// Non-whitespace characters of the body text, by the rule the server
    /// counts `BookChapter::chars` with (`text@1`).
    pub chars: usize,
}

impl Doc {
    /// The block an anchor points at, if the chapter has it.
    pub fn anchor(&self, id: &str) -> Option<usize> {
        self.anchors.get(id).map(|&i| i.min(self.blocks.len().saturating_sub(1)))
    }
}

// ---- tokenizer ------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Start { name: String, attrs: Vec<(String, String)>, closed: bool },
    End(String),
    Text(String),
}

/// Elements whose content is not markup; skipped whole.
const RAW: &[&str] = &["style", "script", "textarea", "title", "noscript", "template"];

fn local(name: &str) -> String {
    name.rsplit(':').next().unwrap_or(name).to_ascii_lowercase()
}

fn find(hay: &str, from: usize, needle: &str) -> Option<usize> {
    hay.get(from..)?.find(needle).map(|p| p + from)
}

fn tokenize(src: &str) -> Vec<Tok> {
    let b = src.as_bytes();
    // Same byte offsets as `src`: used for case-insensitive end tags.
    let lower = src.to_ascii_lowercase();
    let mut out = Vec::new();
    let (mut i, mut text_at) = (0, 0);
    let flush = |out: &mut Vec<Tok>, from: usize, to: usize| {
        if to > from {
            out.push(Tok::Text(decode(&src[from..to])));
        }
    };
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &src[i..];
        if rest.starts_with("<!--") {
            flush(&mut out, text_at, i);
            i = find(src, i + 4, "-->").map_or(b.len(), |p| p + 3);
            text_at = i;
            continue;
        }
        if rest.starts_with("<![CDATA[") {
            flush(&mut out, text_at, i);
            let end = find(src, i + 9, "]]>").unwrap_or(b.len());
            out.push(Tok::Text(src[i + 9..end].to_string()));
            i = (end + 3).min(b.len());
            text_at = i;
            continue;
        }
        if rest.starts_with("<!") || rest.starts_with("<?") {
            flush(&mut out, text_at, i);
            i = find(src, i, ">").map_or(b.len(), |p| p + 1);
            text_at = i;
            continue;
        }
        let end_tag = b.get(i + 1) == Some(&b'/');
        let name_at = if end_tag { i + 2 } else { i + 1 };
        // A `<` that starts no tag is text.
        if !b.get(name_at).is_some_and(|c| c.is_ascii_alphabetic()) {
            i += 1;
            continue;
        }
        flush(&mut out, text_at, i);
        let mut j = name_at;
        while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'/' && b[j] != b'>' {
            j += 1;
        }
        let name = local(&src[name_at..j]);
        let mut attrs = Vec::new();
        let mut closed = false;
        while j < b.len() {
            let c = b[j];
            if c.is_ascii_whitespace() {
                j += 1;
                continue;
            }
            if c == b'>' {
                j += 1;
                break;
            }
            if c == b'/' {
                closed = true;
                j += 1;
                continue;
            }
            closed = false;
            let k = j;
            while j < b.len() && !b[j].is_ascii_whitespace() && !matches!(b[j], b'=' | b'>' | b'/') {
                j += 1;
            }
            if j == k {
                // A lone `=`: skip it.
                j += 1;
                continue;
            }
            let key = src[k..j].to_ascii_lowercase();
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            let mut value = String::new();
            if b.get(j) == Some(&b'=') {
                j += 1;
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                match b.get(j) {
                    Some(&q @ (b'"' | b'\'')) => {
                        let end = src[j + 1..].find(q as char).map_or(b.len(), |p| p + j + 1);
                        value = decode(&src[j + 1..end]);
                        j = (end + 1).min(b.len());
                    }
                    _ => {
                        let v = j;
                        while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'>' {
                            j += 1;
                        }
                        value = decode(&src[v..j]);
                    }
                }
            }
            attrs.push((key, value));
        }
        i = j;
        text_at = j;
        if end_tag {
            out.push(Tok::End(name));
            continue;
        }
        if !closed && RAW.contains(&name.as_str()) {
            // Skip to the matching end tag; nothing inside is shown.
            let close = format!("</{name}");
            i = find(&lower, j, &close).map_or(b.len(), |p| find(src, p, ">").map_or(b.len(), |q| q + 1));
            text_at = i;
            continue;
        }
        out.push(Tok::Start { name, attrs, closed });
    }
    flush(&mut out, text_at, b.len());
    out
}

/// Decodes character references; an unknown one stays as written.
pub fn decode(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let semi = rest[1..].find(';').map(|p| p + 1).filter(|&p| p <= 32);
        let decoded = semi.and_then(|p| {
            let name = &rest[1..p];
            let c = if let Some(num) = name.strip_prefix('#') {
                let n = match num.strip_prefix(['x', 'X']) {
                    Some(h) => u32::from_str_radix(h, 16).ok(),
                    None => num.parse::<u32>().ok(),
                }?;
                // A reference to nothing (0, surrogates, past U+10FFFF)
                // reads as the replacement character, as browsers do.
                char::from_u32(n).filter(|&c| c != '\0').unwrap_or('\u{fffd}')
            } else {
                named(name)?
            };
            Some((c, p + 1))
        });
        match decoded {
            Some((c, len)) => {
                out.push(c);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn named(n: &str) -> Option<char> {
    Some(match n {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => '\u{a0}',
        "ensp" => '\u{2002}',
        "emsp" => '\u{2003}',
        "thinsp" => '\u{2009}',
        "zwnj" => '\u{200c}',
        "zwj" => '\u{200d}',
        "lrm" => '\u{200e}',
        "rlm" => '\u{200f}',
        "shy" => '\u{ad}',
        "ndash" => '–',
        "mdash" => '—',
        "hellip" => '…',
        "lsquo" => '‘',
        "rsquo" => '’',
        "sbquo" => '‚',
        "ldquo" => '“',
        "rdquo" => '”',
        "bdquo" => '„',
        "laquo" => '«',
        "raquo" => '»',
        "lsaquo" => '‹',
        "rsaquo" => '›',
        "bull" => '•',
        "middot" => '·',
        "dagger" => '†',
        "Dagger" => '‡',
        "permil" => '‰',
        "prime" => '′',
        "Prime" => '″',
        "copy" => '©',
        "reg" => '®',
        "trade" => '™',
        "sect" => '§',
        "para" => '¶',
        "deg" => '°',
        "plusmn" => '±',
        "times" => '×',
        "divide" => '÷',
        "minus" => '−',
        "frac12" => '½',
        "frac14" => '¼',
        "frac34" => '¾',
        "sup1" => '¹',
        "sup2" => '²',
        "sup3" => '³',
        "iexcl" => '¡',
        "iquest" => '¿',
        "cent" => '¢',
        "pound" => '£',
        "yen" => '¥',
        "euro" => '€',
        "larr" => '←',
        "rarr" => '→',
        "uarr" => '↑',
        "darr" => '↓',
        "aacute" => 'á',
        "eacute" => 'é',
        "iacute" => 'í',
        "oacute" => 'ó',
        "uacute" => 'ú',
        "agrave" => 'à',
        "egrave" => 'è',
        "acirc" => 'â',
        "ecirc" => 'ê',
        "ocirc" => 'ô',
        "atilde" => 'ã',
        "otilde" => 'õ',
        "ntilde" => 'ñ',
        "ccedil" => 'ç',
        "auml" => 'ä',
        "ouml" => 'ö',
        "uuml" => 'ü',
        "szlig" => 'ß',
        _ => return None,
    })
}

/// `%XX` escapes decoded (the server percent-encodes resource paths).
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(h) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(h);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The book member a resource URL names (`/api/items/{id}/book/res/<path>`).
fn member(url: &str) -> Option<String> {
    let at = url.find("/book/res/")? + "/book/res/".len();
    let tail = url[at..].split(['?', '#']).next().unwrap_or("");
    (!tail.is_empty()).then(|| percent_decode(tail))
}

// ---- blocks -----------------------------------------------------------------

/// Elements that start and end a block.
const BLOCKS: &[&str] = &[
    "p",
    "div",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "li",
    "ul",
    "ol",
    "dl",
    "dt",
    "dd",
    "blockquote",
    "pre",
    "section",
    "article",
    "aside",
    "header",
    "footer",
    "nav",
    "main",
    "figure",
    "figcaption",
    "table",
    "thead",
    "tbody",
    "tfoot",
    "tr",
    "caption",
    "address",
    "center",
    "details",
    "summary",
    "body",
    "html",
    "svg",
    "math",
];

/// Elements that never have content.
const VOID: &[&str] = &["br", "hr", "img", "wbr", "col", "source", "track", "meta", "link", "input", "area", "param"];

/// Text between table cells.
const CELL_GAP: &str = "  |  ";

struct Open {
    name: String,
    style: u8,
    link: Option<Link>,
    center: bool,
    /// `ol`/`ul`: ordered, and the next number.
    list: Option<(bool, i64)>,
}

#[derive(Default)]
struct Builder {
    doc: Doc,
    stack: Vec<Open>,
    cur: Option<Block>,
    marker: Option<String>,
    pending: Vec<String>,
    /// Inside `<head>`: nothing there is text.
    head: usize,
    /// A new table cell began: the separator goes in before its text, so
    /// an empty last cell leaves none behind.
    gap: bool,
}

fn attr<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn centered(attrs: &[(String, String)]) -> bool {
    if attr(attrs, "align").is_some_and(|a| a.eq_ignore_ascii_case("center")) {
        return true;
    }
    attr(attrs, "style").is_some_and(|s| {
        let s: String = s.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase();
        s.contains("text-align:center")
    })
}

fn style_of(name: &str) -> u8 {
    match name {
        "b" | "strong" => BOLD,
        "i" | "em" | "cite" | "dfn" | "var" | "q" => ITALIC,
        "u" | "ins" => UNDERLINE,
        "s" | "strike" | "del" => STRIKE,
        "sup" => SUP,
        "sub" => SUB,
        "small" => SMALL,
        "code" | "kbd" | "samp" | "tt" => CODE,
        "th" => BOLD,
        _ => 0,
    }
}

fn link_of(attrs: &[(String, String)]) -> Option<Link> {
    if let Some(n) = attr(attrs, "data-lain-chapter").and_then(|n| n.trim().parse::<i64>().ok()) {
        return Some(Link::Chapter(n, attr(attrs, "data-lain-anchor").unwrap_or("").to_string()));
    }
    let href = attr(attrs, "href")?.trim();
    if let Some(a) = href.strip_prefix('#') {
        return (!a.is_empty()).then(|| Link::Anchor(percent_decode(a)));
    }
    let lower = href.to_ascii_lowercase();
    (lower.starts_with("http://") || lower.starts_with("https://")).then(|| Link::External(href.to_string()))
}

impl Builder {
    fn in_stack(&self, name: &str) -> bool {
        self.stack.iter().any(|o| o.name == name)
    }

    fn indent(&self) -> u8 {
        self.stack.iter().filter(|o| matches!(o.name.as_str(), "blockquote" | "ul" | "ol" | "dd")).count().min(8) as u8
    }

    fn begin(&mut self) -> &mut Block {
        if self.cur.is_none() {
            let heading = self.stack.iter().rev().find_map(|o| match o.name.as_bytes() {
                [b'h', d @ b'1'..=b'6'] => Some(d - b'0'),
                _ => None,
            });
            let kind = if self.in_stack("pre") {
                Kind::Pre
            } else if let Some(h) = heading {
                Kind::Heading(h)
            } else if let Some(m) = self.marker.take() {
                Kind::Item(m)
            } else if self.in_stack("tr") {
                Kind::Row
            } else {
                Kind::Para
            };
            let center = self.stack.iter().any(|o| o.center);
            let at = self.doc.blocks.len();
            for a in self.pending.drain(..) {
                self.doc.anchors.entry(a).or_insert(at);
            }
            self.cur = Some(Block { kind, spans: Vec::new(), indent: self.indent(), center, chars: 0 });
        }
        self.cur.as_mut().expect("just set")
    }

    /// Ends the block being built; an empty one is dropped and its
    /// anchors move to the next block.
    fn flush(&mut self) {
        self.gap = false;
        let Some(mut b) = self.cur.take() else { return };
        if b.kind != Kind::Pre {
            while let Some(last) = b.spans.last_mut() {
                let t = last.text.trim_end_matches([' ', '\n']).len();
                last.text.truncate(t);
                if last.text.is_empty() {
                    b.spans.pop();
                } else {
                    break;
                }
            }
        }
        if b.spans.iter().all(|s| s.text.trim().is_empty()) {
            let at = self.doc.blocks.len();
            let moved: Vec<String> =
                self.doc.anchors.iter().filter(|(_, i)| **i == at).map(|(k, _)| k.clone()).collect();
            for k in moved {
                self.doc.anchors.remove(&k);
                self.pending.push(k);
            }
            return;
        }
        self.doc.blocks.push(b);
    }

    fn push_block(&mut self, kind: Kind) {
        self.flush();
        let at = self.doc.blocks.len();
        for a in self.pending.drain(..) {
            self.doc.anchors.entry(a).or_insert(at);
        }
        let center = self.stack.iter().any(|o| o.center);
        self.doc.blocks.push(Block { kind, spans: Vec::new(), indent: self.indent(), center, chars: 0 });
    }

    /// Appends text with the open elements' style; `counted` is false for
    /// the app's own separators.
    fn append(&mut self, text: &str, counted: bool) {
        let style = self.stack.iter().fold(0, |s, o| s | o.style);
        let link = self.stack.iter().rev().find_map(|o| o.link.clone());
        let pre = self.in_stack("pre");
        let fresh = self.cur.is_none();
        let gap = self.gap && counted && !text.trim().is_empty();
        if gap {
            self.gap = false;
        }
        let b = self.begin();
        let mut t = text.to_string();
        if !pre {
            let ends_blank = fresh || b.spans.last().is_none_or(|s| s.text.ends_with(' ') || s.text.ends_with('\n'));
            if ends_blank {
                t = t.trim_start_matches(' ').to_string();
            }
        }
        if t.is_empty() {
            return;
        }
        if gap && !b.spans.is_empty() {
            if let Some(last) = b.spans.last_mut() {
                let keep = last.text.trim_end_matches(' ').len();
                last.text.truncate(keep);
            }
            b.spans.retain(|s| !s.text.is_empty());
            b.spans.push(Span { text: CELL_GAP.to_string(), style: 0, link: None });
            t = t.trim_start_matches(' ').to_string();
        }
        if counted {
            b.chars += t.chars().filter(|c| !c.is_whitespace()).count();
        }
        match b.spans.last_mut() {
            Some(s) if s.style == style && s.link == link => s.text.push_str(&t),
            _ => b.spans.push(Span { text: t, style, link }),
        }
    }

    fn text(&mut self, raw: &str) {
        if self.head > 0 {
            return;
        }
        self.doc.chars += raw.chars().filter(|c| !c.is_whitespace()).count();
        if self.in_stack("pre") {
            let t = raw.replace("\r\n", "\n");
            // The newline right after `<pre>` is not content.
            let t = if self.cur.is_none() { t.strip_prefix('\n').unwrap_or(&t).to_string() } else { t };
            if !t.is_empty() {
                self.append(&t, true);
            }
            return;
        }
        // Collapse runs of document whitespace (not U+00A0) to one space.
        let mut t = String::with_capacity(raw.len());
        let mut blank = false;
        for c in raw.chars() {
            if matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c') {
                if !blank {
                    t.push(' ');
                }
                blank = true;
            } else {
                t.push(c);
                blank = false;
            }
        }
        if self.cur.is_none() && t.trim_matches(' ').is_empty() {
            return;
        }
        self.append(&t, true);
    }

    fn start(&mut self, name: String, attrs: Vec<(String, String)>, closed: bool) {
        if name == "head" {
            if !closed {
                self.head += 1;
            }
            return;
        }
        if self.head > 0 {
            return;
        }
        for key in ["id", "name"] {
            if let Some(id) = attr(&attrs, key).filter(|v| !v.is_empty())
                && (key == "id" || name == "a")
            {
                self.pending.push(id.to_string());
            }
        }
        match name.as_str() {
            "br" => {
                if self.cur.is_some() {
                    self.append("\n", false);
                }
                return;
            }
            "hr" => {
                self.push_block(Kind::Rule);
                return;
            }
            "img" | "image" => {
                let src = attr(&attrs, "src").or_else(|| attr(&attrs, "href")).or_else(|| attr(&attrs, "xlink:href"));
                if let Some(src) = src.and_then(member) {
                    let alt = attr(&attrs, "alt").unwrap_or("").trim().to_string();
                    self.push_block(Kind::Image { src, alt });
                }
                if name == "img" || closed {
                    return;
                }
            }
            "td" | "th" => {
                self.gap = self.cur.as_ref().is_some_and(|b| !b.spans.is_empty());
            }
            "li" => {
                self.flush();
                let marker = self
                    .stack
                    .iter_mut()
                    .rev()
                    .find_map(|o| o.list.as_mut())
                    .map(|(ordered, next)| {
                        if *ordered {
                            *next += 1;
                            format!("{}.", *next - 1)
                        } else {
                            "•".to_string()
                        }
                    })
                    .unwrap_or_else(|| "•".to_string());
                self.marker = Some(marker);
            }
            _ => {}
        }
        if BLOCKS.contains(&name.as_str()) && name != "li" {
            self.flush();
        }
        if closed || VOID.contains(&name.as_str()) {
            return;
        }
        let list = match name.as_str() {
            "ol" => Some((true, attr(&attrs, "start").and_then(|s| s.trim().parse().ok()).unwrap_or(1))),
            "ul" => Some((false, 0)),
            _ => None,
        };
        let link = if name == "a" { link_of(&attrs) } else { None };
        let center = centered(&attrs);
        // A deep stack is malformed input, not a book: stop nesting.
        if self.stack.len() < 512 {
            self.stack.push(Open { style: style_of(&name), name, link, center, list });
        }
    }

    fn end(&mut self, name: &str) {
        if name == "head" {
            self.head = self.head.saturating_sub(1);
            return;
        }
        if self.head > 0 {
            return;
        }
        // Close up to the matching open element; a stray end is ignored.
        let Some(at) = self.stack.iter().rposition(|o| o.name == name) else { return };
        if self.stack[at..].iter().any(|o| BLOCKS.contains(&o.name.as_str())) {
            self.flush();
        }
        self.stack.truncate(at);
        if name == "li" {
            self.marker = None;
        }
    }
}

/// Parses one sanitized chapter document into blocks.
pub fn parse(html: &str) -> Doc {
    let mut b = Builder::default();
    for t in tokenize(html) {
        match t {
            Tok::Start { name, attrs, closed } => b.start(name, attrs, closed),
            Tok::End(name) => b.end(&name),
            Tok::Text(t) => b.text(&t),
        }
    }
    b.flush();
    let last = b.doc.blocks.len().saturating_sub(1);
    for a in std::mem::take(&mut b.pending) {
        b.doc.anchors.entry(a).or_insert(last);
    }
    b.doc
}

// ---- progress (D-307), as web/src/lib/reader/book.ts --------------------------

/// The progress record for a place in the book.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BookProgress {
    /// The 1-based virtual page, fractional.
    pub position: f64,
    /// The book's virtual pages.
    pub duration: f64,
    pub completed: bool,
}

/// Characters before each chapter, and the total at the end.
pub fn offsets(chars: &[i64]) -> Vec<i64> {
    let mut out = Vec::with_capacity(chars.len() + 1);
    out.push(0);
    for &c in chars {
        let last = *out.last().unwrap_or(&0);
        out.push(last + c.max(0));
    }
    out
}

/// Virtual pages of the whole book (at least one).
pub fn total_pages(chars: &[i64]) -> f64 {
    let total = *offsets(chars).last().unwrap_or(&0) as f64;
    (total / PAGE_CHARS).ceil().max(1.)
}

/// `progressAt`: chapter and fraction (0..1) → the progress record. The
/// last page of the last chapter finishes the book.
pub fn progress_at(chapter: usize, fraction: f64, chars: &[i64]) -> BookProgress {
    let off = offsets(chars);
    let n = chars.len();
    let ch = chapter.min(n.saturating_sub(1));
    let f = if fraction.is_finite() { fraction.clamp(0., 1.) } else { 0. };
    let at = off.get(ch).copied().unwrap_or(0) as f64 + f * chars.get(ch).copied().unwrap_or(0).max(0) as f64;
    let pages = total_pages(chars);
    let position = (1. + at / PAGE_CHARS).min(pages);
    let completed = n > 0 && ch == n - 1 && f >= 0.98;
    BookProgress { position: (position * 1000.).round() / 1000., duration: pages, completed }
}

/// `positionOf`: a progress record → chapter and fraction. A finished or
/// unread book opens at the start.
pub fn position_of(position: f64, completed: bool, chars: &[i64]) -> (usize, f64) {
    if completed || !position.is_finite() || position <= 1. || chars.is_empty() {
        return (0, 0.);
    }
    let off = offsets(chars);
    let at = (position - 1.) * PAGE_CHARS;
    for (i, &c) in chars.iter().enumerate() {
        let len = c.max(0) as f64;
        if at < off[i] as f64 + len || i == chars.len() - 1 {
            if len == 0. {
                continue;
            }
            // A stored position is rounded to 1/1000 page (about half a
            // character), so a chapter's start can read as the last
            // character of the one before: that place is the next start.
            if off[i] as f64 + len - at < 0.75
                && let Some(next) = (i + 1..chars.len()).find(|&j| chars[j] > 0)
            {
                return (next, 0.);
            }
            return (i, ((at - off[i] as f64) / len).clamp(0., 1.));
        }
    }
    (chars.len() - 1, 1.)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(d: &Doc) -> Vec<String> {
        d.blocks.iter().map(Block::text).collect()
    }

    #[test]
    fn structure_and_entities() {
        let d = parse(
            "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><style>p{color:red}</style></head>\
             <body><h1 style=\"text-align: center\">Caf&eacute; &amp; Co</h1>\
             <p>One&nbsp;two &#x2014; &#8220;three&#8221; &bogus; &amp</p>\
             <blockquote><p>Quoted</p></blockquote><hr/><pre>  a\n  b</pre></body></html>",
        );
        assert_eq!(texts(&d), ["Café & Co", "One\u{a0}two — “three” &bogus; &amp", "Quoted", "", "  a\n  b"]);
        assert_eq!(d.blocks[0].kind, Kind::Heading(1));
        assert!(d.blocks[0].center);
        assert_eq!(d.blocks[2].indent, 1);
        assert_eq!(d.blocks[3].kind, Kind::Rule);
        assert_eq!(d.blocks[4].kind, Kind::Pre);
    }

    #[test]
    fn inline_styles_nest_and_whitespace_collapses() {
        let d = parse("<p>  plain <b>bold <i>both</i></b>\n\n <code>x</code><br/>next </p>");
        let b = &d.blocks[0];
        assert_eq!(b.text(), "plain bold both x\nnext");
        let styles: Vec<(String, u8)> = b.spans.iter().map(|s| (s.text.clone(), s.style)).collect();
        assert_eq!(styles[1], ("bold ".into(), BOLD));
        assert_eq!(styles[2], ("both".into(), BOLD | ITALIC));
        assert!(styles.iter().any(|(t, s)| t == "x" && *s == CODE));
    }

    #[test]
    fn lists_number_and_nest() {
        let d = parse("<ol start=\"3\"><li>a</li><li>b<ul><li>c</li></ul></li></ol><ul><li><p>d</p><p>e</p></li></ul>");
        let kinds: Vec<&Kind> = d.blocks.iter().map(|b| &b.kind).collect();
        assert_eq!(kinds[0], &Kind::Item("3.".into()));
        assert_eq!(kinds[1], &Kind::Item("4.".into()));
        assert_eq!(kinds[2], &Kind::Item("•".into()));
        assert_eq!(d.blocks[2].indent, 2);
        assert_eq!(kinds[3], &Kind::Item("•".into()));
        assert_eq!(kinds[4], &Kind::Para);
        assert_eq!(texts(&d), ["a", "b", "c", "d", "e"]);
    }

    #[test]
    fn malformed_markup_never_fails() {
        let d = parse("<p>a < b </i> and <b>open<p>next</div>x<!-- never closed");
        assert_eq!(texts(&d), ["a < b and open", "nextx"]);
        assert_eq!(d.blocks[1].spans[0].style, BOLD);
        // Garbage, truncation and lone brackets.
        for s in ["<", "<p", "<a href=\"x", "</", "<p>&#xZZ;&#99999999;", "<![CDATA[x", "\u{0}<\u{feff}>", "<p id=>"] {
            let _ = parse(s);
        }
        assert_eq!(texts(&parse("<p>&#99999999;&#0;</p>")), ["\u{fffd}\u{fffd}"]);
    }

    #[test]
    fn images_and_svg_images() {
        let d = parse(
            "<p>before<img src=\"/api/items/i1/book/res/OEBPS/images/a%20b.jpg\" alt=\" Cover \"/>after</p>\
             <svg><image href=\"/api/items/i1/book/res/OEBPS/c.png\"/></svg><img src=\"https://x/y.png\">",
        );
        assert_eq!(d.blocks.len(), 4);
        assert_eq!(d.blocks[1].kind, Kind::Image { src: "OEBPS/images/a b.jpg".into(), alt: "Cover".into() });
        assert_eq!(texts(&d)[0], "before");
        assert_eq!(texts(&d)[2], "after");
        assert_eq!(d.blocks[3].kind, Kind::Image { src: "OEBPS/c.png".into(), alt: String::new() });
    }

    #[test]
    fn links_and_anchors() {
        let d = parse(
            "<p id=\"top\">x <a href=\"#n%201\" data-lain-chapter=\"4\" data-lain-anchor=\"n 1\">four</a> \
             <a href=\"#here\">here</a> <a href=\"https://example.org/\" target=\"_blank\">web</a></p>\
             <div id=\"empty\"></div><h2 id=\"here\">Here</h2><a id=\"end\"/>",
        );
        let links: Vec<Option<Link>> = d.blocks[0].spans.iter().map(|s| s.link.clone()).collect();
        assert!(links.contains(&Some(Link::Chapter(4, "n 1".into()))));
        assert!(links.contains(&Some(Link::Anchor("here".into()))));
        assert!(links.contains(&Some(Link::External("https://example.org/".into()))));
        assert_eq!(d.anchor("top"), Some(0));
        // An empty element's id lands on the next block.
        assert_eq!(d.anchor("empty"), Some(1));
        assert_eq!(d.anchor("here"), Some(1));
        assert_eq!(d.anchor("end"), Some(1));
        assert_eq!(d.anchor("missing"), None);
    }

    #[test]
    fn tables_flatten_to_rows() {
        let d = parse("<table><tr><th>A</th>\n<th> B</th><td> </td></tr><tr><td>1</td><td><p>2</p></td></tr></table>");
        assert_eq!(d.blocks[0].kind, Kind::Row);
        assert_eq!(d.blocks[0].text(), "A  |  B");
        assert_eq!(d.blocks[0].spans[0].style, BOLD);
        assert_eq!(d.blocks[1].text(), "1");
        // The separator is the app's, not the book's: not counted.
        assert_eq!(d.blocks[0].chars, 2);
    }

    #[test]
    fn characters_count_like_the_server() {
        // text@1: non-whitespace characters of body text; the head and
        // dropped elements do not count, entities count once decoded.
        let d = parse(
            "<html><head><title>Not me</title><style>p{}</style></head><body>\
             <p>Ab c&amp;d</p>\n<p> e\u{a0}f </p><pre> g h </pre><script>no</script></body></html>",
        );
        assert_eq!(d.chars, 9);
        assert_eq!(d.blocks.iter().map(|b| b.chars).sum::<usize>(), 9);
    }

    #[test]
    fn the_servers_chapters_count_as_its_manifest() {
        // Chapters 0 and 1 of the read fixture's EPUB as the server sends
        // them (conformance/fixtures/read); its manifest says 38 and 7.
        let head = "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" \
                    content=\"default-src 'none'; img-src 'self' data:\">";
        let one = format!(
            "{head}<style>body {{ background: url(\"/api/items/item-read-epub/book/res/OEBPS/cover.png\") }} \
             p {{ margin: 0 }}</style></head><body><h1>One</h1><p>The first page of a tiny book. \
             <a href=\"#end\" data-lain-chapter=\"1\" data-lain-anchor=\"end\">On to the end.</a></p>\
             <img src=\"/api/items/item-read-epub/book/res/OEBPS/cover.png\" alt=\"cover\"></body></html>"
        );
        let d = parse(&one);
        assert_eq!(d.chars, 38);
        assert_eq!(texts(&d)[..2], ["One", "The first page of a tiny book. On to the end."]);
        assert_eq!(d.blocks[1].spans[1].link, Some(Link::Chapter(1, "end".into())));
        assert_eq!(d.blocks[2].kind, Kind::Image { src: "OEBPS/cover.png".into(), alt: "cover".into() });
        let two = parse(&format!("{head}</head><body><p id=\"end\">The end.</p></body></html>"));
        assert_eq!((two.chars, two.anchor("end")), (7, Some(0)));
    }

    #[test]
    fn progress_round_trips() {
        let chars = [2048, 0, 1024, 512];
        assert_eq!(total_pages(&chars), 4.);
        let p = progress_at(0, 0.5, &chars);
        assert_eq!(p, BookProgress { position: 2., duration: 4., completed: false });
        assert_eq!(position_of(p.position, p.completed, &chars), (0, 0.5));
        let p = progress_at(2, 0.25, &chars);
        assert_eq!(p.position, 3.25);
        assert_eq!(position_of(p.position, false, &chars), (2, 0.25));
        // An empty chapter is skipped: its place belongs to the next.
        assert_eq!(position_of(3., false, &chars), (2, 0.));
        // A chapter start rounded down still opens that chapter.
        let odd = [4486, 4371, 4377];
        let start = progress_at(2, 0., &odd);
        assert_eq!(start.position, 9.649);
        assert_eq!(position_of(start.position, false, &odd), (2, 0.));
        // The end of the last chapter finishes the book.
        let end = progress_at(3, 1., &chars);
        assert!(end.completed);
        // The page count is rounded up, so the stored end is the last
        // page's start: resuming it lands in the last chapter.
        assert_eq!(end.position, 4.);
        assert_eq!(position_of(end.position, false, &chars).0, 3);
        // Unread, finished and odd records open at the start.
        assert_eq!(position_of(1., false, &chars), (0, 0.));
        assert_eq!(position_of(3., true, &chars), (0, 0.));
        assert_eq!(position_of(f64::NAN, false, &chars), (0, 0.));
        assert_eq!(position_of(99., false, &chars), (3, 1.));
        assert_eq!(position_of(2., false, &[]), (0, 0.));
        // Out-of-range input clamps instead of failing.
        assert_eq!(progress_at(9, 7., &chars).position, 4.);
        assert_eq!(progress_at(0, f64::NAN, &[]), BookProgress { position: 1., duration: 1., completed: false });
    }
}
