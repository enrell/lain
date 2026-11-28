//! The table of contents: the book's NCX index when it has one,
//! otherwise headings, otherwise one entry per document.

use super::epub::TocEntry;
use super::html;
use super::indx;
use super::palmdb::PalmDb;

/// Nesting deeper than this is flattened.
const MAX_DEPTH: usize = 16;
/// NCX entries kept.
const MAX_ENTRIES: usize = 100_000;

/// One NCX index entry.
pub(crate) struct NcxEntry {
    pub label: String,
    pub level: u32,
    pub parent: Option<usize>,
    /// Tag 1: a text offset (`filepos` in MOBI 7).
    pub pos: Option<u64>,
    /// Tag 6: a KF8 fragment number and offset.
    pub fid: Option<(u64, u64)>,
}

/// The NCX index at record `first`; a damaged index yields no entries.
pub(crate) fn read_ncx(db: &PalmDb<'_>, first: usize, utf8: bool) -> Vec<NcxEntry> {
    let Ok(index) = indx::read(db, first) else { return Vec::new() };
    let mut out = Vec::new();
    for e in index.entries.iter().take(MAX_ENTRIES) {
        let mut label = String::new();
        if let Some(raw) = e.first(3).and_then(|o| index.string(o)) {
            html::decode_into(&mut label, raw, utf8, true);
        }
        let fid = e.values(6).filter(|v| v.len() >= 2).map(|v| (v[0], v[1]));
        out.push(NcxEntry {
            label: html::collapse_ws(&label),
            level: e.first(4).and_then(|l| u32::try_from(l).ok()).unwrap_or(0),
            parent: e.first(21).and_then(|p| usize::try_from(p).ok()),
            pos: e.first(1),
            fid,
        });
    }
    out
}

/// The tree of NCX entries; `hrefs[i]` is entry `i`'s target. Entries
/// without a target are skipped (their children move up).
pub(crate) fn from_ncx(entries: &[NcxEntry], hrefs: &[Option<String>]) -> Vec<TocEntry> {
    let n = entries.len().min(hrefs.len());
    if !entries[..n].iter().any(|e| e.parent.is_some()) {
        let flat = (0..n).filter_map(|i| Some((entries[i].level, label(&entries[i].label), hrefs[i].clone()?)));
        return nest(flat.collect());
    }
    // Parents always come before their children, so the links form a forest.
    let mut children = vec![Vec::new(); n];
    let mut roots = Vec::new();
    for (i, e) in entries[..n].iter().enumerate() {
        match e.parent {
            Some(p) if p < i => children[p].push(i),
            _ => roots.push(i),
        }
    }
    let node =
        |i: usize| hrefs[i].clone().map(|h| TocEntry { label: label(&entries[i].label), href: h, children: vec![] });
    build(&roots, &children, &node, 0)
}

fn build(
    ids: &[usize],
    children: &[Vec<usize>],
    node: &dyn Fn(usize) -> Option<TocEntry>,
    depth: usize,
) -> Vec<TocEntry> {
    let mut out = Vec::new();
    for &i in ids {
        if depth + 1 >= MAX_DEPTH {
            // Too deep: this node and all its descendants, flat.
            let mut stack = vec![i];
            while let Some(j) = stack.pop() {
                out.extend(node(j));
                stack.extend(children[j].iter().rev());
            }
            continue;
        }
        let kids = build(&children[i], children, node, depth + 1);
        match node(i) {
            Some(mut e) => {
                e.children = kids;
                out.push(e);
            }
            None => out.extend(kids),
        }
    }
    out
}

/// Nests a flat list by level (0 or 1 is the top).
pub(crate) fn nest(items: Vec<(u32, String, String)>) -> Vec<TocEntry> {
    let mut roots: Vec<TocEntry> = Vec::new();
    let mut stack: Vec<(u32, TocEntry)> = Vec::new();
    fn close(stack: &mut Vec<(u32, TocEntry)>, roots: &mut Vec<TocEntry>) {
        if let Some((_, e)) = stack.pop() {
            match stack.last_mut() {
                Some((_, parent)) => parent.children.push(e),
                None => roots.push(e),
            }
        }
    }
    for (level, label, href) in items {
        while stack.last().is_some_and(|(l, _)| *l >= level) {
            close(&mut stack, &mut roots);
        }
        if stack.len() >= MAX_DEPTH {
            close(&mut stack, &mut roots);
        }
        stack.push((level, TocEntry { label, href, children: Vec::new() }));
    }
    while !stack.is_empty() {
        close(&mut stack, &mut roots);
    }
    roots
}

/// A label fit for the navigation (never empty).
pub(crate) fn label(s: &str) -> String {
    let s = html::collapse_ws(s);
    if s.is_empty() { "\u{2022}".to_string() } else { s }
}

/// The first words of an XHTML body, for a document's TOC label.
pub(crate) fn snippet(body: &str) -> String {
    let mut text = String::new();
    let mut in_tag = false;
    for c in body.chars() {
        match c {
            '<' => {
                in_tag = true;
                text.push(' ');
            }
            '>' => in_tag = false,
            _ if !in_tag => text.push(c),
            _ => {}
        }
        if text.len() > 400 {
            break;
        }
    }
    let text = text.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&amp;", "&");
    let words = html::collapse_ws(&text);
    let mut out = String::new();
    for w in words.split(' ') {
        if out.len() + w.len() > 60 {
            if out.is_empty() {
                out.extend(w.chars().take(60));
            }
            out.push('\u{2026}');
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(w);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(t: &[TocEntry]) -> String {
        t.iter()
            .map(
                |e| {
                    if e.children.is_empty() {
                        e.label.clone()
                    } else {
                        format!("{}({})", e.label, labels(&e.children))
                    }
                },
            )
            .collect::<Vec<_>>()
            .join(",")
    }

    #[test]
    fn levels_and_parents_nest() {
        let flat = vec![(0, "a".into(), "x".into()), (1, "b".into(), "x".into()), (0, "c".into(), "x".into())];
        assert_eq!(labels(&nest(flat)), "a(b),c");
        // Breadth-first order with parent pointers, as kindlegen writes it.
        let e = |label: &str, level, parent| NcxEntry { label: label.into(), level, parent, pos: None, fid: None };
        let entries = [e("a", 0, None), e("c", 0, None), e("b", 1, Some(0)), e("d", 1, Some(1))];
        let hrefs: Vec<Option<String>> = vec![Some("1".into()), Some("2".into()), Some("3".into()), Some("4".into())];
        assert_eq!(labels(&from_ncx(&entries, &hrefs)), "a(b),c(d)");
        // A deep chain is flattened, not recursed.
        let chain: Vec<NcxEntry> = (0..1000).map(|i| e("n", i, i.checked_sub(1).map(|p| p as usize))).collect();
        let hrefs = vec![Some("h".to_string()); 1000];
        assert!(!from_ncx(&chain, &hrefs).is_empty());
        let deep: Vec<(u32, String, String)> = (0..1000).map(|i| (i, "n".into(), "h".into())).collect();
        assert!(!nest(deep).is_empty());
    }

    #[test]
    fn snippets() {
        assert_eq!(snippet("<h2 id=\"x\">Chapter&amp;One</h2><p>text</p>"), "Chapter&One text");
    }
}
