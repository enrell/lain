//! Phase 2 automation (A-17…A-25): quality profiles, monitored titles,
//! RSS sync, scheduled and on-demand searches, upgrades with the holding
//! folder, the blocklist and stall detection (in the monitor).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use lain_api::CatalogItem;
use lain_api::acquire::{
    AutomationState, BlockEntry, GRAB_DOWNLOADING, GRAB_IMPORTING, GRAB_METADATA, GRAB_PAUSED, GRAB_QUEUED, Grab,
    GrabInput, HeldFile, IMPORT_MOVE, IndexerFailure, IndexerSearchInput, Monitored, NUMBER_ABSOLUTE, Profile, Report,
    SearchResult, Unit, Wanted,
};
use lain_api::{Library, Release};
use lain_store::Bucket;

use crate::decide::{default_profile, evaluate, is_upgrade, validate_profile};
use crate::errf;
use crate::error::*;
use crate::importer::place;
use crate::manager::{GrabRequest, Manager, search_kind};
use crate::monitored::{self, covers, names, needs, upgradable};
use crate::util::{base_name, clip, loose_key, new_id, now, within};

impl Manager {
    // ---- profiles -------------------------------------------------------

    /// Quality profiles; the default one always exists.
    pub fn profiles(&self) -> Vec<Profile> {
        let ps = self.st.profiles();
        if ps.iter().any(|p| p.id == "default") {
            return ps;
        }
        let def = default_profile();
        let _ = self.st.put(Bucket::AcqProfiles, &def.id, &def);
        self.st.profiles()
    }

    pub(crate) fn profile(&self, id: &str) -> Result<Profile> {
        let id = if id.is_empty() || id == "default" {
            self.profiles();
            "default"
        } else {
            id
        };
        self.st.get(Bucket::AcqProfiles, id, "profile")
    }

    pub fn create_profile(&self, mut p: Profile) -> Result<Profile> {
        p.id = new_id();
        let p = validate_profile(p)?;
        self.st.put(Bucket::AcqProfiles, &p.id, &p)?;
        Ok(p)
    }

    pub fn update_profile(&self, id: &str, mut p: Profile) -> Result<Profile> {
        self.profile(id)?;
        p.id = id.into();
        let p = validate_profile(p)?;
        self.st.put(Bucket::AcqProfiles, id, &p)?;
        Ok(p)
    }

    /// Removes an unused profile; the default one stays.
    pub fn delete_profile(&self, id: &str) -> Result<()> {
        if id == "default" {
            return Err(errf!(CODE_STATE, "the default profile cannot be deleted"));
        }
        if let Some(m) = self.st.monitored().into_iter().find(|m| m.profile_id == id) {
            return Err(errf!(CODE_STATE, "{} uses this profile", m.title));
        }
        self.st.delete(Bucket::AcqProfiles, id, "profile")
    }

    // ---- monitored titles -----------------------------------------------

    pub fn monitored_list(&self) -> Vec<Monitored> {
        self.st.monitored()
    }

    fn check_monitored(&self, m: Monitored, self_id: &str) -> Result<Monitored> {
        let mut m = monitored::validate(m)?;
        let lib = (self.d.library)(&m.library_id)
            .ok_or_else(|| errf!(CODE_NO_LIBRARY, "choose the library to keep complete"))?;
        if lib.kind != m.kind {
            return Err(errf!(CODE_INVALID, "{} is a {} library, not {}", lib.name, lib.kind, m.kind));
        }
        if m.profile_id.is_empty() {
            m.profile_id = "default".into();
        }
        self.profile(&m.profile_id)?;
        let key = loose_key(&m.title);
        if let Some(other) = self
            .st
            .monitored()
            .into_iter()
            .find(|o| o.id != self_id && o.library_id == m.library_id && names(o).contains(&key))
        {
            return Err(errf!(CODE_STATE, "{} is already monitored in this library", other.title));
        }
        Ok(m)
    }

    /// Starts keeping a title complete.
    pub fn create_monitored(&self, m: Monitored) -> Result<Monitored> {
        let mut m = self.check_monitored(m, "")?;
        m.id = new_id();
        m.created_at = now();
        m.enabled = true;
        self.st.put(Bucket::AcqMonitored, &m.id, &m)?;
        Ok(m)
    }

    /// Replaces a monitored title's settings.
    pub fn update_monitored(&self, id: &str, m: Monitored) -> Result<Monitored> {
        let old = self.monitored(id)?;
        let mut m = self.check_monitored(m, id)?;
        m.id = id.into();
        m.created_at = old.created_at;
        m.last_search_at = old.last_search_at;
        if m.metadata_episodes == 0 {
            m.metadata_episodes = old.metadata_episodes;
        }
        self.st.put(Bucket::AcqMonitored, id, &m)?;
        Ok(m)
    }

    pub fn monitored(&self, id: &str) -> Result<Monitored> {
        self.st.get(Bucket::AcqMonitored, id, "monitored title")
    }

    /// Stops monitoring; grabs and files are untouched.
    pub fn delete_monitored(&self, id: &str) -> Result<()> {
        self.st.delete(Bucket::AcqMonitored, id, "monitored title")
    }

    pub(crate) fn items(&self, library_id: &str) -> Vec<CatalogItem> {
        self.d.items.as_ref().map(|f| f(library_id)).unwrap_or_default()
    }

    /// What a monitored title still needs.
    pub fn wanted_for(&self, id: &str) -> Result<Wanted> {
        let m = self.monitored(id)?;
        let p = self.profile(&m.profile_id)?;
        Ok(monitored::wanted(&m, &self.items(&m.library_id), &p, &self.quality_of(&m.kind)))
    }

    /// A library file's resolution: the import ledger first, then its
    /// name.
    pub(crate) fn quality_of(&self, kind: &str) -> impl Fn(&str) -> String + '_ {
        let kind = kind.to_string();
        move |p: &str| match self.st.get::<String>(Bucket::AcqQuality, p, "quality") {
            Ok(q) => q,
            Err(_) => (self.d.parse)(&base_name(p), &kind).resolution,
        }
    }

    // ---- blocklist ------------------------------------------------------

    pub fn blocklist(&self) -> Vec<BlockEntry> {
        self.st.blocklist()
    }

    pub fn unblock(&self, id: &str) -> Result<()> {
        self.st.delete(Bucket::AcqBlocklist, id, "blocklist entry")
    }

    /// Records a failed grab's release (A-22).
    pub(crate) fn block(&self, g: &Grab, reason: &str) {
        if g.info_hash.is_empty() && g.title.is_empty() {
            return;
        }
        let dup = self.st.blocklist().iter().any(|e| {
            (!g.info_hash.is_empty() && e.info_hash == g.info_hash) || e.title.to_lowercase() == g.title.to_lowercase()
        });
        if dup {
            return;
        }
        let e = BlockEntry {
            id: new_id(),
            info_hash: g.info_hash.clone(),
            title: g.title.clone(),
            indexer_id: g.indexer_id.clone(),
            monitored_id: g.monitored_id.clone(),
            reason: clip(reason, 300),
            at: now(),
        };
        if let Err(err) = self.st.put(Bucket::AcqBlocklist, &e.id, &e) {
            tracing::warn!(grab = %g.id, err = %err, "blocklist write failed");
        }
    }

    // ---- searching ------------------------------------------------------

    pub fn automation_status(&self) -> AutomationState {
        let i = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        AutomationState {
            enabled: i.settings.automation,
            paused: i.auto_paused.clone(),
            last_rss_at: i.last_rss.unwrap_or(0),
        }
    }

    /// Asks every enabled indexer in turn; an empty text is an RSS read.
    fn query(&self, text: &str, kind: &str) -> (Vec<SearchResult>, Vec<IndexerFailure>) {
        let deadline = Instant::now() + Duration::from_secs(45);
        let (mut out, mut fails) = (Vec::new(), Vec::new());
        for ix in self.st.indexers().into_iter().filter(|ix| ix.enabled) {
            let res = self.wait(&ix, deadline).and_then(|_| {
                let mut input =
                    IndexerSearchInput { indexer: ix.clone(), query: text.into(), limit: 100, ..Default::default() };
                if !text.is_empty() {
                    input.kind = search_kind(kind).into();
                }
                (self.d.search)(&input)
            });
            match res {
                Ok(r) => out.extend(r),
                Err(e) => fails.push(IndexerFailure { indexer_id: ix.id, name: ix.name, error: e.to_string() }),
            }
        }
        (out, fails)
    }

    /// Picks the releases worth grabbing for one title and grabs them,
    /// best first, never twice for the same unit.
    fn consider(&self, m: &Monitored, results: &[SearchResult], rep: &mut Report) {
        let Ok(p) = self.profile(&m.profile_id) else { return };
        let w = monitored::wanted(m, &self.items(&m.library_id), &p, &self.quality_of(&m.kind));
        let mut taken: HashSet<Unit> = HashSet::new();
        for g in self.st.grabs() {
            if g.monitored_id == m.id
                && matches!(
                    g.state.as_str(),
                    GRAB_QUEUED | GRAB_METADATA | GRAB_DOWNLOADING | GRAB_IMPORTING | GRAB_PAUSED
                )
            {
                taken.extend(g.units.iter().copied());
            }
        }
        let bl = self.st.blocklist();
        struct Opt {
            r: SearchResult,
            score: i64,
            covers: Vec<Unit>,
            replace: Vec<String>,
        }
        let mut opts: Vec<Opt> = Vec::new();
        // Every monitored title's names, not only this one's: a release
        // the parser answers as another monitored title is that title's
        // (D-279).
        let claimed = monitored::claimed(&self.st.monitored());
        for r in results {
            let rel = monitored::settle_for(m, (self.d.parse)(&r.title, &m.kind), &claimed);
            let cov = covers(m, &rel);
            if cov.is_empty() {
                continue;
            }
            rep.considered += 1;
            let mut needed = 0;
            let mut replace = Vec::new();
            for u in &cov {
                if taken.contains(u) {
                    continue;
                }
                if needs(&w, *u) {
                    needed += 1;
                } else if let Some(up) = upgradable(&w, *u)
                    && is_upgrade(&p, &up.quality, &rel.resolution)
                {
                    replace.push(up.path.clone());
                }
            }
            if needed == 0 && replace.is_empty() {
                continue;
            }
            if blocked(&bl, r) {
                rep.rejected += 1;
                continue;
            }
            let units = if cov.len() == 1 && cov[0].number == 0 { 12 } else { cov.len() as i64 }; // a typical cour
            let d = evaluate(&p, r, &rel, units);
            if !d.accepted {
                rep.rejected += 1;
                continue;
            }
            opts.push(Opt { r: r.clone(), score: d.score, covers: cov, replace });
        }
        opts.sort_by_key(|x| std::cmp::Reverse(x.score));
        for o in opts {
            if !o.covers.iter().any(|u| !taken.contains(u)) {
                continue;
            }
            let req = GrabRequest {
                input: GrabInput { result: Some(o.r.clone()), library_id: m.library_id.clone(), ..Default::default() },
                monitored_id: m.id.clone(),
                units: o.covers.clone(),
                replace_from: o.replace.clone(),
                ..Default::default()
            };
            match self.grab(req) {
                Ok(g) => {
                    self.set_auto_paused("");
                    taken.extend(o.covers.iter().copied());
                    rep.grabbed.push(g.id);
                }
                Err(e) if e.code == CODE_QUOTA || e.code == CODE_DISK_FULL => {
                    rep.paused = format!("download budget full: {}", e.msg);
                    self.set_auto_paused(&rep.paused);
                    return;
                }
                Err(e) => tracing::info!(monitored = %m.id, err = %e, "automatic grab skipped"),
            }
        }
    }

    fn set_auto_paused(&self, reason: &str) {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).auto_paused = reason.into();
    }

    /// Searches indexers for one monitored title now.
    pub fn search_monitored(&self, id: &str) -> Result<Report> {
        let mut m = self.monitored(id)?;
        let mut rep = Report::default();
        if m.numbering == NUMBER_ABSOLUTE
            && let Some(f) = &self.d.episode_count
        {
            let n = f(&m.title, &m.kind);
            if n > 0 && n < 100_000 {
                m.metadata_episodes = n;
            }
        }
        let (results, fails) = self.query(&m.title, &m.kind);
        rep.failures = fails;
        self.consider(&m, &results, &mut rep);
        m.last_search_at = now();
        let _ = self.st.put(Bucket::AcqMonitored, &m.id, &m);
        Ok(rep)
    }

    /// Reads every indexer's latest releases once and offers them to
    /// every enabled monitored title.
    pub fn rss_sync(&self) -> Result<Report> {
        let mut rep = Report::default();
        let (results, fails) = self.query("", "");
        rep.failures = fails;
        for m in self.st.monitored().into_iter().filter(|m| m.enabled) {
            self.consider(&m, &results, &mut rep);
            if !rep.paused.is_empty() {
                break;
            }
        }
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).last_rss = Some(now());
        Ok(rep)
    }

    /// Runs due automation in the background, one pass at a time.
    pub(crate) fn schedule(&self, now_s: i64) {
        let (s, last) = {
            let mut i = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if i.auto_busy || !i.settings.automation {
                return;
            }
            i.auto_busy = true;
            (i.settings.clone(), i.last_rss)
        };
        self.spawn("acquire-automation", move |m| {
            if rss_due(&s, last, now_s) {
                if let Err(e) = m.rss_sync() {
                    tracing::warn!(err = %e, "rss sync failed");
                }
            } else if let Some(mon) = m.st.monitored().into_iter().find(|x| search_due(&s, x, now_s)) {
                // One missing-search per pass spreads indexer load.
                let _ = m.search_monitored(&mon.id);
            } else if let Some(mon) =
                m.st.monitored().into_iter().find(|x| crate::subtitles::subtitle_due(&s, x, now_s))
            {
                // Then one subtitle pass per tick (A-36).
                let _ = m.subtitles_for_monitored(&mon.id);
            }
            m.inner.lock().unwrap_or_else(|e| e.into_inner()).auto_busy = false;
        });
    }

    // ---- upgrades: the holding folder -----------------------------------

    pub(crate) fn hold_dir(&self) -> PathBuf {
        Path::new(&self.settings().dir).join("replaced")
    }

    /// Moves the library files an upgrade replaces into
    /// `<dir>/replaced/<grab>/`, refusing anything outside its library.
    pub(crate) fn hold_replaced(&self, g: &Grab, lib: &Library) -> Result<Vec<Held>> {
        let mut held = Vec::new();
        for p in &g.replace_from {
            let p = Path::new(p);
            if !within(Path::new(&lib.path), p) {
                self.restore_held(&held);
                return Err(errf!(CODE_IMPORT, "refusing to replace a file outside the library: {}", p.display()));
            }
            if std::fs::symlink_metadata(p).is_err() {
                continue; // already gone (an earlier attempt held it)
            }
            let dst = free_hold_path(&self.hold_dir().join(&g.id), &base_name(&p.to_string_lossy()));
            if let Err(e) = place(p, &dst, IMPORT_MOVE) {
                self.restore_held(&held);
                return Err(e);
            }
            held.push(Held { from: p.to_path_buf(), to: dst });
        }
        Ok(held)
    }

    /// Puts held files back where their path is still free.
    pub(crate) fn restore_held(&self, held: &[Held]) {
        for h in held {
            if let Err(e) = place(&h.to, &h.from, IMPORT_MOVE) {
                tracing::warn!(path = %h.to.display(), err = %e, "could not restore a replaced file; it stays held");
            }
        }
    }

    /// Held files. They are never deleted automatically.
    pub fn replaced(&self) -> Vec<HeldFile> {
        let root = self.hold_dir();
        let mut out = Vec::new();
        let mut stack = vec![root.clone()];
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else { continue };
            let mut ents: Vec<_> = rd.flatten().collect();
            ents.sort_by_key(|e| e.file_name());
            for e in ents.into_iter().rev() {
                let Ok(ft) = e.file_type() else { continue };
                if ft.is_dir() {
                    stack.push(e.path());
                    continue;
                }
                let Ok(meta) = e.metadata() else { continue };
                let rel = e.path().strip_prefix(&root).map(|r| r.to_path_buf()).unwrap_or_default();
                let grab =
                    rel.components().next().map(|c| c.as_os_str().to_string_lossy().into_owned()).unwrap_or_default();
                let at = meta.modified().ok().and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok());
                out.push(HeldFile {
                    grab_id: grab,
                    path: e.path().to_string_lossy().into(),
                    size: meta.len() as i64,
                    at: at.map(|d| d.as_secs() as i64).unwrap_or(0),
                });
            }
        }
        out.sort_by(|a, b| a.path.cmp(&b.path));
        out
    }

    /// Deletes every held file: an explicit admin action.
    pub fn purge_replaced(&self) -> Result<i64> {
        let mut n = 0;
        for h in self.replaced() {
            std::fs::remove_file(&h.path).map_err(|e| errf!(CODE_IMPORT, "could not delete {}: {e}", h.path))?;
            n += 1;
        }
        if let Ok(rd) = std::fs::read_dir(self.hold_dir()) {
            for e in rd.flatten() {
                let _ = std::fs::remove_dir(e.path());
            }
        }
        Ok(n)
    }
}

/// A held library file and where it came from.
pub(crate) struct Held {
    pub from: PathBuf,
    pub to: PathBuf,
}

/// `dir/name`, or `dir/name.N` for the first free N.
pub(crate) fn free_hold_path(dir: &Path, name: &str) -> PathBuf {
    let mut dst = dir.join(name);
    let mut i = 1;
    while std::fs::symlink_metadata(&dst).is_ok() {
        dst = dir.join(format!("{name}.{i}"));
        i += 1;
    }
    dst
}

fn blocked(list: &[BlockEntry], r: &SearchResult) -> bool {
    list.iter().any(|e| {
        (!r.info_hash.is_empty() && e.info_hash.eq_ignore_ascii_case(&r.info_hash))
            || e.title.to_lowercase() == r.title.to_lowercase()
    })
}

pub(crate) fn rss_due(s: &lain_api::acquire::AcquireSettings, last: Option<i64>, now: i64) -> bool {
    s.automation && last.is_none_or(|l| now - l >= s.rss_minutes * 60)
}

pub(crate) fn search_due(s: &lain_api::acquire::AcquireSettings, m: &Monitored, now: i64) -> bool {
    s.automation && m.enabled && s.search_hours > 0 && now - m.last_search_at >= s.search_hours * 3600
}

/// The release a file name parses to, with the title's numbers (the
/// subtitle pass's fallback when an import is not catalogued yet).
pub(crate) fn item_from_path(parse: &crate::importer::Parser, p: &str, kind: &str) -> CatalogItem {
    let r: Release = parse(&base_name(p), kind);
    CatalogItem {
        file_path: p.into(),
        season: r.season,
        episode: r.episodes.first().copied().unwrap_or(0),
        ..Default::default()
    }
}
