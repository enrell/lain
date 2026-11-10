//! The desktop extension host's test guest. `config` picks what it does:
//!
//! - `{"mode":"ok"}` — one rail, one panel per item, and an `events` rail
//!   that lists the events it received (and invalidates on each);
//! - `{"mode":"calls"}` — `rails` makes every host call and reports each
//!   outcome as a card (`title` = call, `subtitle` = `ok …` or the error);
//! - `{"mode":"loop"}` — `rails` never returns (fuel or the epoch stops it);
//! - `{"mode":"oom"}` — `rails` allocates until memory runs out;
//! - `{"mode":"panic"}` — `rails` panics (a trap);
//! - `{"mode":"big"}` — `rails` returns far more than the host keeps;
//! - `{"mode":"refuse"}` — `activate` fails.

use lain_ext_api::{Card, Event, Extension, HostError, Panel, Rail, Row, host, register_extension};

struct Probe {
    mode: String,
    events: Vec<String>,
}

fn mode_of(config: &str) -> String {
    // `{"mode":"<m>"}` without a JSON crate: the probe stays tiny.
    let Some(at) = config.find("\"mode\"") else { return "ok".into() };
    let rest = &config[at + 6..];
    let Some(q1) = rest.find('"') else { return "ok".into() };
    let rest = &rest[q1 + 1..];
    rest.split('"').next().unwrap_or("ok").to_string()
}

fn card(title: &str, subtitle: String) -> Card {
    Card { title: title.into(), subtitle, item_id: None }
}

fn outcome<T>(r: Result<T, HostError>, ok: impl FnOnce(T) -> String) -> String {
    match r {
        Ok(v) => format!("ok {}", ok(v)),
        Err(HostError::Denied(s)) => format!("denied {s}"),
        Err(e) => format!("error {e:?}"),
    }
}

impl Probe {
    fn calls(&self) -> Vec<Card> {
        let mut out = vec![
            card("granted", host::granted_scopes().join(",")),
            card("list-libraries", outcome(host::list_libraries(), |v| v.len().to_string())),
            card("list-catalog", outcome(host::list_catalog(0, 10), |v| v.len().to_string())),
            card("get-item", outcome(host::get_item("item-1"), |v| v.title)),
            card("get-progress", outcome(host::get_progress("item-1"), |v| format!("{}", v.is_some()))),
            card("continue-watching", outcome(host::continue_watching(), |v| v.len().to_string())),
            card("http-allowed", outcome(host::http_get("https://allowed.example/x"), |r| r.status.to_string())),
            card("http-other", outcome(host::http_get("https://other.example/x"), |r| r.status.to_string())),
        ];
        let fs = std::fs::write("/data/probe.txt", b"hi").and_then(|_| std::fs::read("/data/probe.txt"));
        out.push(card(
            "fs",
            match fs {
                Ok(b) => format!("ok {}", b.len()),
                Err(e) => format!("error {e}"),
            },
        ));
        let env = std::env::vars().count();
        out.push(card("env", format!("ok {env}")));
        out
    }
}

impl Extension for Probe {
    fn new(config: &str) -> Result<Self, String> {
        let mode = mode_of(config);
        if mode == "refuse" {
            return Err("the probe refuses this config".into());
        }
        host::log(lain_ext_api::LogLevel::Info, &format!("probe activated in mode {mode}"));
        Ok(Probe { mode, events: Vec::new() })
    }

    fn rails(&mut self) -> Vec<Rail> {
        match self.mode.as_str() {
            "calls" => vec![Rail { id: "calls".into(), title: "Calls".into(), cards: self.calls() }],
            "loop" => {
                let mut n: u64 = 0;
                loop {
                    n = std::hint::black_box(n.wrapping_add(1));
                }
            }
            "oom" => {
                let mut hoard: Vec<Vec<u8>> = Vec::new();
                loop {
                    hoard.push(vec![7u8; 1 << 20]);
                    std::hint::black_box(&hoard);
                }
            }
            "panic" => panic!("the probe panics on purpose"),
            "big" => (0..20)
                .map(|r| Rail {
                    id: format!("r{r}"),
                    title: "T".repeat(10_000),
                    cards: (0..300).map(|c| card(&format!("c{c}"), "s".repeat(300))).collect(),
                })
                .collect(),
            _ => vec![
                Rail {
                    id: "probe".into(),
                    title: "Probe".into(),
                    cards: vec![Card { title: "One".into(), subtitle: "first".into(), item_id: Some("item-1".into()) }],
                },
                Rail {
                    id: "events".into(),
                    title: "Events".into(),
                    cards: self.events.iter().map(|e| card(e, String::new())).collect(),
                },
            ],
        }
    }

    fn panels(&mut self, item_id: &str) -> Vec<Panel> {
        vec![Panel {
            id: "probe".into(),
            title: "Probe".into(),
            rows: vec![Row { label: "item".into(), value: item_id.into() }],
            text: String::new(),
        }]
    }

    fn on_event(&mut self, event: Event) {
        let name = match event {
            Event::ProgressChanged(id) => format!("progress-changed {id}"),
            Event::LibraryChanged => "library-changed".into(),
            Event::ItemOpened(id) => format!("item-opened {id}"),
            Event::PlaybackStarted(id) => format!("playback-started {id}"),
            Event::PlaybackStopped(id) => format!("playback-stopped {id}"),
        };
        self.events.push(name);
        host::invalidate();
    }
}

register_extension!(Probe);
