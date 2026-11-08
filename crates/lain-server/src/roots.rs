//! Portable library roots (D-157): a library declares a named root and
//! items store `root` + `rel_path` (always `/`-separated). Each machine
//! maps root names to directories in its own `roots.json`, so moving
//! from Windows/WSL to Linux is a mapping change, not a catalog
//! rewrite:
//!
//! ```json
//! {"format": "lain.roots@1", "roots": {
//!    "media": {"path": "/mnt/v/media", "aliases": ["V:\\media"]},
//!    "anime": "/srv/anime"}}
//! ```
//!
//! `path` is where the root lives on this machine; `aliases` are other
//! spellings of the same directory (the Windows drive of a WSL mount,
//! an old machine's path) that the one-shot migration recognizes.
//! Paths are parsed by their own style, not the host's: a Linux server
//! can migrate `V:\media\...` written by a Windows one.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;
use serde_json::Value;

pub const FORMAT: &str = "lain.roots@1";
pub const FILE_NAME: &str = "roots.json";

/// One root on this machine.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RootDef {
    pub path: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
}

/// The root mapping of this machine.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Roots {
    pub map: BTreeMap<String, RootDef>,
}

/// Root names: lower-case letters, digits, `-` and `_`.
pub fn valid_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 64
        && n.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// A parsed absolute path: its style and components.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Parsed {
    windows: bool,
    /// `v:`, `\\server\share`, or empty for POSIX.
    prefix: String,
    parts: Vec<String>,
}

fn parse(path: &str) -> Option<Parsed> {
    let b = path.as_bytes();
    let (windows, prefix, rest) = if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        if b.len() > 2 && b[2] != b'\\' && b[2] != b'/' {
            return None; // drive-relative ("C:foo")
        }
        (true, path[..2].to_ascii_lowercase(), &path[2..])
    } else if let Some(unc) = path.strip_prefix(r"\\") {
        let mut it = unc.split(['\\', '/']).filter(|s| !s.is_empty());
        let server = it.next()?;
        let share = it.next()?;
        let prefix = format!(r"\\{}\{}", server.to_ascii_lowercase(), share.to_ascii_lowercase());
        let rest_start = path.len() - unc.len() + unc.find(share)? + share.len();
        (true, prefix, &path[rest_start..])
    } else if path.starts_with('/') {
        (false, String::new(), path)
    } else {
        return None;
    };
    let sep: &[char] = if windows { &['\\', '/'] } else { &['/'] };
    let mut parts = Vec::new();
    for p in rest.split(sep).filter(|p| !p.is_empty() && *p != ".") {
        if p == ".." {
            parts.pop()?;
        } else {
            parts.push(p.to_string());
        }
    }
    Some(Parsed { windows, prefix, parts })
}

fn same_part(windows: bool, a: &str, b: &str) -> bool {
    if windows { a.to_lowercase() == b.to_lowercase() } else { a == b }
}

/// The `/`-separated path of `path` below `base`, if it is below it.
fn below(base: &str, path: &str) -> Option<Vec<String>> {
    let (b, p) = (parse(base)?, parse(path)?);
    if b.windows != p.windows || b.prefix != p.prefix || p.parts.len() < b.parts.len() {
        return None;
    }
    for (x, y) in b.parts.iter().zip(&p.parts) {
        if !same_part(b.windows, x, y) {
            return None;
        }
    }
    Some(p.parts[b.parts.len()..].to_vec())
}

/// Checks a stored relative path: `/`-separated, no `..`, not absolute.
pub fn valid_rel(rel: &str) -> bool {
    !rel.starts_with('/') && !rel.contains('\\') && rel.split('/').all(|p| p != ".." && p != ".") && !rel.contains("//")
}

impl Roots {
    /// Parses `roots.json`.
    pub fn parse(raw: &[u8]) -> Result<Roots, String> {
        let v: Value = serde_json::from_slice(raw).map_err(|e| format!("roots.json: {e}"))?;
        if v["format"] != FORMAT {
            return Err(format!("roots.json: format must be {FORMAT}"));
        }
        let mut map = BTreeMap::new();
        for (name, def) in v["roots"].as_object().ok_or("roots.json: roots must be an object")? {
            if !valid_name(name) {
                return Err(format!("roots.json: bad root name {name:?} (lower-case letters, digits, - and _)"));
            }
            let def = match def {
                Value::String(p) => RootDef { path: p.clone(), aliases: Vec::new() },
                Value::Object(o) => RootDef {
                    path: o
                        .get("path")
                        .and_then(Value::as_str)
                        .ok_or_else(|| format!("roots.json: {name}.path required"))?
                        .into(),
                    aliases: o
                        .get("aliases")
                        .and_then(Value::as_array)
                        .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                        .unwrap_or_default(),
                },
                _ => return Err(format!("roots.json: {name} must be a path or {{path, aliases}}")),
            };
            for p in std::iter::once(&def.path).chain(&def.aliases) {
                if parse(p).is_none() {
                    return Err(format!("roots.json: {name}: {p:?} is not an absolute path"));
                }
            }
            map.insert(name.clone(), def);
        }
        Ok(Roots { map })
    }

    /// `<data_dir>/roots.json`, or no roots when it is absent.
    pub fn load(data_dir: &Path) -> Result<Roots, String> {
        Self::load_file(&data_dir.join(FILE_NAME))
    }

    pub fn load_file(path: &Path) -> Result<Roots, String> {
        match std::fs::read(path) {
            Ok(raw) => Roots::parse(&raw),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Roots::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// The root an absolute path lives in and its relative path, by the
    /// longest matching root path or alias.
    pub fn relativize(&self, abs: &str) -> Option<(String, String)> {
        let mut best: Option<(usize, String, Vec<String>)> = None;
        for (name, def) in &self.map {
            for base in std::iter::once(&def.path).chain(&def.aliases) {
                if let Some(rest) = below(base, abs) {
                    let depth = parse(base).map(|p| p.parts.len()).unwrap_or(0);
                    if best.as_ref().is_none_or(|(d, _, _)| depth > *d) {
                        best = Some((depth, name.clone(), rest));
                    }
                }
            }
        }
        best.map(|(_, name, rest)| (name, rest.join("/")))
    }

    /// The absolute path of `root` + `rel` on this machine, in the root
    /// path's own style.
    pub fn resolve(&self, root: &str, rel: &str) -> Option<String> {
        let def = self.map.get(root)?;
        if !valid_rel(rel) {
            return None;
        }
        let windows = parse(&def.path)?.windows;
        let sep = if windows { '\\' } else { '/' };
        let base = def.path.trim_end_matches(['/', '\\']);
        let base = if base.is_empty() { "" } else { base };
        if rel.is_empty() {
            return Some(if base.is_empty() || base.ends_with(':') {
                format!("{base}{sep}")
            } else {
                base.to_string()
            });
        }
        Some(format!("{base}{sep}{}", if windows { rel.replace('/', "\\") } else { rel.to_string() }))
    }

    /// The API view (`GET /api/roots`).
    pub fn view(&self) -> Value {
        let list: Vec<Value> = self
            .map
            .iter()
            .map(|(name, d)| {
                serde_json::json!({"name": name, "path": d.path, "aliases": d.aliases, "available": Path::new(&d.path).is_dir()})
            })
            .collect();
        serde_json::json!({"format": FORMAT, "roots": list})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots() -> Roots {
        Roots::parse(
            br#"{"format":"lain.roots@1","roots":{
                "media":{"path":"/mnt/v/media","aliases":["V:\\media","\\\\nas\\share\\media"]},
                "anime":"/mnt/v/media/anime",
                "win":"D:\\Library"}}"#,
        )
        .unwrap()
    }

    #[test]
    fn linux_and_windows_spellings_map_to_one_root() {
        let r = roots();
        let linux = r.relativize("/mnt/v/media/movies/Akira (1988)/Akira.mkv").unwrap();
        let windows = r.relativize(r"V:\media\movies\Akira (1988)\Akira.mkv").unwrap();
        let lower_drive = r.relativize(r"v:/Media/movies/Akira (1988)/Akira.mkv").unwrap();
        let unc = r.relativize(r"\\NAS\share\media\movies\Akira (1988)\Akira.mkv").unwrap();
        let want = ("media".to_string(), "movies/Akira (1988)/Akira.mkv".to_string());
        assert_eq!(linux, want);
        assert_eq!(windows, want);
        assert_eq!(unc, want);
        // Windows paths compare case-insensitively; the stored rel keeps the source's case.
        assert_eq!(lower_drive.0, "media");
        assert_eq!(r.resolve("media", &want.1).unwrap(), "/mnt/v/media/movies/Akira (1988)/Akira.mkv");
        assert_eq!(r.resolve("win", "a/b.mkv").unwrap(), r"D:\Library\a\b.mkv");
    }

    #[test]
    fn the_deepest_root_wins_and_strangers_stay_unmapped() {
        let r = roots();
        assert_eq!(
            r.relativize(r"V:\media\anime\Frieren\01.mkv"),
            Some(("media".into(), "anime/Frieren/01.mkv".into()))
        );
        assert_eq!(r.relativize("/mnt/v/media/anime/Frieren/01.mkv"), Some(("anime".into(), "Frieren/01.mkv".into())));
        assert_eq!(r.relativize("/mnt/v/mediax/a.mkv"), None);
        assert_eq!(r.relativize("/MNT/v/media/a.mkv"), None, "POSIX paths are case-sensitive");
        assert_eq!(r.relativize("relative/a.mkv"), None);
        assert_eq!(r.relativize("/mnt/v/media"), Some(("media".into(), "".into())));
        assert_eq!(r.relativize("/mnt/v/media/x/../movies/a.mkv"), Some(("media".into(), "movies/a.mkv".into())));
    }

    #[test]
    fn resolve_rejects_escapes_and_unknown_roots() {
        let r = roots();
        assert!(r.resolve("media", "../etc/passwd").is_none());
        assert!(r.resolve("media", "/etc").is_none());
        assert!(r.resolve("nope", "a").is_none());
        assert_eq!(r.resolve("media", "").unwrap(), "/mnt/v/media");
    }

    #[test]
    fn bad_documents() {
        assert!(Roots::parse(br#"{"format":"lain.roots@2","roots":{}}"#).is_err());
        assert!(Roots::parse(br#"{"format":"lain.roots@1","roots":{"Bad Name":"/x"}}"#).is_err());
        assert!(Roots::parse(br#"{"format":"lain.roots@1","roots":{"a":"relative"}}"#).is_err());
        assert!(Roots::parse(br#"{"format":"lain.roots@1","roots":{"a":"C:relative"}}"#).is_err());
    }
}
