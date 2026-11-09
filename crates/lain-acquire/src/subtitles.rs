//! Phase 3: subtitle acquisition (A-30…A-36). Providers are searched
//! through `lain.subtitle@1`; the core fetches the file, checks it
//! against the media (A-33), normalizes it to UTF-8 (A-34) and writes it
//! next to the media without overwriting (A-35).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use lain_api::CatalogItem;
use lain_api::acquire::{
    AcquireSettings, FileSubtitles, Grab, HI_EXCLUDE, HI_PREFER, HeldFile, IMPORT_MOVE, Monitored, Profile,
    SUBTITLE_OPENSUBTITLES, SidecarFile, SubtitleCandidate, SubtitleChoice, SubtitleDownloadInput,
    SubtitleDownloadOutput, SubtitleProvider, SubtitleProviderInput, SubtitleRecord, SubtitleRefusal,
    SubtitleSearchInput,
};
use lain_downloads::http::Request;
use lain_store::Bucket;

use crate::automation::{free_hold_path, item_from_path};
use crate::decide::rank_of;
use crate::errf;
use crate::error::*;
use crate::importer::{VIDEO_EXTS, place};
use crate::manager::{Manager, go_duration};
use crate::monitored::names;
use crate::util::{base_name, clean_name, ext_lower, http_url, loose_key, new_id, now, url_host, within};

/// One stream of the probe, as subtitle needs read it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProbeStream {
    /// `audio`, `subtitle`, `video`.
    pub kind: String,
    pub language: String,
    pub forced: bool,
}

/// What the probe tells subtitle checks (`lain.media.probe@1`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProbeInfo {
    /// Seconds; 0 = unknown.
    pub duration: f64,
    pub streams: Vec<ProbeStream>,
}

pub type ProbeFn = dyn Fn(&Path) -> Result<ProbeInfo> + Send + Sync;
pub type SubSearchFn = dyn Fn(&SubtitleSearchInput) -> Result<Vec<SubtitleCandidate>> + Send + Sync;
pub type SubDownloadFn = dyn Fn(&SubtitleDownloadInput) -> Result<SubtitleDownloadOutput> + Send + Sync;

/// The probe and `lain.subtitle@1`. All optional.
#[derive(Clone, Default)]
pub struct SubtitleDeps {
    pub probe: Option<Arc<ProbeFn>>,
    pub search: Option<Arc<SubSearchFn>>,
    pub download: Option<Arc<SubDownloadFn>>,
}

/// The media a subtitle is for.
#[derive(Debug, Clone, Default)]
pub struct SubtitleTarget {
    pub path: PathBuf,
    pub kind: String,
    pub title: String,
    pub season: i64,
    pub episode: i64,
    pub year: i64,
}

fn sub_block_key(provider: &str, file: &str) -> String {
    format!("{provider}\u{0}{file}")
}

fn patch_secret(dst: &mut String, input: &Option<String>) -> Result<()> {
    let Some(v) = input else { return Ok(()) };
    match v.trim() {
        "" => {}
        "-" => dst.clear(),
        v if v.len() > 512 => return Err(errf!(CODE_INVALID, "credential too long")),
        v => *dst = v.to_string(),
    }
    Ok(())
}

fn apply_provider(input: &SubtitleProviderInput, p: &mut SubtitleProvider) -> Result<()> {
    if let Some(n) = &input.name {
        p.name = clean_name(n, 60)?;
    }
    if let Some(k) = &input.kind {
        p.kind = k.clone();
    }
    if let Some(u) = &input.base_url {
        p.base_url = u.trim().to_string();
    }
    if let Some(u) = &input.username {
        p.username = clean_name(u, 100)?;
    }
    patch_secret(&mut p.api_key, &input.api_key)?;
    patch_secret(&mut p.password, &input.password)?;
    if let Some(e) = input.enabled {
        p.enabled = e;
    }
    if let Some(pr) = input.priority {
        p.priority = pr;
    }
    if p.name.is_empty() {
        return Err(errf!(CODE_INVALID, "name required"));
    }
    if p.kind != SUBTITLE_OPENSUBTITLES {
        return Err(errf!(CODE_INVALID, "kind must be {SUBTITLE_OPENSUBTITLES}"));
    }
    if !p.base_url.is_empty() && !http_url(&p.base_url) {
        return Err(errf!(CODE_INVALID, "base_url must be http(s) without credentials"));
    }
    if p.api_key.is_empty() {
        return Err(errf!(CODE_INVALID, "an API key is required"));
    }
    Ok(())
}

/// A-33 against the media's duration (seconds; 0 = unknown, skipped).
fn sync_check(first_ms: i64, last_ms: i64, duration: f64) -> Result<()> {
    if duration <= 0.0 {
        return Ok(());
    }
    let d = Duration::from_secs_f64(duration);
    let limit = Duration::from_secs_f64(duration * 1.10).max(d + Duration::from_secs(120));
    let (first, last) = (Duration::from_millis(first_ms.max(0) as u64), Duration::from_millis(last_ms.max(0) as u64));
    let round = |x: Duration| go_duration(Duration::from_secs((x.as_millis() as u64 + 500) / 1000));
    if last > limit {
        return Err(errf!(
            CODE_SUBTITLE_MISMATCH,
            "the subtitle runs to {} but the video is {} long (another cut, episode or frame rate)",
            round(last),
            round(d)
        ));
    }
    if first > d {
        return Err(errf!(CODE_SUBTITLE_MISMATCH, "the subtitle starts after the video ends"));
    }
    Ok(())
}

fn format_of(name: &str) -> &'static str {
    match ext_lower(name).trim_start_matches('.') {
        "srt" => "srt",
        "ass" => "ass",
        "ssa" => "ssa",
        "vtt" => "vtt",
        _ => "srt",
    }
}

fn sidecar_files(media: &Path) -> Vec<SidecarFile> {
    lain_subtitle::discover(media)
        .into_iter()
        .map(|f| SidecarFile {
            name: f.view.name,
            language: f.view.language,
            tag: f.view.tag,
            forced: f.view.forced,
            hi: f.view.hi,
            format: f.view.format,
        })
        .collect()
}

pub(crate) fn subtitle_due(s: &AcquireSettings, m: &Monitored, now: i64) -> bool {
    s.automation && m.enabled && s.subtitle_hours > 0 && now - m.last_subtitle_at >= s.subtitle_hours * 3600
}

/// Writes `data` at `dst` without ever replacing an existing file.
fn write_new(dst: &Path, data: &[u8]) -> Result<()> {
    use std::io::Write;
    let tmp = PathBuf::from(format!("{}.lain-sub", dst.display()));
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .map_err(|e| errf!(CODE_IMPORT, "create: {e}"))?;
    let w = f.write_all(data).and_then(|_| f.sync_all());
    drop(f);
    if let Err(e) = w {
        let _ = std::fs::remove_file(&tmp);
        return Err(errf!(CODE_IMPORT, "write: {e}"));
    }
    let linked = std::fs::hard_link(&tmp, dst);
    let _ = std::fs::remove_file(&tmp);
    match linked {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(errf!(CODE_STATE, "{} already exists", base_name(&dst.to_string_lossy())))
        }
        Err(e) => Err(errf!(CODE_IMPORT, "place: {e}")),
    }
}

impl Manager {
    // ---- provider accounts ----------------------------------------------

    fn providers_by_priority(&self) -> Vec<SubtitleProvider> {
        let mut out: Vec<SubtitleProvider> = self.st.list(Bucket::AcqSubProviders);
        out.sort_by_key(|p| p.priority);
        out
    }

    /// Accounts without secrets.
    pub fn subtitle_providers(&self) -> Vec<SubtitleProvider> {
        self.providers_by_priority().iter().map(SubtitleProvider::public).collect()
    }

    /// Which secrets an account has stored: (api key, password).
    pub fn subtitle_provider_secrets(&self, id: &str) -> (bool, bool) {
        match self.st.get::<SubtitleProvider>(Bucket::AcqSubProviders, id, "subtitle provider") {
            Ok(p) => (!p.api_key.is_empty(), !p.password.is_empty()),
            Err(_) => (false, false),
        }
    }

    pub fn create_subtitle_provider(&self, input: &SubtitleProviderInput) -> Result<SubtitleProvider> {
        let mut p = SubtitleProvider { id: new_id(), enabled: true, created_at: now(), ..Default::default() };
        apply_provider(input, &mut p)?;
        self.st.put(Bucket::AcqSubProviders, &p.id, &p)?;
        Ok(p.public())
    }

    pub fn update_subtitle_provider(&self, id: &str, input: &SubtitleProviderInput) -> Result<SubtitleProvider> {
        let mut p: SubtitleProvider = self.st.get(Bucket::AcqSubProviders, id, "subtitle provider")?;
        apply_provider(input, &mut p)?;
        self.st.put(Bucket::AcqSubProviders, id, &p)?;
        Ok(p.public())
    }

    pub fn delete_subtitle_provider(&self, id: &str) -> Result<()> {
        self.st.delete(Bucket::AcqSubProviders, id, "subtitle provider")
    }

    // ---- what a file needs ------------------------------------------------

    fn probe(&self, path: &Path) -> ProbeInfo {
        let Some(f) = &self.d.subtitles.probe else { return ProbeInfo::default() };
        match f(path) {
            Ok(i) => i,
            Err(e) => {
                tracing::info!(path = %path.display(), err = %e, "probe failed; subtitle checks run without stream data");
                ProbeInfo::default()
            }
        }
    }

    /// The profile's languages a file still lacks: present through an
    /// embedded subtitle stream, a full (not forced) sidecar, or, unless
    /// `subtitle_even_with_audio`, an audio stream (A-32, D-071).
    pub fn missing_subtitles(&self, path: &Path, p: &Profile) -> Vec<String> {
        if p.subtitle_languages.is_empty() {
            return Vec::new();
        }
        let mut have = HashSet::new();
        for s in self.probe(path).streams {
            let lang = lain_subtitle::language(&s.language);
            if lang.is_empty() {
                continue;
            }
            if (s.kind == "subtitle" && !s.forced) || (s.kind == "audio" && !p.subtitle_even_with_audio) {
                have.insert(lang);
            }
        }
        for sc in lain_subtitle::discover(path) {
            if !sc.view.forced {
                have.insert(sc.view.language);
            }
        }
        p.subtitle_languages.iter().filter(|l| !have.contains(*l)).cloned().collect()
    }

    // ---- refusals -------------------------------------------------------

    fn subtitle_refused(&self, provider: &str, file: &str) -> bool {
        self.st.has(Bucket::AcqSubBlock, &sub_block_key(provider, file))
    }

    /// Refused files, newest first. Entries holding the reason alone (a
    /// JSON string) still list, their ids from the key.
    pub fn subtitle_refusals(&self) -> Vec<SubtitleRefusal> {
        let mut out: Vec<SubtitleRefusal> = self
            .st
            .raw(Bucket::AcqSubBlock)
            .into_iter()
            .map(|(k, v)| {
                let mut r: SubtitleRefusal = serde_json::from_slice(&v).unwrap_or_else(|_| SubtitleRefusal {
                    reason: serde_json::from_slice::<String>(&v).unwrap_or_default(),
                    ..Default::default()
                });
                let key = String::from_utf8_lossy(&k).into_owned();
                let (p, f) = key.split_once('\u{0}').unwrap_or((&key, ""));
                r.provider_id = p.into();
                r.file_id = f.into();
                r
            })
            .collect();
        out.sort_by_key(|x| std::cmp::Reverse(x.at));
        out
    }

    pub fn clear_subtitle_refusal(&self, provider: &str, file: &str) -> Result<()> {
        self.st.delete(Bucket::AcqSubBlock, &sub_block_key(provider, file), "refused subtitle")
    }

    pub fn clear_subtitle_refusals(&self) -> i64 {
        self.st.clear(Bucket::AcqSubBlock).unwrap_or_else(|e| {
            tracing::warn!(err = %e, "clearing subtitle refusals failed");
            0
        })
    }

    // ---- searching and ranking --------------------------------------------

    /// Asks every enabled provider and ranks the answers: hash match,
    /// then release group, then resolution/source, then popularity.
    pub fn search_subtitles(
        &self,
        t: &SubtitleTarget,
        p: &Profile,
        languages: &[String],
    ) -> Result<Vec<SubtitleChoice>> {
        let Some(search) = &self.d.subtitles.search else {
            return Err(errf!(CODE_UNSUPPORTED, "no subtitle provider is available"));
        };
        let enabled: Vec<SubtitleProvider> = self.providers_by_priority().into_iter().filter(|p| p.enabled).collect();
        if enabled.is_empty() {
            return Err(errf!(CODE_NOT_FOUND, "no enabled subtitle provider; add one under Settings › Acquisition"));
        }
        let hash = lain_subtitle::movie_hash(&t.path).unwrap_or_default();
        let size = std::fs::metadata(&t.path).map(|m| m.len() as i64).unwrap_or(0);
        let file = (self.d.parse)(&base_name(&t.path.to_string_lossy()), &t.kind);
        let wanted: HashSet<&String> = languages.iter().collect();
        let mut out: Vec<SubtitleChoice> = Vec::new();
        let mut last_err = None;
        for pr in enabled {
            let input = SubtitleSearchInput {
                provider: pr.clone(),
                hash: hash.clone(),
                size,
                query: t.title.clone(),
                movie: t.kind == "movie",
                year: t.year,
                season: t.season,
                episode: t.episode,
                languages: languages.to_vec(),
            };
            match search(&input) {
                Ok(cands) => {
                    for mut c in cands {
                        if c.provider_id.is_empty() {
                            c.provider_id = pr.id.clone();
                        }
                        out.push(self.judge_subtitle(c, p, &wanted, t, &file));
                    }
                }
                Err(e) => {
                    tracing::info!(provider = %pr.id, err = %e, "subtitle search failed");
                    last_err = Some(e);
                }
            }
        }
        if out.is_empty()
            && let Some(e) = last_err
        {
            return Err(e);
        }
        // Accepted first by score; rejected ones keep provider order.
        let (mut acc, rej): (Vec<_>, Vec<_>) = out.into_iter().partition(|c| c.accepted);
        acc.sort_by_key(|x| std::cmp::Reverse(x.score));
        acc.extend(rej);
        Ok(acc)
    }

    fn judge_subtitle(
        &self,
        c: SubtitleCandidate,
        p: &Profile,
        wanted: &HashSet<&String>,
        t: &SubtitleTarget,
        file: &lain_api::Release,
    ) -> SubtitleChoice {
        let mut rej = Vec::new();
        if !wanted.contains(&c.language) {
            rej.push(format!("language {} not wanted", c.language));
        }
        if c.forced {
            rej.push("forced (foreign parts only)".to_string());
        }
        if c.hi && p.subtitle_hi == HI_EXCLUDE {
            rej.push("hearing-impaired excluded by the profile".into());
        }
        if t.episode > 0 && c.episode > 0 && c.episode != t.episode {
            rej.push(format!("for episode {}, not {}", c.episode, t.episode));
        }
        if t.season > 0 && c.season > 0 && c.season != t.season {
            rej.push(format!("for season {}, not {}", c.season, t.season));
        }
        if self.subtitle_refused(&c.provider_id, &c.file_id) {
            rej.push("refused earlier by the sync check".into());
        }
        let mut score = 0;
        if c.hash_match {
            score += 1000;
        }
        if !c.release.is_empty() {
            let rel = (self.d.parse)(&c.release, &t.kind);
            if !file.group.is_empty() && rel.group.to_lowercase() == file.group.to_lowercase() {
                score += 100;
            }
            if !file.resolution.is_empty() && rel.resolution == file.resolution {
                score += 20;
            }
            if !file.source.is_empty() && rel.source == file.source {
                score += 20;
            }
        }
        if c.hi && p.subtitle_hi == HI_PREFER {
            score += 30;
        }
        score += crate::decide::log2_points(5.0, c.downloads);
        SubtitleChoice { candidate: c, accepted: rej.is_empty(), rejections: rej, score }
    }

    // ---- downloading and placing -------------------------------------------

    pub fn subtitle_ledger(&self) -> Vec<SubtitleRecord> {
        let mut out: Vec<SubtitleRecord> = self.st.list(Bucket::AcqSubtitles);
        out.sort_by_key(|x| std::cmp::Reverse(x.at));
        out
    }

    pub(crate) fn in_library(&self, p: &Path) -> bool {
        self.library_roots().iter().any(|r| within(r, p))
    }

    fn fetch_subtitle(&self, link: &str) -> Result<Vec<u8>> {
        if !http_url(link) {
            return Err(errf!(CODE_FETCH, "the provider gave no usable link"));
        }
        let host = url_host(link);
        let res = self.d.http.send(&Request::get(link)).map_err(|_| errf!(CODE_FETCH, "{host} did not answer"))?;
        if res.status != 200 {
            return Err(errf!(CODE_FETCH, "{host} answered {}", res.status));
        }
        res.read_all(lain_subtitle::MAX_BYTES)
            .map_err(|_| errf!(CODE_FETCH, "the subtitle from {host} is unreadable or too large"))
    }

    /// Fetches a candidate, checks it and writes it next to `media`. An
    /// existing sidecar of the same name is kept unless `replace`, which
    /// moves it to the holding folder (D-128), never deletes it.
    pub fn download_subtitle(&self, media: &Path, c: &SubtitleCandidate, replace: bool) -> Result<SubtitleRecord> {
        let Some(download) = &self.d.subtitles.download else {
            return Err(errf!(CODE_UNSUPPORTED, "no subtitle provider is available"));
        };
        if !self.in_library(media) {
            return Err(errf!(CODE_INVALID, "subtitles are only written next to library files"));
        }
        let lang = lain_subtitle::language(&c.language);
        if lang.is_empty() {
            return Err(errf!(CODE_INVALID, "unknown subtitle language"));
        }
        let pr: SubtitleProvider = self.st.get(Bucket::AcqSubProviders, &c.provider_id, "subtitle provider")?;
        let link = download(&SubtitleDownloadInput { provider: pr, file_id: c.file_id.clone() })
            .map_err(|e| errf!(CODE_FETCH, "{e}"))?;
        let raw = self.fetch_subtitle(&link.link)?;
        let refuse = |e: AcqError| -> Result<SubtitleRecord> {
            // Every refusal is a mismatch: the reason holds the message.
            let r = SubtitleRefusal {
                provider_id: c.provider_id.clone(),
                file_id: c.file_id.clone(),
                reason: e.msg.clone(),
                media_path: media.to_string_lossy().into(),
                at: now(),
            };
            if let Err(pe) = self.st.put(Bucket::AcqSubBlock, &sub_block_key(&c.provider_id, &c.file_id), &r) {
                tracing::warn!(file_id = %c.file_id, err = %pe, "subtitle refusal write failed");
            }
            Err(e)
        };
        let cues = match lain_subtitle::parse(&raw) {
            Ok(c) => c,
            Err(_) => return refuse(errf!(CODE_SUBTITLE_MISMATCH, "the file holds no readable subtitles")),
        };
        let first = cues.iter().map(|c| c.start_ms).min().unwrap_or(0);
        let last = cues.iter().map(|c| c.end_ms).max().unwrap_or(0);
        if let Err(e) = sync_check(first, last, self.probe(media).duration) {
            return refuse(e);
        }
        let name = if c.file_name.is_empty() { &link.file_name } else { &c.file_name };
        let dst = lain_subtitle::sidecar_path(media, &lang, &c.region, c.forced, c.hi, format_of(name));
        if std::fs::symlink_metadata(&dst).is_ok() {
            if !replace {
                return Err(errf!(CODE_STATE, "{} already exists", base_name(&dst.to_string_lossy())));
            }
            self.hold_file(&dst, "subtitles")?;
        }
        write_new(&dst, lain_subtitle::decode_text(&raw).as_bytes())?;
        let rec = SubtitleRecord {
            path: dst.to_string_lossy().into(),
            media_path: media.to_string_lossy().into(),
            provider_id: c.provider_id.clone(),
            file_id: c.file_id.clone(),
            language: lang,
            hash_match: c.hash_match,
            at: now(),
        };
        if let Err(e) = self.st.put(Bucket::AcqSubtitles, &rec.path, &rec) {
            tracing::warn!(path = %rec.path, err = %e, "subtitle ledger write failed");
        }
        tracing::info!(path = %rec.path, provider = %c.provider_id, hash_match = c.hash_match, "subtitle placed");
        Ok(rec)
    }

    /// Moves a library file into `<dir>/replaced/<group>/` (D-128).
    fn hold_file(&self, p: &Path, group: &str) -> Result<PathBuf> {
        let dst = free_hold_path(&self.hold_dir().join(group), &base_name(&p.to_string_lossy()));
        place(p, &dst, IMPORT_MOVE)?;
        Ok(dst)
    }

    /// Moves one sidecar of `media` into the holding folder (A-35,
    /// D-128): nothing is deleted. `name` must be a sidecar discovery
    /// finds for that media, so no other file is reachable.
    pub fn remove_sidecar(&self, media: &Path, name: &str) -> Result<HeldFile> {
        if !self.in_library(media) {
            return Err(errf!(CODE_INVALID, "not a library file"));
        }
        let Some(sc) = lain_subtitle::discover(media).into_iter().find(|s| s.view.name == name) else {
            return Err(errf!(CODE_NOT_FOUND, "unknown sidecar"));
        };
        let meta = std::fs::symlink_metadata(&sc.path).map_err(|_| errf!(CODE_NOT_FOUND, "unknown sidecar"))?;
        let dst = self
            .hold_file(&sc.path, "subtitles")
            .map_err(|e| errf!(CODE_IMPORT, "could not move {name} out of the library: {e}"))?;
        self.st.remove(Bucket::AcqSubtitles, &sc.path.to_string_lossy());
        tracing::info!(from = %sc.path.display(), to = %dst.display(), "sidecar held");
        Ok(HeldFile {
            grab_id: "subtitles".into(),
            path: dst.to_string_lossy().into(),
            size: meta.len() as i64,
            at: now(),
        })
    }

    // ---- automation -----------------------------------------------------

    fn title_files(&self, m: &Monitored) -> Vec<CatalogItem> {
        let names = names(m);
        let mut out: Vec<CatalogItem> = self
            .items(&m.library_id)
            .into_iter()
            .filter(|it| {
                !it.missing
                    && names.contains(&loose_key(&it.title))
                    && VIDEO_EXTS.contains(&ext_lower(&it.file_path).as_str())
            })
            .collect();
        out.sort_by(|a, b| a.file_path.cmp(&b.file_path));
        out
    }

    /// Each video file of a monitored title.
    pub fn subtitle_status(&self, id: &str) -> Result<Vec<FileSubtitles>> {
        let m = self.monitored(id)?;
        let p = self.profile(&m.profile_id)?;
        Ok(self
            .title_files(&m)
            .into_iter()
            .map(|it| FileSubtitles {
                missing: self.missing_subtitles(Path::new(&it.file_path), &p),
                sidecars: sidecar_files(Path::new(&it.file_path)),
                item_id: it.id,
                path: it.file_path,
            })
            .collect())
    }

    /// Any video item and the languages its governing profile wants.
    pub fn item_subtitle_status(&self, it: &CatalogItem) -> Result<(FileSubtitles, Vec<String>)> {
        let p = self.profile_for_item(it)?;
        let path = Path::new(&it.file_path);
        let f = FileSubtitles {
            item_id: it.id.clone(),
            path: it.file_path.clone(),
            missing: self.missing_subtitles(path, &p),
            sidecars: sidecar_files(path),
        };
        Ok((f, p.subtitle_languages))
    }

    /// Fetches the best accepted subtitle for every missing language of
    /// the given files (A-36); answers how many were written.
    pub fn subtitle_pass(&self, mut m: Monitored, files: &[CatalogItem]) -> i64 {
        let Ok(p) = self.profile(&m.profile_id) else { return 0 };
        if p.subtitle_languages.is_empty() || self.d.subtitles.search.is_none() {
            return 0;
        }
        let mut written = 0;
        for it in files {
            let path = Path::new(&it.file_path);
            let missing = self.missing_subtitles(path, &p);
            if missing.is_empty() {
                continue;
            }
            let t = SubtitleTarget {
                path: path.to_path_buf(),
                kind: m.kind.clone(),
                title: m.title.clone(),
                season: it.season,
                episode: it.episode,
                year: m.year,
            };
            let choices = match self.search_subtitles(&t, &p, &missing) {
                Ok(c) => c,
                Err(e) => {
                    tracing::info!(path = %it.file_path, err = %e, "subtitle pass skipped a file");
                    continue;
                }
            };
            let mut done: HashMap<String, bool> = HashMap::new();
            for ch in choices {
                if !ch.accepted || done.contains_key(&ch.candidate.language) {
                    continue;
                }
                match self.download_subtitle(path, &ch.candidate, false) {
                    Ok(_) => {
                        done.insert(ch.candidate.language.clone(), true);
                        written += 1;
                    }
                    Err(e) => {
                        tracing::info!(path = %it.file_path, file = %ch.candidate.file_id, err = %e, "subtitle not placed")
                    }
                }
            }
        }
        m.last_subtitle_at = now();
        let _ = self.st.put(Bucket::AcqMonitored, &m.id.clone(), &m);
        written
    }

    /// A pass over all of a title's files now.
    pub fn subtitles_for_monitored(&self, id: &str) -> Result<i64> {
        let m = self.monitored(id)?;
        let files = self.title_files(&m);
        Ok(self.subtitle_pass(m, &files))
    }

    /// A pass over freshly imported files (the rescan has catalogued
    /// them by then, else their names are read).
    pub(crate) fn subtitles_after_import(&self, g: &Grab) {
        if g.monitored_id.is_empty() || g.imported.is_empty() || self.d.subtitles.search.is_none() {
            return;
        }
        let Ok(m) = self.monitored(&g.monitored_id) else { return };
        let paths: HashSet<&String> = g.imported.iter().collect();
        let mut files: Vec<CatalogItem> =
            self.title_files(&m).into_iter().filter(|it| paths.contains(&it.file_path)).collect();
        if files.is_empty() {
            let mut ps: Vec<&String> = paths.into_iter().collect();
            ps.sort();
            files = ps.into_iter().map(|p| item_from_path(&*self.d.parse, p, &m.kind)).collect();
        }
        self.subtitle_pass(m, &files);
    }

    /// The profile governing an item: its monitored title's (same
    /// library, same title) or the default.
    pub fn profile_for_item(&self, it: &CatalogItem) -> Result<Profile> {
        let key = loose_key(&it.title);
        match self.st.monitored().into_iter().find(|m| m.library_id == it.library_id && names(m).contains(&key)) {
            Some(m) => self.profile(&m.profile_id),
            None => self.profile(""),
        }
    }
}

/// Parses `?languages=en,pt-BR` into ISO 639-2/T codes.
pub fn parse_languages(raw: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for l in raw.split(',') {
        let code = lain_subtitle::language(l.trim());
        if code.is_empty() {
            return Err(errf!(CODE_INVALID, "unknown language {}", l.trim()));
        }
        out.push(code);
    }
    Ok(out)
}

/// Whether a code is already in a list (case-insensitive).
pub fn has_language(list: &[String], l: &str) -> bool {
    rank_of(list, l).is_some()
}
