//! System font substitution: a font a PDF names without embedding it
//! (the standard 14, or a common office font) is drawn with an installed
//! look-alike. Lain ships no font data; the operator's system provides
//! it (the Docker image installs the URW base 35 set, the metric clones
//! of the standard 14). When nothing fits, the text is not drawn and the
//! report says so ([`available`]).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// What a substitute must look like.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Family {
    Sans,
    Serif,
    Mono,
    Symbol,
    Dingbats,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Style {
    pub family: Family,
    pub bold: bool,
    pub italic: bool,
}

/// The style a font name and its descriptor flags ask for.
pub fn style_for(base_font: &str, flags: i64, weight: f64) -> Style {
    // Subset fonts carry a `ABCDEF+` tag.
    let name = match base_font.split_once('+') {
        Some((tag, rest)) if tag.len() == 6 && tag.bytes().all(|b| b.is_ascii_uppercase()) => rest,
        _ => base_font,
    };
    let low = name.to_ascii_lowercase();
    let family = if low.contains("symbol") {
        Family::Symbol
    } else if low.contains("dingbat") {
        Family::Dingbats
    } else if low.contains("courier")
        || low.contains("mono")
        || low.contains("consol")
        || low.contains("typewriter")
        || flags & 1 != 0
    {
        Family::Mono
    } else if [
        "times",
        "serif",
        "roman",
        "georgia",
        "garamond",
        "palatino",
        "book",
        "cambria",
        "minion",
        "baskerville",
        "caslon",
        "century",
        "bodoni",
    ]
    .iter()
    .any(|k| low.contains(k))
        && !low.contains("sans")
        || (flags & 2 != 0 && !low.contains("sans") && !low.contains("arial") && !low.contains("helvetica"))
    {
        Family::Serif
    } else {
        Family::Sans
    };
    let bold = ["bold", "black", "heavy", "semibold", "demi"].iter().any(|k| low.contains(k))
        || weight >= 600.0
        || flags & (1 << 18) != 0;
    let italic = low.contains("italic") || low.contains("oblique") || flags & 64 != 0;
    Style { family, bold, italic }
}

/// Candidate file stems for a style, best first: URW base 35 (exact
/// metric clones), Liberation, then the usual Windows and macOS names
/// and DejaVu as a last resort.
fn candidates(s: Style) -> Vec<&'static str> {
    use Family::*;
    match (s.family, s.bold, s.italic) {
        (Sans, false, false) => vec![
            "NimbusSans-Regular",
            "n019003l",
            "LiberationSans-Regular",
            "Arial",
            "arial",
            "Helvetica",
            "DejaVuSans",
        ],
        (Sans, true, false) => {
            vec!["NimbusSans-Bold", "n019004l", "LiberationSans-Bold", "Arial Bold", "arialbd", "DejaVuSans-Bold"]
        }
        (Sans, false, true) => vec![
            "NimbusSans-Italic",
            "n019023l",
            "LiberationSans-Italic",
            "Arial Italic",
            "ariali",
            "DejaVuSans-Oblique",
        ],
        (Sans, true, true) => vec![
            "NimbusSans-BoldItalic",
            "n019024l",
            "LiberationSans-BoldItalic",
            "Arial Bold Italic",
            "arialbi",
            "DejaVuSans-BoldOblique",
        ],
        (Serif, false, false) => vec![
            "NimbusRoman-Regular",
            "n021003l",
            "LiberationSerif-Regular",
            "Times New Roman",
            "times",
            "DejaVuSerif",
        ],
        (Serif, true, false) => vec![
            "NimbusRoman-Bold",
            "n021004l",
            "LiberationSerif-Bold",
            "Times New Roman Bold",
            "timesbd",
            "DejaVuSerif-Bold",
        ],
        (Serif, false, true) => vec![
            "NimbusRoman-Italic",
            "n021023l",
            "LiberationSerif-Italic",
            "Times New Roman Italic",
            "timesi",
            "DejaVuSerif-Italic",
        ],
        (Serif, true, true) => vec![
            "NimbusRoman-BoldItalic",
            "n021024l",
            "LiberationSerif-BoldItalic",
            "Times New Roman Bold Italic",
            "timesbi",
            "DejaVuSerif-BoldItalic",
        ],
        (Mono, false, false) => {
            vec!["NimbusMonoPS-Regular", "n022003l", "LiberationMono-Regular", "Courier New", "cour", "DejaVuSansMono"]
        }
        (Mono, true, false) => vec![
            "NimbusMonoPS-Bold",
            "n022004l",
            "LiberationMono-Bold",
            "Courier New Bold",
            "courbd",
            "DejaVuSansMono-Bold",
        ],
        (Mono, false, true) => vec![
            "NimbusMonoPS-Italic",
            "n022023l",
            "LiberationMono-Italic",
            "Courier New Italic",
            "couri",
            "DejaVuSansMono-Oblique",
        ],
        (Mono, true, true) => vec![
            "NimbusMonoPS-BoldItalic",
            "n022024l",
            "LiberationMono-BoldItalic",
            "Courier New Bold Italic",
            "courbi",
            "DejaVuSansMono-BoldOblique",
        ],
        (Symbol, ..) => vec!["StandardSymbolsPS", "s050000l", "Symbol", "symbol"],
        (Dingbats, ..) => vec!["D050000L", "d050000l", "ZapfDingbats"],
    }
}

fn font_dirs() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = Vec::new();
    if let Ok(extra) = std::env::var("LAIN_FONT_DIR") {
        v.extend(std::env::split_paths(&extra));
    }
    v.extend(
        [
            "/usr/share/fonts",
            "/usr/local/share/fonts",
            "/usr/X11R6/lib/X11/fonts",
            "/System/Library/Fonts",
            "/Library/Fonts",
        ]
        .map(PathBuf::from),
    );
    if let Some(home) = std::env::var_os("HOME") {
        v.push(Path::new(&home).join(".local/share/fonts"));
        v.push(Path::new(&home).join(".fonts"));
        v.push(Path::new(&home).join("Library/Fonts"));
    }
    if let Some(win) = std::env::var_os("WINDIR") {
        v.push(Path::new(&win).join("Fonts"));
    }
    v
}

/// Every font file under the font folders, by lowercase file stem.
fn index() -> &'static HashMap<String, PathBuf> {
    static INDEX: OnceLock<HashMap<String, PathBuf>> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut out = HashMap::new();
        for d in font_dirs() {
            walk(&d, 0, &mut out);
        }
        out
    })
}

fn walk(dir: &Path, depth: usize, out: &mut HashMap<String, PathBuf>) {
    if depth > 6 || out.len() > 50_000 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            walk(&p, depth + 1, out);
            continue;
        }
        let ext = p.extension().and_then(|x| x.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
        // Type 1 (.t1/.pfb), OpenType and TrueType; .otf first wins.
        let rank = match ext.as_str() {
            "otf" => 0,
            "ttf" => 1,
            "t1" | "pfb" => 2,
            _ => continue,
        };
        let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else { continue };
        let key = stem.to_ascii_lowercase();
        let better = out.get(&key).is_none_or(|old: &PathBuf| {
            let old_ext = old.extension().and_then(|x| x.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
            let old_rank = match old_ext.as_str() {
                "otf" => 0,
                "ttf" => 1,
                _ => 2,
            };
            rank < old_rank
        });
        if better {
            out.insert(key, p);
        }
    }
}

/// The file standing in for `style`, if any is installed.
pub fn find(style: Style) -> Option<PathBuf> {
    let idx = index();
    for c in candidates(style) {
        if let Some(p) = idx.get(&c.to_ascii_lowercase()) {
            return Some(p.clone());
        }
    }
    // Any style of the family beats nothing.
    if style.bold || style.italic {
        return find(Style { bold: false, italic: false, ..style });
    }
    if style.family != Family::Sans && matches!(style.family, Family::Serif | Family::Mono) {
        return find(Style { family: Family::Sans, ..style });
    }
    None
}

/// Whether any substitute is installed (for `lain doctor` and the book
/// manifest's warning).
pub fn available() -> bool {
    find(Style { family: Family::Sans, bold: false, italic: false }).is_some()
}

/// Loaded substitute programs, shared by every document.
pub fn load(path: &Path) -> Option<Arc<Vec<u8>>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Arc<Vec<u8>>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(v) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(path) {
        return Some(v.clone());
    }
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > 64 << 20 {
        return None;
    }
    let data = Arc::new(std::fs::read(path).ok()?);
    cache.lock().unwrap_or_else(|e| e.into_inner()).insert(path.to_path_buf(), data.clone());
    Some(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styles_from_names() {
        let s = style_for("ABCDEF+Times-BoldItalic", 0, 0.0);
        assert_eq!(s, Style { family: Family::Serif, bold: true, italic: true });
        assert_eq!(style_for("Helvetica", 0, 0.0).family, Family::Sans);
        assert_eq!(style_for("ArialMT", 0, 0.0).family, Family::Sans);
        assert_eq!(style_for("CourierNewPSMT", 0, 0.0).family, Family::Mono);
        assert_eq!(style_for("Symbol", 0, 0.0).family, Family::Symbol);
        assert_eq!(style_for("Unknown", 2, 700.0), Style { family: Family::Serif, bold: true, italic: false });
        assert!(!candidates(s).is_empty());
    }
}
