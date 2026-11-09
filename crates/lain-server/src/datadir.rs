//! The data dir per OS (D-164), overridable by `LAIN_HOME`:
//!
//! | OS      | default                                      |
//! |---------|----------------------------------------------|
//! | Linux   | `$XDG_DATA_HOME/lain` or `~/.local/share/lain` |
//! | macOS   | `~/Library/Application Support/lain`         |
//! | Windows | `%LOCALAPPDATA%\lain`                         |

use std::path::PathBuf;

/// The data directory name.
pub const APP_DIR: &str = "lain";

/// The target OS family a path is resolved for (testable on any host).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Linux,
    MacOs,
    Windows,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(target_os = "macos") {
            Os::MacOs
        } else if cfg!(windows) {
            Os::Windows
        } else {
            Os::Linux
        }
    }
}

/// Resolves the data dir from an environment lookup.
pub fn resolve(os: Os, env: &dyn Fn(&str) -> Option<String>) -> Result<PathBuf, String> {
    let get = |k: &str| env(k).filter(|v| !v.is_empty());
    if let Some(home) = get("LAIN_HOME") {
        return Ok(PathBuf::from(home));
    }
    match os {
        Os::Linux => {
            if let Some(xdg) = get("XDG_DATA_HOME") {
                return Ok(PathBuf::from(xdg).join(APP_DIR));
            }
            let home = get("HOME").ok_or("HOME is not set; pass --data-dir or LAIN_HOME")?;
            Ok(PathBuf::from(home).join(".local").join("share").join(APP_DIR))
        }
        Os::MacOs => {
            let home = get("HOME").ok_or("HOME is not set; pass --data-dir or LAIN_HOME")?;
            Ok(PathBuf::from(home).join("Library").join("Application Support").join(APP_DIR))
        }
        Os::Windows => {
            let base = get("LOCALAPPDATA").ok_or("LOCALAPPDATA is not set; pass --data-dir or LAIN_HOME")?;
            Ok(PathBuf::from(base).join(APP_DIR))
        }
    }
}

/// The data dir for this process.
pub fn default_data_dir() -> Result<PathBuf, String> {
    resolve(Os::current(), &|k| std::env::var(k).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: Vec<(String, String)> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |k| m.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone())
    }

    #[test]
    fn per_os_defaults_and_override() {
        let e = env(&[("HOME", "/home/u"), ("LOCALAPPDATA", r"C:\Users\u\AppData\Local")]);
        assert_eq!(resolve(Os::Linux, &e).unwrap(), PathBuf::from("/home/u/.local/share/lain"));
        assert_eq!(resolve(Os::MacOs, &e).unwrap(), PathBuf::from("/home/u/Library/Application Support/lain"));
        assert_eq!(resolve(Os::Windows, &e).unwrap(), PathBuf::from(r"C:\Users\u\AppData\Local").join("lain"));
        let x = env(&[("HOME", "/home/u"), ("XDG_DATA_HOME", "/data")]);
        assert_eq!(resolve(Os::Linux, &x).unwrap(), PathBuf::from("/data/lain"));
        let o = env(&[("HOME", "/home/u"), ("LAIN_HOME", "/srv/lain")]);
        for os in [Os::Linux, Os::MacOs, Os::Windows] {
            assert_eq!(resolve(os, &o).unwrap(), PathBuf::from("/srv/lain"));
        }
        assert!(resolve(Os::Windows, &env(&[])).is_err());
        assert_eq!(
            resolve(Os::Linux, &env(&[("HOME", "/h"), ("LAIN_HOME", "")])).unwrap(),
            PathBuf::from("/h/.local/share/lain")
        );
    }
}
