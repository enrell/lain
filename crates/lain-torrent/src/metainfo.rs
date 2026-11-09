//! v1 metainfo (`.torrent`), info dictionaries and magnet links.

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::sync::Arc;

use sha1::{Digest, Sha1};

use crate::Error;
use crate::bencode::{self, Value};
use crate::error::meta;

// Bounds on what a metainfo may declare.
const MAX_PIECE_LENGTH: i64 = 64 << 20;
const MAX_TOTAL_BYTES: i64 = 1 << 40;
const MAX_FILES: usize = 100_000;
const MAX_PATH_DEPTH: usize = 32;

pub(crate) fn sha1(data: &[u8]) -> [u8; 20] {
    Sha1::digest(data).into()
}

/// Identifies a torrent: SHA-1 of the bencoded info dictionary.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InfoHash(pub [u8; 20]);

impl InfoHash {
    pub fn hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }
    /// Reads 40 hex characters or 32 base32 ones.
    pub fn parse(s: &str) -> Result<InfoHash, Error> {
        match s.len() {
            40 => {
                let mut h = [0u8; 20];
                for (i, out) in h.iter_mut().enumerate() {
                    *out = u8::from_str_radix(s.get(2 * i..2 * i + 2).unwrap_or("zz"), 16)
                        .map_err(|_| meta("info hash must be 40 hex characters"))?;
                }
                Ok(InfoHash(h))
            }
            32 => base32(s).map(InfoHash).ok_or_else(|| meta("bad base32 info hash")),
            _ => Err(meta("info hash must be 40 hex characters")),
        }
    }
}

impl fmt::Display for InfoHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.hex())
    }
}

impl fmt::Debug for InfoHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "InfoHash({})", self.hex())
    }
}

fn base32(s: &str) -> Option<[u8; 20]> {
    let mut out = [0u8; 20];
    let (mut acc, mut bits, mut n) = (0u64, 0, 0);
    for c in s.bytes() {
        let v = match c.to_ascii_uppercase() {
            c @ b'A'..=b'Z' => c - b'A',
            c @ b'2'..=b'7' => c - b'2' + 26,
            _ => return None,
        };
        acc = (acc << 5) | v as u64;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            *out.get_mut(n)? = (acc >> bits) as u8;
            n += 1;
        }
    }
    (n == 20).then_some(out)
}

/// One file of a torrent, in piece order. `path` is relative (the
/// torrent name first for a multi-file torrent) and validated: no empty,
/// "." or ".." components, no separators.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    pub path: Vec<String>,
    pub length: i64,
    pub offset: i64,
}

impl File {
    /// The components joined with the OS separator.
    pub fn rel_path(&self) -> String {
        self.path.join(std::path::MAIN_SEPARATOR_STR)
    }
}

/// The validated info dictionary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    pub name: String,
    pub piece_length: i64,
    pub pieces: Vec<[u8; 20]>,
    pub files: Vec<File>,
    pub length: i64,
    pub private: bool,
    /// A directory torrent (files under `name/`).
    pub multi: bool,
    /// The exact bencoded info dictionary (served over ut_metadata).
    pub raw: Vec<u8>,
}

impl Info {
    pub fn num_pieces(&self) -> usize {
        self.pieces.len()
    }
    /// The length of piece n (the last one may be short).
    pub fn piece_size(&self, n: usize) -> i64 {
        if n + 1 == self.pieces.len() {
            let r = self.length - n as i64 * self.piece_length;
            if r > 0 {
                return r;
            }
        }
        self.piece_length
    }
}

/// A parsed `.torrent` file.
#[derive(Debug, Clone)]
pub struct MetaInfo {
    pub info_hash: InfoHash,
    pub info: Arc<Info>,
    /// Announce URLs, tiers flattened in order, deduplicated.
    pub trackers: Vec<String>,
}

/// Parses and validates a `.torrent` file.
pub fn parse_metainfo(data: &[u8]) -> Result<MetaInfo, Error> {
    let (top, raw) = bencode::decode_dict(data)?;
    let raw_info = raw.get(&b"info"[..]).ok_or_else(|| meta("no info dictionary"))?;
    let info = parse_info(raw_info)?;
    let mut mi = MetaInfo { info_hash: InfoHash(sha1(raw_info)), info: Arc::new(info), trackers: Vec::new() };
    let mut seen = HashSet::new();
    let mut add = |u: &[u8]| {
        if let Ok(u) = std::str::from_utf8(u)
            && !u.is_empty()
            && valid_tracker_url(u)
            && seen.insert(u.to_string())
        {
            mi.trackers.push(u.to_string());
        }
    };
    if let Some(Value::List(tiers)) = top.get(&b"announce-list"[..]) {
        for tier in tiers {
            if let Value::List(l) = tier {
                for u in l {
                    if let Value::Bytes(s) = u {
                        add(s);
                    }
                }
            }
        }
    }
    if let Some(Value::Bytes(a)) = top.get(&b"announce"[..]) {
        add(a);
    }
    Ok(mi)
}

/// Splits `scheme://authority/rest` into the lower-cased scheme and the
/// host (authority without user info); `None` when it is not a URL.
pub(crate) fn split_url(s: &str) -> Option<(String, String, String)> {
    let (scheme, rest) = s.split_once("://")?;
    if scheme.is_empty()
        || !scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        || !scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
        || s.bytes().any(|b| b < 0x20 || b == 0x7f || b == b' ')
    {
        return None;
    }
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    let host = authority.rsplit_once('@').map(|(_, h)| h).unwrap_or(authority);
    Some((scheme.to_ascii_lowercase(), host.to_string(), rest[end..].to_string()))
}

pub(crate) fn valid_tracker_url(s: &str) -> bool {
    match split_url(s) {
        Some((scheme, host, _)) => !host.is_empty() && matches!(scheme.as_str(), "http" | "https" | "udp"),
        None => false,
    }
}

/// Rejects path parts that could escape the download dir.
fn safe_component(s: &str) -> bool {
    !(s.is_empty() || s == "." || s == ".." || s.len() > 255 || s.contains(['/', '\\', '\0']))
}

fn text(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str).map(str::to_string)
}

/// Validates a raw bencoded info dictionary.
pub fn parse_info(raw: &[u8]) -> Result<Info, Error> {
    let v = bencode::decode(raw)?;
    let Value::Dict(d) = &v else { return Err(meta("info is not a dictionary")) };
    let get = |k: &str| d.get(k.as_bytes());
    let name = text(get("name")).filter(|n| safe_component(n)).ok_or_else(|| meta("unsafe or missing name"))?;
    let piece_length = get("piece length")
        .and_then(Value::as_int)
        .filter(|&n| n > 0 && n <= MAX_PIECE_LENGTH)
        .ok_or_else(|| meta("bad piece length"))?;
    let pieces = get("pieces")
        .and_then(Value::as_bytes)
        .filter(|p| !p.is_empty() && p.len() % 20 == 0)
        .ok_or_else(|| meta("bad pieces"))?;
    let mut info = Info {
        name,
        piece_length,
        pieces: Vec::new(),
        files: Vec::new(),
        length: 0,
        private: get("private").and_then(Value::as_int) == Some(1),
        multi: false,
        raw: raw.to_vec(),
    };
    if let Some(length) = get("length").and_then(Value::as_int) {
        if length < 0 {
            return Err(meta("negative length"));
        }
        info.files = vec![File { path: vec![info.name.clone()], length, offset: 0 }];
        info.length = length;
    } else {
        let files = get("files")
            .and_then(Value::as_list)
            .filter(|f| !f.is_empty() && f.len() <= MAX_FILES)
            .ok_or_else(|| meta("no length and no files"))?;
        info.multi = true;
        let mut seen = HashSet::new();
        for fv in files {
            let Value::Dict(fd) = fv else { return Err(meta("file entry is not a dictionary")) };
            let length = fd
                .get(&b"length"[..])
                .and_then(Value::as_int)
                .filter(|&n| n >= 0)
                .ok_or_else(|| meta("bad file length"))?;
            let parts = fd
                .get(&b"path"[..])
                .and_then(Value::as_list)
                .filter(|p| !p.is_empty() && p.len() <= MAX_PATH_DEPTH)
                .ok_or_else(|| meta("bad file path"))?;
            let mut f = File { path: vec![info.name.clone()], length, offset: info.length };
            for p in parts {
                let s = p.as_str().filter(|s| safe_component(s)).ok_or_else(|| meta("unsafe file path"))?;
                f.path.push(s.to_string());
            }
            if !seen.insert(f.path.join("/").to_lowercase()) {
                return Err(meta("duplicate file path"));
            }
            info.files.push(f);
            info.length += length;
            if info.length > MAX_TOTAL_BYTES {
                return Err(meta("torrent too large"));
            }
        }
    }
    if info.length > MAX_TOTAL_BYTES {
        return Err(meta("torrent too large"));
    }
    let n = pieces.len() / 20;
    let want = if info.length == 0 { 1 } else { (info.length + piece_length - 1) / piece_length };
    if n as i64 != want {
        return Err(meta(format!("piece count {n} does not match length (want {want})")));
    }
    info.pieces = pieces.as_chunks::<20>().0.to_vec();
    Ok(info)
}

/// A parsed magnet link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Magnet {
    pub info_hash: InfoHash,
    pub name: String,
    pub trackers: Vec<String>,
}

/// Reads `magnet:?xt=urn:btih:<hex|base32>&dn=..&tr=..`.
pub fn parse_magnet(s: &str) -> Result<Magnet, Error> {
    let not = || meta("not a magnet link");
    let (scheme, rest) = s.split_once(':').ok_or_else(not)?;
    if !scheme.eq_ignore_ascii_case("magnet") {
        return Err(not());
    }
    let rest = rest.split('#').next().unwrap_or("");
    let query = rest.split_once('?').map(|(_, q)| q).unwrap_or("");
    let pairs = parse_query(query);
    let first = |k: &str| pairs.iter().find(|(pk, _)| pk == k).map(|(_, v)| v.clone());
    let mut m = Magnet { info_hash: InfoHash::default(), name: first("dn").unwrap_or_default(), trackers: Vec::new() };
    let mut found = false;
    for (k, xt) in &pairs {
        const P: &str = "urn:btih:";
        if k != "xt" || xt.len() < P.len() || !xt[..P.len()].eq_ignore_ascii_case(P) {
            continue;
        }
        let v = &xt[P.len()..];
        m.info_hash = match v.len() {
            40 => InfoHash::parse(v)?,
            32 => InfoHash(base32(v).ok_or_else(|| meta("bad base32 info hash"))?),
            _ => return Err(meta("bad info hash length")),
        };
        found = true;
        break;
    }
    if !found {
        return Err(meta("magnet has no btih"));
    }
    let mut seen = HashSet::new();
    for (k, tr) in &pairs {
        if k == "tr" && valid_tracker_url(tr) && seen.insert(tr.clone()) {
            m.trackers.push(tr.clone());
        }
    }
    Ok(m)
}

impl fmt::Display for Magnet {
    /// Go's `url.Values.Encode`: keys sorted (dn, tr, xt).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut q: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        q.entry("xt").or_default().push(format!("urn:btih:{}", self.info_hash.hex()));
        if !self.name.is_empty() {
            q.entry("dn").or_default().push(self.name.clone());
        }
        for tr in &self.trackers {
            q.entry("tr").or_default().push(tr.clone());
        }
        let parts: Vec<String> = q
            .iter()
            .flat_map(|(k, vs)| vs.iter().map(move |v| format!("{k}={}", query_escape(v.as_bytes()))))
            .collect();
        write!(f, "magnet:?{}", parts.join("&"))
    }
}

/// Go's `url.QueryEscape` over raw bytes.
pub(crate) fn query_escape(b: &[u8]) -> String {
    let mut out = String::with_capacity(b.len() * 3);
    for &c in b {
        match c {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(c as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{c:02X}")),
        }
    }
    out
}

/// Percent-decodes form encoding into bytes (`+` is a space); `None`
/// on a bad escape.
pub(crate) fn unescape_bytes(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' => {
                let hex = b.get(i + 1..i + 3)?;
                out.push(u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?);
                i += 2;
            }
            c => out.push(c),
        }
        i += 1;
    }
    Some(out)
}

/// A query string as text pairs; pairs that do not decode are dropped,
/// as Go's `URL.Query` does.
pub(crate) fn parse_query(q: &str) -> Vec<(String, String)> {
    q.split('&')
        .filter(|p| !p.is_empty())
        .filter_map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            let k = String::from_utf8(unescape_bytes(k)?).ok()?;
            let v = String::from_utf8(unescape_bytes(v)?).ok()?;
            Some((k, v))
        })
        .collect()
}

/// Content for [`build`].
#[derive(Debug, Clone)]
pub struct SourceFile {
    /// Relative to the torrent name; one element for a single file.
    pub path: Vec<String>,
    pub data: Vec<u8>,
}

/// Creates a `.torrent` from in-memory content. Lain never authors
/// torrents for publishing; tests and fixtures use it to make swarms.
pub fn build(
    name: &str,
    files: &[SourceFile],
    piece_length: i64,
    trackers: &[String],
) -> Result<(Vec<u8>, MetaInfo), Error> {
    if piece_length <= 0 {
        return Err(meta("bad piece length"));
    }
    let mut all = Vec::new();
    let mut info: Vec<(&str, Value)> = vec![("name", Value::str(name)), ("piece length", Value::Int(piece_length))];
    if files.len() == 1 && files[0].path.len() <= 1 {
        info.push(("length", Value::Int(files[0].data.len() as i64)));
        all.extend_from_slice(&files[0].data);
    } else {
        let list = files
            .iter()
            .map(|f| {
                all.extend_from_slice(&f.data);
                Value::dict([
                    ("length", Value::Int(f.data.len() as i64)),
                    ("path", Value::List(f.path.iter().map(Value::str).collect())),
                ])
            })
            .collect();
        info.push(("files", Value::List(list)));
    }
    let mut pieces = Vec::new();
    if all.is_empty() {
        pieces.extend_from_slice(&sha1(&[]));
    }
    for chunk in all.chunks(piece_length as usize) {
        pieces.extend_from_slice(&sha1(chunk));
    }
    info.push(("pieces", Value::Bytes(pieces)));
    let mut top = vec![("info", Value::dict(info))];
    if let Some(first) = trackers.first() {
        top.push(("announce", Value::str(first)));
        top.push(("announce-list", Value::List(vec![Value::List(trackers.iter().map(Value::str).collect())])));
    }
    let raw = bencode::encode(&Value::dict(top));
    let mi = parse_metainfo(&raw)?;
    Ok((raw, mi))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::Rng;

    fn fixture(n: usize, seed: u8) -> Vec<u8> {
        let mut x = seed as u32 + 1;
        (0..n)
            .map(|_| {
                x = x.wrapping_mul(1664525).wrapping_add(1013904223);
                (x >> 24) as u8
            })
            .collect()
    }

    fn sf(path: &[&str], data: Vec<u8>) -> SourceFile {
        SourceFile { path: path.iter().map(|s| s.to_string()).collect(), data }
    }

    #[test]
    fn build_and_parse_multi_file() {
        let files =
            [sf(&["[Fansub-A] Show - 01.mkv"], fixture(40000, 1)), sf(&["extras", "notes.txt"], fixture(123, 2))];
        let trackers =
            ["http://tracker-exemplo.invalid/announce".to_string(), "udp://tracker-exemplo.invalid:6969".to_string()];
        let (raw, mi) = build("[Fansub-A] Show", &files, 16384, &trackers).unwrap();
        let info = &mi.info;
        assert!(info.multi && info.files.len() == 2 && info.length == 40123 && info.num_pieces() == 3);
        assert_eq!(info.files[1].offset, 40000);
        assert_eq!(info.files[1].path, vec!["[Fansub-A] Show", "extras", "notes.txt"]);
        #[cfg(unix)]
        assert_eq!(info.files[1].rel_path(), "[Fansub-A] Show/extras/notes.txt");
        assert_eq!(info.piece_size(2), 40123 - 2 * 16384);
        assert_eq!(mi.trackers.len(), 2);
        let (_, rawd) = bencode::decode_dict(&raw).unwrap();
        assert_eq!(mi.info_hash.0, sha1(&rawd[&b"info"[..]]), "info hash over the original bytes");
        assert_eq!(info.raw, rawd[&b"info"[..]]);
    }

    #[test]
    fn single_file() {
        let (_, mi) = build("Movie (2001).mkv", &[sf(&[], fixture(1000, 3))], 16384, &[]).unwrap();
        assert!(!mi.info.multi);
        assert_eq!(mi.info.files[0].rel_path(), "Movie (2001).mkv");
        assert_eq!(mi.info.num_pieces(), 1);
    }

    #[test]
    fn rejects_unsafe_metainfo() {
        let pieces = Value::Bytes(vec![0; 20]);
        let file = |len: i64, path: &[&str]| {
            Value::dict([("length", Value::Int(len)), ("path", Value::List(path.iter().map(Value::str).collect()))])
        };
        let base = |name: &str, plen: i64, pieces: Value| {
            vec![("name", Value::str(name)), ("piece length", Value::Int(plen)), ("pieces", pieces)]
        };
        let with = |mut v: Vec<(&'static str, Value)>, k: &'static str, x: Value| {
            v.push((k, x));
            v
        };
        let cases: Vec<(&str, Vec<(&str, Value)>)> = vec![
            ("dotdot name", with(base("..", 16, pieces.clone()), "length", Value::Int(1))),
            ("slash name", with(base("a/b", 16, pieces.clone()), "length", Value::Int(1))),
            ("traversal", with(base("x", 16, pieces.clone()), "files", Value::List(vec![file(1, &["..", "etc"])]))),
            ("zero plen", with(base("x", 0, pieces.clone()), "length", Value::Int(1))),
            ("pieces count", with(base("x", 16, Value::Bytes(vec![0; 40])), "length", Value::Int(1))),
            ("ragged pieces", with(base("x", 16, Value::str("abc")), "length", Value::Int(1))),
            (
                "duplicate",
                with(base("x", 16, pieces.clone()), "files", Value::List(vec![file(1, &["a"]), file(0, &["A"])])),
            ),
        ];
        for (name, info) in cases {
            let raw = bencode::encode(&Value::dict([("info", Value::dict(info))]));
            assert!(matches!(parse_metainfo(&raw), Err(Error::Meta(_))), "{name}");
        }
    }

    #[test]
    fn magnet() {
        let m = parse_magnet("magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Show&tr=http%3A%2F%2Ftracker-exemplo.invalid%2Fa&tr=ftp%3A%2F%2Fx").unwrap();
        assert_eq!(m.name, "Show");
        assert_eq!(m.trackers, vec!["http://tracker-exemplo.invalid/a"]);
        assert_eq!(m.info_hash.hex(), "0123456789abcdef0123456789abcdef01234567");
        let b32 = parse_magnet("magnet:?xt=urn:btih:AERUKZ4JVPG66AJDIVTYTK6N54ASGRLH").unwrap();
        assert_eq!(b32.info_hash, m.info_hash);
        let again = parse_magnet(&m.to_string()).unwrap();
        assert_eq!(again, m, "magnet round trip");
        for bad in ["http://x", "magnet:?dn=x", "magnet:?xt=urn:btih:zz"] {
            assert!(parse_magnet(bad).is_err(), "{bad} accepted");
        }
        assert_eq!(InfoHash::parse("AERUKZ4JVPG66AJDIVTYTK6N54ASGRLH").unwrap(), m.info_hash);
        assert!(InfoHash::parse("0123456789abcdef0123456789abcdef0123456z").is_err());
    }

    #[test]
    fn garbage_metainfo_is_safe() {
        let files = [sf(&["a"], b"hello".to_vec()), sf(&["b", "c"], b"world".to_vec())];
        let (raw, _) = build("x", &files, 4, &["udp://tracker-exemplo.invalid:1".into()]).unwrap();
        let seeds = [raw, b"d4:infod4:name1:x12:piece lengthi1e6:pieces20:aaaaaaaaaaaaaaaaaaaa6:lengthi1eee".to_vec()];
        let mut rng = Rng::new(11);
        for i in 0..20_000 {
            let mut data = seeds[i % 2].clone();
            rng.mutate(&mut data);
            let Ok(mi) = parse_metainfo(&data) else { continue };
            let mut total = 0;
            for f in &mi.info.files {
                assert!(f.path.iter().all(|p| safe_component(p)), "unsafe component in {:?}", f.path);
                assert_eq!(f.offset, total, "offsets must be contiguous");
                total += f.length;
            }
            assert_eq!(total, mi.info.length);
        }
    }
}
