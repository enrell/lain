//! The app: one root view that owns every screen's state, routes keys,
//! runs actions and draws the shell (top navigation built from
//! `GET /api/modules`, the Ctrl+K palette, toasts, confirm dialogs).
//! Screens live in `screens/`; each is `impl LainApp` with its own state
//! struct here.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use gpui::prelude::*;
use gpui::{
    App, ClickEvent, Context, Entity, FocusHandle, FontWeight, KeyDownEvent, ScrollHandle, SharedString, Window, div,
    linear_color_stop, linear_gradient, px,
};
use lain_client::ClientError;
use lain_client::types::{CatalogItem, Library, ModulesView, Progress, User};

use crate::backend::{self, Backend, Net, Poke};
use crate::config::Config;
use crate::images::ImageStore;
use crate::layout::Layout;
use crate::screens::{connect, home, item, library, player, reader, search, settings};
use crate::theme::{self, Palette, Theme, ThemeEntry};
use crate::ui::{Field, Tone};
use crate::{Args, platform};

/// A value easing from `from` to `to` over `dur_ms` with the theme's curve.
#[derive(Debug, Clone, Copy)]
pub struct Anim {
    pub from: f32,
    pub to: f32,
    pub start: Instant,
    pub dur_ms: f32,
}

impl Anim {
    pub fn still(v: f32) -> Anim {
        Anim { from: v, to: v, start: Instant::now(), dur_ms: 0. }
    }
    fn t(&self) -> f32 {
        if self.dur_ms <= 0. { 1. } else { (self.start.elapsed().as_secs_f32() * 1000. / self.dur_ms).min(1.) }
    }
    pub fn value(&self, th: &Theme) -> f32 {
        self.from + (self.to - self.from) * th.ease(self.t())
    }
    pub fn running(&self) -> bool {
        self.t() < 1.
    }
    /// Retargets from the current value (keeps motion continuous).
    pub fn go(&mut self, to: f32, dur_ms: f32, th: &Theme) {
        let now = self.value(th);
        *self = Anim { from: now, to, start: Instant::now(), dur_ms };
    }
    pub fn set(&mut self, v: f32) {
        *self = Anim::still(v);
    }
}

/// A remote value.
#[derive(Debug, Clone)]
pub enum Load<T> {
    Idle,
    Loading,
    Ready(T),
    Failed(ClientError),
}

impl<T> Load<T> {
    pub fn ready(&self) -> Option<&T> {
        match self {
            Load::Ready(v) => Some(v),
            _ => None,
        }
    }
    pub fn is_loading(&self) -> bool {
        matches!(self, Load::Loading)
    }
    pub fn from(r: Result<T, ClientError>) -> Load<T> {
        match r {
            Ok(v) => Load::Ready(v),
            Err(e) => Load::Failed(e),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Account,
    Profile,
    Privacy,
    Appearance,
    Extensions,
    Playback,
    Users,
    Libraries,
    Plugins,
    Modules,
    Server,
    Backup,
}

impl Section {
    pub const YOU: &[Section] =
        &[Section::Account, Section::Profile, Section::Privacy, Section::Appearance, Section::Extensions, Section::Playback];
    pub const SERVER: &[Section] =
        &[Section::Users, Section::Libraries, Section::Plugins, Section::Modules, Section::Server, Section::Backup];
    pub fn label(self) -> &'static str {
        match self {
            Section::Account => "Account",
            Section::Profile => "Profile",
            Section::Privacy => "Private libraries",
            Section::Appearance => "Appearance",
            Section::Extensions => "Extensions",
            Section::Playback => "Playback & downloads",
            Section::Users => "Users",
            Section::Libraries => "Libraries",
            Section::Plugins => "Plugins",
            Section::Modules => "Modules",
            Section::Server => "Transcode & integrations",
            Section::Backup => "Backup",
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            Section::Account => "account",
            Section::Profile => "profile",
            Section::Privacy => "privacy",
            Section::Appearance => "appearance",
            Section::Extensions => "extensions",
            Section::Playback => "playback",
            Section::Users => "users",
            Section::Libraries => "libraries",
            Section::Plugins => "plugins",
            Section::Modules => "modules",
            Section::Server => "server",
            Section::Backup => "backup",
        }
    }
    pub fn admin(self) -> bool {
        Section::SERVER.contains(&self)
    }
    pub fn parse(s: &str) -> Option<Section> {
        Section::YOU.iter().chain(Section::SERVER).copied().find(|x| x.id() == s)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Screen {
    Home,
    /// Library grid: the watch libraries, or the reading ones; `lib` is
    /// a library id (empty: the first one).
    Library {
        read: bool,
        lib: String,
    },
    /// A title (series) page, by any of its items.
    Item(String),
    Search,
    Settings(Section),
    Downloads,
    Reader(String),
    /// Torrents, downloads and wanted titles (admins, acquire module).
    Acquire,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Connect,
    Main,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibSort {
    Title,
    Recent,
    Year,
}

/// Everything a button, field or command can do.
#[derive(Debug, Clone)]
pub enum Act {
    Nav(Screen),
    Back,
    Palette,
    PaletteRun(usize),
    Refresh,
    Focus(&'static str),
    // first run
    ConnectServer,
    Setup,
    Login,
    ChangeServer,
    Logout,
    OfflineMode,
    // items
    Play(String),
    PlayAt(String, f64),
    MarkWatched(String, bool),
    Download(String),
    RemoveDownload(String),
    PauseDownload(String),
    ResumeDownload(String),
    // library
    LibSort(LibSort),
    LibPage(i32),
    SearchKind(String),
    // account
    SaveProfile,
    SetMascot(String),
    UploadAvatar,
    ClearAvatar,
    SetLanguage,
    ChangePassword,
    SetPin,
    ClearPin,
    Unlock(String),
    Lock(String),
    // appearance and client
    PickTheme(String),
    InstallTheme,
    OpenThemesFolder,
    ToggleAutoplay,
    SaveOffline,
    SavePathMap,
    // extensions (tier 3)
    ExtAllow,
    ExtDeny,
    ExtReview(String),
    ExtToggle(String),
    ExtScope(String, String),
    ExtAskUninstall(String),
    ExtUninstall(String),
    InstallExtension,
    OpenExtensionsFolder,
    // admin
    CreateUser,
    UserDisabled(String, bool),
    UserRole(String, String),
    UserPassword(String),
    CreateLibrary,
    AskDeleteLibrary(String, String),
    DeleteLibrary(String),
    LibraryPrivate(String, bool),
    Scan,
    BrowseDir(String),
    Swap(String, Vec<String>, u64),
    Withdraw(String),
    Backup,
    SaveIntegrations,
    ConfirmYes,
    ConfirmNo,
    AcquireOp(&'static str, String, &'static str),
    LinkOp(String, &'static str),
    SaveTranscode,
    DiscardTranscode,
    ToggleTranscode(&'static str),
    CancelTranscode(String),
}

pub struct FocusItem {
    pub id: SharedString,
    pub act: Option<Act>,
    pub field: bool,
}

/// One title: the items of a library that share a title.
#[derive(Debug, Clone)]
pub struct Series {
    pub title: String,
    pub kind: String,
    pub library_id: String,
    /// Catalog indices in episode order.
    pub items: Vec<usize>,
    pub latest: i64,
    pub year: i64,
}

impl Series {
    pub fn reading(&self) -> bool {
        matches!(self.kind.as_str(), "comic" | "manga" | "book")
    }
}

pub struct Toast {
    pub text: String,
    pub bad: bool,
    pub at: Instant,
}

pub struct PaletteEntry {
    pub label: String,
    pub hint: String,
    pub act: Act,
}

pub struct PaletteState {
    pub sel: usize,
    pub entries: Vec<PaletteEntry>,
}

/// Font families actually used (the theme's, or a system fallback).
pub struct FontSet {
    pub ui: SharedString,
    pub display: SharedString,
    pub mono: SharedString,
    pub notes: Vec<String>,
}

pub struct LainApp {
    pub args: Args,
    pub dir: PathBuf,
    pub config: Config,
    // theme
    pub theme: Theme,
    pub pal: Palette,
    pub fonts: FontSet,
    pub theme_dir: PathBuf,
    theme_stamp: Option<SystemTime>,
    pub theme_gen: u64,
    pub theme_errors: Vec<String>,
    pub themes: Vec<ThemeEntry>,
    /// The active theme's layout slots (tier 2, `layout.rs`).
    pub layout: Layout,
    // extensions (tier 3, `ext/`)
    pub ext: crate::ext::Manager,
    /// The extension whose permission prompt is showing.
    pub ext_prompt: Option<String>,
    // plumbing
    pub net: Arc<Net>,
    pub backend: Option<Arc<Backend>>,
    pub images: Entity<ImageStore>,
    pub focus: FocusHandle,
    pub hwnd: isize,
    events_gen: u64,
    // session
    pub phase: Phase,
    pub connect: connect::ConnectState,
    pub me: Option<User>,
    pub online: bool,
    // data
    pub modules: Load<ModulesView>,
    pub libraries: Load<Vec<Library>>,
    pub items: Load<()>,
    pub catalog: Vec<CatalogItem>,
    pub by_id: HashMap<String, usize>,
    pub series: Vec<Series>,
    pub series_of: HashMap<String, usize>,
    pub continues: Vec<Progress>,
    pub progress: HashMap<String, Progress>,
    // navigation and input
    pub screen: Screen,
    pub history: Vec<Screen>,
    pub focus_order: Vec<FocusItem>,
    pub focused: Option<SharedString>,
    pub row_of: HashMap<SharedString, usize>,
    /// Where each focus stop was last painted (to scroll it into view).
    pub stop_bounds: std::rc::Rc<std::cell::RefCell<HashMap<SharedString, gpui::Bounds<gpui::Pixels>>>>,
    pub cur_row: usize,
    pub fields: HashMap<&'static str, Field>,
    pub palette: Option<PaletteState>,
    pub toasts: Vec<Toast>,
    pub confirm: Option<(String, Act)>,
    pub scroll: ScrollHandle,
    // screens
    pub home: home::HomeState,
    pub lib: library::LibState,
    pub item: item::ItemState,
    pub search: search::SearchState,
    pub settings: settings::SettingsState,
    pub reader: Option<reader::ReaderState>,
    pub dl: crate::screens::downloads::DlState,
    pub acq: crate::screens::acquire::AcquireState,
    pub links: Load<Vec<lain_client::types::Link>>,
    pub tc: crate::screens::transcode::TranscodeState,
    pub player: Option<player::PlayerSession>,
    pub orphans: Vec<player::PlayerSession>,
    pub next_session: u64,
    pub demo: crate::demo::Demo,
}

fn resolve_fonts(th: &Theme, cx: &App) -> FontSet {
    let names = cx.text_system().all_font_names();
    let mut notes = Vec::new();
    let (fb_ui, fb_display, fb_mono) =
        if cfg!(windows) { ("Segoe UI", "Georgia", "Cascadia Mono") } else { ("sans-serif", "serif", "monospace") };
    let mut pick = |want: &str, fallback: &str, role: &str| -> SharedString {
        if names.iter().any(|n| n == want) {
            SharedString::from(want.to_string())
        } else if let Some(n) = names.iter().find(|n| n.starts_with(want)) {
            // DirectWrite may group a family under a longer WWS name
            // (the bundled Fraunces shows up as "Fraunces NonWonky").
            notes.push(format!("{role} font {want:?} resolved to {n:?}"));
            SharedString::from(n.to_string())
        } else {
            notes.push(format!("{role} font {want:?} not found; using {fallback:?}"));
            SharedString::from(fallback.to_string())
        }
    };
    let ui = pick(&th.fonts.ui, fb_ui, "ui");
    let display = pick(&th.fonts.display, fb_display, "display");
    let mono = pick(&th.fonts.mono, fb_mono, "mono");
    FontSet { ui, display, mono, notes }
}

impl LainApp {
    pub fn new(args: Args, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let dir = args.client_dir.clone();
        let mut config = Config::load(&dir);
        if let Some(s) = &args.server {
            config.server = s.clone();
        }
        let tdir = theme::themes_dir(&dir);
        theme::install_builtins(&tdir);
        let theme_dir = match &args.theme {
            Some(p) => p.clone(),
            None => tdir.join(&config.theme),
        };
        let (theme, terr) = match theme::load_dir(&theme_dir) {
            Ok(t) => (t, None),
            Err(e) => (Theme::default(), Some(e)),
        };
        let (pal, mut theme_errors) = theme.palette();
        theme_errors.extend(terr);
        let (layout, layout_errors) = Layout::load(&theme_dir);
        theme_errors.extend(layout_errors);
        let (ext, mut ext_rx) = crate::ext::Manager::new(crate::ext::manifest::extensions_dir(&dir));
        theme::set_icon_overrides(&theme_dir);
        let fonts = resolve_fonts(&theme, cx);
        for n in fonts.notes.iter().chain(theme_errors.iter()) {
            eprintln!("theme: {n}");
        }
        let net = Net::new(&dir).expect("network runtime");
        if let Some(e) = &net.db_error {
            eprintln!("client db: {e} (progress queue and offline copies are off)");
        }
        let images = cx.new(|_| ImageStore::default());
        cx.observe(&images, |_, _, cx| cx.notify()).detach();
        let focus = cx.focus_handle();
        window.focus(&focus);

        let mut app = LainApp {
            layout,
            ext,
            ext_prompt: None,
            themes: theme::list(&tdir),
            theme_stamp: theme::stamp(&theme_dir),
            theme_dir,
            theme_gen: 0,
            theme_errors,
            theme,
            pal,
            fonts,
            config,
            dir,
            net,
            backend: None,
            images,
            focus,
            hwnd: platform::window_handle(window).unwrap_or(0),
            events_gen: 0,
            phase: Phase::Connect,
            connect: connect::ConnectState::default(),
            me: None,
            online: false,
            modules: Load::Idle,
            libraries: Load::Idle,
            items: Load::Idle,
            catalog: Vec::new(),
            by_id: HashMap::new(),
            series: Vec::new(),
            series_of: HashMap::new(),
            continues: Vec::new(),
            progress: HashMap::new(),
            screen: Screen::Home,
            history: Vec::new(),
            focus_order: Vec::new(),
            focused: None,
            row_of: HashMap::new(),
            stop_bounds: Default::default(),
            cur_row: 0,
            fields: HashMap::new(),
            palette: None,
            toasts: Vec::new(),
            confirm: None,
            scroll: ScrollHandle::new(),
            home: home::HomeState::default(),
            lib: library::LibState::default(),
            item: item::ItemState::default(),
            search: search::SearchState::default(),
            settings: settings::SettingsState::default(),
            reader: None,
            dl: Default::default(),
            acq: Default::default(),
            links: Load::Idle,
            tc: Default::default(),
            player: None,
            orphans: Vec::new(),
            next_session: 1,
            demo: crate::demo::Demo::new(args.demo.clone(), args.shot.clone()),
            args,
        };
        app.start(cx);

        // Extension output (rails, panels, failures) onto the UI thread.
        cx.spawn(async move |this, cx| {
            while let Some(first) = ext_rx.recv().await {
                let mut batch = vec![first];
                while let Ok(o) = ext_rx.try_recv() {
                    batch.push(o);
                }
                let alive = this.update(cx, |app, cx| {
                    for o in batch {
                        app.on_ext(o, cx);
                    }
                    cx.notify();
                });
                if alive.is_err() {
                    return;
                }
            }
        })
        .detach();

        // The UI tick: player state, progress saves, theme hot reload, toasts.
        cx.spawn(async move |this, cx| {
            let mut n: u64 = 0;
            loop {
                cx.background_executor().timer(Duration::from_millis(33)).await;
                n += 1;
                if this.update(cx, |app, cx| app.tick(n, cx)).is_err() {
                    return;
                }
            }
        })
        .detach();
        if app.demo.enabled() {
            cx.spawn_in(window, async move |this, cx| crate::demo::run(this, cx).await).detach();
        }
        // Save the playback position when the window closes mid-video.
        let me = cx.entity().downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            let _ = me.update(cx, |app, _| app.save_now());
            true
        });
        app
    }

    // ---- session ---------------------------------------------------------

    /// Opens the configured server: a saved session goes straight to the
    /// app, otherwise the connect screen asks.
    pub fn start(&mut self, cx: &mut Context<Self>) {
        let server = self.config.server.clone();
        if !server.is_empty() {
            self.set_text("connect.url", &server);
        } else {
            self.set_text("connect.url", "http://127.0.0.1:9360");
        }
        self.focused = Some("connect.url".into());
        if server.is_empty() {
            return;
        }
        self.run_connect(cx);
    }

    fn offline_dir(&self) -> PathBuf {
        if self.config.offline_dir.is_empty() {
            self.dir.join("offline")
        } else {
            PathBuf::from(&self.config.offline_dir)
        }
    }

    /// Makes the backend for the URL in the connect field and probes it.
    pub fn run_connect(&mut self, cx: &mut Context<Self>) {
        let url = self.text("connect.url").trim().to_string();
        let b = match Backend::new(self.net.clone(), &url, &self.offline_dir(), self.config.offline_max_gib) {
            Ok(b) => b,
            Err(e) => {
                self.connect.error = Some(backend::reason(&e));
                return;
            }
        };
        self.set_backend(b.clone(), cx);
        self.connect.busy = true;
        self.connect.error = None;
        let c = b.client.clone();
        self.spawn_ui(
            cx,
            async move {
                let health = c.health().await?;
                let setup = c.setup_status().await?;
                let me = if setup.setup_required || c.session().is_none() { None } else { c.me().await.ok() };
                Ok::<_, ClientError>((health, setup, me))
            },
            |app, r, cx| {
                app.connect.busy = false;
                match r {
                    Ok((h, setup, me)) => {
                        app.connect.server_version = h.version;
                        app.config.server = app.backend.as_ref().map(|b| b.client.server()).unwrap_or_default();
                        app.config.save(&app.dir);
                        if setup.setup_required {
                            app.connect.step = connect::Step::Setup;
                            app.focused = Some("setup.user".into());
                        } else if let Some(me) = me {
                            app.enter_main(me, cx);
                        } else {
                            app.connect.step = connect::Step::Login;
                            if let Some(s) = app.backend.as_ref().and_then(|b| b.client.session()) {
                                app.set_text("login.user", &s.username);
                                app.focused = Some("login.pass".into());
                            } else {
                                app.focused = Some("login.user".into());
                            }
                        }
                    }
                    Err(e) => {
                        app.connect.step = connect::Step::Server;
                        app.connect.error = Some(backend::reason(&e));
                    }
                }
            },
        );
    }

    fn set_backend(&mut self, b: Arc<Backend>, cx: &mut Context<Self>) {
        self.backend = Some(b.clone());
        self.images.update(cx, |s, _| s.reset(Some(b)));
    }

    pub fn enter_main(&mut self, me: User, cx: &mut Context<Self>) {
        self.me = Some(me);
        self.phase = Phase::Main;
        self.online = true;
        self.screen = Screen::Home;
        self.history.clear();
        self.focused = None;
        self.start_events(cx);
        self.reload_all(cx);
        self.ext_connect(cx);
    }

    pub fn is_admin(&self) -> bool {
        self.me.as_ref().map(|u| u.role == "admin").unwrap_or(false)
    }

    pub fn logout(&mut self, cx: &mut Context<Self>) {
        if let Some(b) = &self.backend {
            let _ = b.client.logout();
        }
        self.close_player(cx);
        self.ext.set_api(None);
        self.ext_prompt = None;
        self.me = None;
        self.phase = Phase::Connect;
        self.connect = connect::ConnectState { step: connect::Step::Login, ..Default::default() };
        self.events_gen += 1;
        self.catalog.clear();
        self.series.clear();
        self.continues.clear();
        self.focused = Some("login.user".into());
    }

    /// Modules, libraries, the catalog and Continue.
    pub fn reload_all(&mut self, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        let c = b.client.clone();
        self.modules = Load::Loading;
        self.spawn_ui(cx, async move { c.modules().await }, |app, r, _| app.modules = Load::from(r));
        self.reload_libraries(cx);
        self.reload_catalog(cx);
        self.refresh_continues(cx);
    }

    pub fn reload_libraries(&mut self, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        let c = b.client.clone();
        if !matches!(self.libraries, Load::Ready(_)) {
            self.libraries = Load::Loading;
        }
        self.spawn_ui(cx, async move { c.libraries().await }, |app, r, _| {
            app.libraries = Load::from(r);
            app.home.dirty = true;
        });
    }

    pub fn reload_catalog(&mut self, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        let c = b.client.clone();
        if !matches!(self.items, Load::Ready(_)) {
            self.items = Load::Loading;
        }
        self.spawn_ui(cx, backend::all_items(c), |app, r, _| match r {
            Ok(items) => {
                app.set_catalog(items);
                app.items = Load::Ready(());
            }
            Err(e) => {
                if e.is_offline() {
                    app.online = false;
                }
                app.items = Load::Failed(e);
            }
        });
    }

    fn set_catalog(&mut self, items: Vec<CatalogItem>) {
        self.catalog = items;
        self.by_id = self.catalog.iter().enumerate().map(|(i, it)| (it.id.clone(), i)).collect();
        let mut groups: HashMap<String, usize> = HashMap::new();
        let mut series: Vec<Series> = Vec::new();
        for (i, it) in self.catalog.iter().enumerate() {
            // A missing item's file is gone (D-068): keep it off the shelves.
            if it.missing {
                continue;
            }
            let key = format!("{}\u{0}{}", it.library_id, it.title.to_lowercase());
            let idx = *groups.entry(key).or_insert_with(|| {
                series.push(Series {
                    title: it.title.clone(),
                    kind: it.kind.clone(),
                    library_id: it.library_id.clone(),
                    items: Vec::new(),
                    latest: 0,
                    year: it.year,
                });
                series.len() - 1
            });
            let s = &mut series[idx];
            s.items.push(i);
            s.latest = s.latest.max(it.updated_at);
            if s.year == 0 {
                s.year = it.year;
            }
        }
        for s in &mut series {
            let cat = &self.catalog;
            s.items.sort_by(|&a, &b| {
                let (x, y) = (&cat[a], &cat[b]);
                (x.season, x.episode, &x.file_path).cmp(&(y.season, y.episode, &y.file_path))
            });
        }
        series.sort_by_key(|a| a.title.to_lowercase());
        self.series_of.clear();
        for (si, s) in series.iter().enumerate() {
            for &i in &s.items {
                self.series_of.insert(self.catalog[i].id.clone(), si);
            }
        }
        self.series = series;
        // Rebuild now: rails hold indices into the old catalog.
        self.build_home();
    }

    pub fn refresh_continues(&mut self, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        let c = b.client.clone();
        self.spawn_ui(cx, async move { c.continue_watching().await }, |app, r, _| match r {
            Ok(list) => {
                for p in &list {
                    app.progress.insert(p.item_id.clone(), p.clone());
                }
                app.continues = list.into_iter().filter(|p| !p.completed).collect();
                app.continues.sort_by_key(|p| std::cmp::Reverse(p.updated_at));
                app.home.dirty = true;
            }
            Err(e) => eprintln!("continue: {e}"),
        });
    }

    fn start_events(&mut self, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        self.events_gen += 1;
        let gen_now = self.events_gen;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let task = b.spawn(backend::watch_events(b.clone(), tx));
        cx.spawn(async move |this, cx| {
            while let Some(first) = rx.recv().await {
                // Coalesce bursts.
                cx.background_executor().timer(Duration::from_millis(250)).await;
                let mut pokes = vec![first];
                while let Ok(p) = rx.try_recv() {
                    pokes.push(p);
                }
                let alive = this.update(cx, |app, cx| {
                    if app.events_gen != gen_now {
                        return false;
                    }
                    app.on_pokes(pokes, cx);
                    cx.notify();
                    true
                });
                if !matches!(alive, Ok(true)) {
                    task.abort();
                    return;
                }
            }
        })
        .detach();
    }

    fn on_pokes(&mut self, pokes: Vec<Poke>, cx: &mut Context<Self>) {
        let (mut prog, mut lib, mut mods, mut plugins) = (false, false, false, false);
        for p in pokes {
            match p {
                Poke::Connected => {
                    if !self.online {
                        self.toast("Back online", false);
                        self.online = true;
                        lib = true;
                        mods = true;
                    }
                    prog = true;
                }
                Poke::Lost => {
                    if self.online {
                        self.online = false;
                        self.toast("Connection to the server lost; retrying", true);
                    }
                }
                Poke::AuthRequired => {
                    self.toast("Your session ended. Log in again.", true);
                    self.logout(cx);
                    return;
                }
                Poke::Progress(id) => {
                    self.ext_event(crate::ext::Event::ProgressChanged(id.clone()));
                    prog = true;
                    if self.item.has(&id) {
                        self.item.dirty = true;
                    }
                }
                Poke::Library => {
                    self.ext_event(crate::ext::Event::LibraryChanged);
                    lib = true;
                }
                Poke::Scan(s) => {
                    let done = s.state != "running";
                    let was_running = matches!(&self.settings.scan, Load::Ready(x) if x.state == "running");
                    self.settings.scan_live = Some((s.libraries_done, s.libraries));
                    self.settings.scan = Load::Ready(lain_client::types::ScanStatus {
                        state: s.state.clone(),
                        stats: s.stats.clone(),
                        error: s.error.clone(),
                        trigger: s.trigger.clone(),
                        ..Default::default()
                    });
                    if done && was_running {
                        self.toast(
                            if s.error.is_empty() {
                                "Scan finished".to_string()
                            } else {
                                format!("Scan failed: {}", s.error)
                            },
                            !s.error.is_empty(),
                        );
                        lib = true;
                    }
                }
                Poke::Modules => mods = true,
                Poke::Plugins => plugins = true,
                Poke::Transcodes => {
                    if matches!(self.screen, Screen::Settings(Section::Server)) && self.is_admin() {
                        self.load_transcode(cx);
                    }
                }
            }
        }
        if prog {
            self.refresh_continues(cx);
        }
        if lib {
            self.reload_libraries(cx);
            self.reload_catalog(cx);
            self.settings.scan_dirty = true;
        }
        if mods {
            let Some(b) = self.backend.clone() else { return };
            let c = b.client.clone();
            self.spawn_ui(cx, async move { c.modules().await }, |app, r, cx| {
                app.modules = Load::from(r);
                app.images.update(cx, |s, _| s.thumbs_off = false);
            });
        }
        if plugins {
            self.settings.plugins_dirty = true;
        }
    }

    // ---- async helper ------------------------------------------------------

    /// Runs `fut` on the network runtime and `done` on the UI thread.
    pub fn spawn_ui<T: Send + 'static>(
        &mut self,
        cx: &mut Context<Self>,
        fut: impl std::future::Future<Output = T> + Send + 'static,
        done: impl FnOnce(&mut LainApp, T, &mut Context<LainApp>) + 'static,
    ) {
        let task = self.net.rt.spawn(fut);
        cx.spawn(async move |this, cx| {
            if let Ok(v) = task.await {
                let _ = this.update(cx, |app, cx| {
                    done(app, v, cx);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub fn toast(&mut self, text: impl Into<String>, bad: bool) {
        let text = text.into();
        eprintln!("toast: {text}");
        self.toasts.push(Toast { text, bad, at: Instant::now() });
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
    }

    /// Toasts a call's outcome.
    pub fn report<T>(&mut self, r: &Result<T, ClientError>, ok: &str) -> bool {
        match r {
            Ok(_) => {
                if !ok.is_empty() {
                    self.toast(ok, false);
                }
                true
            }
            Err(e) => {
                self.toast(backend::reason(e), true);
                false
            }
        }
    }

    // ---- modules and navigation ------------------------------------------------

    /// Whether module `id` is on (unknown modules view: assume on).
    pub fn module_on(&self, id: &str) -> bool {
        match &self.modules {
            Load::Ready(m) => {
                m.modules.iter().find(|x| x.id == id).map(|x| x.enabled && x.state != "disabled").unwrap_or(false)
            }
            _ => true,
        }
    }

    /// The top navigation: destinations of enabled modules this app has
    /// screens for, plus Downloads (client-side).
    pub fn nav(&self) -> Vec<(&'static str, &'static str, &'static str, Screen)> {
        let mut ids: Vec<String> = match &self.modules {
            Load::Ready(m) => m
                .modules
                .iter()
                .filter(|x| x.enabled && x.state != "disabled")
                .flat_map(|x| x.ui.destinations.iter().map(|d| d.id.clone()))
                .collect(),
            _ => vec!["home".into(), "library".into(), "read".into(), "search".into(), "settings".into()],
        };
        ids.push("downloads".into());
        let mut out = Vec::new();
        let table: &[(&str, &str, &str, Screen)] = &[
            ("home", "Home", "home", Screen::Home),
            ("library", "Library", "library", Screen::Library { read: false, lib: String::new() }),
            ("read", "Read", "book", Screen::Library { read: true, lib: String::new() }),
            ("search", "Search", "search", Screen::Search),
            ("acquire", "Acquire", "plus", Screen::Acquire),
            ("downloads", "Downloads", "download", Screen::Downloads),
            ("settings", "Settings", "settings", Screen::Settings(Section::Account)),
        ];
        for (id, label, icon, screen) in table {
            // Acquire is an admin surface (D-116).
            if *id == "acquire" && !self.is_admin() {
                continue;
            }
            if ids.iter().any(|x| x == id) {
                out.push((*id, *label, *icon, screen.clone()));
            }
        }
        out
    }

    pub fn navigate(&mut self, s: Screen, cx: &mut Context<Self>) {
        if self.screen != s {
            let prev = std::mem::replace(&mut self.screen, s.clone());
            self.history.push(prev);
            if self.history.len() > 50 {
                self.history.remove(0);
            }
        }
        self.focused = None;
        self.scroll = ScrollHandle::new();
        self.on_enter(cx);
    }

    pub fn back(&mut self, cx: &mut Context<Self>) {
        if let Some(prev) = self.history.pop() {
            self.screen = prev;
            self.focused = None;
            self.scroll = ScrollHandle::new();
            self.on_enter(cx);
        }
    }

    /// Loads what the current screen needs.
    fn on_enter(&mut self, cx: &mut Context<Self>) {
        match self.screen.clone() {
            Screen::Item(id) => self.load_item(&id, cx),
            Screen::Settings(sec) => self.enter_settings(sec, cx),
            Screen::Search => self.focused = Some("search.q".into()),
            Screen::Reader(id) => self.open_reader(&id, cx),
            Screen::Library { .. } => self.lib.page = 0,
            Screen::Acquire => self.load_acquire(cx),
            Screen::Home | Screen::Downloads => {}
        }
    }

    // ---- actions ---------------------------------------------------------------

    pub fn run(&mut self, act: Act, window: &mut Window, cx: &mut Context<Self>) {
        match act {
            Act::Nav(s) => self.navigate(s, cx),
            Act::Back => self.back(cx),
            Act::Palette => self.toggle_palette(),
            Act::PaletteRun(i) => {
                if let Some(p) = self.palette.take()
                    && let Some(e) = p.entries.into_iter().nth(i)
                {
                    self.run(e.act, window, cx);
                }
            }
            Act::Refresh => {
                self.reload_all(cx);
                self.ext_refresh();
                self.on_enter(cx);
                self.toast("Refreshed", false);
            }
            Act::Focus(id) => self.focused = Some(id.into()),
            Act::ConnectServer => self.run_connect(cx),
            Act::Setup => self.run_setup(cx),
            Act::Login => self.run_login(cx),
            Act::ChangeServer => {
                self.phase = Phase::Connect;
                self.connect.step = connect::Step::Server;
                self.focused = Some("connect.url".into());
            }
            Act::Logout => self.logout(cx),
            Act::OfflineMode => {
                self.phase = Phase::Main;
                self.online = false;
                self.screen = Screen::Downloads;
            }
            Act::Play(id) => self.open_player(&id, None, cx),
            Act::PlayAt(id, t) => self.open_player(&id, Some(t), cx),
            Act::MarkWatched(id, w) => self.mark_watched(&id, w, cx),
            Act::Download(id) => self.download(&id, cx),
            Act::RemoveDownload(id) => self.offline_op(&id, "remove", cx),
            Act::PauseDownload(id) => self.offline_op(&id, "pause", cx),
            Act::ResumeDownload(id) => self.download(&id, cx),
            Act::LibSort(s) => {
                self.lib.sort = s;
                self.lib.page = 0;
            }
            Act::LibPage(d) => self.lib_page(d),
            Act::SearchKind(k) => {
                self.search.kind = if self.search.kind == k { String::new() } else { k };
                self.run_search(cx);
            }
            Act::ConfirmYes => {
                if let Some((_, a)) = self.confirm.take() {
                    self.run(a, window, cx);
                }
            }
            Act::ConfirmNo => self.confirm = None,
            Act::AcquireOp(k, id, a) => self.acquire_op(k, id, a, cx),
            Act::LinkOp(p, op) => self.link_op(p, op, cx),
            Act::SaveTranscode => self.save_transcode(cx),
            Act::DiscardTranscode => self.load_transcode(cx),
            Act::ToggleTranscode(k) => {
                if k == "tone" {
                    self.tc.tone_mapping = !self.tc.tone_mapping;
                } else {
                    self.tc.throttle = !self.tc.throttle;
                }
            }
            Act::CancelTranscode(s) => self.cancel_session(s, cx),
            Act::ExtAllow
            | Act::ExtDeny
            | Act::ExtReview(_)
            | Act::ExtToggle(_)
            | Act::ExtScope(..)
            | Act::ExtAskUninstall(_)
            | Act::ExtUninstall(_)
            | Act::InstallExtension
            | Act::OpenExtensionsFolder => self.run_ext(act, cx),
            other => self.run_settings(other, window, cx),
        }
        cx.notify();
    }

    /// Asks before a destructive action.
    pub fn ask(&mut self, text: impl Into<String>, act: Act) {
        self.confirm = Some((text.into(), act));
    }

    fn mark_watched(&mut self, id: &str, watched: bool, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        let dur = self.progress.get(id).map(|p| p.duration_sec).unwrap_or(0.);
        let pos = if watched { dur } else { 0. };
        let id2 = id.to_string();
        self.spawn_ui(cx, backend::save_progress(b, id.to_string(), pos, dur, watched), move |app, r, cx| {
            if app.report(&r, if watched { "Marked watched" } else { "Marked unwatched" }) {
                let p = app.progress.entry(id2.clone()).or_default();
                p.item_id = id2;
                p.completed = watched;
                p.position_sec = pos;
                app.refresh_continues(cx);
                app.item.dirty = true;
            }
        });
    }

    // ---- palette -----------------------------------------------------------------

    fn toggle_palette(&mut self) {
        if self.palette.is_some() {
            self.palette = None;
            return;
        }
        self.set_text("palette.q", "");
        self.palette = Some(PaletteState { sel: 0, entries: self.palette_entries("") });
        self.focused = Some("palette.q".into());
    }

    pub fn palette_entries(&self, q: &str) -> Vec<PaletteEntry> {
        let q = q.trim().to_lowercase();
        let mut all: Vec<PaletteEntry> = Vec::new();
        for (_, label, _, s) in self.nav() {
            all.push(PaletteEntry { label: format!("Go to {label}"), hint: "navigation".into(), act: Act::Nav(s) });
        }
        if let Some(libs) = self.libraries.ready() {
            for l in libs {
                let read = matches!(l.kind.as_str(), "manga" | "comic");
                all.push(PaletteEntry {
                    label: format!("Open library {}", l.name),
                    hint: l.kind.clone(),
                    act: Act::Nav(Screen::Library { read, lib: l.id.clone() }),
                });
            }
        }
        for &s in Section::YOU.iter().chain(Section::SERVER.iter().filter(|_| self.is_admin())) {
            all.push(PaletteEntry {
                label: format!("Settings: {}", s.label()),
                hint: if s.admin() { "server".into() } else { "you".into() },
                act: Act::Nav(Screen::Settings(s)),
            });
        }
        for t in &self.themes {
            all.push(PaletteEntry {
                label: format!("Theme: {}", t.name),
                hint: "appearance".into(),
                act: Act::PickTheme(t.id.clone()),
            });
        }
        if self.is_admin() {
            all.push(PaletteEntry { label: "Scan libraries".into(), hint: "library.scan".into(), act: Act::Scan });
        }
        if let Some(c) = self.continues.first() {
            all.push(PaletteEntry {
                label: "Resume last watched".into(),
                hint: "watch.resume".into(),
                act: Act::Play(c.item_id.clone()),
            });
        }
        all.push(PaletteEntry { label: "Refresh".into(), hint: "F5".into(), act: Act::Refresh });
        all.push(PaletteEntry { label: "Log out".into(), hint: "account".into(), act: Act::Logout });
        let mut out: Vec<PaletteEntry> =
            if q.is_empty() { all } else { all.into_iter().filter(|e| fuzzy(&e.label.to_lowercase(), &q)).collect() };
        if q.len() >= 2 {
            for s in self.series.iter().filter(|s| s.title.to_lowercase().contains(&q)).take(12) {
                if let Some(&first) = s.items.first() {
                    out.push(PaletteEntry {
                        label: s.title.clone(),
                        hint: format!("{} · {}", s.kind, s.items.len()),
                        act: Act::Nav(Screen::Item(self.catalog[first].id.clone())),
                    });
                }
            }
        }
        out.truncate(40);
        out
    }

    // ---- ticking -----------------------------------------------------------------

    fn tick(&mut self, n: u64, cx: &mut Context<Self>) {
        if n.is_multiple_of(15) {
            self.check_theme(cx);
        }
        platform::tick_placement(self, n);
        if self.player.is_some() {
            self.player_tick(cx);
        }
        if n.is_multiple_of(30) {
            self.settings_tick(cx);
            self.downloads_tick(cx);
            if self.screen == Screen::Acquire && n.is_multiple_of(60) {
                self.load_acquire(cx);
            }
            if self.item.dirty
                && let Screen::Item(id) = self.screen.clone()
            {
                self.item.dirty = false;
                self.load_item_progress(&id, cx);
            }
        }
        let before = self.toasts.len();
        self.toasts.retain(|t| t.at.elapsed() < Duration::from_secs(4));
        if before != self.toasts.len() {
            cx.notify();
        }
    }

    fn check_theme(&mut self, cx: &mut Context<Self>) {
        let m = theme::stamp(&self.theme_dir);
        if m.is_some() && m != self.theme_stamp {
            self.theme_stamp = m;
            self.apply_theme_dir(self.theme_dir.clone(), cx);
            eprintln!("theme reloaded ({})", self.theme.name);
        }
    }

    pub fn apply_theme_dir(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        match theme::load_dir(&dir) {
            Ok(t) => {
                let (pal, mut errs) = t.palette();
                let (layout, layout_errors) = Layout::load(&dir);
                errs.extend(layout_errors);
                self.layout = layout;
                self.home.dirty = true;
                self.pal = pal;
                self.theme = t;
                self.theme_errors = errs;
                self.fonts = resolve_fonts(&self.theme, cx);
                theme::set_icon_overrides(&dir);
                self.theme_stamp = theme::stamp(&dir);
                self.theme_dir = dir;
                self.theme_gen += 1;
                // Blur and sizes come from the theme: re-decode art.
                let b = self.backend.clone();
                self.images.update(cx, |s, _| s.reset(b));
            }
            Err(e) => {
                self.theme_errors = vec![e.clone()];
                self.toast(format!("Theme: {e}"), true);
            }
        }
        self.themes = theme::list(&theme::themes_dir(&self.dir));
        cx.notify();
    }

    /// Saves progress synchronously (window close).
    pub fn save_now(&mut self) {
        let Some(p) = &self.player else { return };
        let st = p.engine_state();
        if st.duration > 0.
            && let Some(b) = self.backend.clone()
        {
            let id = p.item_id.clone();
            let done = st.eof || st.time / st.duration > 0.92;
            let b2 = b.clone();
            let _ = b.block_on(async move {
                tokio::time::timeout(Duration::from_secs(2), backend::save_progress(b2, id, st.time, st.duration, done))
                    .await
            });
        }
    }

    // ---- keys ----------------------------------------------------------------------

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = ev.keystroke.key.as_str();
        let m = ev.keystroke.modifiers;
        let ctrl = m.control || m.platform;
        if self.ext_prompt.is_some() && self.confirm.is_none() {
            match k {
                "enter" | "space" => {
                    let deny = self.focused.as_ref().map(|f| f.as_ref()) == Some("ext.deny");
                    self.run(if deny { Act::ExtDeny } else { Act::ExtAllow }, window, cx);
                }
                "escape" => self.run(Act::ExtDeny, window, cx),
                "tab" | "left" | "right" => {
                    let deny = self.focused.as_ref().map(|f| f.as_ref()) == Some("ext.deny");
                    self.focused = Some(if deny { "ext.allow" } else { "ext.deny" }.into());
                    cx.notify();
                }
                _ => {}
            }
            return;
        }
        if let Some(_c) = &self.confirm {
            match k {
                "enter" | "y" => self.run(Act::ConfirmYes, window, cx),
                "escape" | "n" => self.run(Act::ConfirmNo, window, cx),
                _ => {}
            }
            return;
        }
        if ctrl && k == "k" && self.phase == Phase::Main && self.player.is_none() {
            self.toggle_palette();
            cx.notify();
            return;
        }
        if self.palette.is_some() {
            self.palette_key(ev, window, cx);
            return;
        }
        if self.player.is_some() {
            self.player_key(ev, window, cx);
            return;
        }
        if self.reader.is_some()
            && matches!(self.screen, Screen::Reader(_))
            && self.focused.is_none()
            && self.reader_key(ev, cx)
        {
            cx.notify();
            return;
        }
        // A focused text field takes text and editing keys.
        let field_id = self
            .focused
            .clone()
            .filter(|f| self.focus_order.iter().any(|x| x.field && &x.id == f))
            .map(|f| f.to_string());
        if let Some(fid) = &field_id
            && let Some(key) = self.fields.keys().copied().find(|k| *k == fid.as_str())
        {
            let clip = if ctrl && k == "v" { cx.read_from_clipboard().and_then(|c| c.text()) } else { None };
            let f = self.fields.entry(key).or_default();
            let edited = f.key(k, ctrl, clip);
            let typed = !edited
                && !ctrl
                && !m.alt
                && ev
                    .keystroke
                    .key_char
                    .as_ref()
                    .is_some_and(|c| !c.is_empty() && !c.chars().any(|ch| ch.is_control()));
            if typed && let Some(c) = &ev.keystroke.key_char {
                f.insert(c);
            }
            if edited || typed {
                self.field_changed(key, cx);
                cx.notify();
                return;
            }
        }
        // Global shortcuts.
        let in_field = field_id.is_some();
        match (k, ctrl, m.shift) {
            ("tab", _, shift) => {
                self.move_focus(if shift { -1 } else { 1 });
                cx.notify();
                return;
            }
            ("enter", _, _) | ("space", false, _) if self.focused.is_some() && !(k == "space" && in_field) => {
                let act =
                    self.focus_order.iter().find(|x| Some(&x.id) == self.focused.as_ref()).and_then(|x| x.act.clone());
                if let Some(a) = act {
                    self.run(a, window, cx);
                    return;
                }
            }
            ("escape", _, _) => {
                if self.focused.is_some() && self.phase == Phase::Main {
                    self.focused = None;
                } else if self.phase == Phase::Main {
                    self.back(cx);
                }
                cx.notify();
                return;
            }
            ("f5", _, _) | ("r", true, _) if self.phase == Phase::Main => {
                self.run(Act::Refresh, window, cx);
                return;
            }
            ("s", true, _) if matches!(self.screen, Screen::Settings(Section::Server)) => {
                self.run(Act::SaveTranscode, window, cx);
                return;
            }
            (",", true, _) if self.phase == Phase::Main => {
                self.navigate(Screen::Settings(Section::Account), cx);
                cx.notify();
                return;
            }
            ("left", false, _) if m.alt && self.phase == Phase::Main => {
                self.back(cx);
                cx.notify();
                return;
            }
            ("/", false, false) if !in_field && self.phase == Phase::Main => {
                self.navigate(Screen::Search, cx);
                cx.notify();
                return;
            }
            (d, true, false) if self.phase == Phase::Main && d.len() == 1 && d.chars().all(|c| c.is_ascii_digit()) => {
                let i = d.parse::<usize>().unwrap_or(0).saturating_sub(1);
                if let Some((_, _, _, s)) = self.nav().into_iter().nth(i) {
                    self.navigate(s, cx);
                }
                cx.notify();
                return;
            }
            _ => {}
        }
        if self.phase == Phase::Connect {
            if matches!(k, "down" | "up") {
                self.move_focus(if k == "down" { 1 } else { -1 });
                cx.notify();
            }
            return;
        }
        // Screen keys (spatial navigation) when no widget is focused.
        let consumed = if self.focused.is_none() {
            match self.screen {
                Screen::Home => self.home_key(ev, window, cx),
                Screen::Library { .. } => self.library_key(ev, window, cx),
                Screen::Item(_) => self.item_key(ev, window, cx),
                _ => false,
            }
        } else {
            false
        };
        if consumed {
            cx.notify();
            return;
        }
        if matches!(k, "down" | "up") || (!in_field && matches!(k, "right" | "left")) {
            self.move_focus(if matches!(k, "down" | "right") { 1 } else { -1 });
            cx.notify();
        }
    }

    pub fn move_focus(&mut self, d: i32) {
        let n = self.focus_order.len();
        if n == 0 {
            return;
        }
        let cur = self.focused.as_ref().and_then(|f| self.focus_order.iter().position(|x| &x.id == f));
        let next = match cur {
            Some(i) => (i as i32 + d).rem_euclid(n as i32) as usize,
            None if d > 0 => 0,
            None => n - 1,
        };
        let id = self.focus_order[next].id.clone();
        if let Some(&row) = self.row_of.get(&id) {
            self.scroll.scroll_to_item(row);
        } else {
            self.reveal(&id);
        }
        self.focused = Some(id);
    }

    /// Scrolls the screen's scroll view so a focus stop is visible.
    fn reveal(&self, id: &SharedString) {
        let Some(b) = self.stop_bounds.borrow().get(id).copied() else { return };
        let view = self.scroll.bounds();
        if f32::from(view.size.height) <= 0. {
            return;
        }
        let mut off = self.scroll.offset();
        let pad = gpui::px(24.);
        if b.bottom() + pad > view.bottom() {
            off.y -= b.bottom() + pad - view.bottom();
        } else if b.top() - pad < view.top() {
            off.y += view.top() - (b.top() - pad);
        } else {
            return;
        }
        off.y = off.y.min(gpui::px(0.));
        self.scroll.set_offset(off);
    }

    fn field_changed(&mut self, key: &'static str, cx: &mut Context<Self>) {
        match key {
            "palette.q" => {
                let q = self.text("palette.q");
                let entries = self.palette_entries(&q);
                if let Some(p) = self.palette.as_mut() {
                    p.entries = entries;
                    p.sel = 0;
                }
            }
            "search.q" => self.search_typed(cx),
            "library.filter" => self.lib.page = 0,
            _ => {}
        }
    }

    fn palette_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = ev.keystroke.key.as_str();
        let n = self.palette.as_ref().map(|p| p.entries.len()).unwrap_or(0);
        match k {
            "escape" => self.palette = None,
            "down" | "tab" => {
                if let Some(p) = self.palette.as_mut() {
                    p.sel = (p.sel + 1).min(n.saturating_sub(1));
                }
            }
            "up" => {
                if let Some(p) = self.palette.as_mut() {
                    p.sel = p.sel.saturating_sub(1);
                }
            }
            "enter" => {
                let sel = self.palette.as_ref().map(|p| p.sel).unwrap_or(0);
                self.run(Act::PaletteRun(sel), window, cx);
                return;
            }
            _ => {
                let m = ev.keystroke.modifiers;
                let ctrl = m.control || m.platform;
                let clip = if ctrl && k == "v" { cx.read_from_clipboard().and_then(|c| c.text()) } else { None };
                let f = self.fields.entry("palette.q").or_default();
                let mut changed = f.key(k, ctrl, clip);
                if !changed
                    && !ctrl
                    && let Some(c) = &ev.keystroke.key_char
                    && !c.chars().any(|ch| ch.is_control())
                {
                    f.insert(c);
                    changed = true;
                }
                if changed {
                    self.field_changed("palette.q", cx);
                }
            }
        }
        cx.notify();
    }

    // ---- rendering -------------------------------------------------------------------

    fn render_nav(&mut self, vw: f32, solid: f32, cx: &mut Context<Self>) -> gpui::Div {
        let th = self.theme.clone();
        let pal = self.pal.clone();
        let margin = th.sp(6);
        let nav = self.nav();
        let mut items = div().flex().items_center().gap(px(th.sp(1)));
        for (i, (id, label, icon, screen)) in nav.into_iter().enumerate() {
            let fid = format!("nav.{id}");
            let focused = self.stop(&fid, Some(Act::Nav(screen.clone())), false);
            let active = std::mem::discriminant(&self.screen) == std::mem::discriminant(&screen)
                && match (&self.screen, &screen) {
                    (Screen::Library { read: a, .. }, Screen::Library { read: b, .. }) => a == b,
                    _ => true,
                };
            let s2 = screen.clone();
            items = items.child(
                div()
                    .id(SharedString::from(fid))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .h(px(34.))
                    .px(px(th.sp(2)))
                    .rounded(px(th.radius.button))
                    .border_1()
                    .border_color(if focused { pal.focus } else { pal.text.opacity(0.) })
                    .bg(if active { pal.text.opacity(0.08) } else { pal.text.opacity(0.) })
                    .text_color(if active { pal.text } else { pal.text.opacity(0.72) })
                    .font_weight(if active { FontWeight::SEMIBOLD } else { FontWeight::MEDIUM })
                    .cursor_pointer()
                    .hover(|s| s.bg(gpui::white().opacity(0.06)))
                    .child(self.icon(icon, 15., if active { pal.accent } else { pal.text.opacity(0.6) }))
                    .child(label)
                    .child(
                        div()
                            .text_size(px(th.ts(-3)))
                            .text_color(pal.muted)
                            .font_family(self.fonts.mono.clone())
                            .child(format!("{}", i + 1)),
                    )
                    .on_click(cx.listener(move |app, _: &ClickEvent, _, cx| {
                        app.navigate(s2.clone(), cx);
                        cx.notify();
                    })),
            );
        }
        let user = self.me.as_ref().map(|u| {
            if u.profile.display_name.is_empty() { u.username.clone() } else { u.profile.display_name.clone() }
        });
        let status = if self.online { pal.ok } else { pal.danger };
        let avatar = self.avatar(28., cx);
        let palette_btn = div()
            .id("nav.palette")
            .flex()
            .items_center()
            .gap(px(th.sp(2)))
            .h(px(34.))
            .px(px(th.sp(2)))
            .rounded(px(th.radius.button))
            .border_1()
            .border_color(pal.line)
            .text_color(pal.muted)
            .cursor_pointer()
            .child(self.icon("search", 14., pal.muted))
            .child("Commands")
            .child(self.kbd("Ctrl K"))
            .on_click(cx.listener(|app, _: &ClickEvent, _, cx| {
                app.toggle_palette();
                cx.notify();
            }));
        div()
            .absolute()
            .top_0()
            .left_0()
            .w(px(vw))
            .h(px(64.))
            .px(px(margin))
            .flex()
            .items_center()
            .justify_between()
            .bg(linear_gradient(
                180.,
                linear_color_stop(pal.scrim.opacity(0.75 + 0.25 * solid), 0.),
                linear_color_stop(pal.scrim.opacity(solid), 1.),
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(th.sp(5)))
                    .child(
                        div()
                            .font_family(self.fonts.display.clone())
                            .text_size(px(th.ts(2)))
                            .font_weight(FontWeight::BOLD)
                            .child("lain"),
                    )
                    .child(items),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(th.sp(3)))
                    .child(palette_btn)
                    .child(div().size(px(7.)).rounded_full().bg(status))
                    .child(
                        div()
                            .text_color(pal.muted)
                            .text_size(px(th.ts(-1)))
                            .child(user.unwrap_or_else(|| "offline".into())),
                    )
                    .when(self.me.is_some(), |d| d.child(avatar)),
            )
    }

    fn render_overlays(&mut self, vw: f32, vh: f32, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
        let th = self.theme.clone();
        let pal = self.pal.clone();
        let mut out = Vec::new();
        // toasts
        if !self.toasts.is_empty() {
            let mut col = div()
                .absolute()
                .right(px(th.sp(5)))
                .bottom(px(th.sp(5)))
                .flex()
                .flex_col()
                .gap(px(th.sp(2)))
                .items_end();
            for t in &self.toasts {
                col = col.child(
                    self.panel()
                        .px(px(th.sp(3)))
                        .py(px(th.sp(2)))
                        .flex()
                        .items_center()
                        .gap(px(th.sp(2)))
                        .child(div().size(px(7.)).rounded_full().bg(if t.bad { pal.danger } else { pal.ok }))
                        .child(t.text.clone()),
                );
            }
            out.push(col.into_any_element());
        }
        // confirm
        if let Some((text, _)) = self.confirm.clone() {
            let yes = self.btn("confirm.yes", "Confirm", Some("Enter"), Tone::Danger, Act::ConfirmYes, cx);
            let no = self.btn("confirm.no", "Cancel", Some("Esc"), Tone::Ghost, Act::ConfirmNo, cx);
            out.push(
                div()
                    .absolute()
                    .inset_0()
                    .bg(pal.bg.opacity(0.6))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        self.panel()
                            .w(px(480.))
                            .p(px(th.sp(5)))
                            .flex()
                            .flex_col()
                            .gap(px(th.sp(4)))
                            .child(self.label("confirm"))
                            .child(div().text_size(px(th.ts(1))).child(text))
                            .child(div().flex().gap(px(th.sp(2))).child(yes).child(no)),
                    )
                    .into_any_element(),
            );
        }
        // extension permission prompt
        if let Some(p) = self.render_ext_prompt(cx) {
            out.push(p);
        }
        // palette
        if self.palette.is_some() {
            let field = self.field("palette.q", "Type a command, a screen or a title…", false, Act::PaletteRun(0), cx);
            let (sel, entries): (usize, Vec<(String, String)>) = {
                let p = self.palette.as_ref().unwrap();
                (p.sel, p.entries.iter().map(|e| (e.label.clone(), e.hint.clone())).collect())
            };
            let mut list = div().id("palette.list").flex().flex_col().max_h(px(vh * 0.55)).overflow_y_scroll();
            for (i, (label, hint)) in entries.into_iter().enumerate() {
                let on = i == sel;
                list = list.child(
                    div()
                        .id(("pal", i))
                        .flex()
                        .justify_between()
                        .items_center()
                        .px(px(th.sp(3)))
                        .py(px(th.sp(2)))
                        .rounded(px(th.radius.button))
                        .bg(if on { pal.accent.opacity(0.22) } else { pal.text.opacity(0.) })
                        .cursor_pointer()
                        .hover(|s| s.bg(gpui::white().opacity(0.05)))
                        .child(div().truncate().child(label))
                        .child(
                            div()
                                .text_size(px(th.ts(-2)))
                                .font_family(self.fonts.mono.clone())
                                .text_color(pal.muted)
                                .child(hint),
                        )
                        .on_click(cx.listener(move |app, _: &ClickEvent, w, cx| app.run(Act::PaletteRun(i), w, cx))),
                );
            }
            out.push(
                div()
                    .absolute()
                    .inset_0()
                    .bg(pal.bg.opacity(0.5))
                    .flex()
                    .justify_center()
                    .pt(px(vh * 0.14))
                    .child(
                        self.panel()
                            .w(px((vw * 0.5).clamp(480., 760.)))
                            .h_auto()
                            .p(px(th.sp(2)))
                            .flex()
                            .flex_col()
                            .gap(px(th.sp(2)))
                            .shadow_lg()
                            .child(field)
                            .child(list)
                            .child(
                                div()
                                    .flex()
                                    .gap(px(th.sp(3)))
                                    .px(px(th.sp(2)))
                                    .text_size(px(th.ts(-2)))
                                    .text_color(pal.muted)
                                    .child(div().flex().gap(px(4.)).child(self.kbd("↑↓")).child("select"))
                                    .child(div().flex().gap(px(4.)).child(self.kbd("Enter")).child("run"))
                                    .child(div().flex().gap(px(4.)).child(self.kbd("Esc")).child("close")),
                            ),
                    )
                    .into_any_element(),
            );
        }
        out
    }
}

/// Subsequence match: every char of `q` appears in order in `s`.
pub fn fuzzy(s: &str, q: &str) -> bool {
    let mut it = s.chars();
    q.chars().filter(|c| !c.is_whitespace()).all(|c| it.any(|x| x == c))
}

impl Render for LainApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.hwnd == 0 {
            self.hwnd = platform::window_handle(window).unwrap_or(0);
        }
        self.focus_order.clear();
        self.row_of.clear();
        self.cur_row = 0;
        let (vw, vh) = crate::ui::viewport(window);
        let mut root = div()
            .id("root")
            .size_full()
            .relative()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .font_family(self.fonts.ui.clone())
            .text_size(px(self.theme.ts(0)))
            .text_color(self.pal.text);
        if self.player.as_ref().is_some_and(|p| !p.is_external()) {
            return root.child(self.render_player(window, cx).into_any_element());
        }
        root = root.bg(self.pal.bg);
        let body = match self.phase {
            Phase::Connect => self.render_connect(window, cx).into_any_element(),
            Phase::Main => {
                let (content, solid) = match self.screen.clone() {
                    Screen::Home => (self.render_home(window, cx).into_any_element(), self.home_nav_solid()),
                    Screen::Library { read, lib } => {
                        (self.render_library(read, &lib, window, cx).into_any_element(), 1.)
                    }
                    Screen::Item(id) => (self.render_item(&id, window, cx).into_any_element(), self.item_nav_solid()),
                    Screen::Search => (self.render_search(window, cx).into_any_element(), 1.),
                    Screen::Settings(s) => (self.render_settings(s, window, cx).into_any_element(), 1.),
                    Screen::Downloads => (self.render_downloads(window, cx).into_any_element(), 1.),
                    Screen::Acquire => (self.render_acquire(window, cx).into_any_element(), 1.),
                    Screen::Reader(id) => (self.render_reader(&id, window, cx).into_any_element(), 1.),
                };
                let mut shell = div().size_full().relative().child(content);
                if !matches!(self.screen, Screen::Reader(_)) {
                    shell = shell.child(self.render_nav(vw, solid, cx));
                }
                if self.player.as_ref().is_some_and(|p| p.is_external()) {
                    shell = shell.child(self.render_player(window, cx).into_any_element());
                }
                shell.into_any_element()
            }
        };
        root = root.child(body);
        for o in self.render_overlays(vw, vh, cx) {
            root = root.child(o);
        }
        self.demo.on_frame();
        root
    }
}
