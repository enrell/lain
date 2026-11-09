use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=MPV_LIBRARY_DIR");
    // Optimus/PowerXpress: make the NVIDIA/AMD driver put this process's
    // OpenGL (mpv's hidden WGL context) on the discrete GPU, the same one
    // wgpu picks with HighPerformance, so the shared D3D11 texture can be
    // opened by wgpu's D3D12 device (shared handles do not cross GPUs).
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-arg-bins=/EXPORT:NvOptimusEnablement");
        println!("cargo:rustc-link-arg-bins=/EXPORT:AmdPowerXpressRequestHighPerformance");
    }
    // Copy libmpv-2.dll next to the exe (target/<profile>/).
    if let Ok(dir) = std::env::var("MPV_LIBRARY_DIR") {
        let src = PathBuf::from(&dir).join("libmpv-2.dll");
        let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
        // OUT_DIR = target/<profile>/build/<pkg>-<hash>/out
        if let Some(profile_dir) = out.ancestors().nth(3) {
            let dst = profile_dir.join("libmpv-2.dll");
            let stale = match (std::fs::metadata(&src), std::fs::metadata(&dst)) {
                (Ok(a), Ok(b)) => a.len() != b.len(),
                (Ok(_), Err(_)) => true,
                _ => false,
            };
            if stale {
                let _ = std::fs::copy(&src, &dst);
            }
        }
    }
}
