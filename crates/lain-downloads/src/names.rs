//! Safe file naming (Go `names.go`).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Keeps a name under common filesystem limits (255) with room for a
/// " (n)" collision suffix and the ".part" extension.
pub const MAX_NAME_BYTES: usize = 200;

/// Go's `path.Base` over `/`-separated text.
fn base(s: &str) -> &str {
    if s.is_empty() {
        return ".";
    }
    let t = s.trim_end_matches('/');
    if t.is_empty() {
        return "/";
    }
    match t.rfind('/') {
        Some(i) => &t[i + 1..],
        None => t,
    }
}

/// Go's `filepath.Ext`: the suffix from the final dot of the last element.
fn ext(name: &str) -> &str {
    for (i, c) in name.char_indices().rev() {
        if c == '/' || c == '\\' {
            break;
        }
        if c == '.' {
            return &name[i..];
        }
    }
    ""
}

/// Reduces a suggested file name to one base name that cannot escape its
/// directory: no separators, no "..", no control characters, no leading
/// dot (hidden files never show up as media). `fallback` when nothing
/// usable is left.
pub fn safe_name(s: &str, fallback: &str) -> String {
    let s = s.replace('\\', "/");
    let s: String = base(&s)
        .chars()
        .filter(|c| !(c.is_control() || *c == '/' || *c == '\u{FFFD}'))
        .map(|c| if matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') { '_' } else { c })
        .collect();
    // Go trims space, dots, space once; ". .x" kept a leading dot there.
    let mut s = s.trim_start_matches(|c: char| c == '.' || c.is_whitespace()).trim_end().to_string();
    if s.len() > MAX_NAME_BYTES {
        let mut e = ext(&s).to_string();
        if e.len() > 16 {
            e.clear();
        }
        let mut cut = MAX_NAME_BYTES - e.len();
        while cut > 0 && !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s = format!("{}{e}", &s[..cut]);
    }
    if s.is_empty() || s == "." || s == ".." {
        return fallback.to_string();
    }
    s
}

fn unescape(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let h = b.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    Some(String::from_utf8_lossy(&out).into_owned())
}

/// The last path segment of a URL, unescaped (`""` when there is none).
pub fn name_from_url(raw: &str) -> String {
    let raw = raw.trim();
    if raw.starts_with(':') {
        return String::new();
    }
    let rest = raw.split(['?', '#']).next().unwrap_or("");
    let path = match rest.find("://") {
        Some(i) => {
            let after = &rest[i + 3..];
            after.find('/').map(|j| &after[j..]).unwrap_or("")
        }
        None => rest,
    };
    let Some(path) = unescape(path) else { return String::new() };
    let seg = base(&path);
    if seg == "/" || seg == "." {
        return String::new();
    }
    seg.to_string()
}

/// `dir/name`, or `dir/"stem (n).ext"` for the first n not taken yet: a
/// download never overwrites a file it did not create. Reserved paths
/// (in-flight downloads) count as taken.
pub fn free_path(dir: &Path, name: &str, reserved: &HashSet<PathBuf>) -> PathBuf {
    let e = ext(name);
    let stem = &name[..name.len() - e.len()];
    let mut cand = name.to_string();
    let mut n = 2;
    loop {
        let p = dir.join(&cand);
        if std::fs::symlink_metadata(&p).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
            && !reserved.contains(&p)
        {
            return p;
        }
        cand = format!("{stem} ({n}){e}");
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_names() {
        for (inp, want) in [
            ("Frieren - 01.cbz", "Frieren - 01.cbz"),
            ("../../etc/passwd", "passwd"),
            (r"..\..\boot.ini", "boot.ini"),
            ("/abs/path/x.mkv", "x.mkv"),
            (".hidden.mkv", "hidden.mkv"),
            ("..", "fallback"),
            ("", "fallback"),
            ("   ", "fallback"),
            ("a\x00b\nc.mkv", "abc.mkv"),
            (r#"what?:"*<>|.mkv"#, "what_______.mkv"),
            ("[Fansub-A] Show - 03.mkv", "[Fansub-A] Show - 03.mkv"),
            (". .x.mkv", "x.mkv"),
        ] {
            assert_eq!(safe_name(inp, "fallback"), want, "{inp:?}");
        }
        let long = format!("{}.mkv", "ü".repeat(300));
        let got = safe_name(&long, "f");
        assert!(got.len() <= MAX_NAME_BYTES && got.ends_with(".mkv"), "{} bytes", got.len());
    }

    /// The output is always one non-hidden path element.
    #[test]
    fn safe_name_never_escapes() {
        let alphabet: Vec<char> = "ab./\\.. \t\n\0:*?<>|\"ü€\u{FFFD}\u{7f}%".chars().collect();
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for _ in 0..4000 {
            let len = (next() % 300) as usize;
            let s: String = (0..len).map(|_| alphabet[(next() % alphabet.len() as u64) as usize]).collect();
            let n = safe_name(&s, "x");
            assert!(
                !n.is_empty()
                    && n != "."
                    && n != ".."
                    && !n.contains(['/', '\\'])
                    && !n.starts_with('.')
                    && n.len() <= MAX_NAME_BYTES,
                "{s:?} -> {n:?}"
            );
        }
    }

    #[test]
    fn names_from_urls() {
        for (inp, want) in [
            ("https://example.test/a/Frieren%20-%2001.cbz?x=1", "Frieren - 01.cbz"),
            ("https://example.test/", ""),
            ("https://example.test", ""),
            ("::bad", ""),
        ] {
            assert_eq!(name_from_url(inp), want, "{inp}");
        }
    }

    #[test]
    fn free_path_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("lain-dl-names-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.mkv"), b"").unwrap();
        let reserved = HashSet::from([dir.join("a (2).mkv")]);
        assert_eq!(free_path(&dir, "a.mkv", &reserved), dir.join("a (3).mkv"));
        assert_eq!(free_path(&dir, "b.mkv", &HashSet::new()), dir.join("b.mkv"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
