//! Scrobble planning: whether finishing an item should advance one of
//! the user's platform entries. Opt-in per account and conservative: it
//! only advances an entry the user already tracks, matched by exact
//! normalized title, never regresses progress and never reopens a
//! completed entry. Go: `internal/gateway/scrobble.go`.

use lain_api::{list_media, list_status};

use crate::{ListEntry, title_key};

/// The finished item, as the catalog knows it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Finished {
    pub kind: String,
    pub title: String,
    pub season: i64,
    pub episode: i64,
}

/// What to write for one finished episode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub remote_id: String,
    pub progress: i64,
    pub status: String,
}

/// Later seasons are skipped: platforms list each season as its own
/// entry, and a per-season episode number against the wrong entry would
/// corrupt the list. An ambiguous title is never guessed.
pub fn plan(item: &Finished, platform: &str, entries: &[ListEntry]) -> Option<Decision> {
    if item.episode <= 0 || item.season > 1 || item.kind == "movie" {
        return None;
    }
    let key = title_key(&item.title);
    let mut matches = entries
        .iter()
        .filter(|e| e.platform == platform && e.media_type == list_media::ANIME && title_key(&e.title) == key);
    let m = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    if m.status == list_status::COMPLETED || m.status == list_status::REPEATING {
        return None;
    }
    if item.episode <= m.progress || (m.progress_total > 0 && item.episode > m.progress_total) {
        return None;
    }
    let status = if m.progress_total > 0 && item.episode == m.progress_total {
        list_status::COMPLETED
    } else {
        list_status::CURRENT
    };
    Some(Decision { remote_id: m.remote_id.clone(), progress: item.episode, status: status.into() })
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
        Finished { kind: "episode".into(), title: title.into(), season, episode }
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
        let d = plan(&ep("frieren ", 1, 4), "anilist", &list).unwrap();
        assert_eq!(d, Decision { remote_id: "1".into(), progress: 4, status: "current".into() });
        assert_eq!(plan(&ep("Frieren", 0, 28), "anilist", &list).unwrap().status, "completed");
        for (item, why) in [
            (ep("Frieren", 1, 3), "no regression"),
            (ep("Frieren", 1, 29), "past the total"),
            (ep("Frieren", 2, 5), "later season"),
            (ep("Frieren", 1, 0), "no episode"),
            (ep("Bocchi the Rock!", 1, 13), "completed stays"),
            (ep("Dup", 1, 1), "ambiguous"),
            (ep("Unknown", 1, 1), "untracked"),
            (Finished { kind: "movie".into(), ..ep("Frieren", 0, 5) }, "movie"),
        ] {
            assert_eq!(plan(&item, "anilist", &list), None, "{why}");
        }
        assert_eq!(plan(&ep("Frieren", 1, 4), "mal", &list), None, "other platform");
        let mut manga = list.clone();
        manga[0].media_type = "manga".into();
        assert_eq!(plan(&ep("Frieren", 1, 4), "anilist", &manga), None, "anime entries only");
    }
}
