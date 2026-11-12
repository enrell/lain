//! Numbered files of one folder are episodes of one title.
//!
//! An identifier sees one file at a time, and "Some Title 04" alone could
//! as well be a film called that, so the name-based identifiers leave it
//! a single video. The folder settles it: several files that differ only
//! by a trailing number are a series. This pass runs over one library's
//! scan, after identification, and only rewrites what the generic
//! identifier named.

use std::collections::HashMap;

use lain_identify::GENERIC_ID;

use crate::commit::ScanEntry;

/// Fewer than this many numbered files in a folder are left alone.
const MIN_SIBLINGS: usize = 3;
/// From this many on, the numbers need no leading zero to count: films
/// rarely reach "Title 5" side by side, episodes do.
const MANY_SIBLINGS: usize = 5;

/// `"Some Title 04"` → `("Some Title", 4, true)`; the flag says the number
/// was written with a leading zero. A year is not an episode number.
fn split_number(title: &str) -> Option<(&str, i64, bool)> {
    let (prefix, digits) = title.rsplit_once(' ')?;
    let prefix = prefix.trim_end();
    if prefix.is_empty() || digits.is_empty() || digits.len() > 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: i64 = digits.parse().ok()?;
    if n == 0 || (digits.len() == 4 && (1900..=2099).contains(&n)) {
        return None;
    }
    Some((prefix, n, digits.starts_with('0')))
}

fn folder(path: &str) -> &str {
    path.rsplit_once(['/', '\\']).map_or("", |(dir, _)| dir)
}

/// An entry's index, its number, and whether it had a leading zero.
type Member = (usize, i64, bool);

pub fn number_siblings(entries: &mut [ScanEntry]) {
    // (folder, lower-cased prefix) -> the entries and their numbers.
    let mut groups: HashMap<(String, String), Vec<Member>> = HashMap::new();
    for (at, e) in entries.iter().enumerate() {
        let p = &e.proposal;
        if !e.identified || p.plugin_id != GENERIC_ID || p.kind != "video" || p.episode != 0 {
            continue;
        }
        if let Some((prefix, n, padded)) = split_number(&p.title) {
            groups
                .entry((folder(&e.candidate.path).to_string(), prefix.to_lowercase()))
                .or_default()
                .push((at, n, padded));
        }
    }
    for members in groups.into_values() {
        let mut numbers: Vec<i64> = members.iter().map(|m| m.1).collect();
        numbers.sort_unstable();
        numbers.dedup();
        let enough = members.len() >= MANY_SIBLINGS || (members.len() >= MIN_SIBLINGS && members.iter().any(|m| m.2));
        // Two files with one number are not a run of episodes.
        if !enough || numbers.len() != members.len() {
            continue;
        }
        for (at, n, _) in members {
            let p = &mut entries[at].proposal;
            if let Some((prefix, _, _)) = split_number(&p.title) {
                p.title = prefix.to_string();
            }
            p.kind = "episode".into();
            p.episode = n;
            p.evidence.push("numbered-siblings".into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lain_identify::{Candidate, Proposal};

    fn entry(path: &str, title: &str, plugin: &str) -> ScanEntry {
        ScanEntry {
            candidate: Candidate { path: path.into(), ..Candidate::default() },
            proposal: Proposal {
                kind: "video".into(),
                title: title.into(),
                confidence: 0.3,
                plugin_id: plugin.into(),
                ..Proposal::default()
            },
            identified: true,
            ..ScanEntry::default()
        }
    }

    fn generic(dir: &str, title: &str) -> ScanEntry {
        entry(&format!("{dir}/{title}.mkv"), title, GENERIC_ID)
    }

    fn outcome(entries: &[ScanEntry]) -> Vec<(String, String, i64)> {
        entries.iter().map(|e| (e.proposal.kind.clone(), e.proposal.title.clone(), e.proposal.episode)).collect()
    }

    #[test]
    fn padded_numbers_in_one_folder_are_episodes() {
        let mut e = vec![
            generic("/m/a", "Some Show 01"),
            generic("/m/a", "Some Show 02"),
            generic("/m/a", "some show 10"),
            generic("/m/a", "Some Show"),
        ];
        number_siblings(&mut e);
        assert_eq!(
            outcome(&e),
            vec![
                ("episode".into(), "Some Show".into(), 1),
                ("episode".into(), "Some Show".into(), 2),
                ("episode".into(), "some show".into(), 10),
                ("video".into(), "Some Show".into(), 0),
            ]
        );
        assert!(e[0].proposal.evidence.contains(&"numbered-siblings".to_string()));
    }

    #[test]
    fn sequels_years_and_other_folders_are_left_alone() {
        // Three unpadded numbers read like sequels.
        let mut e = vec![generic("/m", "A Film 2"), generic("/m", "A Film 3"), generic("/m", "A Film 4")];
        number_siblings(&mut e);
        assert!(e.iter().all(|x| x.proposal.kind == "video" && x.proposal.episode == 0));
        // Five or more are a run even without padding.
        let mut e: Vec<ScanEntry> = (1..=5).map(|n| generic("/m", &format!("A Run {n}"))).collect();
        number_siblings(&mut e);
        assert!(e.iter().all(|x| x.proposal.kind == "episode" && x.proposal.title == "A Run"));
        // Years, a repeated number, different folders, another identifier.
        let mut e = vec![
            generic("/m", "Event 2019"),
            generic("/m", "Event 2020"),
            generic("/m", "Event 2021"),
            generic("/m", "Twice 01"),
            generic("/m", "Twice 01"),
            generic("/m", "Twice 02"),
            generic("/x", "Apart 01"),
            generic("/y", "Apart 02"),
            generic("/z", "Apart 03"),
            entry("/m/Named 01.mkv", "Named 01", "other"),
            entry("/m/Named 02.mkv", "Named 02", "other"),
            entry("/m/Named 03.mkv", "Named 03", "other"),
        ];
        let before = outcome(&e);
        number_siblings(&mut e);
        assert_eq!(outcome(&e), before);
    }
}
