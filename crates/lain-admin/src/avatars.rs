//! Uploaded profile pictures (D-086, D-199): one file per user under
//! `<data dir>/avatars/`, named by the user id only, so a request can
//! never address a path. A picture is identified by its content (PNG,
//! JPEG, GIF or WebP up to 4096 px per side), never by the client's
//! Content-Type.

use std::path::{Path, PathBuf};

/// Uploads are at most this many bytes.
pub const MAX_BYTES: usize = 2 << 20;

/// Decoded sides are at most this many pixels: a tiny file declaring a
/// huge canvas is refused before any client tries to render it.
pub const MAX_SIDE: u32 = 4096;

/// A refused or failed upload: the HTTP status and Go's message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvatarError {
    pub status: u16,
    pub msg: &'static str,
}

/// The avatar store.
#[derive(Debug, Clone)]
pub struct Avatars {
    dir: PathBuf,
}

fn safe_id(id: &str) -> bool {
    !id.is_empty() && id != "." && id != ".." && !id.contains(['/', '\\', '\0'])
}

impl Avatars {
    pub fn new(data_dir: &Path) -> Avatars {
        Avatars { dir: data_dir.join("avatars") }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, id: &str) -> Option<PathBuf> {
        safe_id(id).then(|| self.dir.join(id))
    }

    /// Validates and stores a picture (temp file + rename, 0600).
    pub fn save(&self, id: &str, body: &[u8]) -> Result<(), AvatarError> {
        if body.len() > MAX_BYTES {
            return Err(AvatarError { status: 413, msg: "the image must be 2 MB or smaller" });
        }
        if lain_read::image::sniff(body, MAX_SIDE).is_none() {
            return Err(AvatarError { status: 415, msg: "use a PNG, JPEG, GIF or WebP image up to 4096 px per side" });
        }
        let unavailable = AvatarError { status: 500, msg: "avatar storage unavailable" };
        let path = self.path(id).ok_or(unavailable.clone())?;
        lain_store::ensure_private_dir(&self.dir).map_err(|_| unavailable.clone())?;
        let tmp = self.dir.join(format!("{id}.tmp"));
        std::fs::write(&tmp, body).map_err(|_| unavailable.clone())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(&tmp, &path).map_err(|_| {
            let _ = std::fs::remove_file(&tmp);
            unavailable
        })
    }

    /// Removes a user's picture; a missing one is not an error.
    pub fn remove(&self, id: &str) {
        if let Some(p) = self.path(id)
            && let Err(e) = std::fs::remove_file(&p)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(user = %id, err = %e, "avatar not removed");
        }
    }

    /// A stored picture and its sniffed mime type.
    pub fn read(&self, id: &str) -> Option<(Vec<u8>, &'static str)> {
        let body = std::fs::read(self.path(id)?).ok()?;
        let mime = lain_read::image::sniff(&body, MAX_SIDE)?;
        Some((body, mime))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lain_read::image::test_png;

    #[test]
    fn save_read_replace_remove() {
        let dir = lain_store::test_dir("avatars");
        let a = Avatars::new(&dir);
        assert_eq!(a.read("user-a"), None);
        a.save("user-a", &test_png(64, 64)).unwrap();
        assert_eq!(a.read("user-a"), Some((test_png(64, 64), "image/png")));
        a.save("user-a", &test_png(8, 8)).unwrap();
        assert_eq!(a.read("user-a").unwrap().0, test_png(8, 8));
        a.remove("user-a");
        a.remove("user-a");
        assert_eq!(a.read("user-a"), None);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn rejects() {
        let dir = lain_store::test_dir("avatars-bad");
        let a = Avatars::new(&dir);
        assert_eq!(a.save("u", b"<svg onload=alert(1)>").unwrap_err().status, 415);
        assert_eq!(a.save("u", &test_png(5000, 1)).unwrap_err().status, 415);
        assert_eq!(a.save("u", &vec![0u8; MAX_BYTES + 1]).unwrap_err().status, 413);
        assert_eq!(a.save("../x", &test_png(1, 1)).unwrap_err().status, 500);
        assert_eq!(a.read("../x"), None);
        std::fs::remove_dir_all(dir).ok();
    }
}
