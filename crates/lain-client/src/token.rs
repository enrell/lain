//! Where a client keeps its login: a [`Session`] per server behind the
//! [`TokenStore`] trait. Shipped: [`FileTokenStore`] (an owner-only JSON
//! file) and [`MemoryTokenStore`]. The OS keychain is a later
//! implementation of the same trait (it needs a crate; Q-045).

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::error::{ClientError, store_err};

/// One login on one server. `server` is the canonical server URL.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub server: String,
    pub token: String,
    pub user_id: String,
    pub username: String,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("server", &self.server)
            .field("token", &"<redacted>")
            .field("user_id", &self.user_id)
            .field("username", &self.username)
            .finish()
    }
}

/// Keeps sessions by server. Implementations must be safe to call from
/// any thread; calls are small and synchronous.
pub trait TokenStore: Send + Sync {
    fn load(&self, server: &str) -> Result<Option<Session>, ClientError>;
    fn save(&self, session: &Session) -> Result<(), ClientError>;
    /// Forgets the server's session; a missing one is not an error.
    fn clear(&self, server: &str) -> Result<(), ClientError>;
}

/// Sessions for this process only (tests, kiosks, "do not remember me").
#[derive(Default)]
pub struct MemoryTokenStore {
    sessions: Mutex<BTreeMap<String, Session>>,
}

impl MemoryTokenStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl TokenStore for MemoryTokenStore {
    fn load(&self, server: &str) -> Result<Option<Session>, ClientError> {
        Ok(self.sessions.lock().unwrap_or_else(|e| e.into_inner()).get(server).cloned())
    }
    fn save(&self, session: &Session) -> Result<(), ClientError> {
        self.sessions.lock().unwrap_or_else(|e| e.into_inner()).insert(session.server.clone(), session.clone());
        Ok(())
    }
    fn clear(&self, server: &str) -> Result<(), ClientError> {
        self.sessions.lock().unwrap_or_else(|e| e.into_inner()).remove(server);
        Ok(())
    }
}

/// The file's document.
#[derive(Default, Serialize, Deserialize)]
struct Doc {
    format: String,
    #[serde(default)]
    sessions: BTreeMap<String, Session>,
}

const FORMAT: &str = "lain.client-sessions@1";

/// Sessions in one JSON file (`lain.client-sessions@1`). On Unix the
/// file is created 0600 in a 0700 directory, and a file found readable
/// by others is tightened back to 0600. On Windows it lives in the
/// user's profile (`%LOCALAPPDATA%`), whose ACL already limits it to
/// that user. Writes are atomic (temp file + rename).
pub struct FileTokenStore {
    path: PathBuf,
    lock: Mutex<()>,
}

impl FileTokenStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        FileTokenStore { path: path.into(), lock: Mutex::new(()) }
    }

    /// `<client dir>/sessions.json` (see [`crate::client_dir`]).
    pub fn open_default() -> Result<Self, ClientError> {
        let dir = crate::client_dir().ok_or_else(|| ClientError::Store { message: "no user data directory".into() })?;
        Ok(Self::new(dir.join("sessions.json")))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn read(&self) -> Result<Doc, ClientError> {
        let raw = match std::fs::read(&self.path) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Doc { format: FORMAT.into(), ..Doc::default() });
            }
            Err(e) => return Err(store_err(format!("{}: {e}", self.path.display()))),
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(m) = std::fs::metadata(&self.path)
                && m.permissions().mode() & 0o077 != 0
            {
                tracing::warn!(path = %self.path.display(), "session file was readable by others; set to 0600");
                let _ = std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600));
            }
        }
        let doc: Doc = serde_json::from_slice(&raw).map_err(|e| store_err(format!("{}: {e}", self.path.display())))?;
        if doc.format != FORMAT {
            return Err(store_err(format!("{}: format {:?} is not {FORMAT}", self.path.display(), doc.format)));
        }
        Ok(doc)
    }

    fn write(&self, doc: &Doc) -> Result<(), ClientError> {
        let dir = self.path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
        private_dir(dir).map_err(|e| store_err(format!("{}: {e}", dir.display())))?;
        let mut raw = serde_json::to_vec_pretty(doc).map_err(store_err)?;
        raw.push(b'\n');
        let tmp = self.path.with_extension(format!("tmp-{}", std::process::id()));
        let res = (|| {
            let mut f = private_file(&tmp)?;
            f.write_all(&raw)?;
            f.sync_all()?;
            drop(f);
            std::fs::rename(&tmp, &self.path)
        })();
        if let Err(e) = res {
            let _ = std::fs::remove_file(&tmp);
            return Err(store_err(format!("{}: {e}", self.path.display())));
        }
        Ok(())
    }
}

impl TokenStore for FileTokenStore {
    fn load(&self, server: &str) -> Result<Option<Session>, ClientError> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        Ok(self.read()?.sessions.remove(server))
    }

    fn save(&self, session: &Session) -> Result<(), ClientError> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut doc = self.read()?;
        doc.sessions.insert(session.server.clone(), session.clone());
        self.write(&doc)
    }

    fn clear(&self, server: &str) -> Result<(), ClientError> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut doc = self.read()?;
        if doc.sessions.remove(server).is_some() {
            self.write(&doc)?;
        }
        Ok(())
    }
}

/// Creates `dir` owner-only (0700 on Unix).
pub(crate) fn private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = std::fs::metadata(dir)?.permissions();
        if perm.mode() & 0o077 != 0 {
            perm.set_mode(0o700);
            std::fs::set_permissions(dir, perm)?;
        }
    }
    Ok(())
}

fn private_file(path: &Path) -> std::io::Result<std::fs::File> {
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(path)
}
