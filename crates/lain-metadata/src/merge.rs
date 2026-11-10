//! The merge of `lain.metadata.search@1` outputs: dedup by provider and
//! normalized title, exact title first, then binding order; hits that
//! do not plausibly answer the search are dropped, so a fuzzy upstream
//! hit never replaces the real identity.

use crate::LOCAL_PROVIDERS;
use crate::types::MetadataCandidate;

/// Lower-case letters and digits only: "Darling in the FranXX" and
/// "darling-in-the-franxx" are one key.
pub fn normalize_title(s: &str) -> String {
    s.chars().map(|c| c.to_lowercase().next().unwrap_or(c)).filter(|c| c.is_alphabetic() || c.is_numeric()).collect()
}

fn plausible_match(want: &str, have: &str) -> bool {
    if have.is_empty() {
        return false;
    }
    if have == want {
        return true;
    }
    let (shorter, longer) = if have.len() < want.len() { (have, want) } else { (want, have) };
    shorter.len() >= 4 && longer.starts_with(shorter)
}

/// An exact title, or a title or synonym that prefixes the other
/// (season and edition variants). Local hits (NFO) are
/// curated: they always pass.
fn plausible(want: &str, c: &MetadataCandidate) -> bool {
    if want.is_empty() || LOCAL_PROVIDERS.contains(&c.provider.as_str()) {
        return true;
    }
    std::iter::once(&c.title).chain(&c.synonyms).any(|t| plausible_match(want, &normalize_title(t)))
}

/// Merges outputs given in binding order (each a candidate list).
pub fn merge_candidates(query: &str, outputs: &[Vec<MetadataCandidate>], limit: usize) -> Vec<MetadataCandidate> {
    let want = normalize_title(query);
    let mut all: Vec<(MetadataCandidate, usize, bool)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (order, list) in outputs.iter().enumerate() {
        for c in list {
            if c.title.is_empty() || !plausible(&want, c) {
                continue;
            }
            let norm = normalize_title(&c.title);
            if !seen.insert(format!("{}\0{norm}", c.provider)) {
                continue;
            }
            all.push((c.clone(), order, norm == want && !want.is_empty()));
        }
    }
    // Stable: exact matches first, then precedence.
    all.sort_by(|a, b| b.2.cmp(&a.2).then(a.1.cmp(&b.1)));
    let limit = if limit == 0 || limit > 20 { 10 } else { limit };
    let n = all.len();
    all.into_iter()
        .take(limit)
        .enumerate()
        .map(|(i, (mut c, _, exact))| {
            c.score = (n - i) as f64 + if exact { 1000.0 } else { 0.0 };
            c
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NFO_ID;

    fn c(p: &str, t: &str) -> MetadataCandidate {
        MetadataCandidate { provider: p.into(), remote_id: t.into(), title: t.into(), ..Default::default() }
    }

    #[test]
    fn exact_first_then_precedence_and_dedup() {
        let out = merge_candidates(
            "Frieren",
            &[
                vec![c("a", "Frieren: Beyond Journey's End"), c("a", "frieren")],
                vec![c("b", "Frieren"), c("b", "FRIEREN"), c("b", "Unrelated")],
            ],
            10,
        );
        let got: Vec<_> = out.iter().map(|c| (c.provider.as_str(), c.title.as_str())).collect();
        assert_eq!(got, vec![("a", "frieren"), ("b", "Frieren"), ("a", "Frieren: Beyond Journey's End")]);
        assert_eq!(out[0].score, 1003.0);
        assert_eq!(out[2].score, 1.0);
    }

    #[test]
    fn unrelated_and_short_prefixes_are_dropped() {
        assert_eq!(normalize_title("Darling in the FranXX!"), "darlinginthefranxx");
        assert!(merge_candidates("Akira", &[vec![c("a", "Akagi")]], 10).is_empty());
        assert!(merge_candidates("Ak", &[vec![c("a", "Akira")]], 10).is_empty(), "prefix shorter than 4");
        assert_eq!(merge_candidates("Akira", &[vec![c(NFO_ID, "Something Else")]], 10).len(), 1);
        let many: Vec<_> = (0..30).map(|i| c("a", &format!("Akira {i}"))).collect();
        assert_eq!(merge_candidates("Akira", std::slice::from_ref(&many), 0).len(), 10);
        assert_eq!(merge_candidates("Akira", &[many], 15).len(), 15);
    }
}
