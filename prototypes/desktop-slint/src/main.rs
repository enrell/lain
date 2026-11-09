// lain desktop prototype (P2b), Slint. See README.md.
// GUI subsystem in every profile: no console window ever opens; use --log.
#![windows_subsystem = "windows"]

mod art;
mod data;
mod metrics;
mod mpv;
mod theme;
mod ui;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lain_client::types::Progress;
use lain_client::{Client, EventStream, ServerEvent, StreamConfig, StreamItem};
use slint::{ComponentHandle, Image, Model, ModelRc, VecModel};
use slint_interpreter::{ComponentInstance, Struct, Value};

use metrics::Frames;
use mpv::{Event, Format, Mpv, Prop, RenderCtx};

// ---- arguments ------------------------------------------------------------

struct Args {
    theme: PathBuf,
    skin: Option<PathBuf>,
    shared: PathBuf,
    media: PathBuf,
    server: String,
    bench: Option<PathBuf>,
    mpv_log: Option<PathBuf>,
    /// directory for in-app snapshots (home.png, and player shots in bench)
    shots: Option<PathBuf>,
    /// exit after this many seconds (unattended runs)
    quit_after: Option<f64>,
    /// physical window rect x,y,w,h; the window then opens without
    /// activation (test runs on a shared desktop)
    window: Option<(i32, i32, u32, u32)>,
    /// no audio output (mpv ao=null)
    mute: bool,
    /// open the hero's episode once Home has loaded (unattended checks)
    play: bool,
    /// stderr goes to this file (the exe has no console)
    log: Option<PathBuf>,
}

fn exe_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_default()
}

/// A relative path is taken from the working directory, else next to the exe.
fn resolve(p: PathBuf) -> PathBuf {
    if p.is_absolute() || p.exists() { p } else { exe_dir().join(p) }
}

fn parse_args() -> Args {
    let mut a = Args {
        theme: PathBuf::new(),
        skin: None,
        shared: PathBuf::from(r"D:\lain-proto\shared"),
        media: PathBuf::from(r"D:\lain-proto\media"),
        server: "http://127.0.0.1:9360".into(),
        bench: None,
        mpv_log: None,
        shots: None,
        quit_after: None,
        window: None,
        mute: false,
        play: false,
        log: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let mut v = || it.next().map(PathBuf::from).unwrap_or_default();
        match k.as_str() {
            "--theme" => a.theme = resolve(v()),
            "--skin" => a.skin = Some(resolve(v())),
            "--shared" => a.shared = v(),
            "--media" => a.media = v(),
            "--server" => a.server = v().to_string_lossy().into_owned(),
            "--bench" => a.bench = Some(v()),
            "--mpv-log" => a.mpv_log = Some(v()),
            "--shots" => a.shots = Some(v()),
            "--mute" => a.mute = true,
            "--play" => a.play = true,
            "--log" => a.log = Some(v()),
            "--window-pos" => {
                let v: Vec<i32> = v().to_string_lossy().split(',').filter_map(|x| x.trim().parse().ok()).collect();
                if let [x, y] = v[..] {
                    let r = a.window.get_or_insert((0, 0, 1600, 900));
                    (r.0, r.1) = (x, y);
                }
            }
            "--window-size" => {
                let v: Vec<u32> = v().to_string_lossy().split('x').filter_map(|x| x.trim().parse().ok()).collect();
                if let [w, h] = v[..] {
                    let r = a.window.get_or_insert((0, 0, 1600, 900));
                    (r.2, r.3) = (w, h);
                }
            }
            "--quit-after" => a.quit_after = v().to_str().and_then(|s| s.parse().ok()),
            other => eprintln!("unknown argument {other}"),
        }
    }
    if a.theme.as_os_str().is_empty() {
        a.theme = a.shared.join("theme.json");
    }
    a
}

// ---- catalog model --------------------------------------------------------

#[derive(Clone)]
struct Item {
    id: String,
    series: usize,
    season: i64,
    episode: i64,
    year: i64,
    updated: i64,
}

fn s(v: &str) -> Value {
    Value::String(v.into())
}
fn n(v: f64) -> Value {
    Value::Number(v)
}
fn st(fields: Vec<(&str, Value)>) -> Value {
    Value::Struct(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect::<Struct>())
}
fn model(v: Vec<Value>) -> Value {
    Value::Model(ModelRc::new(VecModel::from(v)))
}
fn fmt_min(sec: f64) -> String {
    let m = (sec / 60.0).round() as i64;
    if m >= 60 { format!("{}h {:02}m", m / 60, m % 60) } else { format!("{m} min") }
}

// ---- the UI-thread state ------------------------------------------------------

struct App {
    inst: ComponentInstance,
    rt: tokio::runtime::Handle,
    client: Option<Client>,
    series: Vec<(String, String)>,
    items: Vec<Item>,
    by_id: HashMap<String, usize>,
    cont: Vec<Progress>,
    rails: Rc<VecModel<Value>>,
    posters: Rc<VecModel<Value>>,
    thumbs: Rc<VecModel<Value>>,
    hero_series: Option<usize>,
    art_params: theme::ArtParams,
    art_gen: u64,
    art_rev: u64,
    loader: Arc<art::Loader>,
    posters_done: usize,
    hero_done: bool,
    art_ready: Rc<Cell<bool>>,
    shared: PathBuf,
    media: PathBuf,
    mpv: Arc<Mpv>,
    player_active: Rc<Cell<bool>>,
    playing: Option<String>,
    bench: bool,
    autoplay: bool,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

/// Runs `f` on the UI thread with the app state.
fn post(f: impl FnOnce(&mut App) + Send + 'static) {
    let _ = slint::invoke_from_event_loop(move || {
        with_app(f);
    });
}

impl App {
    fn set_home(&self, prop: &str, v: Value) {
        if let Err(e) = self.inst.set_global_property("Home", prop, v) {
            eprintln!("Home.{prop}: {e}");
        }
    }
    fn set_player(&self, prop: &str, v: Value) {
        if let Err(e) = self.inst.set_global_property("Player", prop, v) {
            eprintln!("Player.{prop}: {e}");
        }
    }

    fn series_title(&self, i: usize) -> &str {
        self.series.get(i).map(|s| s.1.as_str()).unwrap_or("?")
    }

    fn card(&self, it: &Item, label: String, progress: f64) -> Value {
        st(vec![
            ("item", s(&it.id)),
            ("title", s(self.series_title(it.series))),
            ("label", s(&label)),
            ("series", n(it.series as f64)),
            ("progress", n(progress)),
        ])
    }

    fn ep_label(it: &Item) -> String {
        format!("S{} · E{:02}", it.season, it.episode)
    }

    fn continue_rail(&self) -> Option<Value> {
        let cards: Vec<Value> = self
            .cont
            .iter()
            .filter_map(|p| {
                let it = &self.items[*self.by_id.get(&p.item_id)?];
                let frac = if p.duration_sec > 0.0 { p.position_sec / p.duration_sec } else { 0.0 };
                let left = (p.duration_sec - p.position_sec).max(0.0);
                Some(self.card(it, format!("{} · {} left", Self::ep_label(it), fmt_min(left)), frac))
            })
            .collect();
        (!cards.is_empty()).then(|| st(vec![("heading", s("Continue watching")), ("kind", n(1.0)), ("cards", model(cards))]))
    }

    /// Rebuilds every rail: Continue, Premieres, All (500), then by decade.
    fn build_rails(&mut self) {
        let mut sorted: Vec<&Item> = self.items.iter().collect();
        sorted.sort_by(|a, b| self.series_title(a.series).cmp(self.series_title(b.series)).then(a.episode.cmp(&b.episode)));
        let rail = |title: String, items: Vec<&Item>| -> Value {
            let cards: Vec<Value> =
                items.iter().map(|it| self.card(it, format!("{} · {}", Self::ep_label(it), it.year), -1.0)).collect();
            st(vec![("heading", s(&title)), ("kind", n(0.0)), ("cards", model(cards))])
        };
        let mut rails = Vec::new();
        if let Some(c) = self.continue_rail() {
            rails.push(c);
        }
        let mut recent: Vec<&Item> = sorted.iter().copied().filter(|i| i.episode == 1).collect();
        recent.sort_by(|a, b| b.updated.cmp(&a.updated));
        rails.push(rail(format!("Premieres · {} series", recent.len()), recent));
        rails.push(rail(format!("All episodes · {}", sorted.len()), sorted.clone()));
        for (from, to) in [(2000, 2009), (2010, 2019), (2020, 2029)] {
            let v: Vec<&Item> = sorted.iter().copied().filter(|i| i.year >= from && i.year <= to).collect();
            if !v.is_empty() {
                rails.push(rail(format!("From the {}s", from), v));
            }
        }
        self.rails.set_vec(rails);
    }

    fn all_rail_row(&self) -> i32 {
        // rows are 1-based (0 = hero); All is after Premieres
        if self.cont.is_empty() { 2 } else { 3 }
    }

    fn refresh_continue(&mut self) {
        let has = self.rails.row_count() > 0
            && self.rails.row_data(0).is_some_and(|r| matches!(&r, Value::Struct(s) if s.get_field("kind") == Some(&n(1.0))));
        match (self.continue_rail(), has) {
            (Some(r), true) => self.rails.set_row_data(0, r),
            (Some(r), false) => self.rails.insert(0, r),
            (None, true) => {
                self.rails.remove(0);
            }
            (None, false) => {}
        }
    }

    fn set_hero(&mut self) {
        // D-022: resume first, otherwise recently added
        let (it, kicker, action, meta_extra) = if let Some(p) = self.cont.first()
            && let Some(&i) = self.by_id.get(&p.item_id)
        {
            let it = self.items[i].clone();
            let left = (p.duration_sec - p.position_sec).max(0.0);
            (it.clone(), "CONTINUE WATCHING", format!("Resume E{:02}", it.episode), format!(" · {} left", fmt_min(left)))
        } else if let Some(it) = self.items.iter().max_by_key(|i| (i.updated, -i.episode)) {
            let first = self.items.iter().find(|i| i.series == it.series && i.episode == 1).unwrap_or(it).clone();
            (first, "RECENTLY ADDED", "Play E01".to_string(), String::new())
        } else {
            return;
        };
        let eps = self.items.iter().filter(|i| i.series == it.series).count();
        self.set_home("hero-kicker", s(kicker));
        self.set_home("hero-title", s(self.series_title(it.series)));
        self.set_home("hero-meta", s(&format!("Season {} · {eps} episodes · {}{meta_extra}", it.season, it.year)));
        self.set_home("hero-action", s(&action));
        self.set_home("hero-item", s(&it.id));
        if self.hero_series != Some(it.series) {
            self.hero_series = Some(it.series);
            self.hero_done = false;
            self.queue_hero();
        }
    }

    fn scale(&self) -> f32 {
        self.inst.window().scale_factor()
    }

    fn queue_hero(&self) {
        let Some(si) = self.hero_series else { return };
        let sz = self.inst.window().size();
        let (w, h) = (sz.width.max(1600), (sz.height.max(900) as f32 * 0.8) as u32);
        let slug = &self.series[si].0;
        self.loader.push(
            vec![art::Job {
                kind: art::Kind::Hero,
                series: si,
                path: self.shared.join("art").join(format!("{slug}-backdrop.jpg")),
                w,
                h,
                blur: self.art_params.hero_blur * self.scale() / 2.0,
                generation: self.art_gen,
            }],
            true,
        );
    }

    /// Queues posters and landscape thumbs for every series, sized for the
    /// current theme and DPI (scaled up by hover_scale so hover stays sharp).
    fn queue_art(&mut self) {
        self.loader.clear();
        self.art_gen += 1;
        self.posters_done = 0;
        let k = self.scale() * self.art_params.hover_scale.max(1.0);
        let p = &self.art_params;
        let (pw, ph) = ((p.card_w * k) as u32, (p.card_h * k) as u32);
        // landscape: Continue cards (16:9 from the shorter card side) and,
        // for wide themes, every card
        let side = p.card_w.min(p.card_h) * 0.9;
        let (tw, th) = if p.card_w > p.card_h {
            ((p.card_w * k) as u32, (p.card_h * k) as u32)
        } else {
            ((side * 16.0 / 9.0 * k) as u32, (side * k) as u32)
        };
        let mut jobs = Vec::new();
        for (i, (slug, _)) in self.series.iter().enumerate() {
            let art = self.shared.join("art");
            jobs.push(art::Job {
                kind: art::Kind::Poster,
                series: i,
                path: art.join(format!("{slug}-poster.jpg")),
                w: pw,
                h: ph,
                blur: 0.0,
                generation: self.art_gen,
            });
            jobs.push(art::Job {
                kind: art::Kind::Thumb,
                series: i,
                path: art.join(format!("{slug}-backdrop.jpg")),
                w: tw,
                h: th,
                blur: 0.0,
                generation: self.art_gen,
            });
        }
        self.loader.push(jobs, false);
        self.queue_hero();
    }

    fn on_art(&mut self, d: art::Done) {
        if d.generation != self.art_gen {
            return;
        }
        let img = Value::Image(Image::from_rgb8(d.buf));
        self.art_rev += 1;
        match d.kind {
            art::Kind::Poster => {
                self.posters.set_row_data(d.series, img);
                self.posters_done += 1;
            }
            art::Kind::Thumb => self.thumbs.set_row_data(d.series, img),
            art::Kind::Hero => {
                if Some(d.series) == self.hero_series {
                    self.set_home("hero-art", img);
                    self.hero_done = true;
                }
            }
        }
        // "first Home frame with art": the hero and the first screenful of posters
        if self.hero_done && self.posters_done >= 12.min(self.series.len()) && !self.items.is_empty() {
            self.art_ready.set(true);
        }
    }

    // ---- player -----------------------------------------------------------

    fn file_for(&self, it: &Item) -> PathBuf {
        if it.episode % 2 == 0 {
            self.media.join("teste-4k-hevc-60fps.mkv")
        } else {
            let a = self.media.join("teste-1080p-hevc-10bit-legendas.mkv");
            if a.exists() { a } else { self.media.join("teste-1080p-hevc-10bit.mkv") }
        }
    }

    fn open(&mut self, id: &str) {
        let Some(&i) = self.by_id.get(id) else { return };
        let it = self.items[i].clone();
        let file = self.file_for(&it);
        self.playing = Some(it.id.clone());
        self.player_active.set(true);
        self.set_player("title", s(self.series_title(it.series)));
        self.set_player(
            "subtitle",
            s(&format!("{} · {}", Self::ep_label(&it), file.file_name().unwrap_or_default().to_string_lossy())),
        );
        self.set_player("message", s("Loading…"));
        self.set_player("pos", n(0.0));
        self.set_player("dur", n(0.0));
        self.set_player("cache", n(0.0));
        self.set_player("info", s(""));
        self.set_player("active", Value::Bool(true));
        let bench = self.bench;
        let client = self.client.clone();
        let id = it.id.clone();
        self.rt.spawn(async move {
            let start = match (&client, bench) {
                (Some(c), false) => data::progress(c, &id).await.filter(|p| !p.completed).map(|p| p.position_sec),
                _ => None,
            };
            post(move |app| {
                if app.playing.as_deref() != Some(id.as_str()) {
                    return;
                }
                let start = start.unwrap_or(0.0);
                app.mpv.set("start", &format!("{start:.3}"));
                if let Err(e) = app.mpv.command(&["loadfile", &file.to_string_lossy(), "replace"]) {
                    app.set_player("message", s(&e));
                }
            });
        });
    }

    fn save_progress(&self) {
        if self.bench {
            return;
        }
        let (Some(id), Some(c)) = (self.playing.clone(), self.client.clone()) else { return };
        let (Some(pos), Some(dur)) = (self.mpv.get_f64("time-pos"), self.mpv.get_f64("duration")) else { return };
        self.rt.spawn(async move { data::save(&c, &id, pos, dur).await });
    }

    fn close_player(&mut self) {
        self.save_progress();
        self.playing = None;
        let _ = self.mpv.command(&["stop"]);
        self.player_active.set(false);
        if self.inst.window().is_fullscreen() {
            set_fullscreen(&self.inst, false);
            self.set_player("fullscreen", Value::Bool(false));
        }
        self.set_player("active", Value::Bool(false));
        self.set_player("message", s(""));
    }

    fn on_mpv(&mut self, m: MpvMsg) {
        match m {
            MpvMsg::Pos(v) => self.set_player("pos", n(v)),
            MpvMsg::Dur(v) => self.set_player("dur", n(v)),
            MpvMsg::Cache(v) => self.set_player("cache", n(v)),
            MpvMsg::Pause(b) => self.set_player("paused", Value::Bool(b)),
            MpvMsg::Volume(v) => self.set_player("volume", n(v)),
            MpvMsg::Mute(b) => self.set_player("muted", Value::Bool(b)),
            MpvMsg::Info(i) => self.set_player("info", s(&i)),
            MpvMsg::Loaded => self.set_player("message", s("")),
            MpvMsg::Error(e) => {
                if self.playing.is_some() {
                    self.set_player("message", s(&e));
                }
            }
            MpvMsg::Tracks(list) => {
                let mk = |kind: &str| -> Vec<Value> {
                    list.iter()
                        .filter(|t| t.0 == kind)
                        .map(|t| st(vec![("id", n(t.1 as f64)), ("label", s(&t.2)), ("selected", Value::Bool(t.3))]))
                        .collect()
                };
                let mut subs = vec![st(vec![
                    ("id", n(0.0)),
                    ("label", s("Off")),
                    ("selected", Value::Bool(!list.iter().any(|t| t.0 == "sub" && t.3))),
                ])];
                subs.extend(mk("sub"));
                self.set_player("audio", model(mk("audio")));
                self.set_player("subs", model(subs));
            }
        }
    }
}

// ---- mpv events → UI ----------------------------------------------------------

enum MpvMsg {
    Pos(f64),
    Dur(f64),
    Cache(f64),
    Pause(bool),
    Volume(f64),
    Mute(bool),
    Info(String),
    Loaded,
    Error(String),
    Tracks(Vec<(String, i64, String, bool)>),
}

fn tracks(m: &Mpv) -> Vec<(String, i64, String, bool)> {
    let count = m.get_i64("track-list/count").unwrap_or(0);
    (0..count)
        .filter_map(|i| {
            let g = |k: &str| m.get_str(&format!("track-list/{i}/{k}")).unwrap_or_default();
            let kind = g("type");
            if kind == "video" {
                return None;
            }
            let id = g("id").parse().ok()?;
            let mut parts: Vec<String> = Vec::new();
            for k in ["lang", "title", "codec"] {
                let v = g(k);
                if !v.is_empty() {
                    parts.push(v);
                }
            }
            if kind == "audio" {
                let ch = g("demux-channel-count");
                if !ch.is_empty() {
                    parts.push(format!("{ch}ch"));
                }
            }
            if parts.is_empty() {
                parts.push(format!("Track {id}"));
            }
            Some((kind, id, parts.join(" · "), g("selected") == "yes"))
        })
        .collect()
}

fn info(m: &Mpv) -> String {
    let hw = m.get_str("hwdec-current").unwrap_or_default();
    let hw = if hw.is_empty() || hw == "no" { "software decode".to_string() } else { format!("hwdec {hw}") };
    let w = m.get_i64("video-params/w").unwrap_or(0);
    let h = m.get_i64("video-params/h").unwrap_or(0);
    let pf = m.get_str("video-params/pixelformat").unwrap_or_default();
    let codec = m.get_str("video-format").unwrap_or_default();
    let fps = m.get_f64("container-fps").unwrap_or(0.0);
    format!("{hw} · {codec} {w}×{h} {pf} · {fps:.2} fps")
}

fn spawn_mpv_events(m: Arc<Mpv>) {
    for (p, f) in [
        ("time-pos", Format::Double),
        ("duration", Format::Double),
        ("pause", Format::Flag),
        ("demuxer-cache-time", Format::Double),
        ("volume", Format::Double),
        ("mute", Format::Flag),
        ("track-list/count", Format::Int),
        ("aid", Format::Str),
        ("sid", Format::Str),
        ("hwdec-current", Format::Str),
        ("video-params/w", Format::Int),
    ] {
        m.observe(p, f);
    }
    std::thread::Builder::new()
        .name("mpv-events".into())
        .spawn(move || {
            let mut last_pos = Instant::now() - Duration::from_secs(1);
            let send = |msg: MpvMsg| post(move |app| app.on_mpv(msg));
            loop {
                match m.wait_event(-1.0) {
                    Event::Shutdown => break,
                    Event::PlaybackRestart => {
                        eprintln!("mpv: playing, current-ao={}", m.get_str("current-ao").unwrap_or_else(|| "(none)".into()));
                    }
                    Event::FileLoaded => {
                        send(MpvMsg::Loaded);
                        // show subtitles by default: mpv only auto-selects
                        // tracks flagged default/forced
                        if m.get_str("sid").as_deref() == Some("no")
                            && let Some(t) = tracks(&m).into_iter().find(|t| t.0 == "sub")
                        {
                            m.set("sid", &t.1.to_string());
                        }
                        send(MpvMsg::Tracks(tracks(&m)));
                        send(MpvMsg::Info(info(&m)));
                    }
                    Event::EndFile { reason, error } if reason == 4 => {
                        send(MpvMsg::Error(format!("Playback failed (mpv error {error})")));
                    }
                    Event::PropertyChange { name, value } => match (name.as_str(), value) {
                        ("time-pos", Prop::Double(v)) => {
                            // a few updates per second is enough for the bar
                            if last_pos.elapsed() >= Duration::from_millis(250) {
                                last_pos = Instant::now();
                                send(MpvMsg::Pos(v));
                            }
                        }
                        ("duration", Prop::Double(v)) => send(MpvMsg::Dur(v)),
                        ("demuxer-cache-time", Prop::Double(v)) => send(MpvMsg::Cache(v)),
                        ("pause", Prop::Flag(b)) => send(MpvMsg::Pause(b)),
                        ("volume", Prop::Double(v)) => send(MpvMsg::Volume(v)),
                        ("mute", Prop::Flag(b)) => send(MpvMsg::Mute(b)),
                        ("track-list/count" | "aid" | "sid", _) => send(MpvMsg::Tracks(tracks(&m))),
                        ("hwdec-current" | "video-params/w", _) => send(MpvMsg::Info(info(&m))),
                        _ => {}
                    },
                    _ => {}
                }
            }
        })
        .expect("spawn mpv thread");
}

// ---- server -----------------------------------------------------------------

fn spawn_server(rt: &tokio::runtime::Runtime, server: String, conta: PathBuf, series: Vec<(String, String)>) {
    rt.spawn(async move {
        let client = match data::client(&server) {
            Ok(c) => c,
            Err(e) => return post(move |a| a.set_home("message", s(&format!("Client: {e}")))),
        };
        let creds = data::creds(&conta);
        let user = match data::ensure_login(&client, creds.as_ref()).await {
            Ok(u) => u,
            Err(e) => {
                return post(move |a| {
                    a.set_home("status", s("offline"));
                    a.set_home("message", s(&format!("Could not sign in to {server}: {e}")));
                });
            }
        };
        let (items, cont) = match tokio::join!(data::catalog(&client), data::continue_list(&client)) {
            (Ok(i), c) => (i, c.unwrap_or_default()),
            (Err(e), _) => return post(move |a| a.set_home("message", s(&e))),
        };
        let index: HashMap<&str, usize> = series.iter().enumerate().map(|(i, s)| (s.1.as_str(), i)).collect();
        let items: Vec<Item> = items
            .into_iter()
            .filter_map(|c| {
                Some(Item {
                    series: *index.get(c.title.as_str())?,
                    id: c.id,
                    season: c.season,
                    episode: c.episode,
                    year: c.year,
                    updated: c.updated_at,
                })
            })
            .collect();
        let c2 = client.clone();
        post(move |a| {
            a.client = Some(c2);
            a.by_id = items.iter().enumerate().map(|(i, it)| (it.id.clone(), i)).collect();
            a.items = items;
            a.cont = cont;
            a.set_home("user", s(&user));
            a.set_home("status", s("online"));
            a.set_home("online", Value::Bool(true));
            a.set_home("message", s(""));
            a.build_rails();
            a.set_hero();
            a.on_art_check();
            if a.autoplay
                && let Ok(Value::String(id)) = a.inst.get_global_property("Home", "hero-item")
            {
                let id = id.to_string();
                a.open(&id);
            }
        });

        // events: refetch Continue on progress-changed and after reconnects
        let stream = EventStream::new(client.clone(), StreamConfig::default());
        let mut connected_once = false;
        while let Some(ev) = stream.next().await {
            let refetch = match ev {
                StreamItem::Connected(_) => {
                    post(|a| {
                        a.set_home("online", Value::Bool(true));
                        a.set_home("status", s("online"));
                    });
                    std::mem::replace(&mut connected_once, true)
                }
                StreamItem::Event(ServerEvent::ProgressChanged(_)) => true,
                StreamItem::Reconnecting { .. } | StreamItem::AuthRequired => {
                    post(|a| {
                        a.set_home("online", Value::Bool(false));
                        a.set_home("status", s("reconnecting"));
                    });
                    false
                }
                _ => false,
            };
            if refetch && let Ok(cont) = data::continue_list(&client).await {
                post(move |a| {
                    a.cont = cont;
                    a.refresh_continue();
                });
            }
        }
    });
}

impl App {
    fn on_art_check(&mut self) {
        if self.hero_done && self.posters_done >= 12.min(self.series.len()) && !self.items.is_empty() {
            self.art_ready.set(true);
        }
    }
}

/// Borderless fullscreen on an explicit monitor: the one holding the
/// window's centre (never winit's "current monitor" guess).
fn set_fullscreen(inst: &ComponentInstance, on: bool) {
    use slint::winit_030::WinitWindowAccessor;
    use slint::winit_030::winit::window::Fullscreen;
    inst.window().with_winit_window(|w| {
        if !on {
            w.set_fullscreen(None);
            return;
        }
        let (Ok(p), s) = (w.outer_position(), w.outer_size()) else { return };
        let (cx, cy) = (p.x + s.width as i32 / 2, p.y + s.height as i32 / 2);
        let m = w.available_monitors().find(|m| {
            let (mp, ms) = (m.position(), m.size());
            cx >= mp.x && cx < mp.x + ms.width as i32 && cy >= mp.y && cy < mp.y + ms.height as i32
        });
        if let Some(m) = m {
            w.set_fullscreen(Some(Fullscreen::Borderless(Some(m))));
        }
    });
}

/// Saves what the window shows, through Slint's own capture.
pub fn snapshot(inst: &ComponentInstance, path: &Path) {
    match inst.window().take_snapshot() {
        Ok(buf) => {
            let r = image::RgbaImage::from_raw(buf.width(), buf.height(), buf.as_bytes().to_vec())
                .ok_or("bad buffer".to_string())
                .and_then(|img| img.save(path).map_err(|e| e.to_string()));
            match r {
                Ok(()) => eprintln!("snapshot {}", path.display()),
                Err(e) => eprintln!("snapshot {}: {e}", path.display()),
            }
        }
        Err(e) => eprintln!("snapshot: {e}"),
    }
}

// ---- main -------------------------------------------------------------------

fn fonts() -> Vec<PathBuf> {
    let dir = exe_dir().join("fonts");
    let dir = if dir.is_dir() { dir } else { PathBuf::from("fonts") };
    let mut v: Vec<PathBuf> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("ttf" | "otf")))
        .map(|p| std::fs::canonicalize(&p).unwrap_or(p))
        .map(|p| PathBuf::from(p.to_string_lossy().trim_start_matches(r"\\?\")))
        .collect();
    v.sort();
    v
}

fn load_series(shared: &Path) -> Vec<(String, String)> {
    #[derive(serde::Deserialize)]
    struct S {
        slug: String,
        title: String,
    }
    std::fs::read_to_string(shared.join("series.json"))
        .ok()
        .and_then(|r| serde_json::from_str::<Vec<S>>(&r).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|s| (s.slug, s.title))
        .collect()
}

fn main() {
    if let Err(e) = run() {
        eprintln!("lain-slint: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = parse_args();
    if let Some(l) = &args.log {
        metrics::redirect_stderr(l);
    }
    let t_main = metrics::process_age_ms();

    let rect = args.window;
    slint::BackendSelector::new()
        .with_winit_window_attributes_hook(move |attrs| {
            use slint::winit_030::winit::dpi::{PhysicalPosition, PhysicalSize};
            match rect {
                Some((x, y, w, h)) => attrs
                    .with_position(PhysicalPosition::new(x, y))
                    .with_inner_size(PhysicalSize::new(w, h))
                    .with_active(false)
                    // the GL window may be created before show(); keep it
                    // hidden until Slint maps it
                    .with_visible(false),
                None => attrs,
            }
        })
        .backend_name("winit".into())
        .renderer_name("femtovg".into())
        // desktop GL 3.3: Slint's default is GLES 2.0, too little for mpv's
        // 10-bit and HDR paths
        .require_opengl_with_version(3, 3)
        .select()
        .map_err(|e| format!("backend: {e}"))?;
    metrics::hide_winit_event_target();

    let th = theme::load(&args.theme).map_err(|e| format!("theme: {e}"))?;
    let fonts = fonts();
    let custom = th.easing.starts_with("cubic-bezier(").then_some(th.easing.as_str());
    let built = ui::build(args.skin.as_deref(), &fonts, custom)?;
    eprintln!("ui compiled in {:.1} ms (skin {}, {} fonts)", built.compile_ms, built.skin, fonts.len());
    let inst = built.def.create().map_err(|e| e.to_string())?;

    for w in theme::apply(&inst, &th) {
        eprintln!("theme: {w}");
    }
    let bundled = ["Inter", "Fraunces", "JetBrains Mono"];
    for f in &th.fonts {
        if !bundled.contains(&f.as_str()) {
            eprintln!("theme: font {f:?} is not bundled; Slint looks it up in the system fonts, else uses its default");
        }
    }
    let _ = inst.set_global_property("Home", "skin", s(&built.skin));

    let series = load_series(&args.shared);
    let posters = Rc::new(VecModel::from(vec![Value::Image(Image::default()); series.len()]));
    let thumbs = Rc::new(VecModel::from(vec![Value::Image(Image::default()); series.len()]));
    let rails = Rc::new(VecModel::<Value>::default());
    inst.set_global_property("Home", "posters", Value::Model(posters.clone().into())).map_err(|e| e.to_string())?;
    inst.set_global_property("Home", "thumbs", Value::Model(thumbs.clone().into())).map_err(|e| e.to_string())?;
    inst.set_global_property("Home", "rails", Value::Model(rails.clone().into())).map_err(|e| e.to_string())?;
    let bench = args.bench.is_some();
    let _ = inst.set_global_property("Home", "bench", Value::Bool(bench));
    let _ = inst.set_global_property("Player", "bench", Value::Bool(bench));

    // mpv: one core for the app's lifetime; video goes through the render API
    let mut opts: Vec<(&str, String)> = vec![
        ("vo", "libmpv".into()),
        ("hwdec", "auto-safe".into()),
        ("keep-open", "yes".into()),
        ("idle", "yes".into()),
        ("osc", "no".into()),
        ("load-scripts", "no".into()),
        ("input-default-bindings", "no".into()),
        ("terminal", "no".into()),
        ("audio-client-name", "lain-slint".into()),
        ("sub-auto", "no".into()),
    ];
    if args.mute {
        // set before mpv_initialize (Mpv::new applies options first)
        opts.push(("ao", "null".into()));
        opts.push(("mute", "yes".into()));
    }
    if let Some(l) = &args.mpv_log {
        opts.push(("log-file", l.to_string_lossy().into_owned()));
    }
    let o: Vec<(&str, &str)> = opts.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let mpv = Arc::new(Mpv::new(&o)?);
    spawn_mpv_events(mpv.clone());
    eprintln!("mpv: ao={} mute={}", mpv.get_str("ao").unwrap_or_default(), mpv.get_str("mute").unwrap_or_default());

    // rendering notifier: mpv underlay + frame metrics
    let player_active = Rc::new(Cell::new(false));
    let frames = Rc::new(RefCell::new(Frames::default()));
    let art_ready = Rc::new(Cell::new(false));
    let cold_start: Rc<Cell<Option<f64>>> = Rc::new(Cell::new(None));
    let scroll: Rc<Cell<Option<(Instant, f64, f64)>>> = Rc::new(Cell::new(None)); // start, speed px/s, max px
    let gl_info: Rc<RefCell<String>> = Rc::default();
    {
        let mpv = mpv.clone();
        let active = player_active.clone();
        let frames = frames.clone();
        let art_ready = art_ready.clone();
        let cold_start = cold_start.clone();
        let scroll = scroll.clone();
        let gl_info = gl_info.clone();
        let weak = inst.as_weak();
        let mut ctx: Option<RenderCtx> = None;
        inst.window()
            .set_rendering_notifier(move |state, _api| match state {
                slint::RenderingState::RenderingSetup => {
                    // Slint renders the first frame before mapping the window:
                    // make the requested rect its normal placement, so the
                    // first ShowWindow cannot put it anywhere else.
                    if let (Some((x, y, _, _)), Some(i)) = (rect, weak.upgrade()) {
                        use slint::winit_030::WinitWindowAccessor;
                        i.window().with_winit_window(|win| {
                            use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
                            if let Ok(hd) = win.window_handle()
                                && let RawWindowHandle::Win32(wh) = hd.as_raw()
                            {
                                let o = win.outer_size();
                                metrics::set_normal_rect(wh.hwnd.get(), x, y, o.width as i32, o.height as i32);
                            }
                        });
                    }
                    *gl_info.borrow_mut() = mpv::gl_info();
                    eprintln!("GL: {}", gl_info.borrow());
                    match RenderCtx::new_gl(mpv.clone()) {
                        Ok(mut c) => {
                            let w = weak.clone();
                            c.set_update_callback(Box::new(move || {
                                let _ = w.upgrade_in_event_loop(|i| i.window().request_redraw());
                            }));
                            ctx = Some(c);
                        }
                        Err(e) => eprintln!("mpv render context: {e}"),
                    }
                }
                slint::RenderingState::BeforeRendering => {
                    frames.borrow_mut().before();
                    if let (Some(c), Some(i)) = (&ctx, weak.upgrade()) {
                        c.update();
                        if active.get() {
                            mpv::gl_reset_for_mpv();
                            let sz = i.window().size();
                            c.render(0, sz.width as i32, sz.height as i32);
                        }
                        if let Some((t0, speed, max)) = scroll.get() {
                            let x = (t0.elapsed().as_secs_f64() * speed).min(max);
                            let _ = i.set_global_property("Home", "bench-x", n(x));
                            i.window().request_redraw();
                        }
                    }
                }
                slint::RenderingState::AfterRendering => {
                    frames.borrow_mut().after();
                    if let Some(c) = &ctx {
                        c.report_swap();
                    }
                    if cold_start.get().is_none() && art_ready.get() {
                        let ms = metrics::process_age_ms();
                        cold_start.set(Some(ms));
                        eprintln!("first Home frame with art: {ms:.0} ms after process start");
                    }
                }
                slint::RenderingState::RenderingTeardown => ctx = None,
                _ => {}
            })
            .map_err(|e| format!("rendering notifier: {e:?}"))?;
    }

    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().map_err(|e| e.to_string())?;
    let loader = Arc::new(art::Loader::new(3, |d| post(move |a| a.on_art(d))));

    APP.with(|a| {
        *a.borrow_mut() = Some(App {
            inst: inst.clone_strong(),
            rt: rt.handle().clone(),
            client: None,
            series: series.clone(),
            items: Vec::new(),
            by_id: HashMap::new(),
            cont: Vec::new(),
            rails,
            posters,
            thumbs,
            hero_series: None,
            art_params: th.art.clone(),
            art_gen: 0,
            art_rev: 0,
            loader,
            posters_done: 0,
            hero_done: false,
            art_ready: art_ready.clone(),
            shared: args.shared.clone(),
            media: args.media.clone(),
            mpv: mpv.clone(),
            player_active: player_active.clone(),
            playing: None,
            bench,
            autoplay: args.play,
        })
    });

    // callbacks
    let cb = |g: &str, name: &str, f: Box<dyn Fn(&mut App, &[Value])>| {
        let r = inst.set_global_callback(g, name, move |args| {
            with_app(|a| f(a, args));
            Value::Void
        });
        if let Err(e) = r {
            eprintln!("callback {g}.{name}: {e}");
        }
    };
    let num = |v: &[Value]| match v.first() {
        Some(Value::Number(x)) => *x,
        _ => 0.0,
    };
    cb("Home", "open", Box::new(|a, v| {
        if let Some(Value::String(id)) = v.first() {
            let id = id.to_string();
            a.open(&id);
        }
    }));
    cb("Player", "toggle-pause", Box::new(|a, _| {
        let _ = a.mpv.command(&["cycle", "pause"]);
    }));
    cb("Player", "seek-to", Box::new(move |a, v| {
        let _ = a.mpv.command(&["seek", &format!("{:.3}", num(v)), "absolute"]);
    }));
    cb("Player", "seek-by", Box::new(move |a, v| {
        let _ = a.mpv.command(&["seek", &format!("{:.3}", num(v)), "relative"]);
    }));
    cb("Player", "set-volume", Box::new(move |a, v| {
        a.mpv.set("volume", &format!("{:.0}", num(v)));
        a.mpv.set("mute", "no");
    }));
    cb("Player", "toggle-mute", Box::new(|a, _| {
        let _ = a.mpv.command(&["cycle", "mute"]);
    }));
    cb("Player", "select-audio", Box::new(move |a, v| a.mpv.set("aid", &format!("{}", num(v) as i64))));
    cb("Player", "select-sub", Box::new(move |a, v| {
        let id = num(v) as i64;
        a.mpv.set("sid", &if id == 0 { "no".to_string() } else { id.to_string() });
    }));
    cb("Player", "toggle-fullscreen", Box::new(|a, _| {
        let fs = !a.inst.window().is_fullscreen();
        set_fullscreen(&a.inst, fs);
        a.set_player("fullscreen", Value::Bool(fs));
    }));
    cb("Player", "close", Box::new(|a, _| a.close_player()));

    // progress: every 5 s while playing (and on close, above)
    let save_timer = slint::Timer::default();
    save_timer.start(slint::TimerMode::Repeated, Duration::from_secs(5), || {
        with_app(|a| {
            if a.playing.is_some() {
                a.save_progress();
            }
        });
    });

    // theme live reload: poll the file's mtime
    let reload_timer = slint::Timer::default();
    {
        let path = args.theme.clone();
        let mut last = theme::mtime(&path);
        reload_timer.start(slint::TimerMode::Repeated, Duration::from_millis(400), move || {
            let now = theme::mtime(&path);
            if now == last {
                return;
            }
            last = now;
            let t = std::time::Instant::now();
            match theme::load(&path) {
                Ok(th) => with_app(|a| {
                    for w in theme::apply(&a.inst, &th) {
                        eprintln!("theme: {w}");
                    }
                    if th.art != a.art_params {
                        a.art_params = th.art.clone();
                        a.hero_done = false;
                        a.queue_art();
                    }
                    eprintln!("theme reloaded in {:.1} ms", t.elapsed().as_secs_f64() * 1000.0);
                })
                .unwrap_or(()),
                Err(e) => eprintln!("theme reload: {e}"),
            }
        });
    }

    spawn_server(&rt, args.server.clone(), args.shared.join("conta-slint.txt"), series);
    if let Some((x, y, w, h)) = args.window {
        let sf = metrics::monitor_scale_at(x + w as i32 / 2, y + h as i32 / 2);
        eprintln!("target monitor scale {sf}");
        let _ = inst.set_property("want-width", n((w as f32 / sf) as f64));
        let _ = inst.set_property("want-height", n((h as f32 / sf) as f64));
        // before show(): an explicit size stops Slint from resizing the
        // window to the component's preferred size when it maps it
        inst.window().set_position(slint::PhysicalPosition::new(x, y));
        inst.window().set_size(slint::PhysicalSize::new(w, h));
    }
    inst.show().map_err(|e| e.to_string())?;
    if args.window.is_some() {
        let p = inst.window().position();
        eprintln!("window at {},{} size {:?}", p.x, p.y, inst.window().size());
    }
    with_app(|a| a.queue_art());
    eprintln!("main() reached at {t_main:.0} ms after process start");

    let _bench_timer = args.bench.clone().map(|out| {
        bench::start(out, args.shots.clone(), frames.clone(), cold_start.clone(), scroll.clone(), gl_info.clone(), built.compile_ms)
    });

    // --shots: Home 4 s after the first frame with art; --quit-after
    let shot_timer = slint::Timer::default();
    {
        let dir = args.shots.clone();
        let quit = args.quit_after;
        let cold = cold_start.clone();
        let t0 = Instant::now();
        let mut shot_at: Option<Instant> = None;
        let mut done = false;
        shot_timer.start(slint::TimerMode::Repeated, Duration::from_millis(250), move || {
            if let (Some(d), false) = (&dir, done) {
                if cold.get().is_some() && shot_at.is_none() {
                    shot_at = Some(Instant::now());
                }
                if shot_at.is_some_and(|t| t.elapsed() > Duration::from_secs(4)) && !bench {
                    done = true;
                    with_app(|a| snapshot(&a.inst, &d.join("home.png")));
                }
            }
            if quit.is_some_and(|q| t0.elapsed().as_secs_f64() > q) {
                let _ = slint::quit_event_loop();
            }
        });
    }
    let size_check = slint::Timer::default();
    if args.window.is_some() {
        let weak = inst.as_weak();
        size_check.start(slint::TimerMode::SingleShot, Duration::from_secs(4), move || {
            if let Some(i) = weak.upgrade() {
                use slint::winit_030::WinitWindowAccessor;
                i.window().with_winit_window(|w| {
                    eprintln!(
                        "size check: slint {:?} winit inner {:?} outer {:?} at {:?}",
                        i.window().size(), w.inner_size(), w.outer_size(), w.outer_position()
                    );
                });
            }
        });
    }
    let dbg_timer = slint::Timer::default();
    if std::env::var_os("LAIN_DEBUG").is_some() {
        let frames = frames.clone();
        dbg_timer.start(slint::TimerMode::Repeated, Duration::from_secs(2), move || {
            let rev = with_app(|a| a.art_rev).unwrap_or(0);
            eprintln!("dbg: frames={} art_rev={rev}", frames.borrow().count);
        });
    }
    slint::run_event_loop().map_err(|e| e.to_string())?;

    // window closed: last progress write, synchronously
    let job = with_app(|a| {
        let (Some(id), Some(c)) = (a.playing.clone(), a.client.clone()) else { return None };
        Some((id, c, a.mpv.get_f64("time-pos")?, a.mpv.get_f64("duration")?))
    })
    .flatten();
    if let Some((id, c, pos, dur)) = job
        && !bench
    {
        rt.block_on(async { tokio::time::timeout(Duration::from_secs(3), data::save(&c, &id, pos, dur)).await.ok() });
    }
    APP.with(|a| a.borrow_mut().take());
    Ok(())
}

mod bench;
