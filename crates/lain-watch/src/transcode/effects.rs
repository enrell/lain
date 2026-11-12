//! Server-side video effects (D-274): the Anime4K chains the web player
//! runs on the viewer's GPU, run instead inside the transcode by
//! ffmpeg's `libplacebo` filter, for clients whose GPU cannot carry
//! them. The shaders are the web player's own files (mpv hook format,
//! which libplacebo reads as it is), compiled in so the two renderers
//! cannot drift apart.

use std::path::{Path, PathBuf};

/// Replaced in the filter chain with the escaped shader file.
pub const EFFECT_PLACEHOLDER: &str = "__EFFECT_FILE__";

/// The presets the server renders, by the web player's ids. Its
/// `anime4k-dog-x2` is not a hook file and stays a client effect.
pub const PRESETS: [&str; 3] = ["anime4k-a", "anime4k-aa", "anime4k-lite"];

/// The tallest and widest frame an upscaling preset produces. Twice the
/// source, but never past 1080p: the point is a phone or a laptop
/// screen, and every doubling quadruples the encode.
const MAX_WIDTH: i64 = 1920;
const MAX_HEIGHT: i64 = 1080;

macro_rules! pack {
    ($name:literal) => {
        include_str!(concat!("../../../../web/src/lib/player/webgl/packs/Anime4K_", $name, ".glsl"))
    };
}

const CLAMP: &str = pack!("Clamp_Highlights");
const RESTORE_S: &str = pack!("Restore_CNN_S");
const RESTORE_M: &str = pack!("Restore_CNN_M");
const UPSCALE_S: &str = pack!("Upscale_CNN_x2_S");
const UPSCALE_M: &str = pack!("Upscale_CNN_x2_M");
const DEBLUR: &str = pack!("Deblur_DoG");
const DARKEN: &str = pack!("Darken_VeryFast");
const DOWNSCALE_2: &str = pack!("AutoDownscalePre_x2");
const DOWNSCALE_4: &str = pack!("AutoDownscalePre_x4");

/// The hook files of a preset, in the order the web player chains them
/// (`web/src/lib/player/webgl/packs/anime4k.ts`).
fn chain(preset: &str) -> Option<&'static [&'static str]> {
    Some(match preset {
        "anime4k-a" => &[CLAMP, RESTORE_M, UPSCALE_M, DOWNSCALE_2, DOWNSCALE_4, UPSCALE_S],
        "anime4k-aa" => &[CLAMP, RESTORE_M, UPSCALE_M, RESTORE_S, DOWNSCALE_2, DOWNSCALE_4, UPSCALE_S],
        "anime4k-lite" => &[CLAMP, RESTORE_S, DEBLUR, DARKEN],
        _ => return None,
    })
}

pub fn known(preset: &str) -> bool {
    chain(preset).is_some()
}

/// One shader file for a preset: its hook files, concatenated.
pub fn shader(preset: &str) -> Option<String> {
    chain(preset).map(|files| files.join("\n"))
}

/// The frame a preset produces from a `width`x`height` source, inside
/// an optional quality box (`0` is unbounded): even, same aspect.
pub fn output_size(preset: &str, width: i64, height: i64, max_width: i64, max_height: i64) -> (i64, i64) {
    if width <= 0 || height <= 0 {
        return (width, height);
    }
    let upscales = preset != "anime4k-lite";
    let (mut box_w, mut box_h) = if upscales {
        ((width * 2).min(MAX_WIDTH.max(width)), (height * 2).min(MAX_HEIGHT.max(height)))
    } else {
        (width, height)
    };
    if max_width > 0 {
        box_w = box_w.min(max_width);
    }
    if max_height > 0 {
        box_h = box_h.min(max_height);
    }
    // Fit the source aspect into the box, comparing cross products so no
    // float decides a pixel.
    let (w, h) =
        if box_w * height <= box_h * width { (box_w, box_w * height / width) } else { (box_h * width / height, box_h) };
    ((w & !1).max(2), (h & !1).max(2))
}

/// The filter for a preset at an output size. `format` keeps the encoder
/// input 8-bit 4:2:0 whatever the source was.
pub fn filter(width: i64, height: i64) -> String {
    format!("libplacebo=custom_shader_path={EFFECT_PLACEHOLDER}:w={width}:h={height}:format=yuv420p")
}

/// Writes a preset's shader under `dir` (once: the content never
/// changes within a build) and returns its path.
pub fn materialize(dir: &Path, preset: &str) -> Result<PathBuf, String> {
    let source = shader(preset).ok_or_else(|| format!("unknown video effect {preset:?}"))?;
    let dir = dir.join("effects");
    std::fs::create_dir_all(&dir).map_err(|e| format!("video effect directory: {e}"))?;
    let path = dir.join(format!("{preset}.glsl"));
    if std::fs::read_to_string(&path).is_ok_and(|have| have == source) {
        return Ok(path);
    }
    let tmp = dir.join(format!("{preset}.glsl.tmp"));
    std::fs::write(&tmp, source).map_err(|e| format!("video effect shader: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("video effect shader: {e}"))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_resolve_to_hook_files() {
        for p in PRESETS {
            let s = shader(p).unwrap();
            assert!(s.contains("//!HOOK MAIN") && s.contains("//!DESC Anime4K"), "{p}");
        }
        assert!(!known("anime4k-dog-x2") && !known("") && shader("off").is_none());
        // Mode A+A is Mode A plus one more restore pass.
        assert!(shader("anime4k-aa").unwrap().len() > shader("anime4k-a").unwrap().len());
    }

    #[test]
    fn output_doubles_up_to_1080p() {
        assert_eq!(output_size("anime4k-a", 1280, 720, 0, 0), (1920, 1080));
        assert_eq!(output_size("anime4k-a", 640, 360, 0, 0), (1280, 720));
        assert_eq!(output_size("anime4k-a", 1920, 1080, 0, 0), (1920, 1080));
        assert_eq!(output_size("anime4k-a", 3840, 2160, 0, 0), (3840, 2160));
        // 4:3 keeps its aspect; a quality box wins; lite never resizes up.
        assert_eq!(output_size("anime4k-a", 1440, 1080, 0, 0), (1440, 1080));
        assert_eq!(output_size("anime4k-a", 720, 480, 0, 0), (1440, 960));
        assert_eq!(output_size("anime4k-aa", 1280, 720, 1280, 720), (1280, 720));
        assert_eq!(output_size("anime4k-a", 1920, 1080, 854, 480), (852, 480));
        assert_eq!(output_size("anime4k-lite", 1280, 720, 0, 0), (1280, 720));
        assert_eq!(output_size("anime4k-a", 0, 0, 0, 0), (0, 0));
    }

    #[test]
    fn materialize_writes_once() {
        let dir = std::env::temp_dir().join(format!("lain-effects-{}", std::process::id()));
        let path = materialize(&dir, "anime4k-lite").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), shader("anime4k-lite").unwrap());
        assert_eq!(materialize(&dir, "anime4k-lite").unwrap(), path);
        assert!(materialize(&dir, "nope").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
