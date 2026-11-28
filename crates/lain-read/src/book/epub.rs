//! EPUB 2 and 3 packages (D-305): the container, the package document
//! (metadata, manifest, spine, cover), the table of contents (the EPUB 3
//! navigation document, else the NCX), and font obfuscation. Content
//! documents another encryption protects are DRM: refused.

use std::collections::HashMap;

use sha1::{Digest, Sha1};

use super::BookError;
use super::markup::{self, Ev, local};
use crate::zip::Archive;

/// Table of contents levels kept; deeper ones are flattened or dropped.
const MAX_TOC_DEPTH: usize = 8;

/// Package documents, navigation documents and NCX files are read up
/// to this size.
const MAX_XML: u64 = 16 << 20;

#[derive(Debug, Clone, Default)]
pub struct Meta {
    pub title: String,
    pub authors: Vec<String>,
    pub language: String,
    pub publisher: String,
    pub description: String,
    pub subjects: Vec<String>,
    pub published: String,
    pub identifier: String,
}

#[derive(Debug, Clone)]
pub struct Item {
    pub path: String,
    pub media_type: String,
    pub properties: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TocNode {
    pub title: String,
    pub path: String,
    pub anchor: String,
    pub children: Vec<TocNode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Obfuscation {
    Idpf,
    Adobe,
}

/// What the package document points at: manifest ids to paths, the
/// spine's NCX id, the cover meta's id, the unique identifier and the
/// guide's cover page.
type PackageRefs = (HashMap<String, String>, Option<String>, Option<String>, Option<String>, Option<String>);

#[derive(Debug, Clone)]
pub struct Epub {
    pub archive: Archive,
    pub meta: Meta,
    /// Manifest items by member path.
    pub items: HashMap<String, Item>,
    /// The reading order: member path and whether it is linear.
    pub spine: Vec<(String, bool)>,
    pub rtl: bool,
    pub toc: Vec<TocNode>,
    pub cover: Option<String>,
    pub obfuscated: HashMap<String, Obfuscation>,
    /// The identifier obfuscated fonts are keyed with.
    font_key: Option<String>,
}

fn read(a: &Archive, name: &str) -> Result<Vec<u8>, BookError> {
    let e = a
        .find(name)
        .or_else(|| a.entries().iter().find(|e| e.name.eq_ignore_ascii_case(name)))
        .ok_or_else(|| BookError::invalid(format!("missing {name}")))?;
    a.read(e, MAX_XML).map_err(|e| BookError::invalid(e.to_string()))
}

impl Epub {
    pub fn open(archive: Archive) -> Result<Epub, BookError> {
        let container = read(&archive, "META-INF/container.xml")?;
        let ev = markup::events(&container);
        let opf_path = ev
            .iter()
            .find_map(|e| match e {
                Ev::Start { name, .. } if local(name) == "rootfile" => {
                    let mt = e.attr("media-type").unwrap_or("application/oebps-package+xml");
                    (mt.contains("oebps-package")).then(|| e.attr("full-path").map(markup::percent_decode)).flatten()
                }
                _ => None,
            })
            .or_else(|| {
                archive.entries().iter().find(|e| e.name.to_ascii_lowercase().ends_with(".opf")).map(|e| e.name.clone())
            })
            .ok_or_else(|| BookError::invalid("no package document"))?;
        let opf = read(&archive, &opf_path)?;
        let base = markup::dir_of(&opf_path).to_string();
        let mut epub = Epub {
            archive,
            meta: Meta::default(),
            items: HashMap::new(),
            spine: Vec::new(),
            rtl: false,
            toc: Vec::new(),
            cover: None,
            obfuscated: HashMap::new(),
            font_key: None,
        };
        let ev = markup::events(&opf);
        let (ids, toc_id, cover_id, unique_id, guide_cover) = epub.package(&ev, &base);
        if epub.spine.is_empty() {
            return Err(BookError::invalid("empty spine"));
        }
        // The cover image (§ EPUB 3 `cover-image`, EPUB 2 `meta name=cover`).
        epub.cover = epub
            .items
            .values()
            .find(|i| i.properties.split_whitespace().any(|p| p == "cover-image"))
            .map(|i| i.path.clone())
            .or_else(|| cover_id.and_then(|id| ids.get(&id).cloned()).filter(|p| epub.is_image(p)))
            .or_else(|| guide_cover.and_then(|p| epub.first_image_of(&p)))
            .or_else(|| {
                let mut imgs: Vec<&Item> = epub
                    .items
                    .values()
                    .filter(|i| i.media_type.starts_with("image/") && i.path.to_ascii_lowercase().contains("cover"))
                    .collect();
                imgs.sort_by(|a, b| a.path.cmp(&b.path));
                imgs.first().map(|i| i.path.clone())
            });
        // The table of contents.
        let nav =
            epub.items.values().find(|i| i.properties.split_whitespace().any(|p| p == "nav")).map(|i| i.path.clone());
        if let Some(nav) = nav
            && let Ok(data) = read(&epub.archive, &nav)
        {
            epub.toc = nav_toc(&markup::events(&data), markup::dir_of(&nav));
        }
        if epub.toc.is_empty() {
            let ncx = toc_id.and_then(|id| ids.get(&id).cloned()).or_else(|| {
                epub.items.values().find(|i| i.media_type == "application/x-dtbncx+xml").map(|i| i.path.clone())
            });
            if let Some(ncx) = ncx
                && let Ok(data) = read(&epub.archive, &ncx)
            {
                epub.toc = ncx_toc(&markup::events(&data), markup::dir_of(&ncx));
            }
        }
        let uid = unique_id.filter(|u| !u.is_empty()).unwrap_or_else(|| epub.meta.identifier.clone());
        epub.encryption(&uid)?;
        Ok(epub)
    }

    /// Reads the package document. Returns the manifest ids to paths,
    /// the spine's NCX id, the cover meta's id, the unique identifier and
    /// the guide's cover page.
    fn package(&mut self, ev: &[Ev], base: &str) -> PackageRefs {
        let mut ids: HashMap<String, String> = HashMap::new();
        let mut spine_refs: Vec<(String, bool)> = Vec::new();
        let mut toc_id = None;
        let mut cover_id = None;
        let mut guide_cover = None;
        let mut uid_attr = None;
        let mut identifiers: Vec<(String, String)> = Vec::new();
        // Creators with an EPUB 3 role refinement: id → role.
        let mut roles: HashMap<String, String> = HashMap::new();
        let mut creators: Vec<(Option<String>, Option<String>, String)> = Vec::new();
        for (i, e) in ev.iter().enumerate() {
            let Ev::Start { name, .. } = e else { continue };
            match (name.as_str(), local(name)) {
                (_, "package") => uid_attr = e.attr("unique-identifier").map(str::to_string),
                (n, "title") if n.starts_with("dc:") || n == "title" => {
                    if self.meta.title.is_empty() {
                        self.meta.title = clean(&markup::text_until_end(ev, i), 500);
                    }
                }
                (n, "creator") if n.starts_with("dc:") => {
                    let t = clean(&markup::text_until_end(ev, i), 200);
                    let role = e.attr("opf:role").map(str::to_string);
                    creators.push((e.attr("id").map(str::to_string), role, t));
                }
                (n, "language") if n.starts_with("dc:") => {
                    if self.meta.language.is_empty() {
                        self.meta.language = clean(&markup::text_until_end(ev, i), 35);
                    }
                }
                (n, "publisher") if n.starts_with("dc:") => {
                    if self.meta.publisher.is_empty() {
                        self.meta.publisher = clean(&markup::text_until_end(ev, i), 200);
                    }
                }
                (n, "description") if n.starts_with("dc:") => {
                    if self.meta.description.is_empty() {
                        // Descriptions are often escaped HTML.
                        let raw = markup::text_until_end(ev, i);
                        self.meta.description = clean(&crate::mobi::html::plain_text(raw.as_bytes(), true), 5000);
                    }
                }
                (n, "subject") if n.starts_with("dc:") => {
                    let s = clean(&markup::text_until_end(ev, i), 100);
                    if !s.is_empty() && self.meta.subjects.len() < 20 && !self.meta.subjects.contains(&s) {
                        self.meta.subjects.push(s);
                    }
                }
                (n, "date") if n.starts_with("dc:") => {
                    if self.meta.published.is_empty() {
                        self.meta.published = clean(&markup::text_until_end(ev, i), 40);
                    }
                }
                (n, "identifier") if n.starts_with("dc:") => {
                    identifiers.push((e.attr("id").unwrap_or("").to_string(), markup::text_until_end(ev, i)));
                }
                (_, "meta") => {
                    if e.attr("name") == Some("cover") {
                        cover_id = e.attr("content").map(str::to_string);
                    }
                    if e.attr("property") == Some("role")
                        && let Some(r) = e.attr("refines")
                    {
                        roles.insert(r.trim_start_matches('#').to_string(), markup::text_until_end(ev, i));
                    }
                }
                (_, "item") => {
                    let (Some(id), Some(href)) = (e.attr("id"), e.attr("href")) else { continue };
                    let href = href.split('#').next().unwrap_or("");
                    let Some(path) = markup::resolve(base, &markup::percent_decode(href)) else { continue };
                    ids.insert(id.to_string(), path.clone());
                    self.items.insert(
                        path.clone(),
                        Item {
                            path,
                            media_type: e.attr("media-type").unwrap_or("").to_ascii_lowercase(),
                            properties: e.attr("properties").unwrap_or("").to_string(),
                        },
                    );
                }
                (_, "spine") => {
                    toc_id = e.attr("toc").map(str::to_string);
                    self.rtl = e.attr("page-progression-direction") == Some("rtl");
                }
                (_, "itemref") => {
                    if let Some(id) = e.attr("idref") {
                        spine_refs.push((id.to_string(), e.attr("linear") != Some("no")));
                    }
                }
                (_, "reference") => {
                    if e.attr("type").is_some_and(|t| t.eq_ignore_ascii_case("cover"))
                        && let Some(h) = e.attr("href")
                    {
                        guide_cover = markup::resolve(base, &markup::percent_decode(h.split('#').next().unwrap_or("")));
                    }
                }
                _ => {}
            }
        }
        // Authors: EPUB 3 refinements, EPUB 2 `opf:role`, else everyone.
        let authors: Vec<String> = creators
            .iter()
            .filter(|(id, role, _)| {
                let r = role.clone().or_else(|| id.as_ref().and_then(|i| roles.get(i).cloned()));
                r.is_none_or(|r| r.trim() == "aut")
            })
            .map(|(_, _, n)| n.clone())
            .filter(|n| !n.is_empty())
            .collect();
        self.meta.authors = if authors.is_empty() {
            creators.into_iter().map(|c| c.2).filter(|n| !n.is_empty()).collect()
        } else {
            authors
        };
        self.meta.authors.truncate(20);
        self.meta.identifier = identifiers
            .iter()
            .find(|(id, _)| Some(id) == uid_attr.as_ref())
            .or(identifiers.first())
            .map(|(_, v)| v.trim().to_string())
            .unwrap_or_default();
        let uid = identifiers.iter().find(|(id, _)| Some(id) == uid_attr.as_ref()).map(|(_, v)| v.clone());
        for (id, linear) in spine_refs {
            if let Some(path) = ids.get(&id)
                && self.archive.find(path).is_some()
                && self.spine.len() < 10_000
            {
                self.spine.push((path.clone(), linear));
            }
        }
        (ids, toc_id, cover_id, uid, guide_cover)
    }

    fn is_image(&self, path: &str) -> bool {
        self.items.get(path).is_some_and(|i| i.media_type.starts_with("image/"))
    }

    /// The first image a cover page shows.
    fn first_image_of(&self, page: &str) -> Option<String> {
        if self.is_image(page) {
            return Some(page.to_string());
        }
        let data = read(&self.archive, page).ok()?;
        let dir = markup::dir_of(page);
        markup::events(&data).iter().find_map(|e| {
            let Ev::Start { name, .. } = e else { return None };
            let src = match local(name) {
                "img" => e.attr("src"),
                "image" => e.attr("xlink:href").or_else(|| e.attr("href")),
                _ => None,
            }?;
            markup::resolve(dir, &markup::percent_decode(src.split('#').next().unwrap_or("")))
                .filter(|p| self.archive.find(p).is_some())
        })
    }

    /// `META-INF/encryption.xml`: obfuscated fonts are noted; anything
    /// else encrypted that the reader needs is DRM.
    fn encryption(&mut self, uid: &str) -> Result<(), BookError> {
        let Ok(data) = read(&self.archive, "META-INF/encryption.xml") else { return Ok(()) };
        let ev = markup::events(&data);
        let mut algorithm = String::new();
        for e in &ev {
            let Ev::Start { name, .. } = e else { continue };
            match local(name) {
                "encryptionmethod" => algorithm = e.attr("algorithm").unwrap_or("").to_string(),
                "cipherreference" => {
                    let Some(uri) = e.attr("uri") else { continue };
                    let Some(path) = markup::resolve("", &markup::percent_decode(uri)) else { continue };
                    let kind = match algorithm.as_str() {
                        "http://www.idpf.org/2008/embedding" => Some(Obfuscation::Idpf),
                        "http://ns.adobe.com/pdf/enc#RC" => Some(Obfuscation::Adobe),
                        _ => None,
                    };
                    match kind {
                        Some(k) => {
                            self.obfuscated.insert(path, k);
                        }
                        None => {
                            let doc = self
                                .items
                                .get(&path)
                                .is_some_and(|i| !i.media_type.contains("font") && !i.media_type.contains("opentype"));
                            if doc || self.spine.iter().any(|(p, _)| *p == path) {
                                return Err(BookError::drm());
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        self.font_key = Some(uid.to_string());
        Ok(())
    }

    /// A member's bytes, fonts de-obfuscated.
    pub fn member(&self, path: &str, limit: u64) -> Result<Vec<u8>, BookError> {
        let e = self.archive.find(path).ok_or_else(|| BookError::not_found(path))?;
        let mut data = self.archive.read(e, limit).map_err(|e| BookError::invalid(e.to_string()))?;
        if let Some(kind) = self.obfuscated.get(path) {
            deobfuscate(&mut data, *kind, self.font_key.as_deref().unwrap_or(&self.meta.identifier));
        }
        Ok(data)
    }

    pub fn media_type(&self, path: &str) -> String {
        if let Some(i) = self.items.get(path)
            && !i.media_type.is_empty()
        {
            return i.media_type.clone();
        }
        mime_by_extension(path).to_string()
    }
}

fn clean(s: &str, max: usize) -> String {
    let t: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    crate::mobi::html::collapse_ws(&t).chars().take(max).collect()
}

pub fn mime_by_extension(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "xhtml" | "html" | "htm" => "application/xhtml+xml",
        "css" => "text/css",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "mp3" => "audio/mpeg",
        "mp4" | "m4v" => "video/mp4",
        "m4a" => "audio/mp4",
        "ogg" => "audio/ogg",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    }
}

/// IDPF (first 1040 bytes XOR the SHA-1 of the identifier without
/// whitespace) and Adobe (first 1024 bytes XOR the UUID's 16 bytes).
fn deobfuscate(data: &mut [u8], kind: Obfuscation, identifier: &str) {
    let key: Vec<u8> = match kind {
        Obfuscation::Idpf => {
            let id: String = identifier.chars().filter(|c| !matches!(c, ' ' | '\t' | '\r' | '\n')).collect();
            Sha1::digest(id.as_bytes()).to_vec()
        }
        Obfuscation::Adobe => {
            let hex: String =
                identifier.trim_start_matches("urn:uuid:").chars().filter(char::is_ascii_hexdigit).collect();
            if hex.len() < 32 {
                return;
            }
            (0..16).filter_map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()).collect()
        }
    };
    if key.is_empty() {
        return;
    }
    let n = if kind == Obfuscation::Idpf { 1040 } else { 1024 };
    for (i, b) in data.iter_mut().take(n).enumerate() {
        *b ^= key[i % key.len()];
    }
}

/// The EPUB 3 navigation document's `toc` nav (or its first nav).
fn nav_toc(ev: &[Ev], dir: &str) -> Vec<TocNode> {
    let navs: Vec<usize> = ev
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e, Ev::Start { name, .. } if local(name) == "nav"))
        .map(|(i, _)| i)
        .collect();
    let start = navs
        .iter()
        .copied()
        .find(|&i| ev[i].attr("epub:type").is_some_and(|t| t.split_whitespace().any(|x| x == "toc")))
        .or_else(|| navs.first().copied());
    let Some(start) = start else { return Vec::new() };
    // Walk the nested lists: a stack of children vectors.
    let mut stack: Vec<Vec<TocNode>> = vec![Vec::new()];
    let mut depth = 0usize;
    let mut i = start + 1;
    while i < ev.len() {
        match &ev[i] {
            Ev::End { name } if local(name) == "nav" => break,
            // Deeper lists are flattened into the deepest kept level.
            Ev::Start { name, closed: false, .. } if local(name) == "ol" => {
                if depth < MAX_TOC_DEPTH {
                    stack.push(Vec::new());
                }
                depth += 1;
            }
            Ev::End { name } if local(name) == "ol" => {
                if depth > 0 {
                    depth -= 1;
                    if depth >= MAX_TOC_DEPTH {
                        i += 1;
                        continue;
                    }
                    let kids = stack.pop().unwrap_or_default();
                    if let Some(parent) = stack.last_mut().and_then(|v| v.last_mut()) {
                        parent.children = kids;
                    } else if let Some(top) = stack.last_mut() {
                        top.extend(kids);
                    }
                }
            }
            Ev::Start { name, .. } if local(name) == "a" || local(name) == "span" => {
                let title = clean(&markup::text_until_end(ev, i), 300);
                let href = ev[i].attr("href").unwrap_or("");
                let (path, anchor) = split_href(dir, href);
                if let Some(level) = stack.last_mut() {
                    level.push(TocNode { title, path, anchor, children: Vec::new() });
                }
            }
            _ => {}
        }
        i += 1;
    }
    while stack.len() > 1 {
        let kids = stack.pop().unwrap_or_default();
        if let Some(top) = stack.last_mut() {
            top.extend(kids);
        }
    }
    let mut out = stack.pop().unwrap_or_default();
    prune(&mut out, 0);
    out
}

/// The NCX `navMap`.
fn ncx_toc(ev: &[Ev], dir: &str) -> Vec<TocNode> {
    let mut skipped = 0usize;
    let mut stack: Vec<Vec<TocNode>> = vec![Vec::new()];
    let mut pending: Vec<TocNode> = Vec::new();
    for (i, e) in ev.iter().enumerate() {
        match e {
            Ev::Start { name, closed: false, .. } if local(name) == "navpoint" => {
                if pending.len() >= MAX_TOC_DEPTH || stack.iter().map(Vec::len).sum::<usize>() > 20_000 {
                    // Too deep or too many: this point is skipped, its end too.
                    skipped += 1;
                    continue;
                }
                pending.push(TocNode::default());
                stack.push(Vec::new());
            }
            Ev::Start { name, .. } if local(name) == "text" => {
                if let Some(p) = pending.last_mut()
                    && p.title.is_empty()
                {
                    p.title = clean(&markup::text_until_end(ev, i), 300);
                }
            }
            Ev::Start { name, .. } if local(name) == "content" => {
                if let Some(p) = pending.last_mut()
                    && p.path.is_empty()
                {
                    let (path, anchor) = split_href(dir, e.attr("src").unwrap_or(""));
                    p.path = path;
                    p.anchor = anchor;
                }
            }
            Ev::End { name } if local(name) == "navpoint" => {
                if skipped > 0 {
                    skipped -= 1;
                    continue;
                }
                if let (Some(mut node), Some(kids)) = (pending.pop(), stack.pop()) {
                    node.children = kids;
                    if let Some(level) = stack.last_mut() {
                        level.push(node);
                    }
                }
            }
            _ => {}
        }
    }
    let mut out = stack.into_iter().next().unwrap_or_default();
    prune(&mut out, 0);
    out
}

fn prune(v: &mut Vec<TocNode>, depth: usize) {
    if depth > 8 {
        v.clear();
        return;
    }
    v.retain(|n| !n.title.is_empty() || !n.children.is_empty());
    v.truncate(5000);
    for n in v.iter_mut() {
        prune(&mut n.children, depth + 1);
    }
}

fn split_href(dir: &str, href: &str) -> (String, String) {
    let (p, a) = href.split_once('#').unwrap_or((href, ""));
    let path =
        if p.is_empty() { String::new() } else { markup::resolve(dir, &markup::percent_decode(p)).unwrap_or_default() };
    (path, markup::percent_decode(a))
}
