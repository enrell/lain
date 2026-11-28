//! Markup events over the tolerant tokenizer the MOBI converter uses:
//! tags with decoded, lower-cased names and attribute values, and
//! decoded text. One reader for OPF, NCX, nav documents and chapters.

use crate::mobi::html::{self, CDATA, END, RAW, START, TEXT};

#[derive(Debug, Clone, PartialEq)]
pub enum Ev {
    Start {
        name: String,
        attrs: Vec<(String, String)>,
        closed: bool,
    },
    End {
        name: String,
    },
    Text(String),
    /// The content of `script`, `style` and other raw-text elements.
    Raw(String),
}

impl Ev {
    pub fn attr(&self, key: &str) -> Option<&str> {
        match self {
            Ev::Start { attrs, .. } => attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str()),
            _ => None,
        }
    }
}

/// The part of a qualified name after its prefix (`dc:title` → `title`).
pub fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// Every event of `src` (UTF-8, or Windows-1252 bytes when not valid
/// UTF-8). Names are lower-cased; the prefix is kept.
pub fn events(src: &[u8]) -> Vec<Ev> {
    let utf8 = std::str::from_utf8(src).is_ok();
    let t = html::tokenize(src);
    let mut out = Vec::with_capacity(t.toks.len());
    let span = |r: (u32, u32)| src.get(r.0 as usize..r.1 as usize).unwrap_or(&[]);
    for tok in &t.toks {
        match tok.kind {
            TEXT => {
                let mut s = String::new();
                html::decode_into(&mut s, span((tok.start, tok.end)), utf8, true);
                out.push(Ev::Text(s));
            }
            RAW => {
                let mut s = String::new();
                html::decode_into(&mut s, span((tok.start, tok.end)), utf8, false);
                out.push(Ev::Raw(s));
            }
            CDATA => {
                let mut s = String::new();
                html::decode_into(&mut s, span(tok.name), utf8, false);
                out.push(Ev::Text(s));
            }
            START => {
                let name = String::from_utf8_lossy(t.name(src, tok)).to_ascii_lowercase();
                let attrs = t
                    .attrs_of(tok)
                    .iter()
                    .map(|a| {
                        let k = String::from_utf8_lossy(span(a.name)).to_ascii_lowercase();
                        let mut v = String::new();
                        if let Some(r) = a.value {
                            html::decode_into(&mut v, span(r), utf8, true);
                        }
                        (k, v)
                    })
                    .collect();
                out.push(Ev::Start { name, attrs, closed: tok.closed });
            }
            END => {
                let name = String::from_utf8_lossy(t.name(src, tok)).to_ascii_lowercase();
                out.push(Ev::End { name });
            }
            _ => {}
        }
    }
    out
}

/// Appends `s` escaped for HTML text or a double-quoted attribute.
pub fn escape(out: &mut String, s: &str, attr: bool) {
    html::escape(out, s, attr);
}

/// The text between the start event at `i` and its end, tags dropped.
pub fn text_until_end(ev: &[Ev], i: usize) -> String {
    let Some(Ev::Start { name, closed, .. }) = ev.get(i) else { return String::new() };
    if *closed {
        return String::new();
    }
    let mut depth = 0usize;
    let mut out = String::new();
    // An element left open would read to the end of the document, for
    // every such element: the walk is bounded.
    for e in ev[i + 1..].iter().take(4096) {
        match e {
            Ev::Start { name: n, closed: false, .. } if n == name => depth += 1,
            Ev::End { name: n } if n == name => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            Ev::Text(t) => out.push_str(t),
            _ => {}
        }
    }
    html::collapse_ws(&out)
}

/// Percent-decodes a URL path (invalid escapes stay as written).
pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hexval(b.get(i + 1)), hexval(b.get(i + 2)))
        {
            out.push(h << 4 | l);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hexval(b: Option<&u8>) -> Option<u8> {
    let c = *b?;
    (c as char).to_digit(16).map(|d| d as u8)
}

/// Percent-encodes a member path for a URL: everything but unreserved
/// characters and `/`, so the result never holds a quote, a space, a
/// parenthesis or `?`.
pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &c in s.as_bytes() {
        if c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.' | b'~' | b'/') {
            out.push(c as char);
        } else {
            out.push_str(&format!("%{c:02X}"));
        }
    }
    out
}

/// `rel` resolved against the directory `base` (`OEBPS/text`), with
/// `.` and `..` folded; `None` when it climbs out of the archive.
pub fn resolve(base: &str, rel: &str) -> Option<String> {
    let mut parts: Vec<&str> =
        if rel.starts_with('/') { Vec::new() } else { base.split('/').filter(|p| !p.is_empty()).collect() };
    for seg in rel.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    Some(parts.join("/"))
}

/// The directory part of a member path (`a/b/c.xhtml` → `a/b`).
pub fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_and_paths() {
        let ev = events(b"<?xml version='1.0'?><dc:Title id=\"t\">A &amp; B</dc:Title><br/><style>p{}</style>");
        assert_eq!(ev[0], Ev::Start { name: "dc:title".into(), attrs: vec![("id".into(), "t".into())], closed: false });
        assert_eq!(text_until_end(&ev, 0), "A & B");
        assert!(matches!(&ev[3], Ev::Start { name, closed: true, .. } if name == "br"));
        assert!(ev.iter().any(|e| *e == Ev::Raw("p{}".into())));
        assert_eq!(local("dc:title"), "title");
        assert_eq!(resolve("OEBPS/text", "../img/a.png").as_deref(), Some("OEBPS/img/a.png"));
        assert_eq!(resolve("", "../x"), None);
        assert_eq!(percent_decode("a%20b%2"), "a b%2");
        assert_eq!(percent_encode("a b/(c)'\".png"), "a%20b/%28c%29%27%22.png");
        assert_eq!(dir_of("a/b/c.x"), "a/b");
    }
}
