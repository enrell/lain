// Links libmpv (MSVC import lib `mpv.lib`) and copies `libmpv-2.dll` next to
// the built exe. The directory comes from LAIN_MPV_DIR, default the shared
// prototype inputs.
use std::path::PathBuf;

fn main() {
    let dir = std::env::var("LAIN_MPV_DIR").unwrap_or_else(|_| r"D:\lain-proto\libmpv".into());
    println!("cargo:rerun-if-env-changed=LAIN_MPV_DIR");
    println!("cargo:rustc-link-search=native={dir}");
    // OUT_DIR = target/<profile>/build/<pkg>/out; the exe lives in target/<profile>.
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    if let Some(profile_dir) = out.ancestors().nth(3) {
        let dll = PathBuf::from(&dir).join("libmpv-2.dll");
        let dst = profile_dir.join("libmpv-2.dll");
        if dll.exists() && !dst.exists() {
            let _ = std::fs::copy(&dll, &dst);
        }
        // bundled fonts next to the exe (the app registers fonts/*.ttf)
        let fonts = profile_dir.join("fonts");
        let _ = std::fs::create_dir_all(&fonts);
        for e in std::fs::read_dir("fonts").into_iter().flatten().flatten() {
            let _ = std::fs::copy(e.path(), fonts.join(e.file_name()));
        }
        println!("cargo:rerun-if-changed=fonts");
    }
}
