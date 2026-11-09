//! A small XML reader for Torznab/Newznab feeds: elements, attributes,
//! text, CDATA, comments, processing instructions and the predefined
//! and numeric entities. Names are matched by local name (prefixes
//! dropped), like Go's `encoding/xml` without namespaces. Malformed
//! documents (unclosed or mismatched tags) are errors. No crate: the
//! server has no XML dependency (AGENTS.md).

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Node {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub text: String,
    pub children: Vec<Node>,
}

impl Node {
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }
    pub fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }
    pub fn text_of(&self, name: &str) -> String {
        self.child(name).map(|c| c.text.clone()).unwrap_or_default()
    }
    pub fn attr(&self, name: &str) -> &str {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str()).unwrap_or("")
    }
}

/// The deepest nesting accepted.
const MAX_DEPTH: usize = 64;

fn local(name: &str) -> String {
    name.rsplit(':').next().unwrap_or(name).to_string()
}

pub fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let Some(end) = after.find(';').filter(|e| *e <= 10) else {
            out.push('&');
            rest = after;
            continue;
        };
        let ent = &after[..end];
        let ch = match ent {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => ent
                .strip_prefix("#x")
                .or_else(|| ent.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &after[end + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn parse_attrs(s: &str) -> Option<Vec<(String, String)>> {
    let mut out = Vec::new();
    let mut rest = s.trim();
    while !rest.is_empty() {
        let eq = rest.find('=')?;
        let key = rest[..eq].trim();
        if key.is_empty() || key.contains(char::is_whitespace) {
            return None;
        }
        let after = rest[eq + 1..].trim_start();
        let quote = after.chars().next().filter(|c| *c == '"' || *c == '\'')?;
        let close = after[1..].find(quote)?;
        out.push((local(key), decode_entities(&after[1..1 + close])));
        rest = after[1 + close + 1..].trim_start();
    }
    Some(out)
}

/// Parses a document into its root element.
pub fn parse(src: &str) -> Option<Node> {
    let mut stack: Vec<Node> = vec![Node::default()];
    let mut i = 0;
    let b = src.as_bytes();
    while i < b.len() {
        if b[i] != b'<' {
            let end = src[i..].find('<').map(|e| i + e).unwrap_or(b.len());
            let text = decode_entities(&src[i..end]);
            stack.last_mut()?.text.push_str(&text);
            i = end;
            continue;
        }
        let rest = &src[i..];
        if let Some(r) = rest.strip_prefix("<!--") {
            i += 4 + r.find("-->")? + 3;
        } else if let Some(r) = rest.strip_prefix("<![CDATA[") {
            let end = r.find("]]>")?;
            stack.last_mut()?.text.push_str(&r[..end]);
            i += 9 + end + 3;
        } else if rest.starts_with("<?") {
            i += rest.find("?>")? + 2;
        } else if rest.starts_with("<!") {
            i += rest.find('>')? + 1;
        } else if let Some(r) = rest.strip_prefix("</") {
            let end = r.find('>')?;
            let name = local(r[..end].trim());
            let node = stack.pop()?;
            if node.name != name || stack.is_empty() {
                return None;
            }
            let parent = stack.last_mut()?;
            parent.children.push(Node { text: node.text.trim().to_string(), ..node });
            i += 2 + end + 1;
        } else {
            let end = tag_end(rest)?;
            let inner = &rest[1..end];
            let (inner, self_closing) = match inner.strip_suffix('/') {
                Some(x) => (x, true),
                None => (inner, false),
            };
            let name_end = inner.find(|c: char| c.is_whitespace()).unwrap_or(inner.len());
            let name = local(&inner[..name_end]);
            if name.is_empty() {
                return None;
            }
            let attrs = parse_attrs(&inner[name_end..])?;
            let node = Node { name, attrs, ..Node::default() };
            if self_closing {
                stack.last_mut()?.children.push(node);
            } else {
                if stack.len() > MAX_DEPTH {
                    return None;
                }
                stack.push(node);
            }
            i += end + 1;
        }
    }
    if stack.len() != 1 {
        return None;
    }
    let mut doc = stack.pop()?;
    let root = doc.children.pop()?;
    doc.children.is_empty().then_some(root)
}

/// The `>` closing a start tag, skipping quoted attribute values.
fn tag_end(s: &str) -> Option<usize> {
    let mut quote = None;
    for (i, c) in s.char_indices().skip(1) {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '>') => return Some(i),
            (None, '<') => return None,
            _ => {}
        }
    }
    None
}

/// Go's integer decoding of element text and attributes: surrounding
/// space trimmed, empty is 0, anything else must be a number.
pub fn int(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() { Some(0) } else { s.parse().ok() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_feeds() {
        let doc = r#"<?xml version="1.0"?><!-- c --><rss xmlns:torznab="x"><channel><item>
            <title>A &amp; B</title><torznab:attr name="seeders" value="5"/>
            <description><![CDATA[<b>raw</b>]]></description><enclosure url="http://h/a?x=1&amp;y=2" length='9' />
            </item></channel></rss>"#;
        let root = parse(doc).unwrap();
        assert_eq!(root.name, "rss");
        let item = root.child("channel").unwrap().child("item").unwrap();
        assert_eq!(item.text_of("title"), "A & B");
        assert_eq!(item.child("attr").unwrap().attr("value"), "5");
        assert_eq!(item.text_of("description"), "<b>raw</b>");
        assert_eq!(item.child("enclosure").unwrap().attr("url"), "http://h/a?x=1&y=2");
    }

    #[test]
    fn refuses_malformed() {
        for bad in ["<a><b></a>", "<a>", "<a></a><b></b>", "", "<a x=1></a>", "<a><!-- x</a>", "not xml"] {
            assert!(parse(bad).is_none(), "{bad:?}");
        }
        assert_eq!(int(" 12 "), Some(12));
        assert_eq!(int(""), Some(0));
        assert_eq!(int("x"), None);
    }
}
