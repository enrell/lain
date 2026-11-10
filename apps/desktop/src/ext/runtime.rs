//! The extension host (D-256): wasmtime's component model with a
//! capability-gated host API, the model Zed uses for its extensions.
//!
//! - One [`Engine`] per app, with fuel and epoch interruption on, and a
//!   ticker thread that advances the epoch every [`TICK`].
//! - One worker thread and one [`Store`] per extension. Every export
//!   call gets a fresh fuel budget and a wall-clock deadline; memories
//!   and tables are capped by a [`StoreLimits`]. Host calls per export
//!   call are capped too.
//! - WASI p2 is linked because every `wasm32-wasip2` guest imports it, but
//!   with nothing behind it: no environment, arguments, stdin or sockets,
//!   and no preopened folder unless `fs:data` is granted (then only the
//!   extension's own `data/`, as `/data`). Its stdout and stderr go to a
//!   capped buffer that is copied into the log.
//! - Each `lain:extension/host` function checks its scope before doing
//!   anything; a missing grant answers `denied(<scope>)` and never
//!   reaches [`HostApi`].
//! - A trap (a panic, fuel or deadline exhausted, memory over the cap, a
//!   failed instantiation) ends the worker and reports
//!   [`Out::Failed`]; the app disables the extension. Nothing a guest
//!   does can unwind into the app: calls run under `catch_unwind`.

use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder, Trap};
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;
use wasmtime_wasi::{FsPerms, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use super::manifest::Scope;

wasmtime::component::bindgen!({
    path: "../../crates/lain-ext-api/wit",
    world: "extension",
});

pub use lain::extension::host::LogLevel;
pub use lain::extension::types::{HostError, HttpResponse, Item, Library, Progress};

/// The epoch ticker's period.
pub const TICK: Duration = Duration::from_millis(10);

/// Per-extension resource caps.
#[derive(Debug, Clone)]
pub struct Limits {
    /// Fuel per export call (about one unit per wasm instruction).
    pub fuel: u64,
    /// Wall-clock time per export call, host calls included.
    pub deadline: Duration,
    /// Bytes per linear memory.
    pub memory: usize,
    /// Host calls per export call.
    pub host_calls: u32,
    /// Bytes `fs:data` may hold; over it the extension is stopped.
    pub data_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            fuel: 4_000_000_000,
            deadline: Duration::from_secs(10),
            memory: 64 << 20,
            host_calls: 4096,
            data_bytes: 64 << 20,
        }
    }
}

/// What output the host keeps, whatever the guest returns.
pub mod caps {
    pub const RAILS: usize = 8;
    pub const CARDS: usize = 200;
    pub const PANELS: usize = 4;
    pub const ROWS: usize = 24;
    pub const ID: usize = 64;
    pub const TITLE: usize = 120;
    pub const SUBTITLE: usize = 160;
    pub const VALUE: usize = 200;
    pub const TEXT: usize = 2000;
    pub const HTTP_BODY: usize = 1 << 20;
    pub const LOG_LINE: usize = 500;
}

/// The server and network side of the host API. The real one goes
/// through `lain-client` with the user's token (`api.rs`); tests use a
/// fake. Scope checks happen before any of these is called.
pub trait HostApi: Send + Sync + 'static {
    fn libraries(&self) -> Result<Vec<Library>, HostError>;
    fn catalog(&self, offset: u32, limit: u32) -> Result<Vec<Item>, HostError>;
    fn item(&self, id: &str) -> Result<Item, HostError>;
    fn progress(&self, item_id: &str) -> Result<Option<Progress>, HostError>;
    fn continue_watching(&self) -> Result<Vec<Progress>, HostError>;
    /// One GET, no redirects, body capped at `max_body`.
    fn http_get(&self, url: &str, max_body: usize) -> Result<HttpResponse, HostError>;
}

/// The scopes in force for one extension.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Grants {
    pub scopes: Vec<Scope>,
}

impl Grants {
    pub fn new(scopes: Vec<Scope>) -> Grants {
        Grants { scopes }
    }
    pub fn has(&self, s: &Scope) -> bool {
        self.scopes.contains(s)
    }
    /// Whether `net:<host>` (or `net:<host>:<port>`) covers this URL.
    fn net_allows(&self, host: &str, port: u16, default_port: bool) -> bool {
        self.scopes.iter().any(|s| match s {
            Scope::Net(h) => match h.rsplit_once(':') {
                Some((name, p)) => name == host && p.parse::<u16>().ok() == Some(port),
                None => h == host && default_port,
            },
            _ => false,
        })
    }
}

/// From a worker to the app.
#[derive(Debug, Clone)]
pub enum Out {
    /// The extension is running (after `activate`).
    Ready {
        ext: String,
    },
    Rails {
        ext: String,
        rails: Vec<Rail>,
    },
    Panels {
        ext: String,
        item: String,
        panels: Vec<Panel>,
    },
    Log {
        ext: String,
        level: LogLevel,
        line: String,
    },
    /// The extension trapped or failed to load; its worker has ended.
    Failed {
        ext: String,
        reason: String,
    },
}

/// From the app to a worker.
#[derive(Debug, Clone)]
pub enum Cmd {
    Rails,
    Panels(String),
    Event(Event),
    Stop,
}

/// The engine, the linker and the epoch ticker, shared by every worker.
pub struct Runtime {
    engine: Engine,
    linker: Arc<Linker<HostState>>,
    ticking: Arc<AtomicBool>,
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.ticking.store(false, Ordering::Relaxed);
    }
}

impl Runtime {
    pub fn new() -> Result<Runtime, String> {
        let mut cfg = Config::new();
        cfg.wasm_component_model(true);
        cfg.consume_fuel(true);
        cfg.epoch_interruption(true);
        cfg.max_wasm_stack(1 << 20);
        let engine = Engine::new(&cfg).map_err(|e| format!("wasm engine: {e}"))?;
        let mut linker: Linker<HostState> = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(|e| format!("wasi: {e}"))?;
        Extension::add_to_linker::<HostState, HasSelf<HostState>>(&mut linker, |s| s)
            .map_err(|e| format!("host api: {e}"))?;
        let ticking = Arc::new(AtomicBool::new(true));
        let (e2, t2) = (engine.clone(), ticking.clone());
        std::thread::Builder::new()
            .name("lain-ext-epoch".into())
            .spawn(move || {
                while t2.load(Ordering::Relaxed) {
                    std::thread::sleep(TICK);
                    e2.increment_epoch();
                }
            })
            .map_err(|e| format!("epoch thread: {e}"))?;
        Ok(Runtime { engine, linker: Arc::new(linker), ticking })
    }

    /// Starts one extension on its own thread. `wasm` is the component's
    /// bytes; `data_dir` is mounted as `/data` only with `fs:data`.
    pub fn start(&self, spec: Spec, out: tokio::sync::mpsc::UnboundedSender<Out>) -> Worker {
        let (tx, rx) = mpsc::channel::<Cmd>();
        let engine = self.engine.clone();
        let linker = self.linker.clone();
        let ext = spec.id.clone();
        let out2 = out.clone();
        let spawned = std::thread::Builder::new().name(format!("lain-ext-{}", spec.id)).spawn(move || {
            let id = spec.id.clone();
            let r = std::panic::catch_unwind(AssertUnwindSafe(|| run_worker(engine, linker, spec, rx, &out2)));
            if r.is_err() {
                let _ = out2.send(Out::Failed { ext: id, reason: "the extension host panicked".into() });
            }
        });
        if let Err(e) = spawned {
            let _ = out.send(Out::Failed { ext, reason: format!("could not start a thread: {e}") });
        }
        Worker { tx }
    }
}

/// What a worker needs to run one extension.
pub struct Spec {
    pub id: String,
    pub wasm: Vec<u8>,
    pub config: String,
    pub grants: Grants,
    pub limits: Limits,
    pub data_dir: PathBuf,
    pub api: Arc<dyn HostApi>,
}

/// The app's handle on a running extension. Dropping it stops the worker.
pub struct Worker {
    tx: mpsc::Sender<Cmd>,
}

impl Worker {
    /// Queues a command; false if the worker has ended.
    pub fn send(&self, c: Cmd) -> bool {
        self.tx.send(c).is_ok()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Stop);
    }
}

// ---- the store's data: WASI plus the gated host API --------------------------

pub struct HostState {
    id: String,
    wasi: WasiCtx,
    table: ResourceTable,
    limits: StoreLimits,
    grants: Grants,
    api: Arc<dyn HostApi>,
    out: tokio::sync::mpsc::UnboundedSender<Out>,
    invalidated: bool,
    calls: u32,
    max_calls: u32,
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.wasi, table: &mut self.table }
    }
}

impl HostState {
    /// Counts a host call and checks its scope.
    fn gate(&mut self, scope: Scope) -> Result<(), HostError> {
        self.budget()?;
        if self.grants.has(&scope) { Ok(()) } else { Err(HostError::Denied(scope.id())) }
    }

    fn budget(&mut self) -> Result<(), HostError> {
        self.calls += 1;
        if self.calls > self.max_calls {
            return Err(HostError::Failed(format!("more than {} host calls in one call", self.max_calls)));
        }
        Ok(())
    }

    fn log_line(&self, level: LogLevel, line: &str) {
        let line = clip(line, caps::LOG_LINE);
        eprintln!("ext {}: {}: {line}", self.id, level_name(level));
        let _ = self.out.send(Out::Log { ext: self.id.clone(), level, line });
    }
}

impl lain::extension::types::Host for HostState {}

impl lain::extension::host::Host for HostState {
    fn log(&mut self, level: LogLevel, message: String) {
        if self.budget().is_ok() {
            self.log_line(level, &message);
        }
    }

    fn granted_scopes(&mut self) -> Vec<String> {
        self.grants.scopes.iter().map(Scope::id).collect()
    }

    fn invalidate(&mut self) {
        self.invalidated = true;
    }

    fn list_libraries(&mut self) -> Result<Vec<Library>, HostError> {
        self.gate(Scope::CatalogRead)?;
        self.api.libraries()
    }

    fn list_catalog(&mut self, offset: u32, limit: u32) -> Result<Vec<Item>, HostError> {
        self.gate(Scope::CatalogRead)?;
        self.api.catalog(offset, limit.clamp(1, 500))
    }

    fn get_item(&mut self, id: String) -> Result<Item, HostError> {
        self.gate(Scope::CatalogRead)?;
        self.api.item(&id)
    }

    fn get_progress(&mut self, item_id: String) -> Result<Option<Progress>, HostError> {
        self.gate(Scope::ProgressRead)?;
        self.api.progress(&item_id)
    }

    fn continue_watching(&mut self) -> Result<Vec<Progress>, HostError> {
        self.gate(Scope::ProgressRead)?;
        self.api.continue_watching()
    }

    fn http_get(&mut self, url: String) -> Result<HttpResponse, HostError> {
        self.budget()?;
        let u = lain_http::Url::parse(&url).map_err(|e| HostError::Failed(e.message))?;
        let default_port = u.port == if u.tls { 443 } else { 80 };
        if !self.grants.net_allows(&u.host, u.port, default_port) {
            let scope = if default_port { format!("net:{}", u.host) } else { format!("net:{}:{}", u.host, u.port) };
            return Err(HostError::Denied(scope));
        }
        self.api.http_get(&url, caps::HTTP_BODY)
    }
}

// ---- the worker -----------------------------------------------------------------

fn run_worker(
    engine: Engine,
    linker: Arc<Linker<HostState>>,
    spec: Spec,
    rx: mpsc::Receiver<Cmd>,
    out: &tokio::sync::mpsc::UnboundedSender<Out>,
) {
    let id = spec.id.clone();
    let fail = |reason: String| {
        eprintln!("ext {id}: stopped: {reason}");
        let _ = out.send(Out::Failed { ext: id.clone(), reason });
    };
    let component = match Component::from_binary(&engine, &spec.wasm) {
        Ok(c) => c,
        Err(e) => return fail(format!("not a valid component: {}", first_line(&e.to_string()))),
    };
    let pipe = MemoryOutputPipe::new(64 << 10);
    let mut wasi = WasiCtxBuilder::new();
    wasi.stdout(pipe.clone()).stderr(pipe.clone()).allow_tcp(false).allow_udp(false).allow_ip_name_lookup(false);
    if spec.grants.has(&Scope::FsData) {
        if let Err(e) = std::fs::create_dir_all(&spec.data_dir) {
            return fail(format!("data folder: {e}"));
        }
        if let Err(e) = wasi.preopened_dir(&spec.data_dir, "/data", FsPerms::ReadWrite) {
            return fail(format!("data folder: {e}"));
        }
    }
    let state = HostState {
        id: id.clone(),
        wasi: wasi.build(),
        table: ResourceTable::new(),
        limits: StoreLimitsBuilder::new()
            .memory_size(spec.limits.memory)
            .memories(4)
            .tables(16)
            .table_elements(100_000)
            .instances(32)
            .trap_on_grow_failure(true)
            .build(),
        grants: spec.grants.clone(),
        api: spec.api.clone(),
        out: out.clone(),
        invalidated: false,
        calls: 0,
        max_calls: spec.limits.host_calls,
    };
    let mut store = Store::new(&engine, state);
    store.limiter(|s| &mut s.limits);
    let mut ctx = CallCtx { store, limits: spec.limits.clone(), pipe, printed: 0, data_dir: spec.data_dir.clone() };

    let bindings = match ctx.arm().and_then(|_| Extension::instantiate(&mut ctx.store, &component, &linker)) {
        Ok(b) => b,
        Err(e) => return fail(ctx.reason("could not load", e)),
    };
    match ctx.call(|s| bindings.call_activate(s, &spec.config)) {
        Ok(Ok(())) => {}
        Ok(Err(msg)) => return fail(format!("activate refused: {}", clip(&msg, caps::TEXT))),
        Err(reason) => return fail(reason),
    }
    let _ = out.send(Out::Ready { ext: id.clone() });

    let rail = spec.grants.has(&Scope::UiRail);
    let panel = spec.grants.has(&Scope::UiPanel);
    let events = spec.grants.has(&Scope::EventsSubscribe);
    let mut want_rails = rail;
    loop {
        if want_rails {
            want_rails = false;
            match ctx.call(|s| bindings.call_rails(s)) {
                Ok(r) => {
                    let _ = out.send(Out::Rails { ext: id.clone(), rails: clip_rails(r) });
                }
                Err(reason) => return fail(reason),
            }
        }
        // Block for the next command, then take everything queued so a
        // burst of events costs one `rails` call.
        let Ok(first) = rx.recv() else { return };
        let mut batch = vec![first];
        while let Ok(c) = rx.try_recv() {
            batch.push(c);
        }
        let mut panels_for: Vec<String> = Vec::new();
        for c in batch {
            match c {
                Cmd::Stop => return,
                Cmd::Rails => want_rails = rail,
                Cmd::Panels(item) if panel => {
                    panels_for.retain(|x| *x != item);
                    panels_for.push(item);
                }
                Cmd::Panels(_) => {}
                Cmd::Event(ev) if events => {
                    if let Err(reason) = ctx.call(|s| bindings.call_on_event(s, &ev)) {
                        return fail(reason);
                    }
                }
                Cmd::Event(_) => {}
            }
        }
        for item in panels_for {
            match ctx.call(|s| bindings.call_panels(s, &item)) {
                Ok(p) => {
                    let _ = out.send(Out::Panels { ext: id.clone(), item, panels: clip_panels(p) });
                }
                Err(reason) => return fail(reason),
            }
        }
        if std::mem::take(&mut ctx.store.data_mut().invalidated) {
            want_rails = rail;
        }
    }
}

/// The store plus what each export call resets and checks.
struct CallCtx {
    store: Store<HostState>,
    limits: Limits,
    pipe: MemoryOutputPipe,
    /// Pipe bytes already copied into the log.
    printed: usize,
    data_dir: PathBuf,
}

impl CallCtx {
    /// A fresh fuel budget and deadline for the next call.
    fn arm(&mut self) -> wasmtime::Result<()> {
        self.store.set_fuel(self.limits.fuel)?;
        let ticks = (self.limits.deadline.as_millis() / TICK.as_millis()).max(1) as u64;
        self.store.set_epoch_deadline(ticks);
        self.store.data_mut().calls = 0;
        Ok(())
    }

    /// Runs one export call; a trap becomes a human reason.
    fn call<T>(&mut self, f: impl FnOnce(&mut Store<HostState>) -> wasmtime::Result<T>) -> Result<T, String> {
        if let Err(e) = self.arm() {
            return Err(self.reason("could not start a call", e));
        }
        let r = std::panic::catch_unwind(AssertUnwindSafe(|| f(&mut self.store)));
        self.flush_output();
        let r = match r {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => Err(self.reason("", e)),
            Err(_) => Err("the extension host panicked during a call".to_string()),
        };
        if r.is_ok() && self.store.data().grants.has(&Scope::FsData) {
            let used = dir_size(&self.data_dir, 0);
            if used > self.limits.data_bytes {
                return Err(format!(
                    "its data folder holds {used} bytes, over the {} byte limit",
                    self.limits.data_bytes
                ));
            }
        }
        r
    }

    /// The guest's stdout/stderr since the last call, into the log.
    fn flush_output(&mut self) {
        let bytes = self.pipe.contents();
        if bytes.len() <= self.printed {
            return;
        }
        let text = String::from_utf8_lossy(&bytes[self.printed..]).into_owned();
        self.printed = bytes.len();
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            self.store.data().log_line(LogLevel::Warn, line);
        }
    }

    fn reason(&self, what: &str, e: wasmtime::Error) -> String {
        let why = match e.downcast_ref::<Trap>() {
            Some(Trap::OutOfFuel) => "it used its whole CPU budget for one call".to_string(),
            Some(Trap::Interrupt) => format!("one call ran longer than {}", secs(self.limits.deadline)),
            Some(Trap::StackOverflow) => "it overflowed its stack".to_string(),
            Some(Trap::UnreachableCodeReached) => "it crashed (a panic or an abort)".to_string(),
            Some(t) => format!("it trapped: {t}"),
            None => {
                let s = format!("{e:#}");
                if s.contains("memory") && (s.contains("grow") || s.contains("limit")) {
                    format!("it used more than {} MiB of memory", self.limits.memory >> 20)
                } else {
                    first_line(&s)
                }
            }
        };
        let why = if what.is_empty() { why } else { format!("{what}: {why}") };
        // A Rust panic prints `thread '…' panicked at <file>:` and then
        // the message on the next line.
        let printed = String::from_utf8_lossy(&self.pipe.contents()).into_owned();
        let lines: Vec<&str> = printed.lines().collect();
        match lines.iter().rposition(|l| l.contains("panicked at")) {
            Some(i) => {
                let msg = lines.get(i + 1).map(|l| l.trim()).filter(|l| !l.is_empty()).unwrap_or(lines[i].trim());
                format!("{why}: \"{}\"", clip(msg, 200))
            }
            None => why,
        }
    }
}

fn secs(d: Duration) -> String {
    if d < Duration::from_secs(1) { format!("{} ms", d.as_millis()) } else { format!("{} s", d.as_secs()) }
}

pub fn level_name(l: LogLevel) -> &'static str {
    match l {
        LogLevel::Debug => "debug",
        LogLevel::Info => "info",
        LogLevel::Warn => "warn",
        LogLevel::Error => "error",
    }
}

fn dir_size(p: &std::path::Path, depth: u32) -> u64 {
    let Ok(rd) = std::fs::read_dir(p) else { return 0 };
    let mut n = 0;
    for e in rd.flatten() {
        match e.metadata() {
            Ok(m) if m.is_dir() && depth < 16 => n += dir_size(&e.path(), depth + 1),
            Ok(m) => n += m.len(),
            Err(_) => {}
        }
    }
    n
}

fn first_line(s: &str) -> String {
    clip(s.lines().next().unwrap_or(s), 300)
}

/// At most `n` characters, on a char boundary, with an ellipsis if cut.
pub fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn clip_rails(mut r: Vec<Rail>) -> Vec<Rail> {
    r.truncate(caps::RAILS);
    for rail in &mut r {
        rail.id = clip(&rail.id, caps::ID);
        rail.title = clip(&rail.title, caps::TITLE);
        rail.cards.truncate(caps::CARDS);
        for c in &mut rail.cards {
            c.title = clip(&c.title, caps::TITLE);
            c.subtitle = clip(&c.subtitle, caps::SUBTITLE);
            if let Some(id) = &mut c.item_id {
                *id = clip(id, caps::ID * 2);
            }
        }
    }
    r
}

fn clip_panels(mut p: Vec<Panel>) -> Vec<Panel> {
    p.truncate(caps::PANELS);
    for panel in &mut p {
        panel.id = clip(&panel.id, caps::ID);
        panel.title = clip(&panel.title, caps::TITLE);
        panel.text = clip(&panel.text, caps::TEXT);
        panel.rows.truncate(caps::ROWS);
        for row in &mut panel.rows {
            row.label = clip(&row.label, caps::TITLE);
            row.value = clip(&row.value, caps::VALUE);
        }
    }
    p
}
