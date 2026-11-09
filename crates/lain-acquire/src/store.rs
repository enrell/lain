//! Acquire tables over `lain-store`: JSON records keyed by id (Go's
//! `acq_*` buckets, same names).

use std::sync::Arc;

use serde::Serialize;
use serde::de::DeserializeOwned;

use lain_api::acquire::{AcquireSettings, BlockEntry, Grab, Indexer, Monitored, Profile};
use lain_store::{Bucket, Store};

use crate::errf;
use crate::error::{CODE_NOT_FOUND, Result};

/// The engine's resume record per info hash.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct TorrentState {
    /// The raw `.torrent`, or one synthesized for a magnet.
    #[serde(default, skip_serializing_if = "Vec::is_empty", with = "b64")]
    pub metainfo: Vec<u8>,
    /// Restarts a magnet whose metadata had not arrived yet.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub magnet: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty", with = "b64")]
    pub have: Vec<u8>,
    pub dir: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub paused: bool,
}

/// Standard base64 for byte fields, as Go's `encoding/json` writes
/// `[]byte`.
mod b64 {
    use serde::{Deserialize, Deserializer, Serializer};

    const ALPHA: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub fn encode(b: &[u8]) -> String {
        let mut out = String::with_capacity(b.len().div_ceil(3) * 4);
        for c in b.chunks(3) {
            let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
            for i in 0..4 {
                if i <= c.len() {
                    out.push(ALPHA[(n >> (18 - 6 * i) & 63) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    pub fn decode(s: &str) -> Option<Vec<u8>> {
        let s = s.trim_end_matches('=');
        let mut out = Vec::with_capacity(s.len() * 3 / 4);
        let (mut acc, mut bits) = (0u32, 0);
        for c in s.bytes() {
            let v = ALPHA.iter().position(|a| *a == c)? as u32;
            acc = acc << 6 | v;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((acc >> bits) as u8);
            }
        }
        Some(out)
    }

    pub fn serialize<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&encode(v))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        decode(&s).ok_or_else(|| serde::de::Error::custom("bad base64"))
    }

    #[test]
    fn round_trips() {
        for n in 0..20 {
            let b: Vec<u8> = (0..n).map(|i| (i * 37 + 11) as u8).collect();
            assert_eq!(decode(&encode(&b)).unwrap(), b);
        }
        assert_eq!(encode(b"hi"), "aGk=");
    }
}

pub const SETTINGS_KEY: &[u8] = b"settings";

#[derive(Clone)]
pub struct AcqStore {
    pub db: Arc<Store>,
}

impl AcqStore {
    pub fn list<T: DeserializeOwned>(&self, b: Bucket) -> Vec<T> {
        self.db
            .read(|tx| tx.all(b))
            .unwrap_or_default()
            .iter()
            .filter_map(|(_, v)| serde_json::from_slice(v).ok())
            .collect()
    }

    pub fn raw(&self, b: Bucket) -> Vec<(Vec<u8>, Vec<u8>)> {
        self.db.read(|tx| tx.all(b)).unwrap_or_default()
    }

    pub fn get<T: DeserializeOwned>(&self, b: Bucket, id: &str, what: &str) -> Result<T> {
        match self.db.read(|tx| tx.get(b, id.as_bytes())) {
            Ok(Some(raw)) => serde_json::from_slice(&raw).map_err(|_| errf!(CODE_NOT_FOUND, "unknown {what}")),
            _ => Err(errf!(CODE_NOT_FOUND, "unknown {what}")),
        }
    }

    pub fn has(&self, b: Bucket, id: &str) -> bool {
        matches!(self.db.read(|tx| tx.get(b, id.as_bytes())), Ok(Some(_)))
    }

    pub fn put<T: Serialize>(&self, b: Bucket, id: &str, v: &T) -> Result<()> {
        Ok(self.db.write(|tx| tx.put_json(b, id.as_bytes(), v))?)
    }

    /// Deletes a record that must exist.
    pub fn delete(&self, b: Bucket, id: &str, what: &str) -> Result<()> {
        let found = self.db.write(|tx| tx.delete(b, id.as_bytes()))?;
        if found { Ok(()) } else { Err(errf!(CODE_NOT_FOUND, "unknown {what}")) }
    }

    pub fn remove(&self, b: Bucket, id: &str) {
        let _ = self.db.write(|tx| tx.delete(b, id.as_bytes()));
    }

    /// Deletes every record of a table; answers how many.
    pub fn clear(&self, b: Bucket) -> Result<i64> {
        Ok(self.db.write(|tx| {
            let n = tx.count(b)?;
            tx.clear(b)?;
            Ok(n as i64)
        })?)
    }

    pub fn settings(&self, def: AcquireSettings) -> AcquireSettings {
        let raw = self.db.read(|tx| tx.get(Bucket::AcqSettings, SETTINGS_KEY)).ok().flatten();
        let Some(raw) = raw else { return def };
        // Fields a newer version added keep their defaults.
        let mut base = serde_json::to_value(&def).unwrap_or_default();
        if let (Ok(serde_json::Value::Object(saved)), Some(obj)) =
            (serde_json::from_slice::<serde_json::Value>(&raw), base.as_object_mut())
        {
            obj.extend(saved);
        }
        serde_json::from_value(base).unwrap_or(def)
    }

    pub fn save_settings(&self, s: &AcquireSettings) -> Result<()> {
        Ok(self.db.write(|tx| tx.put_json(Bucket::AcqSettings, SETTINGS_KEY, s))?)
    }

    /// Indexers by priority, then name.
    pub fn indexers(&self) -> Vec<Indexer> {
        let mut out: Vec<Indexer> = self.list(Bucket::AcqIndexers);
        out.sort_by(|a, b| a.priority.cmp(&b.priority).then_with(|| a.name.cmp(&b.name)));
        out
    }

    /// Grabs, newest first.
    pub fn grabs(&self) -> Vec<Grab> {
        let mut out: Vec<Grab> = self.list(Bucket::AcqGrabs);
        out.sort_by_key(|x| std::cmp::Reverse(x.created_at));
        out
    }

    pub fn grab(&self, id: &str) -> Result<Grab> {
        self.get(Bucket::AcqGrabs, id, "grab")
    }

    pub fn put_grab(&self, g: &mut Grab) -> Result<()> {
        g.updated_at = crate::util::now();
        self.put(Bucket::AcqGrabs, &g.id.clone(), g)
    }

    pub fn profiles(&self) -> Vec<Profile> {
        let mut out: Vec<Profile> = self.list(Bucket::AcqProfiles);
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    pub fn monitored(&self) -> Vec<Monitored> {
        let mut out: Vec<Monitored> = self.list(Bucket::AcqMonitored);
        out.sort_by_key(|m| m.title.to_lowercase());
        out
    }

    /// Newest first.
    pub fn blocklist(&self) -> Vec<BlockEntry> {
        let mut out: Vec<BlockEntry> = self.list(Bucket::AcqBlocklist);
        out.sort_by_key(|x| std::cmp::Reverse(x.at));
        out
    }

    pub fn torrent(&self, ih: &str) -> Option<TorrentState> {
        self.get(Bucket::AcqTorrents, ih, "torrent").ok()
    }

    pub fn put_torrent(&self, ih: &str, st: &TorrentState) -> Result<()> {
        self.put(Bucket::AcqTorrents, ih, st)
    }

    pub fn delete_torrent(&self, ih: &str) {
        self.remove(Bucket::AcqTorrents, ih)
    }

    pub fn torrents(&self) -> Vec<(String, TorrentState)> {
        self.raw(Bucket::AcqTorrents)
            .into_iter()
            .filter_map(|(k, v)| Some((String::from_utf8(k).ok()?, serde_json::from_slice(&v).ok()?)))
            .collect()
    }
}
