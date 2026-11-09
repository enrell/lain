//! Phase 3 tests (Go `subtitles_test.go`): a fake provider and probe,
//! subtitle files served by a local HTTP server (A-13: no network).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lain_api::CatalogItem;
use lain_api::acquire::*;
use lain_store::Bucket;

use crate::error::*;
use crate::manager::{Deps, Manager};
use crate::subtitles::{ProbeInfo, ProbeStream, SubtitleTarget};
use crate::testsupport::{Fixture, Reply, serve};
use crate::util::within;
use crate::world_tests::{World, add_indexer, monitor, settings, wait_grab};

#[derive(Default)]
struct SubState {
    bodies: HashMap<String, Vec<u8>>,
    cands: Vec<SubtitleCandidate>,
    searches: Vec<SubtitleSearchInput>,
    info: ProbeInfo,
}

struct SubWorld {
    st: Arc<Mutex<SubState>>,
    files: Fixture,
}

impl SubWorld {
    fn new() -> SubWorld {
        let st: Arc<Mutex<SubState>> = Arc::default();
        let s2 = st.clone();
        let files = serve(move |s| match s2.lock().unwrap().bodies.get(s.path.trim_start_matches("/f/")) {
            Some(b) => Reply::ok(b.clone()),
            None => Reply::status(404),
        });
        SubWorld { st, files }
    }

    fn wire(&self, d: &mut Deps) {
        let (a, b) = (self.st.clone(), self.st.clone());
        d.subtitles.probe = Some(Arc::new(move |_: &Path| Ok(a.lock().unwrap().info.clone())));
        d.subtitles.search = Some(Arc::new(move |i: &SubtitleSearchInput| {
            let mut s = b.lock().unwrap();
            s.searches.push(i.clone());
            Ok(s.cands.clone())
        }));
        let url = self.files.url.clone();
        d.subtitles.download = Some(Arc::new(move |i: &SubtitleDownloadInput| {
            Ok(SubtitleDownloadOutput { link: format!("{url}/f/{}", i.file_id), remaining: 9, ..Default::default() })
        }));
    }

    fn set(&self, f: impl FnOnce(&mut SubState)) {
        f(&mut self.st.lock().unwrap());
    }
}

const GOOD_SRT: &[u8] = b"1\n00:00:01,000 --> 00:00:02,000\nOl\xe1\n\n2\n00:20:00,000 --> 00:20:01,000\nfim\n";

/// Windows-1252 bytes, as a provider may serve them.
fn good() -> Vec<u8> {
    GOOD_SRT.to_vec()
}

fn setup(tag: &str) -> (World, SubWorld, Arc<Manager>) {
    let w = World::new();
    let sw = SubWorld::new();
    let dir = lain_store::test_dir(tag);
    std::fs::create_dir_all(&dir).unwrap();
    let m = w.manager_with(&dir, settings(0.0), |d| sw.wire(d));
    (w, sw, m)
}

fn sub_profile(m: &Manager, langs: &[&str]) -> Profile {
    let p = Profile {
        subtitle_languages: langs.iter().map(|s| s.to_string()).collect(),
        ..crate::decide::default_profile()
    };
    m.update_profile("default", p).unwrap()
}

fn add_provider(m: &Manager) -> SubtitleProvider {
    m.create_subtitle_provider(&SubtitleProviderInput {
        name: Some("os".into()),
        kind: Some(SUBTITLE_OPENSUBTITLES.into()),
        api_key: Some("k".into()),
        ..Default::default()
    })
    .unwrap()
}

fn media(lib: &str, name: &str) -> PathBuf {
    let p = Path::new(lib).join("SHOW").join(name);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, vec![0u8; 200_000]).unwrap();
    p
}

fn stream(kind: &str, lang: &str) -> ProbeStream {
    ProbeStream { kind: kind.into(), language: lang.into(), forced: false }
}

fn cand(provider: &str, file: &str, lang: &str) -> SubtitleCandidate {
    SubtitleCandidate {
        provider_id: provider.into(),
        file_id: file.into(),
        language: lang.into(),
        ..Default::default()
    }
}

#[test]
fn wanted_subtitle_languages() {
    let (w, sw, m) = setup("acq-s1");
    let mut p = sub_profile(&m, &["eng", "por", "spa"]);
    let file = media(&w.lib.path, "[Fansub-A] SHOW - 01.mkv");
    sw.set(|s| {
        s.info = ProbeInfo { duration: 1440.0, streams: vec![stream("audio", "spa"), stream("subtitle", "eng")] }
    });
    let dir = file.parent().unwrap();
    std::fs::write(dir.join("[Fansub-A] SHOW - 01.pt-BR.forced.srt"), GOOD_SRT).unwrap();
    // eng: embedded; spa: the audio; por: only a forced sidecar.
    assert_eq!(m.missing_subtitles(&file, &p), vec!["por"]);
    std::fs::write(dir.join("[Fansub-A] SHOW - 01.pt.srt"), GOOD_SRT).unwrap();
    assert!(m.missing_subtitles(&file, &p).is_empty(), "a full sidecar satisfies the language");
    p.subtitle_even_with_audio = true;
    assert_eq!(m.missing_subtitles(&file, &p), vec!["spa"]);
    m.close();
}

#[test]
fn subtitle_ranking() {
    let (w, sw, m) = setup("acq-s2");
    add_provider(&m);
    let mut p = sub_profile(&m, &["por"]);
    p.subtitle_hi = HI_EXCLUDE.into();
    let file = media(&w.lib.path, "[Fansub-A] SHOW - 05 [1080p].mkv");
    let c = |id: &str| cand("", id, "por");
    sw.set(|s| {
        s.cands = vec![
            SubtitleCandidate { downloads: 5000, release: "[Fansub-B] Show - 05 [720p]".into(), ..c("1") },
            SubtitleCandidate { hash_match: true, downloads: 1, ..c("2") },
            SubtitleCandidate { release: "[Fansub-A] Show - 05 [1080p]".into(), downloads: 10, ..c("3") },
            SubtitleCandidate { forced: true, hash_match: true, ..c("4") },
            SubtitleCandidate { hi: true, hash_match: true, ..c("5") },
            SubtitleCandidate { episode: 6, hash_match: true, ..c("6") },
            SubtitleCandidate { hash_match: true, ..cand("", "7", "eng") },
        ]
    });
    let t = SubtitleTarget { path: file, kind: "anime".into(), title: "Show".into(), episode: 5, ..Default::default() };
    let res = m.search_subtitles(&t, &p, &["por".into()]).unwrap();
    let ids =
        |acc: bool| res.iter().filter(|c| c.accepted == acc).map(|c| c.candidate.file_id.as_str()).collect::<Vec<_>>();
    assert_eq!(ids(true), vec!["2", "3", "1"]);
    assert_eq!(ids(false), vec!["4", "5", "6", "7"]);
    let st = sw.st.lock().unwrap();
    assert_eq!(st.searches.len(), 1);
    assert!(!st.searches[0].hash.is_empty() && st.searches[0].episode == 5 && st.searches[0].languages == vec!["por"]);
    drop(st);
    m.close();
}

#[test]
fn download_places_a_sidecar_without_overwriting() {
    let (w, sw, m) = setup("acq-s3");
    let prov = add_provider(&m);
    let file = media(&w.lib.path, "[Fansub-A] SHOW - 01.mkv");
    sw.set(|s| {
        s.info.duration = 1440.0;
        s.bodies.insert("9".into(), good());
    });
    let c = SubtitleCandidate { region: "pt-BR".into(), file_name: "x.srt".into(), ..cand(&prov.id, "9", "por") };
    let rec = m.download_subtitle(&file, &c, false).unwrap();
    let want = Path::new(&w.lib.path).join("SHOW/[Fansub-A] SHOW - 01.pt-BR.srt");
    assert_eq!(Path::new(&rec.path), want);
    assert!(std::fs::read_to_string(&want).unwrap().contains("Olá"), "sidecar must be UTF-8");
    let ledger = m.subtitle_ledger();
    assert!(ledger.len() == 1 && ledger[0].file_id == "9");
    assert_eq!(m.download_subtitle(&file, &c, false).unwrap_err().code, CODE_STATE);
    sw.set(|s| {
        s.bodies.insert("9".into(), String::from_utf8_lossy(GOOD_SRT).replace("fim", "novo").into_bytes());
    });
    m.download_subtitle(&file, &c, true).unwrap();
    assert!(std::fs::read_to_string(&want).unwrap().contains("novo"));
    assert_eq!(m.replaced().len(), 1, "the old sidecar must be held, not deleted");
    let outside = lain_store::test_dir("acq-s3-out").join("x.mkv");
    std::fs::create_dir_all(outside.parent().unwrap()).unwrap();
    std::fs::write(&outside, b"v").unwrap();
    assert_eq!(m.download_subtitle(&outside, &c, false).unwrap_err().code, CODE_INVALID);
    m.close();
}

#[test]
fn sync_check_refuses_mismatched_subtitles() {
    let (w, sw, m) = setup("acq-s4");
    let prov = add_provider(&m);
    let file = media(&w.lib.path, "[Fansub-A] SHOW - 01.mkv");
    sw.set(|s| {
        s.info.duration = 600.0; // 10 min; the subtitle runs 20 min
        s.bodies.insert("9".into(), good());
    });
    let e = m.download_subtitle(&file, &cand(&prov.id, "9", "eng"), false).unwrap_err();
    assert_eq!(e.code, CODE_SUBTITLE_MISMATCH);
    assert_eq!(e.msg, "the subtitle runs to 20m1s but the video is 10m0s long (another cut, episode or frame rate)");
    assert!(!Path::new(&w.lib.path).join("SHOW/[Fansub-A] SHOW - 01.en.srt").exists());
    let p = sub_profile(&m, &["eng"]);
    sw.set(|s| s.cands = vec![SubtitleCandidate { hash_match: true, ..cand(&prov.id, "9", "eng") }]);
    let t = SubtitleTarget {
        path: file.clone(),
        kind: "anime".into(),
        title: "Show".into(),
        episode: 1,
        ..Default::default()
    };
    let res = m.search_subtitles(&t, &p, &["eng".into()]).unwrap();
    assert!(res.len() == 1 && !res[0].accepted, "a refused file must stay refused");
    // Unknown duration: the check cannot run, the file is kept.
    sw.set(|s| {
        s.info = ProbeInfo::default();
        s.bodies.insert("10".into(), good());
        s.bodies.insert("11".into(), b"nothing".to_vec());
    });
    m.download_subtitle(&file, &cand(&prov.id, "10", "eng"), false).unwrap();
    assert_eq!(
        m.download_subtitle(&file, &cand(&prov.id, "11", "spa"), false).unwrap_err().code,
        CODE_SUBTITLE_MISMATCH
    );
    m.close();
}

#[test]
fn subtitles_follow_a_monitored_import() {
    let (w, sw, m) = setup("acq-s5");
    let prov = add_provider(&m);
    sub_profile(&m, &["eng"]);
    sw.set(|s| {
        s.info.duration = 1440.0;
        s.bodies.insert("9".into(), good());
        s.cands = vec![SubtitleCandidate { hash_match: true, ..cand(&prov.id, "9", "eng") }];
    });
    add_indexer(&m, &w);
    let mo = monitor(&m, &w, Monitored { title: "Show".into(), from: 1, to: 2, ..Default::default() });
    let rep = m.search_monitored(&mo.id).unwrap();
    assert_eq!(rep.grabbed.len(), 1);
    wait_grab(&m, &rep.grabbed[0], &[GRAB_DONE]);
    // Both episodes' sidecars, from the pass that follows the import.
    let show = Path::new(&w.lib.path).join("SHOW");
    let want = [show.join("[Fansub-A] SHOW - 01.en.srt"), show.join("[Fansub-A] SHOW - 02.en.srt")];
    let deadline = Instant::now() + Duration::from_secs(15);
    while !want.iter().all(|p| p.exists()) || m.subtitle_ledger().len() < 2 {
        assert!(Instant::now() < deadline, "the subtitle never arrived");
        std::thread::sleep(Duration::from_millis(50));
    }
    let st = m.subtitle_status(&mo.id).unwrap();
    assert!(st.len() == 2 && st[0].missing.is_empty(), "{st:?}");
    assert_eq!(m.subtitles_for_monitored(&mo.id).unwrap(), 0, "nothing missing any more");
    m.close();
}

#[test]
fn subtitle_refusals_can_be_listed_and_cleared() {
    let (w, sw, m) = setup("acq-s6");
    let prov = add_provider(&m);
    let file = media(&w.lib.path, "[Fansub-A] SHOW - 01.mkv");
    sw.set(|s| {
        s.info.duration = 600.0;
        s.bodies.insert("9".into(), good());
        s.bodies.insert("10".into(), b"nothing".to_vec());
    });
    for id in ["9", "10"] {
        assert_eq!(
            m.download_subtitle(&file, &cand(&prov.id, id, "eng"), false).unwrap_err().code,
            CODE_SUBTITLE_MISMATCH
        );
    }
    let refs = m.subtitle_refusals();
    assert_eq!(refs.len(), 2);
    for r in &refs {
        assert!(r.provider_id == prov.id && !r.reason.is_empty() && !r.reason.starts_with(CODE_SUBTITLE_MISMATCH));
        assert!(Path::new(&r.media_path) == file && r.at > 0, "{r:?}");
    }
    let p = sub_profile(&m, &["eng"]);
    sw.set(|s| s.cands = vec![cand(&prov.id, "9", "eng")]);
    let t = SubtitleTarget {
        path: file.clone(),
        kind: "anime".into(),
        title: "Show".into(),
        episode: 1,
        ..Default::default()
    };
    assert!(!m.search_subtitles(&t, &p, &["eng".into()]).unwrap()[0].accepted);
    m.clear_subtitle_refusal(&prov.id, "9").unwrap();
    assert_eq!(m.clear_subtitle_refusal(&prov.id, "9").unwrap_err().code, CODE_NOT_FOUND);
    assert!(m.search_subtitles(&t, &p, &["eng".into()]).unwrap()[0].accepted);
    assert_eq!(m.clear_subtitle_refusals(), 1);
    assert!(m.subtitle_refusals().is_empty());
    m.close();
}

#[test]
fn subtitle_refusals_read_bare_reasons() {
    let (_w, _sw, m) = setup("acq-s7");
    // A refusal holding the reason alone still lists and blocks.
    m.st.put(Bucket::AcqSubBlock, "p1\u{0}42", &"too long").unwrap();
    let refs = m.subtitle_refusals();
    assert!(refs.len() == 1 && refs[0].provider_id == "p1" && refs[0].file_id == "42" && refs[0].reason == "too long");
    m.close();
}

#[test]
fn remove_sidecar_holds_it_and_never_deletes() {
    let (w, sw, m) = setup("acq-s8");
    let prov = add_provider(&m);
    let file = media(&w.lib.path, "[Fansub-A] SHOW - 01.mkv");
    sw.set(|s| {
        s.bodies.insert("9".into(), good());
    });
    let rec = m.download_subtitle(&file, &cand(&prov.id, "9", "eng"), false).unwrap();
    let dir = file.parent().unwrap().to_path_buf();
    let own = dir.join("[Fansub-A] SHOW - 01.ja.ass");
    std::fs::write(&own, b"[Script Info]\n").unwrap();
    let other = media(&w.lib.path, "[Fansub-A] SHOW - 02.mkv");
    let other_sub = other.parent().unwrap().join("[Fansub-A] SHOW - 02.en.srt");
    std::fs::write(&other_sub, GOOD_SRT).unwrap();
    let base = |p: &Path| p.file_name().unwrap().to_string_lossy().into_owned();
    for name in [base(Path::new(&rec.path)), base(&own)] {
        let held = m.remove_sidecar(&file, &name).unwrap();
        assert!(!dir.join(&name).exists(), "{name} still in the library");
        assert!(Path::new(&held.path).exists() && within(&m.hold_dir(), Path::new(&held.path)), "{held:?}");
    }
    assert!(m.subtitle_ledger().is_empty(), "a removed sidecar leaves the ledger");
    assert_eq!(m.replaced().len(), 2);
    for name in [base(&other_sub), base(&file), format!("../SHOW/{}", base(&other_sub)), "nope.en.srt".into()] {
        assert_eq!(m.remove_sidecar(&file, &name).unwrap_err().code, CODE_NOT_FOUND, "{name}");
    }
    assert!(other_sub.exists() && file.exists());
    m.close();
}

#[test]
fn item_subtitle_status_uses_the_governing_profile() {
    let (w, _sw, m) = setup("acq-s9");
    let file = media(&w.lib.path, "[Fansub-A] SHOW - 01.mkv");
    std::fs::write(file.parent().unwrap().join("[Fansub-A] SHOW - 01.en.srt"), GOOD_SRT).unwrap();
    sub_profile(&m, &["eng", "por"]);
    let it = CatalogItem {
        id: "i1".into(),
        library_id: w.lib.id.clone(),
        title: "Unmonitored".into(),
        file_path: file.to_string_lossy().into(),
        ..Default::default()
    };
    let (st, langs) = m.item_subtitle_status(&it).unwrap();
    assert_eq!(langs, vec!["eng", "por"]);
    assert!(st.missing == vec!["por"] && st.sidecars.len() == 1 && st.item_id == "i1", "{st:?}");
    m.close();
}

#[test]
fn provider_accounts() {
    let (_w, _sw, m) = setup("acq-s10");
    let bad = |f: fn(&mut SubtitleProviderInput)| {
        let mut i = SubtitleProviderInput {
            name: Some("os".into()),
            kind: Some(SUBTITLE_OPENSUBTITLES.into()),
            api_key: Some("k".into()),
            ..Default::default()
        };
        f(&mut i);
        m.create_subtitle_provider(&i).unwrap_err().code
    };
    assert_eq!(bad(|i| i.kind = Some("other".into())), CODE_INVALID);
    assert_eq!(bad(|i| i.api_key = None), CODE_INVALID);
    assert_eq!(bad(|i| i.base_url = Some("ftp://x".into())), CODE_INVALID);
    assert_eq!(bad(|i| i.name = Some(String::new())), CODE_INVALID);
    let p = add_provider(&m);
    assert!(p.api_key.is_empty());
    assert_eq!(m.subtitle_provider_secrets(&p.id), (true, false));
    m.update_subtitle_provider(&p.id, &SubtitleProviderInput { password: Some("pw".into()), ..Default::default() })
        .unwrap();
    assert_eq!(m.subtitle_provider_secrets(&p.id), (true, true));
    m.update_subtitle_provider(&p.id, &SubtitleProviderInput { password: Some("-".into()), ..Default::default() })
        .unwrap();
    assert_eq!(m.subtitle_provider_secrets(&p.id), (true, false));
    assert_eq!(m.subtitle_providers().len(), 1);
    m.delete_subtitle_provider(&p.id).unwrap();
    assert_eq!(m.delete_subtitle_provider(&p.id).unwrap_err().code, CODE_NOT_FOUND);
    m.close();
}
