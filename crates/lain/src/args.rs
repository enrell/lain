//! Flag parsing shared by every subcommand: `--name value` or
//! `--name=value`, bare `--switch`es and positionals, like the Go CLI.

/// Flags that take a value, so `positional` skips that value.
const VALUE_FLAGS: &[&str] = &[
    "--pick",
    "--player",
    "--id",
    "--server",
    "--data-dir",
    "--out",
    "--in",
    "--username",
    "--port",
    "--bind",
    "--dir",
    "--max",
    "--min-free",
    "--evict-watched",
    "--retries",
    "--name",
    "--lines",
    "--log-level",
    "--log-file",
    "--os",
    "--watch",
    "--list-sync",
    "--audio",
    "--subtitles",
    "--subtitle-mode",
];

/// The value of `--name value` or `--name=value`.
pub fn flag(args: &[String], name: &str) -> Option<String> {
    let long = format!("--{name}");
    let eq = format!("--{name}=");
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if *a == long {
            return it.next().cloned();
        }
        if let Some(v) = a.strip_prefix(&eq) {
            return Some(v.to_string());
        }
    }
    None
}

/// Whether the bare switch `--name` is present.
pub fn has(args: &[String], name: &str) -> bool {
    let long = format!("--{name}");
    args.contains(&long)
}

/// The arguments that are not flags nor a value-flag's value.
pub fn positional(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut skip = false;
    for a in args {
        if skip {
            skip = false;
            continue;
        }
        if VALUE_FLAGS.contains(&a.as_str()) {
            skip = true;
            continue;
        }
        if a.starts_with('-') {
            continue;
        }
        out.push(a.clone());
    }
    out
}

/// A byte count with an optional `KiB`, `MiB`, `GiB` or `TiB` suffix;
/// `allow_zero` accepts `0` (no limit).
pub fn byte_size(raw: &str, allow_zero: bool) -> Result<u64, String> {
    let s = raw.trim();
    if allow_zero && s == "0" {
        return Ok(0);
    }
    let mut mult: u64 = 1;
    let mut num = s;
    for (suffix, m) in [("KiB", 1u64 << 10), ("MiB", 1 << 20), ("GiB", 1 << 30), ("TiB", 1 << 40)] {
        if let Some(n) = s.strip_suffix(suffix) {
            mult = m;
            num = n.trim();
            break;
        }
    }
    match num.parse::<u64>() {
        Ok(n) if n > 0 => n.checked_mul(mult).ok_or_else(size_err),
        _ => Err(size_err()),
    }
}

fn size_err() -> String {
    "must be a positive byte count with optional KiB, MiB, GiB or TiB suffix".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn flags_and_positionals() {
        let a = v(&["frieren", "--pick", "2", "--once", "--player=vlc", "02"]);
        assert_eq!(flag(&a, "pick").as_deref(), Some("2"));
        assert_eq!(flag(&a, "player").as_deref(), Some("vlc"));
        assert!(has(&a, "once") && !has(&a, "next"));
        assert_eq!(positional(&a), v(&["frieren", "02"]));
        assert_eq!(flag(&a, "id"), None);
    }

    #[test]
    fn byte_sizes() {
        assert_eq!(byte_size("20GiB", false), Ok(20 << 30));
        assert_eq!(byte_size(" 512 MiB ", false), Ok(512 << 20));
        assert_eq!(byte_size("1024", false), Ok(1024));
        assert_eq!(byte_size("0", true), Ok(0));
        assert!(byte_size("0", false).is_err());
        assert!(byte_size("-1GiB", false).is_err());
        assert!(byte_size("lots", true).is_err());
        assert!(byte_size("99999999999TiB", false).is_err());
    }
}
