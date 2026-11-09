// libmpv is loaded at run time (src/mpv.rs), so nothing links against it.
// For `cargo run` convenience this copies `libmpv-2.dll` next to the built
// exe when it is found in `LAIN_MPV_DIR` or `vendor/libmpv/`.
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=LAIN_MPV_DIR");
    if std::env::var("CARGO_CFG_WINDOWS").is_err() {
        return;
    }
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let dir =
        std::env::var("LAIN_MPV_DIR").map(PathBuf::from).unwrap_or_else(|_| manifest.join("vendor").join("libmpv"));
    let dll = dir.join("libmpv-2.dll");
    // OUT_DIR is target/<profile>/build/<pkg>-<hash>/out; the exe lives three levels up.
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap_or_default());
    let (Some(profile_dir), Ok(src)) = (out.ancestors().nth(3), std::fs::metadata(&dll)) else { return };
    let dest = profile_dir.join("libmpv-2.dll");
    if std::fs::metadata(&dest).map(|d| d.len() != src.len()).unwrap_or(true) {
        let _ = std::fs::copy(&dll, &dest);
    }
}
