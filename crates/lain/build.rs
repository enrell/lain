//! Embeds the web SPA (D-235): every file under `crates/lain/webui/`
//! (or `$LAIN_WEB_DIR`) becomes a `(path, bytes)` entry of `WEB_FILES`
//! through `include_bytes!`, so the binary serves the build with no
//! files beside it. `just web` (and CI) copies `web/build/` there. An
//! empty directory (a fresh checkout has only `.gitkeep`) embeds
//! nothing, and the server then answers the "web UI not built" page.
//!
//! The directory always exists, so `rerun-if-changed` on it never sees
//! a missing path (which would rebuild the binary on every build).

use std::path::{Path, PathBuf};

fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            walk(root, &p, out);
        } else if ft.is_file() {
            let rel = p.strip_prefix(root).unwrap_or(&p);
            let name: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
            let name = name.join("/");
            if name != ".gitkeep" {
                out.push((name, p));
            }
        }
    }
}

fn main() {
    println!("cargo:rerun-if-env-changed=LAIN_WEB_DIR");
    println!("cargo:rerun-if-env-changed=LAIN_VERSION");
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let dir = match std::env::var("LAIN_WEB_DIR") {
        Ok(d) if !d.is_empty() => PathBuf::from(d),
        _ => manifest.join("webui"),
    };
    if dir.is_dir() {
        println!("cargo:rerun-if-changed={}", dir.display());
    }
    let mut files = Vec::new();
    walk(&dir, &dir, &mut files);
    let mut src = String::from("pub static WEB_FILES: &[(&str, &[u8])] = &[\n");
    for (name, path) in &files {
        let abs = path.canonicalize().unwrap_or_else(|_| path.clone());
        src.push_str(&format!("    ({name:?}, include_bytes!({:?})),\n", abs.to_string_lossy()));
    }
    src.push_str("];\n");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("webui.rs");
    std::fs::write(out, src).expect("write webui.rs");
}
