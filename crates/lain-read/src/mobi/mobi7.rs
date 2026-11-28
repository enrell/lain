//! MOBI 6/7 books: one HTML stream with `filepos` links (byte offsets
//! into the text), `recindex` images and `<mbp:pagebreak/>` page
//! breaks, which become the chapter boundaries.

use std::collections::{BTreeMap, HashSet};

use super::epub::{Doc, SpineItem, TocEntry};
use super::header::Header;
use super::html::{self, Anchor, Ctx, Hooks, Options, Tokens, Url};
use super::palmdb::PalmDb;
use super::resources::{Resources, Used};
use super::{Content, MobiError, external_link, fragment, text, toc};

pub(crate) fn convert(db: &PalmDb<'_>, h: &Header<'_>, res: &Resources<'_>) -> Result<Content, MobiError> {
    let text = text::extract(db, h)?;
    let utf8 = h.utf8();
    let t = html::tokenize(&text);

    // Every link target: filepos attributes, the guide and the NCX.
    let mut offsets: BTreeMap<u32, usize> = BTreeMap::new();
    let mut guide_toc = None;
    for tok in t.toks.iter().filter(|x| x.kind == html::START) {
        let Some(pos) = t.attr(&text, tok, b"filepos").and_then(digits) else { continue };
        offsets.insert(pos, 0);
        if guide_toc.is_none()
            && t.name(&text, tok).eq_ignore_ascii_case(b"reference")
            && t.attr(&text, tok, b"type").is_some_and(|v| v.eq_ignore_ascii_case(b"toc"))
        {
            guide_toc = Some(pos);
        }
    }
    let ncx = h.ncx.map(|i| toc::read_ncx(db, i, utf8)).unwrap_or_default();
    for e in &ncx {
        if let Some(p) = e.pos.and_then(|p| u32::try_from(p).ok()) {
            offsets.insert(p, 0);
        }
    }
    let mut gen_ids = Vec::with_capacity(offsets.len());
    let mut anchors = Vec::with_capacity(offsets.len());
    for (k, (&off, key)) in offsets.iter_mut().enumerate() {
        *key = k;
        gen_ids.push(format!("fpos{off}"));
        let (tok, at, attach) = locate(&t, &text, off, utf8);
        anchors.push(Anchor { tok, off: at, attach, key: k });
    }
    anchors.sort_by_key(|a| (a.tok, a.off));

    let mut hooks = Hooks7 { res, offsets: &offsets, used: Used::default() };
    let mut ctx = Ctx::new(gen_ids);
    let opts = Options { split_pagebreaks: true, split_size: true, drop_empty_tail: true };
    html::emit(&text, utf8, &t, &anchors, &mut hooks, &opts, &mut ctx);
    drop(t);

    let paths: Vec<String> = (1..=ctx.docs.len()).map(|i| format!("chapter{i:04}.xhtml")).collect();
    let hrefs: Vec<Option<String>> = ctx
        .targets
        .iter()
        .map(|t| {
            let (d, id) = t.as_ref()?;
            let p = paths.get(*d)?;
            Some(match id {
                Some(i) => format!("{p}#{}", fragment(i)),
                None => p.clone(),
            })
        })
        .collect();
    let key_href = |pos: u32| offsets.get(&pos).and_then(|&k| hrefs.get(k).cloned().flatten());

    // The NCX, else the guide's TOC page, else headings, else documents.
    let mut toc_entries = if ncx.is_empty() {
        Vec::new()
    } else {
        let targets: Vec<Option<String>> =
            ncx.iter().map(|e| e.pos.and_then(|p| u32::try_from(p).ok()).and_then(key_href)).collect();
        toc::from_ncx(&ncx, &targets)
    };
    if toc_entries.is_empty()
        && let Some(page) = guide_toc.and_then(|o| offsets.get(&o)).and_then(|&k| ctx.targets[k].as_ref())
    {
        let page = page.0;
        let mut seen = HashSet::new();
        toc_entries = ctx
            .links
            .iter()
            .filter(|l| l.doc == page && seen.insert(l.key))
            .filter_map(|l| {
                Some(TocEntry { label: toc::label(&l.text), href: hrefs.get(l.key)?.clone()?, children: vec![] })
            })
            .collect();
        if toc_entries.len() < 2 {
            toc_entries.clear();
        }
    }
    if toc_entries.is_empty() {
        toc_entries = headings_toc(&ctx, &paths);
    }
    let docs: Vec<Doc> = ctx
        .docs
        .into_iter()
        .zip(&paths)
        .map(|(d, path)| Doc {
            path: path.clone(),
            title: d.title,
            head: d.head,
            body_attrs: d.body_attrs,
            body: html::resolve_links(d.body, |k| hrefs.get(k).cloned().flatten()),
            svg: d.svg,
        })
        .collect();
    if toc_entries.is_empty() {
        toc_entries = documents_toc(&docs);
    }
    let spine = (0..docs.len()).map(|doc| SpineItem { doc, props: None }).collect();
    Ok(Content { docs, spine, items: hooks.used.into_items(), toc: toc_entries, rtl: false })
}

/// The TOC from h1-h3 headings, when there are at least two.
pub(crate) fn headings_toc(ctx: &Ctx, paths: &[String]) -> Vec<TocEntry> {
    if ctx.headings.len() < 2 {
        return Vec::new();
    }
    let items = ctx
        .headings
        .iter()
        .filter_map(|h| {
            let p = paths.get(h.doc)?;
            Some((u32::from(h.level), toc::label(&h.text), format!("{p}#{}", fragment(&h.id))))
        })
        .collect();
    toc::nest(items)
}

/// One entry per document, labelled with its first words.
pub(crate) fn documents_toc(docs: &[Doc]) -> Vec<TocEntry> {
    docs.iter()
        .enumerate()
        .map(|(i, d)| {
            let s = toc::snippet(&d.body);
            let label = if s.is_empty() { format!("{}", i + 1) } else { s };
            TocEntry { label, href: d.path.clone(), children: Vec::new() }
        })
        .collect()
}

struct Hooks7<'r, 'a> {
    res: &'r Resources<'a>,
    offsets: &'r BTreeMap<u32, usize>,
    used: Used,
}

impl Hooks for Hooks7<'_, '_> {
    fn url(&mut self, tag: &str, attr: &str, value: &str) -> Url {
        let found = match (tag, attr) {
            ("a", "filepos") => {
                return digits(value.as_bytes())
                    .and_then(|o| self.offsets.get(&o))
                    .map_or(Url::Drop, |&k| Url::Link(k));
            }
            ("a", "href") => return external_link(value),
            ("img", "recindex" | "hirecindex" | "lorecindex") => {
                digits(value.as_bytes()).and_then(|n| self.used.image(self.res, n as usize))
            }
            _ => None,
        };
        found.map_or(Url::Drop, Url::Keep)
    }
}

/// The leading decimal number of an attribute value.
fn digits(v: &[u8]) -> Option<u32> {
    let v = v.trim_ascii_start();
    let n = v.iter().take_while(|b| b.is_ascii_digit()).count();
    if n == 0 || n > 10 {
        return None;
    }
    std::str::from_utf8(&v[..n]).ok()?.parse::<u64>().ok().and_then(|x| u32::try_from(x).ok())
}

/// Where the anchor for byte `off` goes: (token, offset, attach).
fn locate(t: &Tokens, src: &[u8], off: u32, utf8: bool) -> (u32, u32, bool) {
    let next = |i: usize| match t.toks.get(i + 1) {
        Some(n) => ((i + 1) as u32, n.start, false),
        None => (t.toks.len() as u32, 0, false),
    };
    let Some(i) = t.at(off) else { return (t.toks.len() as u32, 0, false) };
    let tok = t.toks[i];
    match tok.kind {
        html::TEXT => {
            if off == tok.start {
                return (i as u32, off, false);
            }
            let o = safe_split(src, tok.start as usize, tok.end as usize, off as usize, utf8);
            if o >= tok.end as usize { next(i) } else { (i as u32, o as u32, false) }
        }
        html::START => (i as u32, tok.start, true),
        _ => next(i),
    }
}

/// Moves a split point inside text out of a character reference and
/// off a UTF-8 continuation byte.
fn safe_split(src: &[u8], start: usize, end: usize, off: usize, utf8: bool) -> usize {
    let mut o = off.min(end);
    let lo = o.saturating_sub(32).max(start);
    if let Some(p) = src[lo..o].iter().rposition(|&b| b == b'&') {
        let amp = lo + p;
        if !src[amp + 1..o].iter().any(|&b| b == b';' || b.is_ascii_whitespace())
            && let Some(q) = src[o..end.min(o + 32)].iter().position(|&b| b == b';')
        {
            o += q + 1;
        }
    }
    if utf8 {
        while o < end && src[o] & 0xc0 == 0x80 {
            o += 1;
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_points_avoid_references_and_utf8() {
        let s = "ab&amp;cd\u{e9}f".as_bytes();
        assert_eq!(safe_split(s, 0, s.len(), 4, true), 7);
        assert_eq!(safe_split(s, 0, s.len(), 10, true), 11);
        assert_eq!(safe_split(s, 0, s.len(), 1, true), 1);
        assert_eq!(digits(b" 0000012345/"), Some(12345));
        assert_eq!(digits(b"x"), None);
    }
}
