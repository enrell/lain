//! Backups (D-076, D-197). The trusted core takes a consistent
//! snapshot of the live store (one read transaction copied into a fresh
//! redb file, so backups run while the server writes) and hands it, with
//! the data dir's documents and files, to `lain.backup.create@1`, which
//! owns what the artifact becomes. The built-in `lain-backup-bundle`
//! writes one uncompressed tar (`lain.backup@1`):
//!
//! ```text
//! manifest.json      {format, created_at, server_version, records}
//! lain.redb          the store snapshot
//! composition.json   the live composition
//! modules.json       when the data dir has one
//! roots.json         when the data dir has one
//! avatars/<user id>  uploaded profile pictures
//! ```
//!
//! Restore: stop the server and extract the archive into an empty data
//! dir; nothing ever overwrites a live store.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use lain_core::{Error, Provider};
use lain_store::Store;

pub const CAP_BACKUP_CREATE: &str = "lain.backup.create@1";
pub const PROVIDER_ID: &str = "lain-backup-bundle";
pub const FORMAT: &str = "lain.backup@1";

/// The trusted-core handoff: a completed snapshot (never the locked
/// live file), JSON documents and files by archive name.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreateInput {
    pub snapshot_path: String,
    pub out_dir: String,
    #[serde(default)]
    pub docs: BTreeMap<String, Value>,
    #[serde(default)]
    pub files: BTreeMap<String, String>,
}

/// The artifact: `path` is what the gateway streams, `filename` what
/// the download is called.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateOutput {
    pub path: String,
    pub filename: String,
}

/// A scratch directory removed on drop (with everything in it).
#[derive(Debug)]
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> std::io::Result<TempDir> {
        let mut rnd = [0u8; 8];
        let nanos =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        rnd.copy_from_slice(&(nanos as u64 ^ u64::from(std::process::id()).rotate_left(32)).to_le_bytes());
        let p = std::env::temp_dir().join(format!("lain-{tag}-{}", u64::from_le_bytes(rnd)));
        lain_store::ensure_private_dir(&p)?;
        Ok(TempDir(p))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Snapshots `store` into `tmp` and gathers the handoff: the manifest,
/// the composition (as the registry holds it), the data dir's optional
/// documents and the avatars.
pub fn prepare(
    store: &Store,
    data_dir: &Path,
    composition: Value,
    version: &str,
    tmp: &Path,
) -> Result<CreateInput, String> {
    let snapshot = tmp.join(lain_store::FILE_NAME);
    let records = store.snapshot(&snapshot).map_err(|e| e.to_string())?;
    let mut docs = BTreeMap::new();
    docs.insert(
        "manifest.json".to_string(),
        json!({
            "format": FORMAT,
            "created_at": lain_core::now_unix(),
            "server_version": version,
            "store_schema": lain_store::SCHEMA,
            "records": records,
        }),
    );
    docs.insert("composition.json".to_string(), composition);
    let mut files = BTreeMap::new();
    for name in ["modules.json", "roots.json"] {
        let p = data_dir.join(name);
        if p.is_file() {
            files.insert(name.to_string(), p.to_string_lossy().into_owned());
        }
    }
    let avatars = crate::avatars::Avatars::new(data_dir);
    if let Ok(rd) = std::fs::read_dir(avatars.dir()) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if e.path().is_file() && !name.ends_with(".tmp") {
                files.insert(format!("avatars/{name}"), e.path().to_string_lossy().into_owned());
            }
        }
    }
    Ok(CreateInput {
        snapshot_path: snapshot.to_string_lossy().into_owned(),
        out_dir: tmp.to_string_lossy().into_owned(),
        docs,
        files,
    })
}

/// `YYYYMMDD-HHMMSS` in UTC.
pub fn stamp(unix: i64) -> String {
    let (days, secs) = (unix.div_euclid(86_400), unix.rem_euclid(86_400));
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}", secs / 3600, secs % 3600 / 60, secs % 60)
}

// ---- tar (ustar) -------------------------------------------------------------

fn octal(field: &mut [u8], v: u64) -> Result<(), String> {
    let s = format!("{v:0w$o}", w = field.len() - 1);
    if s.len() > field.len() - 1 {
        return Err(format!("{v} does not fit a tar header"));
    }
    field[..s.len()].copy_from_slice(s.as_bytes());
    field[s.len()] = 0;
    Ok(())
}

fn header(name: &str, size: u64, mtime: i64) -> Result<[u8; 512], String> {
    if name.len() > 100 || name.is_empty() {
        return Err(format!("tar name {name:?} out of range"));
    }
    let mut h = [0u8; 512];
    h[..name.len()].copy_from_slice(name.as_bytes());
    octal(&mut h[100..108], 0o600)?;
    octal(&mut h[108..116], 0)?;
    octal(&mut h[116..124], 0)?;
    octal(&mut h[124..136], size)?;
    octal(&mut h[136..148], mtime.max(0) as u64)?;
    h[148..156].copy_from_slice(b"        ");
    h[156] = b'0';
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    let sum: u32 = h.iter().map(|&b| u32::from(b)).sum();
    let s = format!("{sum:06o}\0 ");
    h[148..156].copy_from_slice(s.as_bytes());
    Ok(h)
}

/// A tar writer over any `Write`.
pub struct Tar<W: Write> {
    w: W,
    mtime: i64,
}

impl<W: Write> Tar<W> {
    pub fn new(w: W, mtime: i64) -> Tar<W> {
        Tar { w, mtime }
    }

    pub fn add_bytes(&mut self, name: &str, data: &[u8]) -> Result<(), String> {
        self.add_reader(name, data.len() as u64, &mut &data[..])
    }

    pub fn add_file(&mut self, name: &str, path: &Path) -> Result<(), String> {
        let f = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let size = f.metadata().map_err(|e| e.to_string())?.len();
        self.add_reader(name, size, &mut f.take(size))
    }

    fn add_reader(&mut self, name: &str, size: u64, r: &mut dyn Read) -> Result<(), String> {
        let e = |e: std::io::Error| e.to_string();
        self.w.write_all(&header(name, size, self.mtime)?).map_err(e)?;
        let n = std::io::copy(r, &mut self.w).map_err(e)?;
        if n != size {
            return Err(format!("{name} changed size while archived"));
        }
        let pad = (512 - size % 512) % 512;
        self.w.write_all(&vec![0u8; pad as usize]).map_err(e)
    }

    pub fn finish(mut self) -> Result<W, String> {
        self.w.write_all(&[0u8; 1024]).map_err(|e| e.to_string())?;
        self.w.flush().map_err(|e| e.to_string())?;
        Ok(self.w)
    }
}

/// Reads a tar back into (name, bytes) pairs (tests and restore checks).
pub fn read_tar(raw: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 512 <= raw.len() {
        let h = &raw[i..i + 512];
        if h.iter().all(|&b| b == 0) {
            break;
        }
        let name = String::from_utf8_lossy(&h[..100]).trim_end_matches('\0').to_string();
        let size_s = String::from_utf8_lossy(&h[124..135]).trim_matches(['\0', ' ']).to_string();
        let size = u64::from_str_radix(&size_s, 8).map_err(|e| format!("{name}: size {e}"))? as usize;
        let start = i + 512;
        if start + size > raw.len() {
            return Err(format!("{name}: truncated"));
        }
        out.push((name, raw[start..start + size].to_vec()));
        i = start + size.div_ceil(512) * 512;
    }
    Ok(out)
}

/// `lain.backup.create@1`: the tar bundle.
pub struct Bundle;

impl Bundle {
    pub fn create(&self, i: &CreateInput) -> Result<CreateOutput, Error> {
        let snap = Path::new(&i.snapshot_path);
        match std::fs::metadata(snap) {
            Ok(m) if m.is_file() && m.len() > 0 => {}
            _ => return Err(Error::unavailable("snapshot missing or empty")),
        }
        let now = lain_core::now_unix();
        let filename = format!("lain-backup-{}.tar", stamp(now));
        let path = Path::new(&i.out_dir).join(&filename);
        let fail = |e: String| Error::internal(format!("backup: {e}"));
        let f = File::create(&path).map_err(|e| fail(e.to_string()))?;
        let mut tar = Tar::new(std::io::BufWriter::new(f), now);
        if let Some(m) = i.docs.get("manifest.json") {
            tar.add_bytes("manifest.json", &serde_json::to_vec_pretty(m).unwrap_or_default()).map_err(fail)?;
        }
        tar.add_file(lain_store::FILE_NAME, snap).map_err(fail)?;
        for (name, doc) in i.docs.iter().filter(|(n, _)| *n != "manifest.json") {
            let mut raw = serde_json::to_vec_pretty(doc).unwrap_or_default();
            raw.push(b'\n');
            tar.add_bytes(name, &raw).map_err(fail)?;
        }
        for (name, src) in &i.files {
            tar.add_file(name, Path::new(src)).map_err(fail)?;
        }
        tar.finish().map_err(fail)?;
        Ok(CreateOutput { path: path.to_string_lossy().into_owned(), filename })
    }
}

impl Provider for Bundle {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn capabilities(&self) -> Vec<String> {
        vec![CAP_BACKUP_CREATE.into()]
    }

    fn health(&self) -> Result<(), Error> {
        Ok(())
    }

    fn invoke(&self, cap: &str, _op: &str, input: Value) -> Result<Value, Error> {
        if cap != CAP_BACKUP_CREATE {
            return Err(Error::invalid(format!("unsupported cap {cap}")));
        }
        let i: CreateInput = serde_json::from_value(input).map_err(|_| Error::invalid("CreateInput required"))?;
        Ok(json!(self.create(&i)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lain_store::Bucket;

    #[test]
    fn stamps_are_utc_civil_dates() {
        assert_eq!(stamp(0), "19700101-000000");
        assert_eq!(stamp(951_782_400), "20000229-000000");
        assert_eq!(stamp(1_794_173_709), "20261108-213509");
    }

    #[test]
    fn bundle_round_trips_and_restores() {
        let root = lain_store::test_dir("backup");
        let data = root.join("data");
        let store = Store::open(&data).unwrap();
        store.write(|tx| tx.put(Bucket::Users, b"user-a", b"{\"id\":\"user-a\"}")).unwrap();
        std::fs::write(data.join("roots.json"), b"{\"format\":\"lain.roots@1\",\"roots\":{}}\n").unwrap();
        let av = crate::avatars::Avatars::new(&data);
        av.save("user-a", &lain_read::image::test_png(4, 4)).unwrap();

        let tmp = TempDir::new("backup-test").unwrap();
        let input = prepare(&store, &data, json!({"version": 8}), "0.1.0-rs", tmp.path()).unwrap();
        let out: CreateOutput =
            serde_json::from_value(Bundle.invoke(CAP_BACKUP_CREATE, "", json!(input)).unwrap()).unwrap();
        assert!(out.filename.starts_with("lain-backup-") && out.filename.ends_with(".tar"));
        let entries = read_tar(&std::fs::read(&out.path).unwrap()).unwrap();
        let names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["manifest.json", "lain.redb", "composition.json", "avatars/user-a", "roots.json"]);
        let manifest: Value = serde_json::from_slice(&entries[0].1).unwrap();
        assert_eq!((manifest["format"].as_str(), manifest["records"].as_u64()), (Some(FORMAT), Some(2)));

        // Extracting into an empty dir gives a data dir that opens.
        let restored = root.join("restored");
        for (n, b) in &entries {
            let p = restored.join(n);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b).unwrap();
        }
        let copy = Store::open(&restored).unwrap();
        assert!(copy.read(|tx| tx.get(Bucket::Users, b"user-a")).unwrap().is_some());
        assert_eq!(crate::avatars::Avatars::new(&restored).read("user-a").unwrap().1, "image/png");

        let gone = tmp.path().to_path_buf();
        drop(tmp);
        assert!(!gone.exists(), "the scratch dir goes with its guard");
        assert_eq!(
            Bundle
                .create(&CreateInput { snapshot_path: "/nonexistent".into(), ..Default::default() })
                .unwrap_err()
                .code,
            "dependency-unavailable"
        );
        drop((store, copy));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn tar_headers() {
        let mut out = Vec::new();
        let mut t = Tar::new(&mut out, 1);
        t.add_bytes("a.txt", b"hello").unwrap();
        assert!(t.add_bytes(&"x".repeat(101), b"").is_err());
        t.finish().unwrap();
        assert_eq!(out.len(), 512 + 512 + 1024, "header, padded data, end; a refused name writes nothing");
    }
}
