//! Scrobble planning: whether finishing an item should advance one of
//! the user's platform entries. Opt-in per account and conservative: it
//! only advances an entry the user already tracks, matched by the
//! platform id the title's metadata names or else by its names
//! ([`match_key`]), never regresses progress and never reopens a
//! completed entry. Episodes advance anime entries; chapters and
//! volumes advance manga entries (D-288). Go: `internal/gateway/scrobble.go`.

use std::collections::BTreeMap;

use lain_api::{list_media, list_status};

use crate::{ListEntry, match_key};

/// The finished item, as the catalog knows it. For a comic or manga
/// `season` is the volume and `episode` the chapter (the identifier's
/// convention).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Finished {
    pub kind: String,
    pub title: String,
    /// Other names of the title (the metadata's, the catalog aliases).
    pub aliases: Vec<String>,
    pub season: i64,
    pub episode: i64,
    /// Platform → the work's id there, when the title's metadata names
    /// one (AniList's metadata provider does).
    pub remote_ids: BTreeMap<String, String>,
}

/// What to write for one finished item. An absent count is left alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub remote_id: String,
    pub title: String,
    pub progress: Option<i64>,
    pub progress_volumes: Option<i64>,
    pub status: String,
}

/// Whether an item is read rather than watched.
pub fn is_reading(kind: &str) -> bool {
    matches!(kind, "manga" | "comic")
}

/// The one entry `item` is, or `None` when no entry or more than one
/// could be it: an ambiguous title is never guessed.
pub fn find<'a>(
    platform: &str,
    remote_id: Option<&str>,
    titles: &[&str],
    media_type: &str,
    entries: &'a [ListEntry],
) -> Option<(&'a ListEntry, &'static str)> {
    let mut cands = entries.iter().filter(|e| e.platform == platform && e.media_type == media_type);
    if let Some(id) = remote_id.filter(|id| !id.is_empty())
        && let Some(m) = cands.clone().find(|e| e.remote_id == id)
    {
        return Some((m, lain_api::list_match::ID));
    }
    let keys: Vec<String> = titles.iter().map(|t| match_key(t)).filter(|k| !k.is_empty()).collect();
    let mut hits = cands
        .by_ref()
        .filter(|e| std::iter::once(&e.title).chain(e.alt_titles.iter()).any(|t| keys.contains(&match_key(t))));
    let m = hits.next()?;
    if hits.any(|o| o.remote_id != m.remote_id) {
        return None;
    }
    Some((m, lain_api::list_match::TITLE))
}

/// Later seasons of anime are skipped: platforms list each season as
/// its own entry, and a per-season episode number against the wrong
/// entry would corrupt the list. A film advances only an entry of one
/// episode (a film on the list).
pub fn plan(item: &Finished, platform: &str, entries: &[ListEntry]) -> Option<Decision> {
    let reading = is_reading(&item.kind);
    let media = if reading { list_media::MANGA } else { list_media::ANIME };
    let titles: Vec<&str> =
        std::iter::once(item.title.as_str()).chain(item.aliases.iter().map(String::as_str)).collect();
    let (m, _) = find(platform, item.remote_ids.get(platform).map(String::as_str), &titles, media, entries)?;
    if m.status == list_status::COMPLETED || m.status == list_status::REPEATING {
        return None;
    }
    let done = |n: i64| m.progress_total > 0 && n == m.progress_total;
    let decide = |progress: Option<i64>, volumes: Option<i64>, complete: bool| Decision {
        remote_id: m.remote_id.clone(),
        title: m.title.clone(),
        progress,
        progress_volumes: volumes,
        status: if complete { list_status::COMPLETED } else { list_status::CURRENT }.into(),
    };
    if reading {
        let (volume, chapter) = (item.season, item.episode);
        let chapter_ok = chapter > m.progress && (m.progress_total == 0 || chapter <= m.progress_total);
        let volume_ok = volume > m.progress_volumes;
        if !chapter_ok && !volume_ok {
            return None;
        }
        return Some(decide(chapter_ok.then_some(chapter), volume_ok.then_some(volume), chapter_ok && done(chapter)));
    }
    if item.kind == "movie" || (item.season == 0 && item.episode == 0) {
        return (m.progress_total == 1 && m.progress < 1).then(|| decide(Some(1), None, true));
    }
    if item.episode <= 0 || item.season > 1 {
        return None;
    }
    if item.episode <= m.progress || (m.progress_total > 0 && item.episode > m.progress_total) {
        return None;
    }
    Some(decide(Some(item.episode), None, done(item.episode)))
}

/// Applies a decision to the local copy, as the platform will hold it.
pub fn apply(e: &mut ListEntry, d: &Decision) {
    if let Some(p) = d.progress {
        e.progress = p;
    }
    if let Some(v) = d.progress_volumes {
        e.progress_volumes = v;
    }
    if !d.status.is_empty() {
        e.status = d.status.clone();
    }
}

/// Plans a run of finished items against one list, as if each were
/// pushed before the next: marking a whole season watched becomes one
/// write per entry (the highest progress, completing it when the last
/// episode is among them), not one per episode (AniList's rate limit).
pub fn plan_many(items: &[Finished], platform: &str, entries: &[ListEntry]) -> Vec<Decision> {
    let mut local = entries.to_vec();
    let mut out: Vec<Decision> = Vec::new();
    for item in items {
        let Some(d) = plan(item, platform, &local) else { continue };
        if let Some(e) = local.iter_mut().find(|e| e.platform == platform && e.remote_id == d.remote_id) {
            apply(e, &d);
        }
        match out.iter_mut().find(|o| o.remote_id == d.remote_id) {
            Some(o) => {
                o.progress = d.progress.or(o.progress);
                o.progress_volumes = d.progress_volumes.or(o.progress_volumes);
                o.status = d.status;
            }
            None => out.push(d),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(rid: &str, title: &str, status: &str, progress: i64, total: i64) -> ListEntry {
        ListEntry {
            platform: "anilist".into(),
            remote_id: rid.into(),
            media_type: "anime".into(),
            title: title.into(),
            status: status.into(),
            progress,
            progress_total: total,
            ..ListEntry::default()
        }
    }

    fn ep(title: &str, season: i64, episode: i64) -> Finished {
        Finished { kind: "episode".into(), title: title.into(), season, episode, ..Finished::default() }
    }

    fn d(rid: &str, title: &str, progress: Option<i64>, volumes: Option<i64>, status: &str) -> Decision {
        Decision {
            remote_id: rid.into(),
            title: title.into(),
            progress,
            progress_volumes: volumes,
            status: status.into(),
        }
    }

    /// Go's `TestPlanScrobble`.
    #[test]
    fn plans() {
        let list = vec![
            e("1", "Frieren", "current", 3, 28),
            e("2", "Bocchi the Rock!", "completed", 12, 12),
            e("3", "Dup", "current", 0, 0),
            e("4", "dup", "planning", 0, 0),
        ];
        let got = plan(&ep("frieren ", 1, 4), "anilist", &list).unwrap();
        assert_eq!(got, d("1", "Frieren", Some(4), None, "current"));
        assert_eq!(plan(&ep("Frieren", 0, 28), "anilist", &list).unwrap().status, "completed");
        for (item, why) in [
            (ep("Frieren", 1, 3), "no regression"),
            (ep("Frieren", 1, 29), "past the total"),
            (ep("Frieren", 2, 5), "later season"),
            (ep("Frieren", 1, 0), "no episode"),
            (ep("Bocchi the Rock!", 1, 13), "completed stays"),
            (ep("Dup", 1, 1), "ambiguous"),
            (ep("Unknown", 1, 1), "untracked"),
            (Finished { kind: "movie".into(), ..ep("Frieren", 0, 5) }, "a film never advances a series"),
        ] {
            assert_eq!(plan(&item, "anilist", &list), None, "{why}");
        }
        assert_eq!(plan(&ep("Frieren", 1, 4), "mal", &list), None, "other platform");
        let mut manga = list.clone();
        manga[0].media_type = "manga".into();
        assert_eq!(plan(&ep("Frieren", 1, 4), "anilist", &manga), None, "episodes advance anime entries only");
    }

    #[test]
    fn the_metadata_id_wins_over_the_title() {
        let mut list = vec![e("10", "Sousou no Frieren", "planning", 0, 28), e("11", "Frieren", "current", 2, 4)];
        list[0].alt_titles = vec!["Frieren: Beyond Journey's End".into()];
        let mut item = ep("Frieren", 1, 3);
        item.remote_ids.insert("anilist".into(), "10".into());
        assert_eq!(plan(&item, "anilist", &list).unwrap(), d("10", "Sousou no Frieren", Some(3), None, "current"));
        // An id the list does not hold falls back to the names, alt titles included.
        item.remote_ids.insert("anilist".into(), "999".into());
        item.title = "Frieren: Beyond Journey's End".into();
        assert_eq!(plan(&item, "anilist", &list).unwrap().remote_id, "10", "an alt title matches");
        item.title = "Something else".into();
        item.aliases = vec!["frieren".into()];
        assert_eq!(plan(&item, "anilist", &list).unwrap().remote_id, "11", "an alias matches");
    }

    #[test]
    fn films_complete_a_one_episode_entry() {
        let list = vec![e("5", "Your Name", "planning", 0, 1)];
        let film = Finished { kind: "movie".into(), title: "Your Name".into(), ..Finished::default() };
        assert_eq!(plan(&film, "anilist", &list).unwrap(), d("5", "Your Name", Some(1), None, "completed"));
        let video = Finished { kind: "video".into(), ..film.clone() };
        assert!(plan(&video, "anilist", &list).is_some(), "a loose video of the film too");
        let series = vec![e("5", "Your Name", "planning", 0, 12)];
        assert_eq!(plan(&film, "anilist", &series), None);
    }

    #[test]
    fn chapters_and_volumes_advance_manga() {
        let mut m = e("7", "One Piece", "current", 10, 0);
        m.media_type = "manga".into();
        m.progress_volumes = 1;
        let mut done = m.clone();
        done.remote_id = "8".into();
        done.title = "Solo".into();
        done.progress = 3;
        done.progress_total = 4;
        let list = vec![m, done];
        let ch = |title: &str, vol: i64, chapter: i64| Finished {
            kind: "manga".into(),
            title: title.into(),
            season: vol,
            episode: chapter,
            ..Finished::default()
        };
        assert_eq!(
            plan(&ch("One Piece", 0, 11), "anilist", &list).unwrap(),
            d("7", "One Piece", Some(11), None, "current")
        );
        assert_eq!(
            plan(&ch("One Piece", 2, 0), "anilist", &list).unwrap(),
            d("7", "One Piece", None, Some(2), "current")
        );
        assert_eq!(
            plan(&ch("One Piece", 2, 9), "anilist", &list).unwrap(),
            d("7", "One Piece", None, Some(2), "current")
        );
        assert_eq!(plan(&ch("One Piece", 1, 10), "anilist", &list), None, "nothing advances");
        assert_eq!(plan(&ch("Solo", 0, 4), "anilist", &list).unwrap().status, "completed");
        let comic = Finished { kind: "comic".into(), ..ch("One Piece", 0, 12) };
        assert_eq!(plan(&comic, "anilist", &list).unwrap().progress, Some(12), "a comic library's chapter too");
        assert_eq!(plan(&ep("One Piece", 1, 12), "anilist", &list), None, "an episode never advances manga");
    }

    #[test]
    fn a_run_of_finishes_is_one_write_per_entry() {
        let list = vec![e("1", "Frieren", "planning", 0, 3), e("2", "Bocchi", "current", 0, 12)];
        let items = vec![ep("Frieren", 1, 1), ep("Frieren", 1, 3), ep("Frieren", 1, 2), ep("Bocchi", 1, 1)];
        let got = plan_many(&items, "anilist", &list);
        assert_eq!(
            got,
            vec![d("1", "Frieren", Some(3), None, "completed"), d("2", "Bocchi", Some(1), None, "current")],
            "the highest episode wins, the out-of-order one is a regression and skipped, the last completes"
        );
        assert!(plan_many(&[ep("Frieren", 1, 0)], "anilist", &list).is_empty());
    }
}
