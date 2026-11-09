//! `lain-metadata-nfo`: Kodi-style `.nfo` sidecars (movie, tvshow,
//! episodedetails) next to the media. Offline, user-curated and first
//! in the default precedence: when the user wrote down what something
//! is, the network does not get a vote. The remote id is the `.nfo`
//! path. A tolerant reader of the small XML subset these files use
//! (elements, attributes, text, CDATA, entities, comments) stands in
//! for Go's non-strict `encoding/xml`.

use std::path::Path;

use serde_json::{Value, json};

use lain_core::{Error, Provider};

use crate::types::{MetadataCandidate, MetadataRecord, MetadataResolveInput, MetadataSearchInput};

pub const NFO_ID: &str = "lain-metadata-nfo";
pub const CAP_METADATA_SEARCH: &str = "lain.metadata.search@1";
pub const CAP_METADATA_RESOLVE: &str = "lain.metadata.resolve@1";

/// The largest `.nfo` read.
const MAX_NFO: u64 = 1 << 20;

#[derive(Debug, Default, Clone, PartialEq)]
struct Node {
    name: String,
    attrs: Vec<(String, String)>,
    text: String,
    children: Vec<Node>,
}

impl Node {
    fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().rev().find(|c| c.name == name)
    }
    fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> {
        self.children.iter().filter(move |c| c.name == name)
    }
    fn text_of(&self, name: &str) -> String {
        self.child(name).map(|c| c.text.clone()).unwrap_or_default()
    }
    fn attr(&self, name: &str) -> &str {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str()).unwrap_or("")
    }
}

fn local(name: &str) -> String {
    name.rsplit(':').next().unwrap_or(name).to_string()
}

fn decode_entities(s: &str) -> String {
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
            Some(c) => out.push(c),
            // Unknown entities stay as written (Go's non-strict mode).
            None => {
                out.push('&');
                out.push_str(ent);
                out.push(';');
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Parses the first root element. Unclosed elements close at the end;
/// a stray end tag closes the nearest open element of that name.
fn parse_xml(src: &str) -> Option<Node> {
    let mut stack: Vec<Node> = Vec::new();
    let mut root: Option<Node> = None;
    let mut rest = src;
    while !rest.is_empty() {
        let Some(lt) = rest.find('<') else {
            if let Some(top) = stack.last_mut() {
                top.text.push_str(&decode_entities(rest));
            }
            break;
        };
        if lt > 0
            && let Some(top) = stack.last_mut()
        {
            top.text.push_str(&decode_entities(&rest[..lt]));
        }
        rest = &rest[lt..];
        if let Some(r) = rest.strip_prefix("<!--") {
            rest = r.find("-->").map(|e| &r[e + 3..]).unwrap_or("");
            continue;
        }
        if let Some(r) = rest.strip_prefix("<![CDATA[") {
            let end = r.find("]]>").unwrap_or(r.len());
            if let Some(top) = stack.last_mut() {
                top.text.push_str(&r[..end]);
            }
            rest = r.get(end + 3..).unwrap_or("");
            continue;
        }
        if rest.starts_with("<?") || rest.starts_with("<!") {
            rest = rest.find('>').map(|e| &rest[e + 1..]).unwrap_or("");
            continue;
        }
        let Some(gt) = rest.find('>') else { break };
        let tag = &rest[1..gt];
        rest = &rest[gt + 1..];
        if let Some(name) = tag.strip_prefix('/') {
            let name = local(name.trim());
            if let Some(pos) = stack.iter().rposition(|n| n.name == name) {
                while stack.len() > pos {
                    let done = stack.pop()?;
                    match stack.last_mut() {
                        Some(parent) => parent.children.push(done),
                        None => root = root.or(Some(done)),
                    }
                }
            }
            if root.is_some() && stack.is_empty() {
                break;
            }
            continue;
        }
        let self_closing = tag.ends_with('/');
        let tag = tag.trim_end_matches('/');
        let (name, attr_src) = tag.split_once(|c: char| c.is_whitespace()).unwrap_or((tag, ""));
        let node = Node { name: local(name), attrs: parse_attrs(attr_src), ..Node::default() };
        if self_closing {
            match stack.last_mut() {
                Some(parent) => parent.children.push(node),
                None => {
                    root = Some(node);
                    break;
                }
            }
        } else {
            stack.push(node);
        }
    }
    while let Some(done) = stack.pop() {
        match stack.last_mut() {
            Some(parent) => parent.children.push(done),
            None => root = root.or(Some(done)),
        }
    }
    root
}

fn parse_attrs(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = s.trim();
    while !rest.is_empty() {
        let Some(eq) = rest.find('=') else { break };
        let key = local(rest[..eq].trim());
        let after = rest[eq + 1..].trim_start();
        let (val, next) = match after.chars().next() {
            Some(q @ ('"' | '\'')) => match after[1..].find(q) {
                Some(e) => (&after[1..1 + e], &after[e + 2..]),
                None => (&after[1..], ""),
            },
            _ => {
                let e = after.find(char::is_whitespace).unwrap_or(after.len());
                (&after[..e], &after[e..])
            }
        };
        out.push((key, decode_entities(val)));
        rest = next.trim_start();
    }
    out
}

/// Reads one `.nfo` into a record.
pub fn resolve_nfo(path: &Path) -> Result<MetadataRecord, Error> {
    let meta = std::fs::metadata(path).map_err(|e| Error::not_found(format!("{}: {e}", path.display())))?;
    if meta.len() > MAX_NFO {
        return Err(Error::invalid(format!("{} is larger than {MAX_NFO} bytes", path.display())));
    }
    let raw = std::fs::read(path).map_err(|e| Error::not_found(format!("{}: {e}", path.display())))?;
    let doc = parse_xml(&String::from_utf8_lossy(&raw))
        .ok_or_else(|| Error::invalid(format!("{}: no XML document", path.display())))?;
    let mut title = doc.text_of("title").trim().to_string();
    if title.is_empty() {
        title = doc.text_of("originaltitle").trim().to_string();
    }
    let year_s = doc.text_of("year");
    let premiered = doc.text_of("premiered");
    let year = match year_s.trim().parse::<i64>() {
        Ok(y) if (1900..=2099).contains(&y) => y,
        _ => premiered.get(..4).and_then(|p| p.parse().ok()).unwrap_or(0),
    };
    let thumbs: Vec<(&str, String)> = doc.all("thumb").map(|t| (t.attr("aspect"), t.text.trim().to_string())).collect();
    let mut poster =
        thumbs.iter().find(|(a, u)| !u.is_empty() && *a == "poster").map(|(_, u)| u.clone()).unwrap_or_default();
    let mut cover = String::new();
    for (_, u) in &thumbs {
        if u.is_empty() {
            continue;
        }
        if poster.is_empty() {
            poster = u.clone();
        } else if cover.is_empty() && *u != poster {
            cover = u.clone();
        }
    }
    if cover.is_empty()
        && let Some(f) = doc.child("fanart").and_then(|f| f.all("thumb").next())
    {
        cover = f.text.trim().to_string();
    }
    Ok(MetadataRecord {
        provider: NFO_ID.into(),
        remote_id: path.to_string_lossy().into_owned(),
        title,
        year,
        genres: doc.all("genre").map(|g| g.text.clone()).collect(),
        synopsis: doc.text_of("plot").trim().to_string(),
        poster,
        cover,
        ..MetadataRecord::default()
    })
}

/// The `.nfo` files of `dir` that answer `query`: show and movie roots
/// always count, other sidecars only when their name mentions the
/// query (a folder of episode files must not all match). At most eight
/// files are read.
pub fn search_nfo(input: &MetadataSearchInput) -> Vec<MetadataCandidate> {
    let mut out = Vec::new();
    if input.dir.is_empty() {
        return out;
    }
    let Ok(rd) = std::fs::read_dir(&input.dir) else { return out };
    let mut names: Vec<(String, bool)> = rd
        .flatten()
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.file_type().is_ok_and(|t| t.is_dir())))
        .collect();
    names.sort();
    let q = input.query.to_lowercase();
    let mut checked = 0;
    for (name, is_dir) in names {
        if is_dir || checked >= 8 {
            continue;
        }
        let low = name.to_lowercase();
        if !low.ends_with(".nfo") {
            continue;
        }
        if low != "tvshow.nfo" && low != "movie.nfo" && !low.contains(&q) {
            continue;
        }
        checked += 1;
        let path = Path::new(&input.dir).join(&name);
        let Ok(rec) = resolve_nfo(&path) else { continue };
        out.push(MetadataCandidate {
            provider: NFO_ID.into(),
            remote_id: path.to_string_lossy().into_owned(),
            title: rec.title,
            year: rec.year,
            kind: input.kind.clone(),
            poster: rec.poster,
            ..MetadataCandidate::default()
        });
        if input.limit > 0 && out.len() as i64 >= input.limit {
            break;
        }
    }
    out
}

pub struct Nfo;

impl Provider for Nfo {
    fn id(&self) -> &str {
        NFO_ID
    }

    fn capabilities(&self) -> Vec<String> {
        vec![CAP_METADATA_SEARCH.into(), CAP_METADATA_RESOLVE.into()]
    }

    fn health(&self) -> Result<(), Error> {
        Ok(())
    }

    fn invoke(&self, cap: &str, op: &str, input: Value) -> Result<Value, Error> {
        match cap {
            CAP_METADATA_SEARCH if op.is_empty() || op == "search" => {
                let i: MetadataSearchInput = serde_json::from_value(input)
                    .map_err(|e| Error::invalid(format!("MetadataSearchInput required: {e}")))?;
                Ok(json!(search_nfo(&i)))
            }
            CAP_METADATA_RESOLVE if op.is_empty() || op == "resolve" => {
                let i: MetadataResolveInput = serde_json::from_value(input)
                    .map_err(|e| Error::invalid(format!("MetadataResolveInput required: {e}")))?;
                Ok(json!(resolve_nfo(Path::new(&i.remote_id))?))
            }
            CAP_METADATA_SEARCH | CAP_METADATA_RESOLVE => {
                Err(Error::invalid(format!("capability {cap} has no op {op}")))
            }
            _ => Err(Error::invalid(format!("unsupported cap {cap}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tolerant_xml() {
        let doc = parse_xml(
            "<?xml version=\"1.0\"?><!-- c --><tvshow><title>A &amp; B</title><plot><![CDATA[<b>x</b>]]></plot>\
             <genre>Drama</genre><genre>Fantasy</genre><thumb aspect='poster'>p.jpg</thumb><thumb/>\
             <fanart><thumb>f.jpg</thumb></fanart><year>&#50;023</year><unclosed>",
        )
        .unwrap();
        assert_eq!(doc.name, "tvshow");
        assert_eq!(doc.text_of("title"), "A & B");
        assert_eq!(doc.text_of("plot"), "<b>x</b>");
        assert_eq!(doc.all("genre").count(), 2);
        assert_eq!(doc.child("thumb").map(|t| t.name.as_str()), Some("thumb"));
        assert_eq!(doc.text_of("year"), "2023");
        assert!(parse_xml("no xml").is_none());
        assert_eq!(decode_entities("&bogus; &#xZZ; & x"), "&bogus; &#xZZ; & x");
    }

    #[test]
    fn search_and_resolve() {
        let dir = std::env::temp_dir().join(format!("lain-nfo-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("tvshow.nfo"),
            "<tvshow><title> </title><originaltitle>Sousou no Frieren</originaltitle><premiered>2023-09-29</premiered>\
             <thumb>a.jpg</thumb><thumb aspect=\"poster\">p.jpg</thumb><thumb>a.jpg</thumb></tvshow>",
        )
        .unwrap();
        std::fs::write(dir.join("Frieren - 01.nfo"), "<episodedetails><title>Ep 1</title></episodedetails>").unwrap();
        std::fs::write(dir.join("Other - 01.nfo"), "<episodedetails><title>Other</title></episodedetails>").unwrap();
        std::fs::write(dir.join("broken.nfo"), "").unwrap();
        let rec = resolve_nfo(&dir.join("tvshow.nfo")).unwrap();
        assert_eq!((rec.title.as_str(), rec.year), ("Sousou no Frieren", 2023));
        assert_eq!((rec.poster.as_str(), rec.cover.as_str()), ("p.jpg", "a.jpg"));
        let hits = search_nfo(&MetadataSearchInput {
            query: "Frieren".into(),
            kind: "anime".into(),
            limit: 5,
            dir: dir.to_string_lossy().into(),
        });
        let titles: Vec<_> = hits.iter().map(|h| h.title.as_str()).collect();
        assert_eq!(titles, vec!["Ep 1", "Sousou no Frieren"]);
        assert!(search_nfo(&MetadataSearchInput { dir: String::new(), ..Default::default() }).is_empty());
        assert_eq!(
            Nfo.invoke(CAP_METADATA_RESOLVE, "", json!({"remote_id": "/nope.nfo"})).unwrap_err().code,
            "not-found"
        );
        std::fs::remove_dir_all(dir).ok();
    }
}
