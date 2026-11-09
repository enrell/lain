// Links libmpv (the MSVC import lib made from shinchiro's libmpv-2.dll) and
// copies the DLL next to the built exe so `cargo run` works.
use std::path::PathBuf;

fn main() {
    let dir = std::env::var("LAIN_MPV_DIR").unwrap_or_else(|_| r"D:\lain-proto\libmpv".to_string());
    println!("cargo:rerun-if-env-changed=LAIN_MPV_DIR");
    println!("cargo:rustc-link-search=native={dir}");
    println!("cargo:rustc-link-lib=dylib=mpv");

    // OUT_DIR is target/<profile>/build/<pkg>-<hash>/out; the exe lives three levels up.
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    if let Some(profile_dir) = out.ancestors().nth(3) {
        let dll = PathBuf::from(&dir).join("libmpv-2.dll");
        let dest = profile_dir.join("libmpv-2.dll");
        let stale = match (std::fs::metadata(&dll), std::fs::metadata(&dest)) {
            (Ok(a), Ok(b)) => a.len() != b.len(),
            (Ok(_), Err(_)) => true,
            _ => false,
        };
        if stale {
            let _ = std::fs::copy(&dll, &dest);
        }
    }
}
