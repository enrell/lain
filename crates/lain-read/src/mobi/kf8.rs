//! KF8 (AZW3) books: the text is split into flows (FDST); flow 0 holds
//! the XHTML as skeletons with fragments to insert into them (the SKEL
//! and FRAG indexes), the other flows CSS and SVG. Links are
//! `kindle:pos:fid:XXXX:off:YYYYYYYYYY` positions, resources
//! `kindle:embed:XXXX` and `kindle:flow:XXXX` (base 32).

use std::collections::{BTreeMap, BTreeSet, HashSet};

use super::encoding::decode;
use super::epub::{Doc, Item, SpineItem};
use super::header::Header;
use super::html::{self, Anchor, Ctx, Hooks, Options, Tokens, Url};
use super::indx;
use super::mobi7::{documents_toc, headings_toc};
use super::palmdb::PalmDb;
use super::resources::{Resources, Used, base32, relative};
use super::{Content, MAX_DOCS, MobiError, be32, external_link, fragment, invalid, text, toc};

/// Bytes moved by out-of-order fragment insertions, at most.
const MAX_MOVED: u64 = 4 << 30;
/// Bytes scanned around an insert position to see if it is inside a tag.
const TAG_SCAN: usize = 4096;
/// Bytes scanned for those checks over the whole book, at most.
const SCAN_BUDGET: usize = 64 << 20;
/// Bytes searched to recompute bad insert positions, at most.
const FIX_BUDGET: usize = 256 << 20;
/// A token index meaning "the top of the part".
const TOP: u32 = u32::MAX;

struct Skel {
    count: usize,
    start: usize,
    len: usize,
}

struct Frag {
    insert: usize,
    aid: Vec<u8>,
    file: usize,
    offset: usize,
    len: usize,
}

struct Part {
    bytes: Vec<u8>,
    start: usize,
    end: usize,
}

pub(crate) fn convert(db: &PalmDb<'_>, h: &Header<'_>, res: &Resources<'_>) -> Result<Content, MobiError> {
    let utf8 = h.utf8();
    let text = text::extract(db, h)?;
    let flows = read_fdst(db, h, text.len())?;
    let (fs, fe) = flows[0];
    let flow0 = &text[fs..fe];
    let skels = match h.skeletons {
        Some(i) => read_skeletons(db, i)?,
        None => vec![Skel { count: 0, start: 0, len: flow0.len() }],
    };
    let frags = match h.fragments {
        Some(i) => read_fragments(db, i)?,
        None => Vec::new(),
    };
    let parts = assemble(flow0, &skels, &frags)?;
    if parts.is_empty() {
        return invalid("no KF8 parts");
    }
    if parts.len() > MAX_DOCS {
        return invalid("too many KF8 parts");
    }
    let toks: Vec<Tokens> = parts.iter().map(|p| html::tokenize(&p.bytes)).collect();
    let anch: Vec<Vec<u32>> = parts.iter().zip(&toks).map(|(p, t)| anchorable(t, &p.bytes)).collect();
    let r = Resolver { parts: &parts, toks: &toks, anch: &anch, frags: &frags };

    // Every link target: kindle:pos hrefs and the NCX.
    let mut keys: BTreeMap<(usize, u32), usize> = BTreeMap::new();
    for (p, t) in toks.iter().enumerate() {
        for tok in t.toks.iter().filter(|x| x.kind == html::START) {
            if let Some(v) = t.attr(&parts[p].bytes, tok, b"href")
                && let Some(target) = r.resolve_url(v)
            {
                let n = keys.len();
                keys.entry(target).or_insert(n);
            }
        }
    }
    let ncx = h.ncx.map(|i| toc::read_ncx(db, i, utf8)).unwrap_or_default();
    let ncx_targets: Vec<Option<(usize, u32)>> = ncx
        .iter()
        .map(|e| match e.fid {
            Some((f, o)) => r.resolve(usize::try_from(f).ok()?, usize::try_from(o).ok()?),
            None => r.resolve_abs(usize::try_from(e.pos?).ok()?),
        })
        .collect();
    for t in ncx_targets.iter().flatten() {
        let n = keys.len();
        keys.entry(*t).or_insert(n);
    }

    let mut gen_ids = vec![String::new(); keys.len()];
    let mut per_part: Vec<Vec<Anchor>> = (0..parts.len()).map(|_| Vec::new()).collect();
    let mut tops = Vec::new();
    for (&(p, tok), &k) in &keys {
        if tok == TOP {
            tops.push((k, p));
            continue;
        }
        let t = &toks[p];
        let tk = t.toks[tok as usize];
        gen_ids[k] = match t.attr(&parts[p].bytes, &tk, b"aid") {
            Some(a) if !a.is_empty() && a.len() <= 32 && a.iter().all(u8::is_ascii_alphanumeric) => {
                format!("aid-{}", String::from_utf8_lossy(a))
            }
            _ => format!("lain-a{k}"),
        };
        per_part[p].push(Anchor { tok, off: tk.start, attach: true, key: k });
    }
    for a in &mut per_part {
        a.sort_by_key(|a| (a.tok, a.off));
    }
    let mut ctx = Ctx::new(gen_ids);
    for (k, p) in tops {
        ctx.targets[k] = Some((p, None));
    }
    let mut hooks = Hooks8 {
        res,
        r: &r,
        keys: &keys,
        flows: &flows,
        text: &text,
        utf8,
        used: Used::default(),
        css: BTreeSet::new(),
        svg_checked: HashSet::new(),
    };
    let opts = Options { split_pagebreaks: false, split_size: false, drop_empty_tail: false };
    for (p, part) in parts.iter().enumerate() {
        html::emit(&part.bytes, utf8, &toks[p], &per_part[p], &mut hooks, &opts, &mut ctx);
        // Without splitting, each part is exactly one document.
        if ctx.docs.len() != p + 1 {
            return invalid("KF8 part did not produce one document");
        }
    }

    // Stylesheets, with their own resource references.
    for &n in &hooks.css.clone() {
        let (s, e) = flows[n];
        let css = decode(&text[s..e], utf8);
        let css = hooks.clean_css(&css, "styles");
        hooks.used.add(Item::new(&css_path(n), "text/css", css.into_bytes(), true));
    }

    let paths: Vec<String> = (0..parts.len()).map(|i| format!("part{i:04}.xhtml")).collect();
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
    let mut toc_entries = Vec::new();
    if !ncx.is_empty() {
        let targets: Vec<Option<String>> =
            ncx_targets.iter().map(|t| t.and_then(|t| keys.get(&t)).and_then(|&k| hrefs[k].clone())).collect();
        toc_entries = toc::from_ncx(&ncx, &targets);
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
    let (spine, rtl) = resc_spine(res, docs.len());
    Ok(Content { docs, spine, items: hooks.used.into_items(), toc: toc_entries, rtl })
}

fn css_path(n: usize) -> String {
    format!("styles/flow{n:04}.css")
}

fn num(v: u64) -> Result<usize, MobiError> {
    usize::try_from(v).map_err(|_| MobiError::Invalid("index value out of range".into()))
}

/// The flows as (start, end) ranges of the text.
fn read_fdst(db: &PalmDb<'_>, h: &Header<'_>, len: usize) -> Result<Vec<(usize, usize)>, MobiError> {
    let whole = vec![(0, len)];
    let Some(rec) = h.fdst.and_then(|i| db.record(i)) else { return Ok(whole) };
    if !rec.starts_with(b"FDST") {
        return Ok(whole);
    }
    let table = (be32(rec, 4).unwrap_or(12) as usize).max(12);
    let count = be32(rec, 8).unwrap_or(0) as usize;
    if count == 0 {
        return Ok(whole);
    }
    if count > rec.len() / 8 {
        return invalid("FDST out of bounds");
    }
    let mut flows = Vec::with_capacity(count);
    for k in 0..count {
        let at = table + 8 * k;
        let (Some(s), Some(e)) = (be32(rec, at), be32(rec, at + 4)) else { return invalid("FDST out of bounds") };
        let (s, e) = (s as usize, (e as usize).min(len));
        if s > e {
            return invalid("bad FDST flow");
        }
        flows.push((s, e));
    }
    Ok(flows)
}

fn read_skeletons(db: &PalmDb<'_>, first: usize) -> Result<Vec<Skel>, MobiError> {
    let index = indx::read(db, first)?;
    index
        .entries
        .iter()
        .map(|e| {
            let count = e.first(1).ok_or_else(|| MobiError::Invalid("skeleton without a fragment count".into()))?;
            let geo = e.values(6).filter(|v| v.len() >= 2);
            let geo = geo.ok_or_else(|| MobiError::Invalid("skeleton without a position".into()))?;
            Ok(Skel { count: num(count)?, start: num(geo[0])?, len: num(geo[1])? })
        })
        .collect()
}

fn read_fragments(db: &PalmDb<'_>, first: usize) -> Result<Vec<Frag>, MobiError> {
    let index = indx::read(db, first)?;
    index
        .entries
        .iter()
        .map(|e| {
            let insert = std::str::from_utf8(e.label).ok().and_then(|s| s.trim().parse::<usize>().ok());
            let insert = insert.ok_or_else(|| MobiError::Invalid("fragment without an insert position".into()))?;
            let selector = e.first(2).and_then(|o| index.string(o)).unwrap_or(b"");
            let geo = e.values(6).filter(|v| v.len() >= 2);
            let geo = geo.ok_or_else(|| MobiError::Invalid("fragment without a position".into()))?;
            Ok(Frag {
                insert,
                aid: selector_aid(selector).to_vec(),
                file: num(e.first(3).unwrap_or(0))?,
                offset: num(geo[0])?,
                len: num(geo[1])?,
            })
        })
        .collect()
}

/// The aid in a selector such as `P-//*[@aid='0']`.
fn selector_aid(sel: &[u8]) -> &[u8] {
    let Some(p) = sel.windows(4).position(|w| w == b"aid=") else { return b"" };
    let rest = &sel[p + 4..];
    let Some(&q) = rest.first().filter(|&&q| q == b'\'' || q == b'"') else { return b"" };
    let rest = &rest[1..];
    &rest[..rest.iter().position(|&b| b == q).unwrap_or(rest.len())]
}

/// Builds a part: a skeleton with fragments inserted at increasing
/// positions is streamed; anything else falls back to in-place inserts.
struct Asm<'a> {
    skel: &'a [u8],
    out: Vec<u8>,
    /// The unconsumed skeleton starts here; `None` once materialized.
    rest: Option<usize>,
}

impl Asm<'_> {
    fn len(&self) -> usize {
        match self.rest {
            Some(r) => self.out.len() + self.skel.len() - r,
            None => self.out.len(),
        }
    }

    fn byte(&self, p: usize) -> Option<u8> {
        if p < self.out.len() {
            return Some(self.out[p]);
        }
        let r = self.rest?;
        self.skel.get(r + p - self.out.len()).copied()
    }

    fn materialize(&mut self) {
        if let Some(r) = self.rest.take() {
            self.out.extend_from_slice(&self.skel[r..]);
        }
    }

    /// Whether position `ip` is inside a tag (judged within a window);
    /// the bytes looked at are taken from `budget`.
    fn inside_tag(&self, ip: usize, budget: &mut usize) -> bool {
        let mut seen = 0usize;
        let mut look = |p: usize| {
            seen += 1;
            self.byte(p).filter(|b| matches!(b, b'<' | b'>'))
        };
        let back = (ip.saturating_sub(TAG_SCAN)..ip).rev().find_map(&mut look);
        let fwd = (ip..(ip + TAG_SCAN).min(self.len())).find_map(&mut look);
        *budget = budget.saturating_sub(seen);
        back == Some(b'<') || fwd == Some(b'>')
    }

    /// The position after the start tag carrying `aid`.
    fn after_aid(&mut self, aid: &[u8]) -> Option<usize> {
        if aid.is_empty() {
            return None;
        }
        self.materialize();
        for q in *b"\"'" {
            let mut needle = b"aid=".to_vec();
            needle.push(q);
            needle.extend_from_slice(aid);
            needle.push(q);
            if let Some(p) = self.out.windows(needle.len()).position(|w| w == needle.as_slice()) {
                let gt = self.out[p..].iter().position(|&b| b == b'>')?;
                return Some(p + gt + 1);
            }
        }
        None
    }

    fn insert(&mut self, ip: usize, piece: &[u8], moved: &mut u64) -> Result<(), MobiError> {
        if let Some(r) = self.rest {
            if ip >= self.out.len() && ip - self.out.len() <= self.skel.len() - r {
                let take = ip - self.out.len();
                self.out.extend_from_slice(&self.skel[r..r + take]);
                self.out.extend_from_slice(piece);
                self.rest = Some(r + take);
                return Ok(());
            }
            self.materialize();
        }
        let ip = ip.min(self.out.len());
        *moved += (self.out.len() - ip) as u64;
        if *moved > MAX_MOVED {
            return invalid("KF8 fragment table too complex");
        }
        self.out.splice(ip..ip, piece.iter().copied());
        Ok(())
    }

    fn finish(mut self) -> Vec<u8> {
        self.materialize();
        self.out
    }
}

fn assemble(flow0: &[u8], skels: &[Skel], frags: &[Frag]) -> Result<Vec<Part>, MobiError> {
    let mut parts = Vec::with_capacity(skels.len());
    let (mut fi, mut moved) = (0usize, 0u64);
    let (mut scan, mut fix) = (SCAN_BUDGET, FIX_BUDGET);
    for s in skels {
        let bad = || MobiError::Invalid("skeleton out of bounds".into());
        let mut base = s.start.checked_add(s.len).ok_or_else(bad)?;
        let skel = flow0.get(s.start..base).ok_or_else(bad)?;
        let mut asm = Asm { skel, out: Vec::with_capacity(skel.len()), rest: Some(0) };
        for _ in 0..s.count {
            let Some(f) = frags.get(fi) else { return invalid("fragment table too short") };
            fi += 1;
            let end = base.checked_add(f.len).ok_or_else(bad)?;
            let piece = flow0.get(base..end).ok_or_else(|| MobiError::Invalid("fragment out of bounds".into()))?;
            base = end;
            let mut ip = f.insert.checked_sub(s.start).unwrap_or(usize::MAX).min(asm.len());
            if scan > 0 && asm.inside_tag(ip, &mut scan) && fix >= asm.len() {
                fix -= asm.len();
                if let Some(p) = asm.after_aid(&f.aid) {
                    ip = p.saturating_add(f.offset).min(asm.len());
                }
            }
            asm.insert(ip, piece, &mut moved)?;
        }
        parts.push(Part { bytes: asm.finish(), start: s.start, end: base });
    }
    Ok(parts)
}

/// Start tags with an id, name or aid, in token order.
fn anchorable(t: &Tokens, src: &[u8]) -> Vec<u32> {
    t.toks
        .iter()
        .enumerate()
        .filter(|(_, x)| x.kind == html::START)
        .filter(|(_, x)| {
            [&b"id"[..], b"name", b"aid"].iter().any(|n| t.attr(src, x, n).is_some_and(|v| !v.trim_ascii().is_empty()))
        })
        .map(|(i, _)| i as u32)
        .collect()
}

struct Resolver<'p> {
    parts: &'p [Part],
    toks: &'p [Tokens],
    anch: &'p [Vec<u32>],
    frags: &'p [Frag],
}

impl Resolver<'_> {
    /// The target of a `kindle:pos` URL.
    fn resolve_url(&self, v: &[u8]) -> Option<(usize, u32)> {
        match parse_kindle(std::str::from_utf8(v).ok()?)? {
            Kindle::Pos(fid, off) => self.resolve(fid, off),
            _ => None,
        }
    }

    /// Fragment `fid` plus `off` bytes: (part, anchor token or TOP).
    fn resolve(&self, fid: usize, off: usize) -> Option<(usize, u32)> {
        let f = self.frags.get(fid)?;
        let pos = f.insert.checked_add(off)?;
        match self.resolve_abs(pos) {
            Some(t) => Some(t),
            // Past the part's end: the top of the fragment's file.
            None => Some((f.file.min(self.parts.len().checked_sub(1)?), TOP)),
        }
    }

    fn resolve_abs(&self, pos: usize) -> Option<(usize, u32)> {
        let i = self.parts.partition_point(|p| p.start <= pos);
        let p = i.checked_sub(1)?;
        let part = &self.parts[p];
        if pos >= part.end {
            return None;
        }
        let npos = u32::try_from(pos - part.start).ok()?;
        let t = &self.toks[p];
        // A position at or inside a tag counts that tag; in text, only
        // the tags before it.
        let limit = match t.at(npos) {
            Some(i) if matches!(t.toks[i].kind, html::START | html::END | html::OTHER) => i + 1,
            Some(i) => i,
            None => t.toks.len(),
        };
        let a = &self.anch[p];
        let k = a.partition_point(|&x| (x as usize) < limit);
        Some((p, k.checked_sub(1).map_or(TOP, |j| a[j])))
    }
}

enum Kindle<'s> {
    Embed(usize),
    Flow(usize, &'s str),
    Pos(usize, usize),
}

fn parse_kindle(v: &str) -> Option<Kindle<'_>> {
    let v = v.trim();
    if let Some(r) = v.strip_prefix("kindle:embed:") {
        return Some(Kindle::Embed(base32(r.split(['?', '#']).next()?)?));
    }
    if let Some(r) = v.strip_prefix("kindle:flow:") {
        let (id, query) = r.split_once('?').unwrap_or((r, ""));
        return Some(Kindle::Flow(base32(id)?, query.strip_prefix("mime=").unwrap_or("")));
    }
    let r = v.strip_prefix("kindle:pos:fid:")?;
    let (fid, rest) = r.split_once(':')?;
    let off = rest.strip_prefix("off:")?.split(['?', '#']).next()?;
    Some(Kindle::Pos(base32(fid)?, base32(off)?))
}

struct Hooks8<'r, 'a> {
    res: &'r Resources<'a>,
    r: &'r Resolver<'r>,
    keys: &'r BTreeMap<(usize, u32), usize>,
    flows: &'r [(usize, usize)],
    text: &'r [u8],
    utf8: bool,
    used: Used,
    css: BTreeSet<usize>,
    svg_checked: HashSet<usize>,
}

impl Hooks8<'_, '_> {
    /// A resource or flow URL as a path relative to `dir`.
    fn resource(&mut self, url: &str, dir: &str) -> Option<String> {
        let path = match parse_kindle(url)? {
            Kindle::Embed(n) => self.used.resource(self.res, n)?,
            Kindle::Flow(n, mime) => {
                if mime.contains("css") {
                    self.css_flow(n)?
                } else {
                    self.svg_flow(n)?
                }
            }
            Kindle::Pos(..) => return None,
        };
        Some(relative(dir, &path))
    }

    fn css_flow(&mut self, n: usize) -> Option<String> {
        if n == 0 || n >= self.flows.len() {
            return None;
        }
        self.css.insert(n);
        Some(css_path(n))
    }

    /// An SVG flow as an image file, if it is plain SVG.
    fn svg_flow(&mut self, n: usize) -> Option<String> {
        if n == 0 || n >= self.flows.len() {
            return None;
        }
        let path = format!("images/flow{n:04}.svg");
        if self.used.items.contains_key(&path) {
            return Some(path);
        }
        if !self.svg_checked.insert(n) {
            return None;
        }
        let (s, e) = self.flows[n];
        let svg = decode(&self.text[s..e], self.utf8);
        if !svg.contains("<svg") || unsafe_markup(&svg) {
            return None;
        }
        let svg = self.rewrite(&svg, "images", &mut HashSet::new());
        self.used.add(Item::new(&path, "image/svg+xml", svg.into_bytes(), true));
        Some(path)
    }

    /// CSS kept in `dir` with its `kindle:` URLs rewritten and any other
    /// URL dropped.
    fn clean_css(&mut self, css: &str, dir: &str) -> String {
        let mut produced = HashSet::new();
        let css = self.rewrite(css, dir, &mut produced);
        html::sanitize_css(&css, &|u| u.starts_with('#') || produced.contains(u))
    }

    /// Rewrites the `kindle:` URLs in CSS or SVG kept in `dir`; the new
    /// URLs are added to `produced`.
    fn rewrite(&mut self, s: &str, dir: &str, produced: &mut HashSet<String>) -> String {
        let mut out = String::with_capacity(s.len());
        let mut rest = s;
        while let Some(p) = rest.find("kindle:") {
            out.push_str(&rest[..p]);
            let tail = &rest[p..];
            let end = tail
                .find(|c: char| matches!(c, ')' | '"' | '\'' | ';' | ',' | '<' | '>') || c.is_whitespace())
                .unwrap_or(tail.len());
            if let Some(path) = self.resource(&tail[..end], dir) {
                out.push_str(&path);
                produced.insert(path);
            }
            rest = &tail[end..];
        }
        out.push_str(rest);
        out
    }
}

impl Hooks for Hooks8<'_, '_> {
    fn url(&mut self, tag: &str, _attr: &str, value: &str) -> Url {
        let v = value.trim();
        let found = match parse_kindle(v) {
            Some(Kindle::Pos(fid, off)) if tag == "a" => {
                return self.r.resolve(fid, off).and_then(|t| self.keys.get(&t)).map_or(Url::Drop, |&k| Url::Link(k));
            }
            Some(Kindle::Embed(n)) => match tag {
                "img" | "image" => self.used.image(self.res, n),
                "a" => self.used.resource(self.res, n),
                _ => None,
            },
            Some(Kindle::Flow(n, _)) => match tag {
                "link" => self.css_flow(n),
                "img" | "image" => self.svg_flow(n),
                _ => None,
            },
            Some(Kindle::Pos(..)) => None,
            None if tag == "a" => {
                if v.len() > 1 && v.starts_with('#') {
                    return Url::Keep(v.to_string());
                }
                return external_link(v);
            }
            None => None,
        };
        found.map_or(Url::Drop, Url::Keep)
    }

    fn css(&mut self, css: &str) -> String {
        self.clean_css(css, "")
    }
}

/// Markup that could run code: scripts, event handlers, foreign objects.
fn unsafe_markup(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    if ["<script", "javascript:", "vbscript:", "<foreignobject", "<iframe", "<!entity"].iter().any(|b| l.contains(b)) {
        return true;
    }
    // Event handler attributes: whitespace, "on", letters, '='.
    let b = l.as_bytes();
    b.windows(3).enumerate().any(|(i, w)| {
        w[0].is_ascii_whitespace() && w[1] == b'o' && w[2] == b'n' && {
            let rest = &b[i + 3..];
            let n = rest.iter().take_while(|c| c.is_ascii_alphabetic()).count();
            n > 0 && rest[n..].iter().find(|c| !c.is_ascii_whitespace()) == Some(&b'=')
        }
    })
}

/// The spine order and page direction from the RESC record, if any.
fn resc_spine(res: &Resources<'_>, docs: usize) -> (Vec<SpineItem>, bool) {
    let mut order = Vec::new();
    let mut seen = vec![false; docs];
    let mut rtl = false;
    if let Some(rec) = res.find(b"RESC")
        && let Some(start) = rec.iter().position(|&b| b == b'<')
    {
        let body = &rec[start..];
        let body = &body[..body.iter().position(|&b| b == 0).unwrap_or(body.len())];
        let t = html::tokenize(body);
        for tok in t.toks.iter().filter(|x| x.kind == html::START) {
            let name = t.name(body, tok);
            if name.eq_ignore_ascii_case(b"spine") {
                let ppd = t.attr(body, tok, b"page-progression-direction").or(t.attr(
                    body,
                    tok,
                    b"page-progession-direction",
                ));
                rtl |= ppd.is_some_and(|v| v.eq_ignore_ascii_case(b"rtl"));
            } else if name.eq_ignore_ascii_case(b"itemref") {
                let skel = t
                    .attr(body, tok, b"skelid")
                    .and_then(|v| std::str::from_utf8(v).ok()?.trim().parse::<usize>().ok());
                let Some(s) = skel.filter(|&s| s < docs && !seen[s]) else { continue };
                seen[s] = true;
                let props = t.attr(body, tok, b"properties").unwrap_or(b"");
                let props = if props.windows(16).any(|w| w == b"page-spread-left") {
                    Some("page-spread-left")
                } else if props.windows(17).any(|w| w == b"page-spread-right") {
                    Some("page-spread-right")
                } else {
                    None
                };
                order.push(SpineItem { doc: s, props });
            }
        }
    }
    order.extend((0..docs).filter(|&d| !seen[d]).map(|doc| SpineItem { doc, props: None }));
    (order, rtl)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kindle_urls() {
        assert!(matches!(parse_kindle("kindle:embed:0003?mime=image/jpeg"), Some(Kindle::Embed(3))));
        assert!(matches!(parse_kindle("kindle:flow:0001?mime=text/css"), Some(Kindle::Flow(1, "text/css"))));
        assert!(matches!(parse_kindle("kindle:pos:fid:000A:off:000000001V"), Some(Kindle::Pos(10, 63))));
        assert!(parse_kindle("kindle:pos:fid:000A").is_none());
        assert!(parse_kindle("http://x").is_none());
        assert_eq!(selector_aid(b"P-//*[@aid='0']"), b"0");
        assert_eq!(selector_aid(b"P-//*[@aid=\"1A\"]"), b"1A");
        assert!(unsafe_markup("<svg onload = 'x'>"));
        assert!(!unsafe_markup("<svg><image width='1'/></svg>"));
    }

    #[test]
    fn fragments_stream_and_fall_back() {
        let flow = b"<body aid=\"0\"></body>AAB";
        let skel = [Skel { count: 2, start: 0, len: 21 }];
        let frags = [
            Frag { insert: 14, aid: b"0".to_vec(), file: 0, offset: 0, len: 2 },
            Frag { insert: 14, aid: b"0".to_vec(), file: 0, offset: 0, len: 1 },
        ];
        let parts = assemble(flow, &skel, &frags).unwrap();
        assert_eq!(parts[0].bytes, b"<body aid=\"0\">BAA</body>");
        assert_eq!((parts[0].start, parts[0].end), (0, 24));
        // An insert position inside a tag is moved after the aid's tag.
        let frags = [Frag { insert: 3, aid: b"0".to_vec(), file: 0, offset: 0, len: 2 }];
        let skel = [Skel { count: 1, start: 0, len: 21 }];
        let parts = assemble(flow, &skel, &frags).unwrap();
        assert_eq!(parts[0].bytes, b"<body aid=\"0\">AA</body>");
    }
}
