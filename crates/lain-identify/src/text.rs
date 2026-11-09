//! String helpers with Go's semantics, so the parsers read a name
//! exactly as the Go reference does: `filepath.Base`/`Ext` on the
//! name's own separators, per-rune simple lower-casing, `strings.Fields`
//! and cutset trims.

/// Per-character simple lower-casing (Go's `strings.ToLower`): a
/// character whose full lower-case form is several characters keeps
/// only the first, like Go's `unicode.ToLower`.
pub fn lower(s: &str) -> String {
    if s.is_ascii() {
        return s.to_ascii_lowercase();
    }
    s.chars().map(|c| c.to_lowercase().next().unwrap_or(c)).collect()
}

fn is_sep(c: char) -> bool {
    c == '/' || (cfg!(windows) && c == '\\')
}

/// The last element of a path (Go's `filepath.Base`).
pub fn base(path: &str) -> &str {
    let p = path.trim_end_matches(is_sep);
    if p.is_empty() {
        return if path.is_empty() { "." } else { "/" };
    }
    match p.rfind(is_sep) {
        Some(i) => &p[i + 1..],
        None => p,
    }
}

/// Everything but the last element (Go's `filepath.Dir`, without
/// cleaning `..`).
pub fn dir(path: &str) -> &str {
    match path.rfind(is_sep) {
        Some(0) => "/",
        Some(i) => path[..i].trim_end_matches(is_sep),
        None => ".",
    }
}

/// The extension with its dot (Go's `filepath.Ext`): from the last `.`
/// of the last element, or empty.
pub fn ext(path: &str) -> &str {
    for (i, c) in path.char_indices().rev() {
        if is_sep(c) {
            return "";
        }
        if c == '.' {
            return &path[i..];
        }
    }
    ""
}

/// Whitespace-separated words joined by one space (`strings.Fields`).
pub fn fields_join(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Trims every leading and trailing character in `cutset`.
pub fn trim_set<'a>(s: &'a str, cutset: &str) -> &'a str {
    s.trim_matches(|c| cutset.contains(c))
}

/// Drops control characters (Go's `unicode.IsControl`, category Cc).
pub fn strip_controls(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect()
}

/// Go's `strconv.Atoi` for a digit string: saturates instead of failing
/// on overflow (the parsers ignore the error and keep the value).
pub fn atoi(s: &str) -> i64 {
    s.bytes().fold(0i64, |n, b| n.saturating_mul(10).saturating_add((b - b'0') as i64))
}

pub fn is_number(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// RE2's `\w`: ASCII letters, digits and `_`.
pub fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_path_semantics() {
        assert_eq!(base("/lib/a b.mkv"), "a b.mkv");
        assert_eq!(base("a.mkv"), "a.mkv");
        assert_eq!(base("/lib/x/"), "x");
        assert_eq!(dir("/lib/Show/v01.cbz"), "/lib/Show");
        assert_eq!(dir("/v01.cbz"), "/");
        assert_eq!(dir("v01.cbz"), ".");
        assert_eq!(ext("a.b.MKV"), ".MKV");
        assert_eq!(ext(".bashrc"), ".bashrc");
        assert_eq!(ext("noext"), "");
        assert_eq!(lower("İSTANBUL Ab"), "istanbul ab");
        assert_eq!(trim_set("- Title –:", " -–:."), "Title");
        assert_eq!(atoi("99999999999999999999999"), i64::MAX);
    }
}
