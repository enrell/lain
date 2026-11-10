//! Seasonal stats, the sample lain extension (P6, `docs/MODDING.md`).
//!
//! - A Home rail, "<Season> <year> · new this season": every title with
//!   a file added since the anime season began (Winter = January, Spring
//!   = April, Summer = July, Fall = October), newest first, with how many
//!   of those files are new and how many you finished.
//! - A panel on each title page: the season, new files, finished ones
//!   and the time you spent on the title this season.
//! - It keeps the catalog it read and drops it when the library changes;
//!   progress events refresh the rail.
//!
//! Scopes: `catalog:read`, `progress:read`, `events:subscribe`, `ui:rail`,
//! `ui:panel`. Config: `{"days": N}` uses the last N days instead of the
//! calendar season (0, the default, keeps the season).

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use lain_ext_api::{Card, Event, Extension, Item, Panel, Rail, Row, host, register_extension};

struct Seasonal {
    days: i64,
    catalog: Option<Vec<Item>>,
}

/// Unix seconds now (the WASI wall clock).
fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Days since 1970-01-01 → (year, month 1-12, day), Howard Hinnant's algorithm.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// (year, month, day) → days since 1970-01-01.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = i64::from(m);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The anime season of `t`: its name and its first second.
fn season(t: i64) -> (String, i64) {
    let (y, m, _) = civil(t.div_euclid(86_400));
    let (name, first) = match m {
        1..=3 => ("Winter", 1),
        4..=6 => ("Spring", 4),
        7..=9 => ("Summer", 7),
        _ => ("Fall", 10),
    };
    (format!("{name} {y}"), days_from_civil(y, first, 1) * 86_400)
}

fn hours(sec: f64) -> String {
    let m = (sec / 60.).round() as i64;
    if m >= 60 { format!("{}h {:02}m", m / 60, m % 60) } else { format!("{m} min") }
}

/// Library + lower-case title, how the app groups files into titles.
fn title_key(it: &Item) -> String {
    format!("{}\u{0}{}", it.library_id, it.title.to_lowercase())
}

impl Seasonal {
    /// The window: its label and its first second.
    fn window(&self) -> (String, i64) {
        if self.days > 0 {
            (format!("Last {} days", self.days), now() - self.days * 86_400)
        } else {
            season(now())
        }
    }

    fn catalog(&mut self) -> Option<&[Item]> {
        if self.catalog.is_none() {
            match lain_ext_api::catalog() {
                Ok(c) => self.catalog = Some(c),
                Err(e) => {
                    lain_ext_api::warn(format!("catalog: {e}"));
                    return None;
                }
            }
        }
        self.catalog.as_deref()
    }
}

impl Extension for Seasonal {
    fn new(config: &str) -> Result<Self, String> {
        // `{"days": N}` without a JSON crate: the component stays small.
        let days = config
            .split_once("\"days\"")
            .and_then(|(_, rest)| {
                let digits: String =
                    rest.trim_start_matches([':', ' ']).chars().take_while(|c| c.is_ascii_digit()).collect();
                digits.parse::<i64>().ok()
            })
            .unwrap_or(0);
        if !(0..=3650).contains(&days) {
            return Err(format!("days must be 0..3650, got {days}"));
        }
        Ok(Seasonal { days, catalog: None })
    }

    fn rails(&mut self) -> Vec<Rail> {
        let (label, since) = self.window();
        // Finished items, from the Continue list (it carries completed ones too).
        let done: HashMap<String, bool> = host::continue_watching()
            .map(|v| v.into_iter().map(|p| (p.item_id, p.completed)).collect())
            .unwrap_or_default();
        let Some(catalog) = self.catalog() else { return Vec::new() };
        // title → (newest item, its time, new files, finished new files)
        let mut titles: HashMap<String, (usize, i64, u32, u32)> = HashMap::new();
        for (i, it) in catalog.iter().enumerate().filter(|(_, it)| it.updated_at >= since) {
            let e = titles.entry(title_key(it)).or_insert((i, it.updated_at, 0, 0));
            if it.updated_at > e.1 {
                (e.0, e.1) = (i, it.updated_at);
            }
            e.2 += 1;
            if done.get(&it.id).copied().unwrap_or(false) {
                e.3 += 1;
            }
        }
        let mut rows: Vec<_> = titles.into_values().collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1));
        let cards: Vec<Card> = rows
            .into_iter()
            .take(40)
            .map(|(i, _, new, finished)| {
                let it = &catalog[i];
                Card {
                    title: it.title.clone(),
                    subtitle: format!("{new} new · {finished} finished"),
                    item_id: Some(it.id.clone()),
                }
            })
            .collect();
        if cards.is_empty() {
            return Vec::new();
        }
        vec![Rail { id: "season".into(), title: format!("{label} · new this season"), cards }]
    }

    fn panels(&mut self, item_id: &str) -> Vec<Panel> {
        let (label, since) = self.window();
        let Ok(item) = host::get_item(item_id) else { return Vec::new() };
        let key = title_key(&item);
        let Some(catalog) = self.catalog() else { return Vec::new() };
        let files: Vec<Item> = catalog.iter().filter(|it| title_key(it) == key).cloned().collect();
        let new = files.iter().filter(|it| it.updated_at >= since).count();
        let (mut finished, mut spent, mut last) = (0, 0f64, 0i64);
        for it in files.iter().take(200) {
            let Ok(Some(p)) = host::get_progress(&it.id) else { continue };
            if p.updated_at < since {
                continue;
            }
            if p.completed {
                finished += 1;
            }
            spent += if p.completed { p.duration_sec } else { p.position_sec };
            last = last.max(p.updated_at);
        }
        let mut rows = vec![
            Row { label: "window".into(), value: label },
            Row { label: "new files".into(), value: format!("{new} of {}", files.len()) },
            Row { label: "finished".into(), value: finished.to_string() },
            Row { label: "time spent".into(), value: hours(spent) },
        ];
        if last > 0 {
            let (y, m, d) = civil(last.div_euclid(86_400));
            rows.push(Row { label: "last played".into(), value: format!("{y}-{m:02}-{d:02}") });
        }
        vec![Panel {
            id: "season".into(),
            title: "Seasonal stats".into(),
            rows,
            text: if new == 0 { "Nothing new for this title this season.".into() } else { String::new() },
        }]
    }

    fn on_event(&mut self, event: Event) {
        match event {
            Event::LibraryChanged => {
                self.catalog = None;
                host::invalidate();
            }
            Event::ProgressChanged(_) => host::invalidate(),
            _ => {}
        }
    }
}

register_extension!(Seasonal);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seasons_start_on_their_month() {
        // 2026-11-09T12:00:00Z
        let t = days_from_civil(2026, 11, 9) * 86_400 + 43_200;
        assert_eq!(season(t), ("Fall 2026".into(), days_from_civil(2026, 10, 1) * 86_400));
        assert_eq!(season(days_from_civil(2027, 2, 28) * 86_400).0, "Winter 2027");
        assert_eq!(civil(days_from_civil(2024, 2, 29)), (2024, 2, 29));
    }
}
