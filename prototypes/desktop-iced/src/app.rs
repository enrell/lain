//! The Elm-architecture core: state, messages, update, subscriptions.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use iced::keyboard::{self, Key, key::Named};
use iced::mouse::ScrollDelta;
use iced::widget::{Id, image, operation, scrollable};
use iced::{Animation, Element, Point, Size, Subscription, Task, event, time, window};

use crate::bench::{self, Bench, Phase};
use crate::data::{self, ArtKey, Catalog, Live, Paths};
use crate::home::{self, ArtMap, Drag, Geo, Rail, RailKind};
use crate::player::{self, Menu, Player, VideoEvt};
use crate::theme::{self, Theme};

pub const HOME_SCROLL: &str = "home";

pub enum Screen {
    Loading,
    Home,
    Player(Box<Player>),
}

#[derive(Debug, Clone)]
pub enum Message {
    Connected(Result<String, String>),
    CatalogLoaded(Result<Catalog, String>),
    ContinueLoaded(Result<Vec<lain_client::types::Progress>, String>),
    Art(ArtKey, Option<image::Handle>),
    ArtPinned(Result<image::Allocation, image::Error>),
    Live(Live),
    Frame(Instant),
    Resized(Size),
    ScaleFactor(f32),
    ThemeTick,
    Key(keyboard::Event),
    HomeScrolled(scrollable::Viewport),
    RailWheel(usize, ScrollDelta),
    RailPress(usize),
    RailMove(usize, Point),
    RailRelease(usize),
    RailExit(usize),
    PlayHero,
    Resume(usize, Option<f64>),
    Video(VideoEvt),
    TogglePause,
    Seek(f64),
    Volume(f64),
    ToggleMute,
    ToggleMenu(Menu),
    SetTrack(String, Option<i64>),
    Fullscreen,
    Back,
    PlayerActivity,
    PlayerTick,
    Saved(Result<(), String>),
    Screenshot(&'static str, window::Screenshot),
    Exit,
}

pub struct App {
    pub paths: Paths,
    pub theme: Theme,
    theme_mtime: Option<std::time::SystemTime>,
    pub theme_reloads: u32,
    pub now: Instant,
    pub window: Size,
    pub scale: f32,
    pub username: Option<String>,
    pub error: Option<String>,
    pub live: Option<String>,
    pub catalog: Catalog,
    pub progress: Vec<lain_client::types::Progress>,
    pub rails: Vec<Rail>,
    pub art: ArtMap,
    art_pending: HashSet<ArtKey>,
    pins: Vec<image::Allocation>,
    pub hovered: Option<(usize, usize)>,
    pub focus: Option<(usize, usize)>,
    lifts: Vec<((usize, usize), Animation<bool>)>,
    pub home_scroll_y: f32,
    pub screen: Screen,
    shift: bool,
    pub fullscreen: bool,
    next_session: u64,
    pub bench: Option<Bench>,
    shots: Option<(PathBuf, u32, Instant)>,
    continue_inflight: bool,
    continue_dirty: bool,
    pub home_ready: bool,
    placement: Option<crate::placement::Rect>,
}

pub struct Args {
    pub paths: Paths,
    pub bench: Option<PathBuf>,
    pub shots: Option<PathBuf>,
    pub window: Option<crate::placement::Rect>,
}

impl App {
    pub fn boot(args: Args) -> (App, Task<Message>) {
        let (theme, err) = match Theme::load(&args.paths.theme) {
            Ok(t) => (t, None),
            Err(e) => (Theme::default(), Some(format!("theme: {e} (using built-in lain-dark)"))),
        };
        let theme_mtime = theme::mtime(&args.paths.theme);
        let app = App {
            theme,
            theme_mtime,
            theme_reloads: 0,
            now: Instant::now(),
            window: Size::new(1600.0, 900.0),
            scale: 1.0,
            username: None,
            error: err,
            live: None,
            catalog: Catalog::default(),
            progress: Vec::new(),
            rails: Vec::new(),
            art: ArtMap::new(),
            art_pending: HashSet::new(),
            pins: Vec::new(),
            hovered: None,
            focus: None,
            lifts: Vec::new(),
            home_scroll_y: 0.0,
            screen: Screen::Loading,
            shift: false,
            fullscreen: false,
            next_session: 1,
            bench: args.bench.map(Bench::new),
            shots: args.shots.map(|d| (d, 0, Instant::now())),
            continue_inflight: true,
            continue_dirty: false,
            home_ready: false,
            paths: args.paths.clone(),
            placement: args.window,
        };
        let connect = Task::perform(data::connect(args.paths), Message::Connected);
        let place = match args.window {
            // Move while hidden, let the deferred DPI change (winit moves the
            // window toward the cursor on WM_DPICHANGED) land while still
            // hidden, then pin the rect again and show without activation.
            Some(r) => window::latest()
                .and_then(move |id| window::run(id, move |w| crate::placement::place(w, r, false)))
                .chain(Task::future(tokio::time::sleep(Duration::from_millis(400))))
                .chain(window::latest().and_then(move |id| window::run(id, move |w| crate::placement::place(w, r, true))))
                .discard(),
            None => Task::none(),
        };
        let scale = window::latest().and_then(window::scale_factor).map(Message::ScaleFactor);
        (app, Task::batch([connect, place.chain(scale)]))
    }

    pub fn title(&self) -> String {
        match &self.screen {
            Screen::Player(p) => format!("{} · {} — lain (iced)", p.item.title, p.item.label()),
            _ => "lain (iced prototype)".into(),
        }
    }

    pub fn hero_h(&self) -> f32 {
        (self.window.height * self.theme.hero_height).round()
    }

    pub fn art_scale(&self) -> f32 {
        self.scale.clamp(1.0, 2.0)
    }

    /// The featured series: the most recent Continue entry's, else the first.
    pub fn featured(&self) -> Option<(String, String)> {
        let idx = self.progress.iter().find_map(|p| self.catalog.by_id.get(&p.item_id).copied());
        let slug = match idx {
            Some(i) => self.catalog.items[i].slug.clone(),
            None => self.catalog.series.first()?.0.clone(),
        };
        let title = self.catalog.series.iter().find(|s| s.0 == slug).map(|s| s.1.clone()).unwrap_or_default();
        Some((slug, title))
    }

    /// What the hero's Play button opens, with its progress if resuming.
    pub fn hero_target(&self) -> Option<(usize, Option<(f64, f64)>)> {
        for p in &self.progress {
            if let Some(&i) = self.catalog.by_id.get(&p.item_id) {
                return Some((i, Some((p.position_sec, p.duration_sec))));
            }
        }
        let (slug, _) = self.featured()?;
        self.catalog.items.iter().position(|i| i.slug == slug).map(|i| (i, None))
    }

    pub fn status_line(&self) -> Option<String> {
        if let Some(e) = &self.error {
            return Some(e.clone());
        }
        self.live.clone()
    }

    /// 0..1 lift of a card (hover or keyboard focus), animated.
    pub fn lift_amount(&self, key: (usize, usize), now: Instant) -> f32 {
        self.lifts.iter().find(|(k, _)| *k == key).map(|(_, a)| a.interpolate(0.0, 1.0, now)).unwrap_or(0.0)
    }

    fn set_lift(&mut self, key: (usize, usize), on: bool) {
        let now = self.now;
        if let Some((_, a)) = self.lifts.iter_mut().find(|(k, _)| *k == key) {
            if a.value() != on {
                a.go_mut(on, now);
            }
        } else if on {
            let a = Animation::new(false).duration(self.theme.fast.max(Duration::from_millis(60))).easing(self.theme.easing);
            self.lifts.push((key, a.go(true, now)));
        }
    }

    fn refresh_lifts(&mut self) {
        let want: Vec<(usize, usize)> = self.hovered.iter().chain(self.focus.iter()).copied().collect();
        let keys: Vec<(usize, usize)> = self.lifts.iter().map(|(k, _)| *k).collect();
        for k in keys {
            if !want.contains(&k) {
                self.set_lift(k, false);
            }
        }
        for k in want {
            self.set_lift(k, true);
        }
        let now = self.now;
        self.lifts.retain(|(_, a)| a.value() || a.is_animating(now));
    }

    fn animating(&self) -> bool {
        let now = self.now;
        self.lifts.iter().any(|(_, a)| a.is_animating(now))
            || self.rails.iter().any(|r| r.offset.is_animating(now) || r.drag.is_some())
            || matches!(&self.screen, Screen::Player(p) if p.chrome.is_animating(now))
    }

    // ---- rails -------------------------------------------------------------------

    fn build_rails(&mut self) {
        let t = &self.theme;
        let mut rails = Vec::new();
        let cont: Vec<(usize, (f64, f64))> = self
            .progress
            .iter()
            .filter_map(|p| self.catalog.by_id.get(&p.item_id).map(|&i| (i, (p.position_sec, p.duration_sec))))
            .collect();
        if !cont.is_empty() {
            let mut r = Rail::new("Continue".into(), RailKind::Continue, cont.iter().map(|c| c.0).collect(), t);
            r.progress = cont.iter().map(|c| c.1).collect();
            rails.push(r);
        }
        let all: Vec<usize> = (0..self.catalog.items.len()).collect();
        rails.push(Rail::new("All episodes".into(), RailKind::Items, all, t));
        // One rail per 10 series.
        let series = &self.catalog.series;
        for (g, chunk) in series.chunks(10).enumerate() {
            let slugs: HashSet<&str> = chunk.iter().map(|s| s.0.as_str()).collect();
            let items: Vec<usize> =
                self.catalog.items.iter().enumerate().filter(|(_, it)| slugs.contains(it.slug.as_str())).map(|(i, _)| i).collect();
            if !items.is_empty() {
                let first = chunk.first().map(|s| s.1.as_str()).unwrap_or("");
                let last = chunk.last().map(|s| s.1.as_str()).unwrap_or("");
                let title = format!("Shelf {} · {} — {}", g + 1, first, last);
                rails.push(Rail::new(title, RailKind::Items, items, t));
            }
        }
        // Keep scroll offsets of rails that survive a rebuild (by title).
        for r in &mut rails {
            if let Some(old) = self.rails.iter().find(|o| o.title == r.title && o.items.len() == r.items.len()) {
                r.offset = old.offset.clone();
                r.target = old.target;
            }
        }
        self.rails = rails;
    }

    fn all_rail(&self) -> Option<usize> {
        self.rails.iter().position(|r| r.items.len() >= 500)
    }

    fn rail_scroll_to(&mut self, r: usize, target: f32, animate: bool) {
        let w = self.window.width;
        let t = self.theme.clone();
        let rail = &mut self.rails[r];
        let g = Geo::of(rail.kind, &t);
        let target = target.clamp(0.0, g.max_offset(rail.items.len(), w));
        rail.target = target;
        if animate {
            rail.offset.go_mut(target, self.now);
        } else {
            rail.offset = home::anim(target, &t);
        }
    }

    fn update_hover(&mut self, r: usize) {
        let rail = &self.rails[r];
        let g = Geo::of(rail.kind, &self.theme);
        let hit = rail.pointer.and_then(|(x, y)| g.hit(x, y, rail.value(self.now), rail.items.len()));
        let new = hit.map(|i| (r, i));
        if new != self.hovered && (new.is_some() || self.hovered.is_some_and(|(hr, _)| hr == r)) {
            self.hovered = new;
            self.refresh_lifts();
        }
    }

    /// Bring the focused card into view (horizontally and vertically).
    fn reveal_focus(&mut self) -> Task<Message> {
        let Some((r, i)) = self.focus else { return Task::none() };
        let t = self.theme.clone();
        let g = Geo::of(self.rails[r].kind, &t);
        let x = g.pad + i as f32 * g.stride - self.rails[r].target;
        let w = self.window.width;
        if x < g.pad {
            self.rail_scroll_to(r, i as f32 * g.stride, true);
        } else if x + g.w > w - g.pad {
            self.rail_scroll_to(r, i as f32 * g.stride + g.w + 2.0 * g.pad - w, true);
        }
        let tops = home::rail_tops(self);
        let top = tops[r];
        let bottom = top + g.block_h(&t);
        let view_top = self.home_scroll_y;
        let view_bottom = self.home_scroll_y + self.window.height;
        let y = if top < view_top + 64.0 {
            Some((top - 64.0).max(0.0))
        } else if bottom > view_bottom {
            Some(bottom - self.window.height + t.sp(3))
        } else {
            None
        };
        match y {
            Some(y) => operation::scroll_to(Id::new(HOME_SCROLL), scrollable::AbsoluteOffset { x: None, y: Some(y) }),
            None => Task::none(),
        }
    }

    // ---- art ---------------------------------------------------------------------

    fn wanted_art(&self) -> Vec<ArtKey> {
        let t = &self.theme;
        let s = self.art_scale();
        let mut keys = Vec::new();
        if let Some((slug, _)) = self.featured() {
            keys.push(home::hero_key(&slug, t.hero_blur));
        }
        let mut kinds: Vec<RailKind> = self.rails.iter().map(|r| r.kind).collect();
        kinds.dedup();
        for kind in [RailKind::Continue, RailKind::Items] {
            if !kinds.contains(&kind) {
                continue;
            }
            let g = Geo::of(kind, t);
            let slugs: Vec<&str> = match kind {
                RailKind::Continue => self
                    .rails
                    .iter()
                    .filter(|r| r.kind == kind)
                    .flat_map(|r| r.items.iter().map(|&i| self.catalog.items[i].slug.as_str()))
                    .collect(),
                RailKind::Items => self.catalog.series.iter().map(|s| s.0.as_str()).collect(),
            };
            for slug in slugs {
                let k = home::card_art_key(slug, g.w, g.h, s);
                if !keys.contains(&k) {
                    keys.push(k);
                }
            }
        }
        keys
    }

    fn request_art(&mut self) -> Task<Message> {
        let mut tasks = Vec::new();
        for key in self.wanted_art() {
            if self.art.contains_key(&key) || self.art_pending.contains(&key) {
                continue;
            }
            self.art_pending.insert(key.clone());
            let path = self.paths.art(&key.slug, key.kind);
            tasks.push(Task::perform(data::load_art(path, key), |(k, h)| Message::Art(k, h)));
        }
        Task::batch(tasks)
    }

    fn art_ready(&self) -> bool {
        let want = self.wanted_art();
        !want.is_empty() && want.iter().all(|k| self.art.contains_key(k))
    }

    // ---- player ------------------------------------------------------------------

    fn open_item(&mut self, idx: usize) -> Task<Message> {
        let Some(item) = self.catalog.items.get(idx) else { return Task::none() };
        let id = item.id.clone();
        Task::perform(data::progress(id), move |p| Message::Resume(idx, p.map(|p| if p.completed { 0.0 } else { p.position_sec })))
    }

    fn start_player(&mut self, idx: usize, resume: f64, pinned: bool) -> Task<Message> {
        let item = self.catalog.items[idx].clone();
        let media = self.paths.media_for(item.episode);
        let chrome = Animation::new(true).duration(self.theme.normal).easing(self.theme.easing);
        let mut p = Player::new(item, media.clone(), resume, chrome, self.now);
        p.pinned = pinned;
        let id = self.next_session;
        self.next_session += 1;
        let w = (self.window.width * self.scale).round() as u32;
        let h = (self.window.height * self.scale).round() as u32;
        if !media.exists() {
            p.error = Some(format!("missing media file {}", media.display()));
        } else {
            match player::open(id, &media, resume, w, h) {
                Ok(v) => p.video = Some(v),
                Err(e) => p.error = Some(e),
            }
        }
        self.screen = Screen::Player(Box::new(p));
        Task::none()
    }

    fn close_player(&mut self) -> Task<Message> {
        let screen = std::mem::replace(&mut self.screen, Screen::Home);
        let mut tasks = vec![];
        if let Screen::Player(p) = screen {
            let (pos, dur, id) = (p.position, p.duration, p.item.id.clone());
            if let Some(v) = p.video.clone() {
                player::close(v);
            }
            tasks.push(Task::perform(data::save_progress(id, pos, dur), Message::Saved));
        }
        if self.fullscreen {
            self.fullscreen = false;
            tasks.push(self.fullscreen_task());
        }
        Task::batch(tasks)
    }

    /// Fullscreen on an explicit monitor (placement.rs), never "current".
    fn fullscreen_task(&self) -> Task<Message> {
        let on = self.fullscreen;
        let anchor = self.placement.map(|r| (r.x + r.w / 2, r.y + r.h / 2));
        window::latest().and_then(move |id| window::run(id, move |w| crate::placement::set_fullscreen(w, on, anchor))).discard()
    }

    fn wake_chrome(&mut self) {
        let now = self.now;
        if let Screen::Player(p) = &mut self.screen {
            p.last_activity = now;
            if !p.chrome.value() {
                p.chrome.go_mut(true, now);
            }
        }
    }

    fn reload_continue(&mut self) -> Task<Message> {
        if self.continue_inflight {
            self.continue_dirty = true;
            return Task::none();
        }
        self.continue_inflight = true;
        Task::perform(data::continue_watching(), Message::ContinueLoaded)
    }

    // ---- update ------------------------------------------------------------------

    pub fn update(&mut self, msg: Message) -> Task<Message> {
        self.now = Instant::now();
        match msg {
            Message::Connected(Ok(user)) => {
                self.username = Some(user);
                Task::perform(data::catalog(self.paths.clone()), Message::CatalogLoaded)
            }
            Message::Connected(Err(e)) => {
                self.error = Some(format!("cannot reach lain-rs at {}: {e}", self.paths.server));
                self.screen = Screen::Home;
                Task::none()
            }
            Message::CatalogLoaded(Ok(c)) => {
                self.catalog = c;
                self.build_rails();
                self.screen = Screen::Home;
                Task::batch([self.request_art(), Task::perform(data::continue_watching(), Message::ContinueLoaded)])
            }
            Message::CatalogLoaded(Err(e)) => {
                self.error = Some(e);
                self.screen = Screen::Home;
                Task::none()
            }
            Message::ContinueLoaded(r) => {
                self.continue_inflight = false;
                match r {
                    Ok(p) => {
                        self.progress = p;
                        self.build_rails();
                    }
                    Err(e) => self.live = Some(e),
                }
                let again = if self.continue_dirty {
                    self.continue_dirty = false;
                    self.reload_continue()
                } else {
                    Task::none()
                };
                Task::batch([self.request_art(), again])
            }
            Message::Art(key, handle) => {
                self.art_pending.remove(&key);
                match handle {
                    Some(h) => {
                        self.art.insert(key, h.clone());
                        // Pin it in the GPU atlas so scrolling never re-uploads.
                        image::allocate(h).map(Message::ArtPinned)
                    }
                    None => {
                        self.live = Some(format!("art missing: {}", key.slug));
                        Task::none()
                    }
                }
            }
            Message::ArtPinned(r) => {
                if let Ok(a) = r {
                    self.pins.push(a);
                    if self.pins.len() > 400 {
                        self.pins.drain(..100);
                    }
                }
                Task::none()
            }
            Message::Live(l) => match l {
                Live::Connected => {
                    self.live = None;
                    self.reload_continue()
                }
                Live::ProgressChanged => self.reload_continue(),
                Live::Down(r) => {
                    self.live = Some(format!("events: {r}"));
                    Task::none()
                }
            },
            Message::Frame(at) => self.on_frame(at),
            Message::Resized(s) => {
                self.window = s;
                Task::none()
            }
            Message::ScaleFactor(s) => {
                let changed = (s - self.scale).abs() > 0.01;
                self.scale = s;
                if changed { self.request_art() } else { Task::none() }
            }
            Message::ThemeTick => {
                let m = theme::mtime(&self.paths.theme);
                if m.is_some() && m != self.theme_mtime {
                    self.theme_mtime = m;
                    match Theme::load(&self.paths.theme) {
                        Ok(t) => {
                            self.theme = t;
                            self.theme_reloads += 1;
                            self.error = None;
                            self.build_rails();
                            for i in 0..self.rails.len() {
                                let target = self.rails[i].target;
                                self.rail_scroll_to(i, target, false);
                            }
                            return self.request_art();
                        }
                        Err(e) => self.error = Some(format!("theme: {e}")),
                    }
                }
                Task::none()
            }
            Message::Key(e) => self.on_key(e),
            Message::HomeScrolled(v) => {
                self.home_scroll_y = v.absolute_offset().y;
                Task::none()
            }
            Message::RailWheel(r, d) => {
                let (dx, dy) = match d {
                    ScrollDelta::Lines { x, y } => (x * 120.0, y * 120.0),
                    ScrollDelta::Pixels { x, y } => (x, y),
                };
                let horizontal = self.shift || dx.abs() > dy.abs();
                if horizontal {
                    let d = if self.shift && dx == 0.0 { dy } else { dx };
                    let target = self.rails[r].target - d * 2.0;
                    self.rail_scroll_to(r, target, true);
                    Task::none()
                } else {
                    operation::scroll_by(Id::new(HOME_SCROLL), scrollable::AbsoluteOffset { x: 0.0, y: -dy })
                }
            }
            Message::RailPress(r) => {
                let off = self.rails[r].value(self.now);
                let x = self.rails[r].pointer.map(|p| p.0).unwrap_or(0.0);
                self.rails[r].drag = Some(Drag { start_x: x, start_off: off, last_x: x, last_t: self.now, vel: 0.0, moved: false });
                Task::none()
            }
            Message::RailMove(r, p) => {
                self.rails[r].pointer = Some((p.x, p.y));
                let now = self.now;
                let mut set = None;
                if let Some(d) = &mut self.rails[r].drag {
                    let dx = p.x - d.start_x;
                    if dx.abs() > 4.0 {
                        d.moved = true;
                    }
                    let dt = now.duration_since(d.last_t).as_secs_f32().max(0.001);
                    let v = (p.x - d.last_x) / dt;
                    d.vel = d.vel * 0.6 + v * 0.4;
                    d.last_x = p.x;
                    d.last_t = now;
                    if d.moved {
                        set = Some(d.start_off - dx);
                    }
                }
                if let Some(target) = set {
                    self.rail_scroll_to(r, target, false);
                }
                self.update_hover(r);
                Task::none()
            }
            Message::RailRelease(r) => {
                let drag = self.rails[r].drag.take();
                match drag {
                    Some(d) if d.moved => {
                        let target = self.rails[r].target - d.vel * 0.25;
                        self.rail_scroll_to(r, target, true);
                        Task::none()
                    }
                    _ => match self.hovered {
                        Some((hr, i)) if hr == r => {
                            let idx = self.rails[r].items[i];
                            self.focus = Some((r, i));
                            self.open_item(idx)
                        }
                        _ => Task::none(),
                    },
                }
            }
            Message::RailExit(r) => {
                self.rails[r].pointer = None;
                if let Some(d) = self.rails[r].drag.take() {
                    if d.moved {
                        let target = self.rails[r].target - d.vel * 0.25;
                        self.rail_scroll_to(r, target, true);
                    }
                }
                if self.hovered.is_some_and(|(hr, _)| hr == r) {
                    self.hovered = None;
                    self.refresh_lifts();
                }
                Task::none()
            }
            Message::PlayHero => match self.hero_target() {
                Some((i, _)) => self.open_item(i),
                None => Task::none(),
            },
                        Message::Resume(i, pos) => {
                let pinned = self.bench.is_some() || self.shots.is_some();
                self.start_player(i, pos.unwrap_or(0.0), pinned)
            }
            Message::Video(e) => {
                if let Screen::Player(p) = &mut self.screen {
                    match e {
                        VideoEvt::Frame => {
                            if let Some(v) = &p.video {
                                v.frame_pending.store(false, std::sync::atomic::Ordering::SeqCst);
                            }
                        }
                        VideoEvt::Wake => p.pump(),
                    }
                }
                Task::none()
            }
            Message::TogglePause => {
                self.wake_chrome();
                if let Screen::Player(p) = &self.screen {
                    if let Some(e) = p.engine() {
                        let _ = e.set_paused(!p.paused);
                    }
                }
                Task::none()
            }
            Message::Seek(s) => {
                self.wake_chrome();
                if let Screen::Player(p) = &mut self.screen {
                    if let Some(e) = p.engine() {
                        let _ = e.command("seek", &[&format!("{s:.3}"), "absolute+keyframes"]);
                    }
                    p.position = s;
                }
                Task::none()
            }
            Message::Volume(v) => {
                self.wake_chrome();
                if let Screen::Player(p) = &mut self.screen {
                    if let Some(e) = p.engine() {
                        let _ = e.set_volume(v);
                    }
                    p.volume = v;
                }
                Task::none()
            }
            Message::ToggleMute => {
                if let Screen::Player(p) = &self.screen {
                    if let Some(e) = p.engine() {
                        let _ = e.set_muted(!p.muted);
                    }
                }
                Task::none()
            }
            Message::ToggleMenu(m) => {
                self.wake_chrome();
                if let Screen::Player(p) = &mut self.screen {
                    p.menu = if p.menu == Some(m) { None } else { Some(m) };
                }
                Task::none()
            }
            Message::SetTrack(kind, id) => {
                if let Screen::Player(p) = &mut self.screen {
                    p.select_track(&kind, id);
                    p.menu = None;
                }
                Task::none()
            }
            Message::Fullscreen => {
                self.fullscreen = !self.fullscreen;
                self.fullscreen_task()
            }
            Message::Back => self.close_player(),
            Message::PlayerActivity => {
                self.wake_chrome();
                Task::none()
            }
            Message::PlayerTick => self.on_player_tick(),
            Message::Saved(r) => {
                if let Err(e) = r {
                    self.live = Some(format!("progress not saved: {e}"));
                }
                Task::none()
            }
            Message::Screenshot(name, shot) => {
                if let Some((dir, _, _)) = &self.shots {
                    let _ = std::fs::create_dir_all(dir);
                    let path = dir.join(format!("{name}.png"));
                    let (w, h) = (shot.size.width, shot.size.height);
                    if let Some(img) = ::image::RgbaImage::from_raw(w, h, shot.rgba.to_vec()) {
                        let _ = img.save(&path);
                    }
                }
                Task::none()
            }
            Message::Exit => iced::exit(),
        }
    }

    fn on_key(&mut self, e: keyboard::Event) -> Task<Message> {
        match e {
            keyboard::Event::ModifiersChanged(m) => {
                self.shift = m.shift();
                Task::none()
            }
            keyboard::Event::KeyPressed { key, modifiers, .. } => {
                self.shift = modifiers.shift();
                if matches!(self.screen, Screen::Player(_)) {
                    self.wake_chrome();
                    return match key.as_ref() {
                        Key::Named(Named::Space) => self.update(Message::TogglePause),
                        Key::Named(Named::ArrowLeft) | Key::Named(Named::ArrowRight) => {
                            let d = if key == Key::Named(Named::ArrowLeft) { -5.0 } else { 5.0 };
                            if let Screen::Player(p) = &mut self.screen {
                                if let Some(e) = p.engine() {
                                    let _ = e.seek_relative(d);
                                }
                                p.position = (p.position + d).max(0.0);
                            }
                            Task::none()
                        }
                        Key::Named(Named::ArrowUp) | Key::Named(Named::ArrowDown) => {
                            let d = if key == Key::Named(Named::ArrowUp) { 5.0 } else { -5.0 };
                            let v = match &self.screen {
                                Screen::Player(p) => (p.volume + d).clamp(0.0, 130.0),
                                _ => 0.0,
                            };
                            self.update(Message::Volume(v))
                        }
                        Key::Character("f") => self.update(Message::Fullscreen),
                        Key::Character("m") => self.update(Message::ToggleMute),
                        Key::Named(Named::Escape) => {
                            let menu_open = matches!(&self.screen, Screen::Player(p) if p.menu.is_some());
                            if menu_open {
                                if let Screen::Player(p) = &mut self.screen {
                                    p.menu = None;
                                }
                                Task::none()
                            } else if self.fullscreen {
                                self.update(Message::Fullscreen)
                            } else {
                                self.close_player()
                            }
                        }
                        Key::Named(Named::Backspace) => self.close_player(),
                        _ => Task::none(),
                    };
                }
                if !matches!(self.screen, Screen::Home) || self.rails.is_empty() {
                    return Task::none();
                }
                let (mut r, mut i) = self.focus.unwrap_or((usize::MAX, 0));
                let fresh = r == usize::MAX;
                match key.as_ref() {
                    Key::Named(Named::ArrowRight) | Key::Named(Named::ArrowLeft) | Key::Named(Named::ArrowUp) | Key::Named(Named::ArrowDown)
                        if fresh =>
                    {
                        r = 0;
                        let g = Geo::of(self.rails[0].kind, &self.theme);
                        i = ((self.rails[0].target / g.stride).ceil() as usize).min(self.rails[0].items.len() - 1);
                    }
                    Key::Named(Named::ArrowRight) => i = (i + 1).min(self.rails[r].items.len() - 1),
                    Key::Named(Named::ArrowLeft) => i = i.saturating_sub(1),
                    Key::Named(Named::ArrowUp) | Key::Named(Named::ArrowDown) => {
                        let nr = if key == Key::Named(Named::ArrowUp) { r.saturating_sub(1) } else { (r + 1).min(self.rails.len() - 1) };
                        if nr != r {
                            // Same screen x on the new rail.
                            let g0 = Geo::of(self.rails[r].kind, &self.theme);
                            let x = g0.pad + i as f32 * g0.stride - self.rails[r].target;
                            let g1 = Geo::of(self.rails[nr].kind, &self.theme);
                            let ni = ((x + self.rails[nr].target - g1.pad) / g1.stride).round().max(0.0) as usize;
                            r = nr;
                            i = ni.min(self.rails[nr].items.len() - 1);
                        }
                    }
                    Key::Named(Named::Enter) if !fresh => {
                        let idx = self.rails[r].items[i];
                        return self.open_item(idx);
                    }
                    Key::Named(Named::Escape) => {
                        self.focus = None;
                        self.refresh_lifts();
                        return Task::none();
                    }
                    _ => return Task::none(),
                }
                self.focus = Some((r, i));
                self.refresh_lifts();
                self.reveal_focus()
            }
            _ => Task::none(),
        }
    }

    fn on_player_tick(&mut self) -> Task<Message> {
        let now = self.now;
        let autohide = self.theme.autohide;
        let Screen::Player(p) = &mut self.screen else { return Task::none() };
        // Belt and braces: drain mpv events even if a wakeup got lost.
        p.pump();
        let idle = now.duration_since(p.last_activity) > autohide;
        if idle && !p.paused && p.menu.is_none() && !p.pinned && p.chrome.value() {
            p.chrome.go_mut(false, now);
        }
        if now.duration_since(p.last_saved) > Duration::from_secs(5) && !p.paused && p.duration > 0.0 {
            p.last_saved = now;
            return Task::perform(data::save_progress(p.item.id.clone(), p.position, p.duration), Message::Saved);
        }
        Task::none()
    }

    fn on_frame(&mut self, at: Instant) -> Task<Message> {
        // A rail moving under a still pointer changes what is hovered.
        for r in 0..self.rails.len() {
            if self.rails[r].pointer.is_some() && self.rails[r].offset.is_animating(self.now) {
                self.update_hover(r);
            }
        }
        self.refresh_lifts();
        // Cold start: the first frame after Home has its art.
        if !self.home_ready && matches!(self.screen, Screen::Home) && self.art_ready() {
            self.home_ready = true;
            if let Some(b) = &mut self.bench {
                b.cold_start_ms = bench::since_process_start_ms();
            }
            if let Some(s) = &mut self.shots {
                s.2 = at;
            }
        }
        let mut tasks = Vec::new();
        if let Some(t) = self.shots_step(at) {
            tasks.push(t);
        }
        if let Some(t) = self.bench_step(at) {
            tasks.push(t);
        }
        Task::batch(tasks)
    }

    /// `--shots <dir>`: Home, a hovered card, the 10-bit file with
    /// subtitles and controls, then exit.
    fn shots_step(&mut self, at: Instant) -> Option<Task<Message>> {
        let (_, step, since) = self.shots.as_ref()?;
        let (step, since) = (*step, *since);
        if !self.home_ready {
            return None;
        }
        let el = at.duration_since(since).as_secs_f32();
        let shot = |name: &'static str| window::latest().and_then(window::screenshot).map(move |s| Message::Screenshot(name, s));
        let next = |app: &mut App| {
            if let Some(s) = &mut app.shots {
                s.1 += 1;
                s.2 = at;
            }
        };
        match step {
            0 if el > 1.5 => {
                next(self);
                Some(shot("home"))
            }
            1 if el > 0.3 => {
                // Hover/focus the 3rd card of the first item rail.
                let r = self.rails.iter().position(|r| r.kind == RailKind::Items).unwrap_or(0);
                self.focus = Some((r, 2));
                self.refresh_lifts();
                next(self);
                Some(self.reveal_focus())
            }
            2 if el > 1.2 => {
                next(self);
                Some(shot("home-focus"))
            }
            3 => {
                let idx = self.catalog.items.iter().position(|i| i.episode % 2 == 1)?;
                self.focus = None;
                self.refresh_lifts();
                next(self);
                Some(Task::done(Message::Resume(idx, Some(20.0))))
            }
            4 if el > 7.0 => {
                next(self);
                Some(shot("player-10bit"))
            }
            5 if el > 0.5 => {
                let idx = self.catalog.items.iter().position(|i| i.episode % 2 == 0)?;
                next(self);
                Some(self.close_player().chain(Task::done(Message::Resume(idx, Some(5.0)))))
            }
            6 if el > 6.0 => {
                next(self);
                Some(shot("player-4k"))
            }
            7 if el > 1.0 => {
                next(self);
                Some(self.close_player())
            }
            // Back on Home: shows a theme reloaded while the player was up.
            8 if el > 1.5 => {
                next(self);
                Some(shot("home-end"))
            }
            9 if el > 0.5 => {
                next(self);
                Some(Task::done(Message::Exit))
            }
            _ => None,
        }
    }

    fn bench_step(&mut self, at: Instant) -> Option<Task<Message>> {
        self.bench.as_mut()?.frame(at);
        let phase = self.bench.as_ref()?.phase;
        match phase {
            Phase::Home if self.home_ready => {
                self.bench.as_mut()?.phase = Phase::Settle(at);
                None
            }
            Phase::Settle(t0) if at.duration_since(t0) > Duration::from_millis(1500) => {
                let b = self.bench.as_mut()?;
                b.rss_home = bench::rss_mb();
                let r = self.all_rail()?;
                let tops = home::rail_tops(self);
                let y = (tops[r] - 80.0).max(0.0);
                self.rail_scroll_to(r, 0.0, false);
                self.bench.as_mut()?.phase = Phase::Scroll(at);
                Some(operation::scroll_to(Id::new(HOME_SCROLL), scrollable::AbsoluteOffset { x: None, y: Some(y) }))
            }
            Phase::Scroll(t0) => {
                let r = self.all_rail()?;
                // Half a second for the vertical jump to land.
                let el = at.duration_since(t0).as_secs_f32() - 0.5;
                if el < 0.0 {
                    return None;
                }
                let speed = self.bench.as_ref()?.px_per_s;
                let g = Geo::of(self.rails[r].kind, &self.theme);
                let max = g.max_offset(self.rails[r].items.len(), self.window.width);
                let x = el * speed;
                self.rail_scroll_to(r, x, false);
                if x >= max {
                    let idx = self.catalog.items.iter().position(|i| i.episode % 2 == 0)?;
                    let b = self.bench.as_mut()?;
                    b.notes.push(format!("scroll: {:.0} px at {} px/s, {} cards", max, speed, self.rails[r].items.len()));
                    b.phase = Phase::Opening(0);
                    return Some(Task::done(Message::Resume(idx, Some(0.0))));
                }
                None
            }
            Phase::Opening(n) => {
                if let Screen::Player(p) = &self.screen {
                    if p.loaded || p.error.is_some() {
                        self.bench.as_mut()?.phase = Phase::Video(n, at);
                    }
                }
                None
            }
            Phase::Video(n, t0) => {
                let el = at.duration_since(t0).as_secs_f32();
                if n == 0 && el > 10.0 && self.bench.as_ref()?.rss_4k.is_none() {
                    self.bench.as_mut()?.rss_4k = bench::rss_mb();
                }
                if el < 20.0 {
                    return None;
                }
                let stats = match &self.screen {
                    Screen::Player(p) => mpv_stats(p),
                    _ => serde_json::Value::Null,
                };
                self.bench.as_mut()?.mpv.push(stats);
                let close = self.close_player();
                if n == 0 {
                    let idx = self.catalog.items.iter().position(|i| i.episode % 2 == 1)?;
                    self.bench.as_mut()?.phase = Phase::Opening(1);
                    Some(close.chain(Task::done(Message::Resume(idx, Some(0.0)))))
                } else {
                    let b = self.bench.as_mut()?;
                    b.phase = Phase::Done;
                    b.notes.push(format!("theme: {}", self.theme.name));
                    if let Err(e) = b.write() {
                        eprintln!("bench: {e}");
                    }
                    Some(close.chain(Task::done(Message::Exit)))
                }
            }
            _ => None,
        }
    }

    // ---- subscriptions -----------------------------------------------------------

    pub fn subscription(&self) -> Subscription<Message> {
        let mut subs = vec![
            keyboard::listen().map(Message::Key),
            window::resize_events().map(|(_, s)| Message::Resized(s)),
            time::every(Duration::from_millis(500)).map(|_| Message::ThemeTick),
        ];
        if data::CLIENT.get().is_some() {
            subs.push(Subscription::run(data::events).map(Message::Live));
        }
        let need_frames = self.animating() || self.bench.is_some() || (self.shots.is_some() && !matches!(self.screen, Screen::Loading));
        let need_frames = need_frames || (!self.home_ready && matches!(self.screen, Screen::Home));
        if need_frames {
            subs.push(window::frames().map(Message::Frame));
        }
        if let Screen::Player(p) = &self.screen {
            if let Some(v) = &p.video {
                subs.push(Subscription::run_with(v.id, player::video_events).map(Message::Video));
            }
            subs.push(time::every(Duration::from_millis(250)).map(|_| Message::PlayerTick));
            subs.push(event::listen_with(|e, _s, _w| match e {
                iced::Event::Mouse(iced::mouse::Event::CursorMoved { .. }) => Some(Message::PlayerActivity),
                _ => None,
            }));
        }
        Subscription::batch(subs)
    }

    pub fn view(&self) -> Element<'_, Message> {
        match &self.screen {
            Screen::Loading => crate::ui::loading(self),
            Screen::Home => home::view(self),
            Screen::Player(p) => crate::ui::player(self, p),
        }
    }
}

fn mpv_stats(p: &Player) -> serde_json::Value {
    let Some(e) = p.engine() else {
        return serde_json::json!({"file": p.media.file_name().map(|f| f.to_string_lossy().to_string()), "error": p.error});
    };
    let s = |k: &str| e.get_property::<String>(k).ok();
    let i = |k: &str| e.get_property::<i64>(k).ok();
    let f = |k: &str| e.get_property::<f64>(k).ok();
    let v = p.video.as_ref();
    serde_json::json!({
        "file": p.media.file_name().map(|f| f.to_string_lossy().to_string()),
        "vo": s("current-vo"),
        "hwdec-current": s("hwdec-current"),
        "frame-drop-count": i("frame-drop-count"),
        "vo-delayed-frame-count": i("vo-delayed-frame-count"),
        "estimated-vf-fps": f("estimated-vf-fps"),
        "container-fps": f("container-fps"),
        "video-codec": s("video-codec"),
        "frames_shown_by_ui": v.map(|v| v.frames_shown.load(std::sync::atomic::Ordering::Relaxed)),
        "import": v.map(|v| *v.import_mode.lock().unwrap_or_else(|e| e.into_inner())),
        "import_error": v.and_then(|v| v.import_error.lock().unwrap_or_else(|e| e.into_inner()).clone()),
        "error": p.error,
    })
}
