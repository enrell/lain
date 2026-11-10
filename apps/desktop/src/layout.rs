//! Layout slots (`lain.ui.layout@1`), tier 2 of the modding model
//! (DESIGN-v2 §8.3, P6, D-255). GPUI loads no markup at run time, so a
//! skin is data that picks and orders the app's own building blocks:
//! which rails Home shows and in what order, the hero's style, the parts
//! of a title page's header and its columns, and the player overlay.
//!
//! The file is `layout.json` in a theme folder, so a skin is a theme
//! folder (tokens + layout) and reloads live with it. Every key is
//! optional; a value that does not parse is reported in Settings →
//! Appearance and replaced by the default. The default reproduces the
//! built-in layout exactly. Spec: `docs/MODDING.md`.

use std::path::Path;

use serde::Deserialize;

pub const FORMAT: &str = "lain.ui.layout@1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hero {
    Full,
    Compact,
    None,
}

/// A rail position on Home.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RailSlot {
    Continue,
    Recent,
    /// Every library not placed by name.
    Libraries,
    /// One library, by name (case-insensitive) or id.
    Library(String),
    /// Every extension rail not placed elsewhere.
    Extensions,
    /// One extension's rails, or one rail of it.
    Ext(String, Option<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderPart {
    Kicker,
    Title,
    Facts,
    Genres,
    Synopsis,
    Actions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    Episodes,
    Details,
    /// Extension panels (`ui:panel`).
    Panels,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Top,
    Bottom,
    Hidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Play,
    Next,
    Time,
    Spacer,
    Mute,
    Volume,
    Audio,
    Subs,
    Fullscreen,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub name: String,
    pub hero: Hero,
    pub rails: Vec<RailSlot>,
    pub recent_limit: usize,
    pub header: Vec<HeaderPart>,
    pub columns: Vec<Column>,
    pub player_title: Place,
    pub controls: Vec<Control>,
    /// The seek bar above the control row (else below it).
    pub seek_above: bool,
    pub hints: bool,
}

impl Default for Layout {
    fn default() -> Self {
        use Control::*;
        Layout {
            name: String::new(),
            hero: Hero::Full,
            rails: vec![RailSlot::Continue, RailSlot::Recent, RailSlot::Libraries, RailSlot::Extensions],
            recent_limit: 30,
            header: vec![
                HeaderPart::Kicker,
                HeaderPart::Title,
                HeaderPart::Facts,
                HeaderPart::Genres,
                HeaderPart::Synopsis,
                HeaderPart::Actions,
            ],
            columns: vec![Column::Episodes, Column::Details, Column::Panels],
            player_title: Place::Top,
            controls: vec![Play, Next, Time, Spacer, Mute, Volume, Audio, Subs, Fullscreen],
            seek_above: true,
            hints: true,
        }
    }
}

// The file, as written: strings checked by `resolve`.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Raw {
    format: String,
    name: String,
    home: RawHome,
    item: RawItem,
    player: RawPlayer,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawHome {
    hero: Option<String>,
    rails: Option<Vec<String>>,
    recent_limit: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawItem {
    header: Option<Vec<String>>,
    columns: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawPlayer {
    title: Option<String>,
    controls: Option<Vec<String>>,
    seek: Option<String>,
    hints: Option<bool>,
}

fn rail_slot(s: &str) -> Option<RailSlot> {
    Some(match s {
        "continue" => RailSlot::Continue,
        "recent" => RailSlot::Recent,
        "libraries" => RailSlot::Libraries,
        "extensions" => RailSlot::Extensions,
        _ => {
            if let Some(name) = s.strip_prefix("library:").filter(|n| !n.trim().is_empty()) {
                RailSlot::Library(name.trim().to_string())
            } else {
                let rest = s.strip_prefix("ext:").filter(|r| !r.is_empty())?;
                match rest.split_once('/') {
                    Some((id, rail)) if !id.is_empty() && !rail.is_empty() => {
                        RailSlot::Ext(id.to_string(), Some(rail.to_string()))
                    }
                    Some(_) => return None,
                    None => RailSlot::Ext(rest.to_string(), None),
                }
            }
        }
    })
}

fn header_part(s: &str) -> Option<HeaderPart> {
    Some(match s {
        "kicker" => HeaderPart::Kicker,
        "title" => HeaderPart::Title,
        "facts" => HeaderPart::Facts,
        "genres" => HeaderPart::Genres,
        "synopsis" => HeaderPart::Synopsis,
        "actions" => HeaderPart::Actions,
        _ => return None,
    })
}

fn column(s: &str) -> Option<Column> {
    Some(match s {
        "episodes" => Column::Episodes,
        "details" => Column::Details,
        "panels" => Column::Panels,
        _ => return None,
    })
}

fn control(s: &str) -> Option<Control> {
    use Control::*;
    Some(match s {
        "play" => Play,
        "next" => Next,
        "time" => Time,
        "spacer" => Spacer,
        "mute" => Mute,
        "volume" => Volume,
        "audio" => Audio,
        "subs" => Subs,
        "fullscreen" => Fullscreen,
        _ => return None,
    })
}

/// Parses a list, dropping (and reporting) unknown names and repeats
/// (`spacer` may repeat).
fn list<T: PartialEq + Clone>(
    key: &str,
    raw: Option<Vec<String>>,
    parse: impl Fn(&str) -> Option<T>,
    default: Vec<T>,
    repeatable: impl Fn(&T) -> bool,
    errors: &mut Vec<String>,
) -> Vec<T> {
    let Some(raw) = raw else { return default };
    let mut out: Vec<T> = Vec::new();
    for s in raw {
        match parse(s.trim()) {
            Some(v) if out.contains(&v) && !repeatable(&v) => errors.push(format!("layout: {key}: {s:?} repeats")),
            Some(v) => out.push(v),
            None => errors.push(format!("layout: {key}: unknown {s:?}")),
        }
    }
    out
}

impl Layout {
    /// Parses `layout.json` bytes: the layout plus every problem found.
    /// A file that is not JSON at all gives the default and one error.
    pub fn parse(raw: &[u8]) -> (Layout, Vec<String>) {
        let mut errors = Vec::new();
        let r: Raw = match serde_json::from_slice(raw) {
            Ok(r) => r,
            Err(e) => return (Layout::default(), vec![format!("layout.json: {e}")]),
        };
        if !r.format.is_empty() && r.format != FORMAT {
            return (Layout::default(), vec![format!("layout.json: format {:?}, expected {FORMAT}", r.format)]);
        }
        let d = Layout::default();
        let hero = match r.home.hero.as_deref() {
            None | Some("full") => Hero::Full,
            Some("compact") => Hero::Compact,
            Some("none") => Hero::None,
            Some(other) => {
                errors.push(format!("layout: home.hero: unknown {other:?} (full, compact, none)"));
                Hero::Full
            }
        };
        let rails = list("home.rails", r.home.rails, rail_slot, d.rails.clone(), |_| false, &mut errors);
        let header = list("item.header", r.item.header, header_part, d.header.clone(), |_| false, &mut errors);
        let columns = list("item.columns", r.item.columns, column, d.columns.clone(), |_| false, &mut errors);
        let controls = list(
            "player.controls",
            r.player.controls,
            control,
            d.controls.clone(),
            |c| *c == Control::Spacer,
            &mut errors,
        );
        let player_title = match r.player.title.as_deref() {
            None | Some("top") => Place::Top,
            Some("bottom") => Place::Bottom,
            Some("hidden") => Place::Hidden,
            Some(other) => {
                errors.push(format!("layout: player.title: unknown {other:?} (top, bottom, hidden)"));
                Place::Top
            }
        };
        let seek_above = match r.player.seek.as_deref() {
            None | Some("above") => true,
            Some("below") => false,
            Some(other) => {
                errors.push(format!("layout: player.seek: unknown {other:?} (above, below)"));
                true
            }
        };
        let recent_limit = r.home.recent_limit.unwrap_or(d.recent_limit).clamp(1, 500);
        let layout = Layout {
            name: r.name,
            hero,
            rails,
            recent_limit,
            header,
            columns,
            player_title,
            controls,
            seek_above,
            hints: r.player.hints.unwrap_or(true),
        };
        (layout, errors)
    }

    /// The theme folder's `layout.json`, or the default when it has none.
    pub fn load(theme_dir: &Path) -> (Layout, Vec<String>) {
        match std::fs::read(theme_dir.join("layout.json")) {
            Ok(raw) => Layout::parse(&raw),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Layout::default(), Vec::new()),
            Err(e) => (Layout::default(), vec![format!("layout.json: {e}")]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_missing_give_the_built_in_layout() {
        assert_eq!(Layout::parse(b"{}"), (Layout::default(), vec![]));
        let dir = std::env::temp_dir().join(format!("lain-layout-none-{}", std::process::id()));
        assert_eq!(Layout::load(&dir), (Layout::default(), vec![]));
    }

    #[test]
    fn slots_parse_and_problems_are_reported() {
        let raw = br#"{
            "format": "lain.ui.layout@1",
            "home": { "hero": "compact", "rails": ["ext:seasonal-stats/season", "library:Anime", "continue", "nope", "continue", "ext:"] },
            "item": { "header": ["title", "actions"], "columns": ["details", "panels", "episodes"] },
            "player": { "title": "bottom", "controls": ["play", "spacer", "subs", "spacer", "warp"], "seek": "below", "hints": false }
        }"#;
        let (l, errs) = Layout::parse(raw);
        assert_eq!(l.hero, Hero::Compact);
        assert_eq!(
            l.rails,
            vec![
                RailSlot::Ext("seasonal-stats".into(), Some("season".into())),
                RailSlot::Library("Anime".into()),
                RailSlot::Continue
            ]
        );
        assert_eq!(l.header, vec![HeaderPart::Title, HeaderPart::Actions]);
        assert_eq!(l.columns, vec![Column::Details, Column::Panels, Column::Episodes]);
        assert_eq!(l.player_title, Place::Bottom);
        assert_eq!(l.controls, vec![Control::Play, Control::Spacer, Control::Subs, Control::Spacer]);
        assert!(!l.seek_above && !l.hints);
        assert_eq!(errs.len(), 4, "{errs:?}");
        assert!(errs.iter().any(|e| e.contains("\"nope\"")));
        assert!(errs.iter().any(|e| e.contains("repeats")));
        assert!(errs.iter().any(|e| e.contains("\"warp\"")));
    }

    #[test]
    fn bad_values_fall_back_and_other_formats_are_refused() {
        let (l, errs) = Layout::parse(br#"{"home":{"hero":"giant"},"player":{"title":"left","seek":"middle"}}"#);
        assert_eq!((l.hero, l.player_title, l.seek_above), (Hero::Full, Place::Top, true));
        assert_eq!(errs.len(), 3);
        let (l, errs) = Layout::parse(br#"{"format":"lain.ui.layout@2","home":{"hero":"none"}}"#);
        assert_eq!(l, Layout::default());
        assert_eq!(errs.len(), 1);
        assert_eq!(Layout::parse(b"not json").1.len(), 1);
    }

    #[test]
    fn the_sample_skin_parses_cleanly() {
        let raw = include_bytes!("../themes/shelf/layout.json");
        let (l, errs) = Layout::parse(raw);
        assert!(errs.is_empty(), "{errs:?}");
        assert_ne!(l, Layout::default());
    }
}
