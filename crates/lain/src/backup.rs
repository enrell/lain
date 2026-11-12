//! `lain backup` and `lain restore` (D-238).
//!
//! A backup is the `lain.backup@1` tar the server's
//! `lain.backup.create@1` writes (`lain-admin`): the manifest, a
//! consistent snapshot of the store, the composition, `modules.json`,
//! `roots.json` and the avatars. Online first: with a saved login and a
//! server that answers, the tar streams from `GET /api/admin/backup`
//! (one read transaction, so it works mid-scan). Otherwise the data dir
//! is read directly, which needs the server stopped (the store is
//! locked while it runs). Restore extracts a tar into an empty data dir;
//! it never overwrites a store: `--replace` moves a stopped server's
//! data dir aside first and keeps it.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use lain_admin::backup as bundle;

use crate::args::{flag, has, positional};
use crate::client::{Session, runtime};
use crate::server::data_dir;

/// The names a backup may hold; anything else is refused on restore.
fn allowed(name: &str) -> bool {
    match name {
        "manifest.json" | "composition.json" | "modules.json" | "roots.json" => true,
        n if n == lain_store::FILE_NAME => true,
        n => n.strip_prefix("avatars/").is_some_and(|a| {
            !a.is_empty() && !a.contains(['/', '\\', '\0']) && a != "." && a != ".." && !a.starts_with('.')
        }),
    }
}

type Entries = Vec<(String, Vec<u8>)>;

/// Checks a backup tar and returns its manifest and entries.
pub fn validate(raw: &[u8]) -> Result<(Value, Entries), String> {
    let files = bundle::read_tar(raw)?;
    let manifest = files.iter().find(|(n, _)| n == "manifest.json").ok_or("not a lain backup: no manifest.json")?;
    let m: Value = serde_json::from_slice(&manifest.1).map_err(|e| format!("manifest.json: {e}"))?;
    if m["format"] != bundle::FORMAT {
        return Err(format!("manifest format {} is not {}", m["format"], bundle::FORMAT));
    }
    if !files.iter().any(|(n, b)| n == lain_store::FILE_NAME && !b.is_empty()) {
        return Err(format!("backup has no {}", lain_store::FILE_NAME));
    }
    if let Some((bad, _)) = files.iter().find(|(n, _)| !allowed(n)) {
        return Err(format!("backup holds an unexpected entry {bad:?}"));
    }
    Ok((m, files))
}

/// Where the tar goes: inside `out` when it is (or will be) a
/// directory, else `out` itself.
fn destination(out: &str, filename: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(out);
    if p.is_dir() || out.ends_with('/') || out.ends_with(std::path::MAIN_SEPARATOR) || p.extension().is_none() {
        lain_store::ensure_private_dir(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        return Ok(p.join(filename));
    }
    if let Some(parent) = p.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    Ok(p)
}

fn write_new(path: &Path, raw: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let mut f = o.open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    f.write_all(raw).and_then(|_| f.sync_all()).map_err(|e| format!("{}: {e}", path.display()))
}

fn summary(m: &Value) -> String {
    format!("{} records, server {}", m["records"], m["server_version"].as_str().unwrap_or("?"))
}

pub fn backup(args: &[String]) -> Result<(), String> {
    let out = flag(args, "out").unwrap_or_else(|| "backups".into());
    let session = Session::open(args);
    if let Ok(s) = &session {
        match online(s, &out) {
            Ok(()) => return Ok(()),
            Err(e) => eprintln!("online backup unavailable ({e}); trying offline"),
        }
    }
    offline(args, &out)
}

fn online(s: &Session, out: &str) -> Result<(), String> {
    let rt = runtime()?;
    rt.block_on(s.client.health()).map_err(|e| e.to_string())?;
    let raw = rt.block_on(s.client.backup()).map_err(|e| e.to_string())?;
    let (m, _) = validate(&raw)?;
    let name = format!("lain-backup-{}.tar", bundle::stamp(m["created_at"].as_i64().unwrap_or(0)));
    let path = destination(out, &name)?;
    write_new(&path, &raw)?;
    println!("backup (online): {} ({})", path.display(), summary(&m));
    Ok(())
}

fn offline(args: &[String], out: &str) -> Result<(), String> {
    let dir = data_dir(args)?;
    if !dir.join(lain_store::FILE_NAME).is_file() {
        return Err(format!("no lain store in {} (and no server to ask; `lain login` first)", dir.display()));
    }
    let store = lain_store::Store::open(&dir).map_err(|e| {
        format!(
            "{}: {e} (a running server holds the store: `lain login` for an online backup, or stop it)",
            dir.display()
        )
    })?;
    let comp = match std::fs::read(dir.join("composition.json")) {
        Ok(raw) => serde_json::from_slice(&raw).map_err(|e| format!("composition.json: {e}"))?,
        Err(_) => json!({}),
    };
    let tmp = bundle::TempDir::new("backup").map_err(|e| e.to_string())?;
    let input = bundle::prepare(&store, &dir, comp, lain_server::VERSION, tmp.path())?;
    let artifact = bundle::Bundle.create(&input).map_err(|e| e.to_string())?;
    let raw = std::fs::read(&artifact.path).map_err(|e| format!("{}: {e}", artifact.path))?;
    let (m, _) = validate(&raw)?;
    let path = destination(out, &artifact.filename)?;
    write_new(&path, &raw)?;
    println!("backup (offline): {} ({})", path.display(), summary(&m));
    Ok(())
}

pub fn restore(args: &[String]) -> Result<(), String> {
    let [archive] =
        positional(args).try_into().map_err(|_| "usage: lain restore BACKUP.tar [--data-dir DIR] [--replace]")?;
    let dir = data_dir(args)?;
    if !has(args, "replace") {
        restore_into(Path::new(&archive), &dir)?;
        println!("restored {archive} into {}", dir.display());
        return Ok(());
    }
    let aside = replace_into(Path::new(&archive), &dir)?;
    println!("restored {archive} into {}", dir.display());
    if let Some(old) = aside {
        println!("the previous data is in {}; delete it once everything is there", old.display());
    }
    Ok(())
}

/// `restore --replace`: one command instead of moving the live data
/// aside by hand. The archive is checked first, the store must not be
/// held by a running server, and the old data dir is kept beside the
/// new one (`<dir>.before-restore`, numbered when taken), never
/// deleted; a restore that fails puts it back. Answers where it went
/// (`None` when there was nothing to move).
pub fn replace_into(archive: &Path, dir: &Path) -> Result<Option<PathBuf>, String> {
    let raw = std::fs::read(archive).map_err(|e| format!("{}: {e}", archive.display()))?;
    validate(&raw)?;
    let empty = std::fs::read_dir(dir).map(|mut rd| rd.next().is_none()).unwrap_or(true);
    if empty {
        restore_into(archive, dir)?;
        return Ok(None);
    }
    if dir.join(lain_store::FILE_NAME).is_file() {
        // redb locks the file while a server has it open. Any other
        // failure (a damaged store) is a reason to restore, not to stop.
        if let Err(e) = lain_store::Store::open(dir)
            && e.to_string().to_lowercase().contains("already open")
        {
            return Err(format!("{}: {e} (a running server holds the store: stop it first)", dir.display()));
        }
    }
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "lain".into());
    let aside = (1..1000)
        .map(|n| {
            let suffix = if n == 1 { String::new() } else { format!("-{n}") };
            dir.with_file_name(format!("{name}.before-restore{suffix}"))
        })
        .find(|p| !p.exists())
        .ok_or_else(|| format!("no free name beside {} to move it to", dir.display()))?;
    std::fs::rename(dir, &aside).map_err(|e| format!("move {} aside: {e}", dir.display()))?;
    if let Err(e) = restore_into(archive, dir) {
        let _ = std::fs::remove_dir_all(dir);
        let back = std::fs::rename(&aside, dir);
        return Err(match back {
            Ok(()) => format!("{e} (the previous data is back in {})", dir.display()),
            Err(b) => format!("{e} (the previous data is in {}: {b})", aside.display()),
        });
    }
    Ok(Some(aside))
}

/// Extracts a backup into `dir`, which must be empty or absent.
pub fn restore_into(archive: &Path, dir: &Path) -> Result<(), String> {
    let raw = std::fs::read(archive).map_err(|e| format!("{}: {e}", archive.display()))?;
    let (_, files) = validate(&raw)?;
    if let Ok(mut rd) = std::fs::read_dir(dir)
        && rd.next().is_some()
    {
        return Err(format!("{} is not empty; restore only into an empty data dir", dir.display()));
    }
    lain_store::ensure_private_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for (name, bytes) in files.iter().filter(|(n, _)| n != "manifest.json") {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            lain_store::ensure_private_dir(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        write_new(&path, bytes)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tar(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut t = bundle::Tar::new(Vec::new(), 0);
        for (n, b) in entries {
            t.add_bytes(n, b).unwrap();
        }
        t.finish().unwrap()
    }

    fn manifest() -> Vec<u8> {
        serde_json::to_vec(&json!({"format": "lain.backup@1", "records": 3, "created_at": 0})).unwrap()
    }

    #[test]
    fn names_are_checked() {
        for ok in ["manifest.json", "lain.redb", "roots.json", "avatars/user-1"] {
            assert!(allowed(ok), "{ok}");
        }
        for bad in ["../x", "avatars/", "avatars/../lain.redb", "avatars/a/b", "etc/passwd", "avatars/.tmp", "x.json"] {
            assert!(!allowed(bad), "{bad}");
        }
    }

    #[test]
    fn restore_needs_an_empty_dir_and_a_real_backup() {
        let root = std::env::temp_dir().join(format!("lain-restore-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let m = manifest();
        let good = root.join("good.tar");
        std::fs::write(&good, tar(&[("manifest.json", &m), ("lain.redb", b"db"), ("avatars/u1", b"png")])).unwrap();
        restore_into(&good, &root.join("data")).unwrap();
        assert_eq!(std::fs::read(root.join("data/lain.redb")).unwrap(), b"db");
        assert_eq!(std::fs::read(root.join("data/avatars/u1")).unwrap(), b"png");
        assert!(!root.join("data/manifest.json").exists());
        let e = restore_into(&good, &root.join("data")).unwrap_err();
        assert!(e.contains("not empty"), "{e}");

        let evil = root.join("evil.tar");
        std::fs::write(&evil, tar(&[("manifest.json", &m), ("lain.redb", b"db"), ("../escape", b"x")])).unwrap();
        assert!(restore_into(&evil, &root.join("d2")).unwrap_err().contains("unexpected entry"));
        let nodb = root.join("nodb.tar");
        std::fs::write(&nodb, tar(&[("manifest.json", &m)])).unwrap();
        assert!(restore_into(&nodb, &root.join("d3")).unwrap_err().contains("lain.redb"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// `--replace`: the live dir goes aside (kept, numbered when taken),
    /// a held store is refused, and a bad archive moves nothing.
    #[test]
    fn replace_keeps_the_old_data_beside_the_new() {
        let root = std::env::temp_dir().join(format!("lain-replace-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let m = manifest();
        let good = root.join("good.tar");
        std::fs::write(&good, tar(&[("manifest.json", &m), ("lain.redb", b"db")])).unwrap();
        let data = root.join("data");
        assert_eq!(replace_into(&good, &data).unwrap(), None, "nothing to move: a plain restore");
        assert_eq!(replace_into(&good, &data).unwrap(), Some(root.join("data.before-restore")));
        assert_eq!(std::fs::read(root.join("data.before-restore/lain.redb")).unwrap(), b"db");
        assert_eq!(replace_into(&good, &data).unwrap(), Some(root.join("data.before-restore-2")));
        let nodb = root.join("nodb.tar");
        std::fs::write(&nodb, tar(&[("manifest.json", &m)])).unwrap();
        assert!(replace_into(&nodb, &data).is_err());
        assert!(data.join("lain.redb").is_file() && !root.join("data.before-restore-3").exists());
        // A store a server holds stays where it is.
        let live = root.join("live");
        let held = lain_store::Store::open(&live).unwrap();
        let e = replace_into(&good, &live).unwrap_err();
        assert!(e.contains("stop it first"), "{e}");
        drop(held);
        assert!(!root.join("live.before-restore").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
