//! The host side: a supervised child process behind a Provider.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::Value;

use lain_core::{Error, Provider, codes};

use crate::manifest::Manifest;
use crate::protocol::{HEALTH_OP, Hello, PROTOCOL_VERSION, Request, Response};
use crate::transport::{Conn, Listener, is_timeout};

/// The time bounds of one component; the defaults are the Go host's.
#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    /// One capability call.
    pub invoke: Duration,
    /// The liveness op.
    pub health: Duration,
    /// Connect plus hello for a fresh child.
    pub handshake: Duration,
    /// The respawn ladder: doubles from `backoff_start` to `backoff_max`
    /// and resets on each clean handshake.
    pub backoff_start: Duration,
    pub backoff_max: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Timeouts {
            invoke: Duration::from_secs(5 * 60),
            health: Duration::from_secs(3),
            handshake: Duration::from_secs(10),
            backoff_start: Duration::from_millis(500),
            backoff_max: Duration::from_secs(30),
        }
    }
}

#[derive(Default)]
struct State {
    conn: Option<Box<dyn Conn>>,
    ready: bool,
    /// The last lifecycle error, surfaced through health.
    last: String,
    seq: u64,
}

struct Shared {
    manifest: Manifest,
    argv: Vec<String>,
    timeouts: Timeouts,
    /// Serializes requests on the one connection; guards its state.
    state: Mutex<State>,
    child: Mutex<Option<Child>>,
    closed: AtomicBool,
    listener: Listener,
}

/// A Provider served by a supervised child process (D-074). Spawning
/// is eager; health stays a cheap liveness op because the supervisor
/// thread owns the lifecycle.
pub struct Process {
    shared: Arc<Shared>,
    supervisor: Mutex<Option<JoinHandle<()>>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Process {
    /// Resolves the manifest, binds `<sock_dir>/<id>.sock` and starts
    /// the supervisor, which launches the child at once and respawns
    /// it with bounded backoff after an unexpected exit.
    pub fn spawn(manifest: Manifest, sock_dir: &Path) -> Result<Process, Error> {
        Self::spawn_with(manifest, sock_dir, Timeouts::default())
    }

    pub fn spawn_with(manifest: Manifest, sock_dir: &Path, timeouts: Timeouts) -> Result<Process, Error> {
        if manifest.execution.kind != "process" {
            return Err(Error::invalid("execution kind must be process"));
        }
        if manifest.execution.entrypoint.is_empty() {
            return Err(Error::invalid("manifest has no entrypoint"));
        }
        if manifest.id.is_empty() || manifest.id.contains('/') || manifest.id.contains('\0') {
            return Err(Error::invalid("manifest id must be a plain name"));
        }
        let listener = Listener::bind(sock_dir, &manifest.id)?;
        let argv = manifest.argv(&listener.address());
        let shared = Arc::new(Shared {
            manifest,
            argv,
            timeouts,
            state: Mutex::new(State::default()),
            child: Mutex::new(None),
            closed: AtomicBool::new(false),
            listener,
        });
        let sup = shared.clone();
        let handle = std::thread::Builder::new()
            .name(format!("component-{}", sup.manifest.id))
            .spawn(move || sup.supervise())
            .map_err(|e| Error::internal(format!("supervisor thread: {e}")))?;
        Ok(Process { shared, supervisor: Mutex::new(Some(handle)) })
    }

    /// The manifest this process runs.
    pub fn manifest(&self) -> &Manifest {
        &self.shared.manifest
    }

    /// The running child's pid, if any (diagnostics and tests).
    pub fn pid(&self) -> Option<u32> {
        lock(&self.shared.child).as_ref().map(Child::id)
    }

    /// Kills the child without telling the supervisor, as a crash would
    /// (tests of the respawn path).
    #[doc(hidden)]
    pub fn kill_child_for_test(&self) {
        if let Some(c) = lock(&self.shared.child).as_mut() {
            let _ = c.kill();
        }
    }

    /// Whether the handshake completed and the connection is up.
    pub fn ready(&self) -> bool {
        lock(&self.shared.state).ready
    }

    /// Blocks until ready or the deadline passes.
    pub fn wait_ready(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.ready() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        self.ready()
    }

    /// Stops the supervisor, kills the child and removes the socket.
    /// Idempotent.
    pub fn close(&self) {
        // Only the first close removes the socket: a successor process
        // for the same id may already be bound to that path.
        if self.shared.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        {
            let mut st = lock(&self.shared.state);
            st.conn = None;
            st.ready = false;
        }
        self.shared.kill_child();
        if let Some(h) = lock(&self.supervisor).take() {
            let _ = h.join();
        }
        self.shared.listener.cleanup();
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        self.close();
    }
}

impl Shared {
    fn closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    fn kill_child(&self) {
        if let Some(mut c) = lock(&self.child).take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    fn fail(&self, msg: &str) {
        let mut st = lock(&self.state);
        st.ready = false;
        st.conn = None;
        st.last = msg.to_string();
    }

    /// Sleeps in small steps so close() is prompt; false once closed.
    fn sleep(&self, d: Duration) -> bool {
        let deadline = Instant::now() + d;
        while Instant::now() < deadline {
            if self.closed() {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20).min(deadline.saturating_duration_since(Instant::now())));
        }
        !self.closed()
    }

    fn next_backoff(&self, d: Duration) -> Duration {
        (d * 2).min(self.timeouts.backoff_max)
    }

    fn child_exited(&self) -> bool {
        match lock(&self.child).as_mut() {
            Some(c) => !matches!(c.try_wait(), Ok(None)),
            None => true,
        }
    }

    fn check_hello(&self, h: &Hello) -> bool {
        h.op == "hello"
            && h.protocol == PROTOCOL_VERSION
            && h.provider == self.manifest.id
            && self.manifest.capabilities.iter().all(|c| h.capabilities.contains(c))
    }

    /// Owns the child lifecycle: spawn, handshake, wait, respawn.
    fn supervise(&self) {
        let mut backoff = self.timeouts.backoff_start;
        while !self.closed() {
            let spawned = Command::new(&self.argv[0]).args(&self.argv[1..]).stdin(Stdio::null()).spawn();
            match spawned {
                Ok(child) => *lock(&self.child) = Some(child),
                Err(e) => {
                    tracing::warn!(id = %self.manifest.id, err = %e, "component spawn failed");
                    self.fail(&e.to_string());
                    if !self.sleep(backoff) {
                        return;
                    }
                    backoff = self.next_backoff(backoff);
                    continue;
                }
            }
            let deadline = Instant::now() + self.timeouts.handshake;
            let mut conn = match self.listener.accept(deadline, &|| self.closed() || self.child_exited()) {
                Ok(c) => c,
                Err(e) => {
                    self.kill_child();
                    if self.closed() {
                        return;
                    }
                    self.fail(&e);
                    if !self.sleep(backoff) {
                        return;
                    }
                    backoff = self.next_backoff(backoff);
                    continue;
                }
            };
            let hello = conn
                .read_frame(Some(self.timeouts.handshake))
                .ok()
                .and_then(|raw| serde_json::from_slice::<Hello>(&raw).ok());
            if !hello.as_ref().is_some_and(|h| self.check_hello(h)) {
                tracing::warn!(id = %self.manifest.id, "component handshake failed");
                self.kill_child();
                self.fail("handshake failed");
                if !self.sleep(backoff) {
                    return;
                }
                backoff = self.next_backoff(backoff);
                continue;
            }
            backoff = self.timeouts.backoff_start;
            {
                let mut st = lock(&self.state);
                st.conn = Some(conn);
                st.ready = true;
                st.last.clear();
            }
            tracing::info!(id = %self.manifest.id, "component ready");
            // Wait for the child to exit (or for close).
            while !self.closed() && !self.child_exited() {
                std::thread::sleep(Duration::from_millis(25));
            }
            {
                let mut st = lock(&self.state);
                st.conn = None;
                st.ready = false;
                if !self.closed() {
                    st.last = "component exited".into();
                }
            }
            self.kill_child();
            if self.closed() {
                return;
            }
            tracing::warn!(id = %self.manifest.id, "component exited; respawning");
            if !self.sleep(backoff) {
                return;
            }
            backoff = self.next_backoff(backoff);
        }
    }

    /// Drops a broken connection and kills the child; the supervisor
    /// respawns a clean peer instead of risking a desynchronized stream.
    fn reset(&self, st: &mut State) {
        st.conn = None;
        st.ready = false;
        st.last = "connection reset".into();
        if let Some(c) = lock(&self.child).as_mut() {
            let _ = c.kill();
        }
    }

    /// One request/response round trip; the caller holds the state lock.
    fn round_trip(&self, st: &mut State, op: &str, cap: &str, input: Value, timeout: Duration) -> Result<Value, Error> {
        st.seq += 1;
        let req = Request { seq: st.seq, cap: cap.to_string(), op: op.to_string(), input };
        let Some(conn) = st.conn.as_mut() else {
            return Err(Error::unavailable("component down"));
        };
        let frame = serde_json::to_vec(&req).map_err(|e| Error::invalid(e.to_string()))?;
        if let Err(e) = conn.write_frame(&frame, Some(timeout)) {
            self.reset(st);
            return Err(Error::unavailable(format!("component write: {e}")));
        }
        let raw = match conn.read_frame(Some(timeout)) {
            Ok(raw) => raw,
            Err(e) => {
                self.reset(st);
                if is_timeout(&e) {
                    return Err(Error::new(
                        codes::OUTCOME_UNKNOWN,
                        "component call timed out; it may or may not have run",
                    ));
                }
                return Err(Error::unavailable(format!("component read: {e}")));
            }
        };
        let resp: Response = match serde_json::from_slice(&raw) {
            Ok(r) => r,
            Err(e) => {
                self.reset(st);
                return Err(Error::internal(format!("bad component frame: {e}")));
            }
        };
        if resp.seq != req.seq {
            self.reset(st);
            return Err(Error::internal("component replied out of order"));
        }
        if let Some(e) = resp.error {
            return Err(Error { code: e.code, msg: e.msg });
        }
        Ok(resp.output.unwrap_or(Value::Null))
    }
}

impl Provider for Process {
    fn id(&self) -> &str {
        &self.shared.manifest.id
    }

    fn capabilities(&self) -> Vec<String> {
        self.shared.manifest.capabilities.clone()
    }

    /// A liveness round trip, or the last lifecycle error when down.
    fn health(&self) -> Result<(), Error> {
        let mut st = lock(&self.shared.state);
        if !st.ready {
            return Err(Error::unavailable(if st.last.is_empty() {
                "component down".to_string()
            } else {
                format!("component down: {}", st.last)
            }));
        }
        self.shared
            .round_trip(&mut st, HEALTH_OP, "", Value::Null, self.shared.timeouts.health)
            .map(|_| ())
            .map_err(|e| Error::unavailable(format!("component unhealthy: {e}")))
    }

    fn invoke(&self, cap: &str, op: &str, input: Value) -> Result<Value, Error> {
        let mut st = lock(&self.shared.state);
        if !st.ready {
            return Err(Error::unavailable("component down"));
        }
        let timeout = self.shared.timeouts.invoke;
        self.shared.round_trip(&mut st, op, cap, input, timeout)
    }
}
