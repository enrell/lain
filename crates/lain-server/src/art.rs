//! What a thumbnail falls back to when the file's own picture cannot be
//! made (D-287): the source is unreachable (an unmounted disk) or the
//! frame could not be cut. A card must never be an empty box, so the
//! route serves, in order, the title's art next to the file, the title's
//! art from its metadata overlay (a local `.nfo` picture, or the
//! provider's address the overlay already holds, as a redirect: the
//! server fetches nothing), and last a placeholder drawn here in the
//! theme's colours with the title's name on it.

use std::path::{Component, Path, PathBuf};

use serde_json::{Value, json};

use lain_api::{CatalogItem, Palette};
use lain_auth::Verified;
use lain_metadata::{CAP_METADATA_ENRICH, NFO_ID};
use lain_watch::thumbnail;

use crate::app::App;
use crate::catalog::CAP_CATALOG_READ;

/// Response header naming what a fallback answer is.
pub const HEADER: &str = "x-lain-thumbnail";
/// Response header saying why the file's own picture was not served.
pub const REASON_HEADER: &str = "x-lain-thumbnail-reason";
/// `HEADER`: the title's art stands in for the file's.
pub const TITLE_ART: &str = "title-art";
/// `HEADER`: nothing to show, so a drawn placeholder.
pub const PLACEHOLDER: &str = "placeholder";
/// `REASON_HEADER`: the file cannot be reached.
pub const SOURCE_UNAVAILABLE: &str = "source-unavailable";
/// `REASON_HEADER`: the file is there but no frame could be cut.
pub const FRAME_UNAVAILABLE: &str = "frame-unavailable";

/// What stands in for the file's picture.
#[derive(Debug, PartialEq)]
pub enum TitleArt {
    /// A picture on this machine.
    File(PathBuf),
    /// The provider's address of the title's art.
    Redirect(String),
    /// An SVG document.
    Placeholder(String),
}

/// The title's art for `it`, or a placeholder. `wide` asks for a 16:9
/// slot (an episode's still, a frame at a time), which prefers the
/// overlay's wide cover over its poster; else the poster comes first.
pub fn title_art(app: &App, v: &Verified, it: &CatalogItem, wide: bool, width: i64) -> TitleArt {
    if let Some(p) = thumbnail::local_cover(Path::new(&it.file_path)) {
        return TitleArt::File(p);
    }
    let overlays = overlays(app, v, it);
    if let Some(art) = choose(&overlays, wide) {
        return art;
    }
    let name = overlays.iter().filter_map(|o| o["title"].as_str()).find(|t| !t.trim().is_empty()).unwrap_or(&it.title);
    TitleArt::Placeholder(placeholder(&palette(app), name, &label(it), wide || !readable(&it.kind), width))
}

/// The overlays of the title `it` belongs to: its own first, then its
/// siblings' (an episode seldom has one of its own), only from libraries
/// this session may see.
fn overlays(app: &App, v: &Verified, it: &CatalogItem) -> Vec<Value> {
    let hidden = app.hidden_libraries(v);
    let mut ids = vec![it.id.clone()];
    if let Ok((out, _)) = app.registry.call_one(CAP_CATALOG_READ, "episodes", json!({"id": it.id})) {
        let siblings: Vec<CatalogItem> = serde_json::from_value(out).unwrap_or_default();
        ids.extend(
            siblings
                .into_iter()
                .filter(|s| s.id != it.id && !hidden.contains(&s.library_id))
                .map(|s| s.id)
                .take(crate::population::MAX_ENRICH_BATCH - 1),
        );
    }
    let Ok((out, _)) = app.registry.call_one(CAP_METADATA_ENRICH, "batch", json!({"ids": ids})) else {
        return Vec::new();
    };
    let mut found: Vec<Value> = out["items"].as_array().cloned().unwrap_or_default();
    // The batch answers in its own order: the item's own overlay first.
    found.sort_by_key(|o| ids.iter().position(|id| o["item_id"].as_str() == Some(id)).unwrap_or(usize::MAX));
    found
}

/// The first usable picture of the overlays, field by field: a wide slot
/// takes any overlay's cover before any poster.
fn choose(overlays: &[Value], wide: bool) -> Option<TitleArt> {
    let fields: [&str; 2] = if wide { ["cover", "poster"] } else { ["poster", "cover"] };
    fields.iter().find_map(|field| {
        overlays.iter().find_map(|o| {
            let value = o[*field].as_str().unwrap_or_default().trim();
            if value.is_empty() {
                return None;
            }
            if let Some(url) = remote(value) {
                return Some(TitleArt::Redirect(url.to_string()));
            }
            nfo_picture(o["provider"].as_str().unwrap_or_default(), o["remote_id"].as_str().unwrap_or_default(), value)
                .map(TitleArt::File)
        })
    })
}

/// A provider's picture address: http(s), nothing a header cannot carry.
fn remote(value: &str) -> Option<&str> {
    let low = value.get(..8).unwrap_or(value).to_ascii_lowercase();
    let web = low.starts_with("https://") || low.starts_with("http://");
    (web && !value.chars().any(|c| c.is_control() || c.is_whitespace())).then_some(value)
}

/// A picture an `.nfo` names beside itself (`<thumb>poster.jpg</thumb>`):
/// a picture's name under the `.nfo`'s folder, never a path out of it or
/// a link, so an overlay cannot make the route serve any other file.
fn nfo_picture(provider: &str, nfo: &str, value: &str) -> Option<PathBuf> {
    if provider != NFO_ID {
        return None;
    }
    let rel = Path::new(value);
    if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        return None;
    }
    let ext = rel.extension()?.to_string_lossy().to_ascii_lowercase();
    if !matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "webp" | "gif") {
        return None;
    }
    let path = Path::new(nfo).parent()?.join(rel);
    std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_file() && m.len() > 0).then_some(path)
}

fn readable(kind: &str) -> bool {
    matches!(kind, "comic" | "manga" | "book")
}

/// The small line under the name: which part of the title this is.
fn label(it: &CatalogItem) -> String {
    match (it.season, it.episode) {
        (s, e) if s > 0 && e > 0 => format!("S{s:02}E{e:02}"),
        (_, e) if e > 0 && readable(&it.kind) => format!("No. {e}"),
        (_, e) if e > 0 => format!("Episode {e}"),
        _ if it.year > 0 => it.year.to_string(),
        _ => String::new(),
    }
}

/// The served theme's palette (the built-in one when its provider fails).
fn palette(app: &App) -> Palette {
    let path = app.theme_path.as_ref().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    app.registry
        .call_one(lain_admin::theme::CAP_UI_THEME, "derive", json!({"colors_path": path}))
        .ok()
        .and_then(|(out, _)| serde_json::from_value(out).ok())
        .unwrap_or_else(lain_admin::theme::default_palette)
}

/// A colour as SVG may take it: `#rrggbb`, else the fallback.
fn color<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    let hex = value.len() == 7 && value.starts_with('#') && value[1..].bytes().all(|b| b.is_ascii_hexdigit());
    if hex { value } else { fallback }
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// The name broken into at most `max` lines of about `per` characters,
/// the last cut with an ellipsis when the name runs over.
fn lines(name: &str, per: usize, max: usize) -> Vec<String> {
    let cut = |s: &str| s.chars().take(per.saturating_sub(1)).collect::<String>().trim_end().to_string() + "…";
    let mut out: Vec<String> = Vec::new();
    for word in name.split_whitespace() {
        let word = if word.chars().count() > per { cut(word) } else { word.to_string() };
        match out.last_mut() {
            Some(line) if line.chars().count() + 1 + word.chars().count() <= per => {
                line.push(' ');
                line.push_str(&word);
            }
            _ => out.push(word),
        }
    }
    if out.len() > max {
        out.truncate(max);
        let last = &mut out[max - 1];
        *last = if last.chars().count() < per { format!("{last}…") } else { cut(last) };
    }
    if out.is_empty() {
        out.push("?".into());
    }
    out
}

/// The placeholder: the title's name in large tight type over the
/// theme's surface, the house's small mono label under it, as the web
/// client draws its own (`PlaceholderPoster`). `wide` is 16:9, else 2:3.
pub fn placeholder(p: &Palette, name: &str, label: &str, wide: bool, width: i64) -> String {
    let w = width.clamp(32, 1280) as f64;
    let h = if wide { (w * 9.0 / 16.0).round() } else { (w * 1.5).round() };
    let pad = (w * 0.07).round();
    let size = if wide { (h * 0.13).max(8.0) } else { (w * 0.12).max(8.0) };
    let per = (((w - 2.0 * pad) / (size * 0.56)).floor() as usize).max(4);
    let text = lines(name, per, 3);
    let label_size = (size * 0.38).max(6.0);
    let bottom = h - pad - if label.is_empty() { 0.0 } else { label_size * 2.0 };
    let line_h = size * 1.05;
    let first = bottom - line_h * (text.len() as f64 - 1.0);
    let (surface, base, fg, muted, accent) = (
        color(&p.surface_active, "#1e222c"),
        color(&p.background, "#08090c"),
        color(&p.foreground, "#e7e9f0"),
        color(&p.muted, "#8a90a3"),
        color(&p.accent, "#5ce1c4"),
    );
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">\
         <defs><linearGradient id=\"g\" x1=\"0\" y1=\"0\" x2=\"1\" y2=\"1\">\
         <stop offset=\"0\" stop-color=\"{surface}\"/><stop offset=\"1\" stop-color=\"{base}\"/></linearGradient></defs>\
         <rect width=\"{w}\" height=\"{h}\" fill=\"url(#g)\"/>\
         <rect x=\"{pad}\" y=\"{pad}\" width=\"{bar}\" height=\"{thick}\" rx=\"{r}\" fill=\"{accent}\"/>\
         <text font-family=\"ui-sans-serif, system-ui, -apple-system, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif\" \
         font-size=\"{size:.1}\" font-weight=\"700\" letter-spacing=\"-0.02em\" fill=\"{fg}\">",
        bar = (w * 0.08).round(),
        thick = (w * 0.008).max(2.0).round(),
        r = (w * 0.004).max(1.0).round(),
    );
    for (i, line) in text.iter().enumerate() {
        svg.push_str(&format!("<tspan x=\"{pad}\" y=\"{:.1}\">{}</tspan>", first + line_h * i as f64, escape(line)));
    }
    svg.push_str("</text>");
    if !label.is_empty() {
        svg.push_str(&format!(
            "<text x=\"{pad}\" y=\"{:.1}\" font-family=\"ui-monospace, 'SF Mono', Menlo, Consolas, 'Liberation Mono', monospace\" \
             font-size=\"{label_size:.1}\" font-weight=\"600\" letter-spacing=\"0.18em\" fill=\"{muted}\">{}</text>",
            h - pad,
            escape(&label.to_uppercase()),
        ));
    }
    svg.push_str("</svg>");
    svg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_is_lettered_safely_in_the_theme() {
        let mut p = lain_admin::theme::default_palette();
        p.foreground = "\"/><script>".into();
        let svg = placeholder(&p, "Tom & Jerry <script>alert(1)</script>", "S01E02", true, 640);
        assert!(svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"640\" height=\"360\""), "{svg}");
        assert!(!svg.contains("<script"), "{svg}");
        assert!(svg.contains("Tom &amp; Jerry"), "{svg}");
        // A colour that is not one falls back to the built-in foreground.
        assert!(svg.contains("fill=\"#e7e9f0\""), "{svg}");
        assert!(svg.contains(">S01E02</text>"), "{svg}");
        // Upright for a book's cover.
        assert!(placeholder(&p, "Book", "", false, 200).contains("height=\"300\""));
    }

    #[test]
    fn a_long_name_wraps_into_three_lines_at_most() {
        assert_eq!(lines("Paper Moon Station", 12, 3), vec!["Paper Moon", "Station"]);
        let long = lines("The Very Long Name Of A Show That Goes On And On", 10, 3);
        assert_eq!(long.len(), 3);
        assert!(long[2].ends_with('…'), "{long:?}");
        assert_eq!(lines("Supercalifragilistic", 8, 3), vec!["Superca…"]);
        assert_eq!(lines("   ", 8, 3), vec!["?"]);
    }

    #[test]
    fn the_overlay_art_is_chosen_for_the_slot() {
        let overlays = vec![
            json!({"item_id": "ep2", "provider": "lain-metadata-anilist", "poster": "https://img.example/p.jpg"}),
            json!({"item_id": "ep1", "provider": "lain-metadata-kitsu", "cover": "https://img.example/c.jpg"}),
        ];
        assert_eq!(choose(&overlays, true), Some(TitleArt::Redirect("https://img.example/c.jpg".into())));
        assert_eq!(choose(&overlays, false), Some(TitleArt::Redirect("https://img.example/p.jpg".into())));
        // What a Location header cannot carry, or a scheme that is not the web's, is no picture.
        let odd = vec![json!({"poster": "https://img.example/a b.jpg", "cover": "javascript:alert(1)"})];
        assert_eq!(choose(&odd, false), None);
        assert_eq!(choose(&[], true), None);
    }

    #[test]
    fn an_nfo_picture_stays_beside_its_nfo() {
        let dir = lain_store::test_dir("art-nfo");
        std::fs::write(dir.join("poster.jpg"), b"jpeg").unwrap();
        std::fs::write(dir.join("empty.jpg"), b"").unwrap();
        std::fs::write(dir.join("notes.txt"), b"text").unwrap();
        let nfo = dir.join("movie.nfo").to_string_lossy().into_owned();
        assert_eq!(nfo_picture(NFO_ID, &nfo, "poster.jpg"), Some(dir.join("poster.jpg")));
        assert_eq!(nfo_picture("lain-metadata-kitsu", &nfo, "poster.jpg"), None);
        for refused in ["../poster.jpg", "/etc/passwd.jpg", "empty.jpg", "notes.txt", "gone.jpg"] {
            assert_eq!(nfo_picture(NFO_ID, &nfo, refused), None, "{refused}");
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.join("poster.jpg"), dir.join("link.jpg")).unwrap();
            assert_eq!(nfo_picture(NFO_ID, &nfo, "link.jpg"), None);
        }
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_label_names_the_part() {
        let ep = CatalogItem { season: 1, episode: 3, ..CatalogItem::default() };
        assert_eq!(label(&ep), "S01E03");
        assert_eq!(label(&CatalogItem { episode: 12, ..CatalogItem::default() }), "Episode 12");
        assert_eq!(label(&CatalogItem { year: 1999, ..CatalogItem::default() }), "1999");
        assert_eq!(label(&CatalogItem::default()), "");
    }
}
