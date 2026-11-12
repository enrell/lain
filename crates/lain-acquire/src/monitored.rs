//! Monitored titles and what they want (A-17, A-18, A-24). Pure: what
//! the library has is read from catalog items handed in, never written.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use lain_api::acquire::{
    Monitored, NUMBER_ABSOLUTE, NUMBER_CHAPTER, NUMBER_MOVIE, NUMBER_SEASONAL, NUMBER_VOLUME, Profile, SeasonStart,
    Unit, UpgradeUnit, Wanted,
};
use lain_api::{CatalogItem, Release};

use crate::decide::{meets_cutoff, rank_of};
use crate::errf;
use crate::error::{CODE_INVALID, CODE_NO_LIBRARY, Result};
use crate::util::{clean_name, loose_key, settle};

/// Converts between absolute and seasonal numbering; split cours are
/// further seasons (A-24).
pub struct SeasonMap<'a>(pub &'a [SeasonStart]);

impl SeasonMap<'_> {
    /// Ascending seasons and strictly increasing starts.
    pub fn validate(&self) -> Result<()> {
        for (i, s) in self.0.iter().enumerate() {
            if s.season < 1 || s.first < 1 {
                return Err(errf!(CODE_INVALID, "season map entries need season ≥ 1 and first episode ≥ 1"));
            }
            if i > 0 && (s.season <= self.0[i - 1].season || s.first <= self.0[i - 1].first) {
                return Err(errf!(CODE_INVALID, "season map must ascend: each season starts after the previous one"));
            }
        }
        if self.0.len() > 100 {
            return Err(errf!(CODE_INVALID, "at most 100 seasons"));
        }
        Ok(())
    }

    fn index(&self, season: i64) -> Option<usize> {
        self.0.iter().position(|s| s.season == season)
    }

    /// SxxEyy to an absolute episode.
    pub fn to_absolute(&self, season: i64, ep: i64) -> Option<i64> {
        let i = self.index(season)?;
        if ep < 1 {
            return None;
        }
        let abs = self.0[i].first + ep - 1;
        if i + 1 < self.0.len() && abs >= self.0[i + 1].first {
            return None; // past the season's end
        }
        Some(abs)
    }

    /// An absolute episode to SxxEyy.
    pub fn to_seasonal(&self, abs: i64) -> Option<(i64, i64)> {
        self.0.iter().rev().find(|s| abs >= s.first).map(|s| (s.season, abs - s.first + 1))
    }

    /// Known for every season but the last.
    pub fn season_length(&self, season: i64) -> Option<i64> {
        let i = self.index(season)?;
        (i + 1 < self.0.len()).then(|| self.0[i + 1].first - self.0[i].first)
    }
}

fn numbering_for(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "movie" => &[NUMBER_MOVIE],
        "series" => &[NUMBER_SEASONAL],
        "anime" => &[NUMBER_ABSOLUTE, NUMBER_SEASONAL],
        "manga" | "comic" => &[NUMBER_CHAPTER, NUMBER_VOLUME],
        _ => return None,
    })
}

/// Normalizes a monitored title or explains why it cannot be saved.
pub fn validate(mut m: Monitored) -> Result<Monitored> {
    let Some(allowed) = numbering_for(&m.kind) else {
        return Err(errf!(CODE_INVALID, "unknown kind {:?}", m.kind));
    };
    if m.library_id.is_empty() {
        return Err(errf!(CODE_NO_LIBRARY, "choose the library to keep complete"));
    }
    let t = clean_name(&m.title, 200)?;
    if t.is_empty() {
        return Err(errf!(CODE_INVALID, "title required"));
    }
    m.title = t;
    if m.aliases.len() > 10 {
        return Err(errf!(CODE_INVALID, "at most 10 aliases"));
    }
    let mut aliases = Vec::new();
    for a in &m.aliases {
        let a = clean_name(a, 200)?;
        if !a.is_empty() && loose_key(&a) != loose_key(&m.title) {
            aliases.push(a);
        }
    }
    m.aliases = aliases;
    if m.numbering.is_empty() {
        m.numbering = allowed[0].into();
    }
    if rank_of(allowed, &m.numbering).is_none() {
        return Err(errf!(CODE_INVALID, "a {} cannot use {} numbering", m.kind, m.numbering));
    }
    SeasonMap(&m.season_map).validate()?;
    if m.from < 0 || m.to < 0 || (m.to > 0 && m.to < m.from) {
        return Err(errf!(CODE_INVALID, "the range must run from a lower number to a higher one"));
    }
    if m.from == 0 && m.numbering != NUMBER_MOVIE && m.numbering != NUMBER_SEASONAL {
        m.from = 1;
    }
    let mut seen = BTreeSet::new();
    for s in m.seasons.iter_mut() {
        if s.season < 1 || seen.contains(&s.season) || s.from < 0 || (s.to > 0 && s.to < s.from) {
            return Err(errf!(CODE_INVALID, "each monitored season needs a number ≥ 1, once, and a valid range"));
        }
        if s.from == 0 {
            s.from = 1;
        }
        seen.insert(s.season);
    }
    m.seasons.sort_by_key(|s| s.season);
    if m.year < 0 || m.year > 3000 {
        return Err(errf!(CODE_INVALID, "bad year"));
    }
    Ok(m)
}

/// Every key the title answers to.
pub fn names(m: &Monitored) -> BTreeSet<String> {
    let mut out = BTreeSet::from([loose_key(&m.title)]);
    out.extend(m.aliases.iter().map(|a| loose_key(a)));
    out
}

/// Every key some monitored title answers to, in any library, for
/// [`settle_for`].
pub fn claimed(all: &[Monitored]) -> BTreeSet<String> {
    all.iter().flat_map(names).filter(|k| !k.is_empty()).collect()
}

/// The release as `m` reads it (D-279): settled against its names and
/// year, unless the parser's own answer names a monitored title
/// (`claimed`, from [`claimed`]). Such a release is that title's, and no
/// other monitored title may read it as an alternative: while "Show 2" is
/// monitored, "Show 2 - 03" answered as "Show 2" never becomes S2E3 of a
/// monitored "Show".
pub fn settle_for(m: &Monitored, rel: Release, claimed: &BTreeSet<String>) -> Release {
    let own = loose_key(&rel.title);
    if !own.is_empty() && claimed.contains(&own) {
        return Release { alternatives: Vec::new(), ..rel };
    }
    let known = names(m);
    settle(rel, |t| known.contains(&loose_key(t)), m.year)
}

/// A present catalog item as a unit of this title.
fn unit_of(m: &Monitored, it: &CatalogItem) -> Option<Unit> {
    let map = SeasonMap(&m.season_map);
    let u = |season, number| Some(Unit { season, number });
    match m.numbering.as_str() {
        NUMBER_MOVIE => u(0, 1),
        NUMBER_CHAPTER => (it.episode > 0).then_some(Unit { season: 0, number: it.episode }),
        NUMBER_VOLUME => (it.season > 0).then_some(Unit { season: 0, number: it.season }),
        NUMBER_SEASONAL => {
            if it.season == 0
                && it.episode > 0
                && let Some((s, e)) = map.to_seasonal(it.episode)
            {
                return u(s, e);
            }
            (it.season > 0 && it.episode > 0).then_some(Unit { season: it.season, number: it.episode })
        }
        _ => {
            if it.season > 0
                && it.episode > 0
                && let Some(abs) = map.to_absolute(it.season, it.episode)
            {
                return u(0, abs);
            }
            (it.episode > 0).then_some(Unit { season: 0, number: it.episode })
        }
    }
}

/// Compares the title's range with the library's items; `quality` tells
/// each present file's resolution, for upgrades (A-21).
pub fn wanted(m: &Monitored, items: &[CatalogItem], p: &Profile, quality: &dyn Fn(&str) -> String) -> Wanted {
    let names = names(m);
    let mut present: HashMap<Unit, &CatalogItem> = HashMap::new();
    for it in items {
        if it.missing || !names.contains(&loose_key(&it.title)) {
            continue;
        }
        if let Some(u) = unit_of(m, it) {
            present.insert(u, it);
        }
    }
    let mut w = Wanted { present: present.len() as i64, ..Wanted::default() };
    let want = |u: Unit, w: &mut Wanted| {
        if !present.contains_key(&u) {
            w.missing.push(u);
        }
    };
    match m.numbering.as_str() {
        NUMBER_MOVIE => want(Unit { season: 0, number: 1 }, &mut w),
        NUMBER_SEASONAL => {
            for s in &m.seasons {
                let mut top = s.to;
                if top == 0 {
                    top = s.from - 1;
                    for u in present.keys() {
                        if u.season == s.season && u.number > top {
                            top = u.number;
                        }
                    }
                    w.open_seasons.insert(s.season, top + 1);
                }
                for n in s.from..=top {
                    want(Unit { season: s.season, number: n }, &mut w);
                }
            }
        }
        _ => {
            let mut top = m.to;
            if top == 0 {
                top = m.from - 1;
                if m.numbering == NUMBER_ABSOLUTE && m.metadata_episodes > top {
                    top = m.metadata_episodes;
                }
                for u in present.keys() {
                    top = top.max(u.number);
                }
                w.open_from = Some(Unit { season: 0, number: top + 1 });
            }
            for n in m.from..=top {
                want(Unit { season: 0, number: n }, &mut w);
            }
        }
    }
    w.missing.sort();
    if m.numbering != NUMBER_CHAPTER && m.numbering != NUMBER_VOLUME {
        // Reading releases carry no resolution: no upgrades for them.
        for (u, it) in &present {
            let q = quality(&it.file_path);
            if !meets_cutoff(p, &q) {
                w.upgrades.push(UpgradeUnit { unit: *u, quality: q, path: it.file_path.clone() });
            }
        }
        w.upgrades.sort_by_key(|x| x.unit);
    }
    w
}

/// Whether `u` is wanted (missing or past an open edge). A season-pack
/// marker `{s,0}` is needed when anything in season `s` is.
pub fn needs(w: &Wanted, u: Unit) -> bool {
    if w.missing.iter().any(|x| *x == u || (u.number == 0 && u.season > 0 && x.season == u.season)) {
        return true;
    }
    if let Some(of) = w.open_from
        && u.season == of.season
        && u.number >= of.number
    {
        return true;
    }
    matches!(w.open_seasons.get(&u.season), Some(from) if u.number == 0 || u.number >= *from)
}

/// The present file of `u` when it is below cutoff.
pub fn upgradable(w: &Wanted, u: Unit) -> Option<&UpgradeUnit> {
    w.upgrades.iter().find(|x| x.unit == u)
}

/// The units a parsed release provides for this title; empty when it is
/// another title or cannot be placed.
pub fn covers(m: &Monitored, rel: &Release) -> Vec<Unit> {
    if !names(m).contains(&loose_key(&rel.title)) {
        return Vec::new();
    }
    let map = SeasonMap(&m.season_map);
    let one = |n| vec![Unit { season: 0, number: n }];
    match m.numbering.as_str() {
        NUMBER_MOVIE => {
            if m.year > 0 && rel.year > 0 && m.year != rel.year {
                return Vec::new();
            }
            one(1)
        }
        NUMBER_CHAPTER => {
            if rel.chapter > 0 {
                one(rel.chapter)
            } else {
                Vec::new()
            }
        }
        NUMBER_VOLUME => {
            if rel.volume > 0 && rel.chapter == 0 {
                one(rel.volume)
            } else {
                Vec::new()
            }
        }
        NUMBER_SEASONAL => {
            if !rel.episodes.is_empty() && (rel.absolute || rel.season == 0) {
                let mut out = Vec::new();
                for e in &rel.episodes {
                    let Some((s, n)) = map.to_seasonal(*e) else { return Vec::new() };
                    out.push(Unit { season: s, number: n });
                }
                out
            } else if !rel.episodes.is_empty() {
                rel.episodes.iter().map(|e| Unit { season: rel.season, number: *e }).collect()
            } else if rel.season_pack && rel.season > 0 {
                vec![Unit { season: rel.season, number: 0 }]
            } else {
                Vec::new()
            }
        }
        _ => {
            if !rel.episodes.is_empty() && rel.season > 0 && !rel.absolute {
                let mut out = Vec::new();
                for e in &rel.episodes {
                    let Some(abs) = map.to_absolute(rel.season, *e) else { return Vec::new() };
                    out.push(Unit { season: 0, number: abs });
                }
                out
            } else if !rel.episodes.is_empty() {
                rel.episodes.iter().map(|e| Unit { season: 0, number: *e }).collect()
            } else if rel.season_pack && rel.season > 0 {
                match (map.to_absolute(rel.season, 1), map.season_length(rel.season)) {
                    (Some(first), Some(n)) => (first..first + n).map(|e| Unit { season: 0, number: e }).collect(),
                    _ => Vec::new(),
                }
            } else {
                Vec::new()
            }
        }
    }
}

/// `open_seasons` as Go's map shape, for tests.
pub fn open_seasons(w: &Wanted) -> BTreeMap<i64, i64> {
    w.open_seasons.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decide::default_profile;

    fn parse(s: &str, kind: &str) -> Release {
        lain_identify::tokenize(s, kind)
    }

    fn quality(kind: &'static str) -> impl Fn(&str) -> String {
        move |p: &str| parse(&crate::util::base_name(p), kind).resolution
    }

    fn item(title: &str, season: i64, ep: i64, path: &str) -> CatalogItem {
        CatalogItem {
            id: path.into(),
            library_id: "lib".into(),
            title: title.into(),
            season,
            episode: ep,
            file_path: path.into(),
            ..Default::default()
        }
    }

    fn u(season: i64, number: i64) -> Unit {
        Unit { season, number }
    }

    fn mon(kind: &str, title: &str, numbering: &str) -> Monitored {
        Monitored { kind: kind.into(), title: title.into(), numbering: numbering.into(), ..Default::default() }
    }

    fn ss(season: i64, first: i64) -> SeasonStart {
        SeasonStart { season, first }
    }

    /// D-279's year rules seen from monitored movies. The bridge gives a
    /// reading without a year of its own the tokenizer's year, unless the
    /// reading's title already holds it; settled against each film's
    /// names and year, the year then keeps the sequel out of the first
    /// film's slot and lets the sequel in when "2049" was read as a year.
    #[test]
    fn the_year_keeps_a_sequel_out_of_the_first_film() {
        let first = Monitored { year: 1982, ..mon("movie", "Blade Runner", NUMBER_MOVIE) };
        let sequel = Monitored { year: 2017, ..mon("movie", "Blade Runner 2049", NUMBER_MOVIE) };
        let read = |title: &str, year, other: &str, other_year| Release {
            title: title.into(),
            year,
            alternatives: vec![lain_api::ReleaseIdentity {
                title: other.into(),
                year: other_year,
                ..Default::default()
            }],
            ..Default::default()
        };
        // Each film monitored alone, then both.
        let alone =
            |m: &Monitored, r: &Release| covers(m, &settle_for(m, r.clone(), &claimed(std::slice::from_ref(m))));
        let both = |m: &Monitored, r: &Release| {
            covers(m, &settle_for(m, r.clone(), &claimed(&[first.clone(), sequel.clone()])))
        };
        let none = Vec::<Unit>::new();
        // "Blade.Runner.2049.2017.1080p": the sequel, and "Blade Runner"
        // with the tokenizer's 2017, which the first film's year refuses.
        let r = read("Blade Runner 2049", 2017, "Blade Runner", 2017);
        assert_eq!(settle_for(&first, r.clone(), &claimed(std::slice::from_ref(&first))).title, "Blade Runner 2049");
        assert_eq!((alone(&first, &r), alone(&sequel, &r)), (none.clone(), vec![u(0, 1)]));
        assert_eq!((both(&first, &r), both(&sequel, &r)), (none.clone(), vec![u(0, 1)]));
        // "Blade.Runner.2049.1080p" answered as the sequel without a year
        // (the bridge does not take a year the title holds), and "Blade
        // Runner" with the tokenizer's 2049.
        let r = read("Blade Runner 2049", 0, "Blade Runner", 2049);
        assert_eq!((alone(&first, &r), alone(&sequel, &r)), (none.clone(), vec![u(0, 1)]));
        assert_eq!((both(&first, &r), both(&sequel, &r)), (none.clone(), vec![u(0, 1)]));
        // The same name with "2049" read as the year: alone, the sequel
        // takes its title, which holds it, with no year. Both monitored,
        // the answer names the first film, so it is the first film's (whose
        // year refuses it), and the sequel never reads it as its own.
        let r = read("Blade Runner", 2049, "Blade Runner 2049", 0);
        assert_eq!((alone(&first, &r), alone(&sequel, &r)), (none.clone(), vec![u(0, 1)]));
        assert_eq!((both(&first, &r), both(&sequel, &r)), (none.clone(), none.clone()));
        // Given the tokenizer's 2049, the sequel's reading would be refused.
        assert_eq!(alone(&sequel, &read("Blade Runner", 2049, "Blade Runner 2049", 2049)), none);
    }

    /// "Show 2 - 03" is episode 3 of the sequel, or of season 2. While the
    /// sequel is monitored too, an answer that names it is the sequel's
    /// (in any library, by title or alias), and "Show" may not read it as
    /// its season 2; with nobody claiming the answer, "Show" may.
    #[test]
    fn a_monitored_sequel_keeps_its_releases() {
        let show = Monitored { season_map: vec![ss(1, 1), ss(2, 13)], ..mon("anime", "Show", NUMBER_ABSOLUTE) };
        let sequel = Monitored { aliases: vec!["Show II".into()], ..mon("anime", "Show 2", NUMBER_ABSOLUTE) };
        let read = |title: &str| Release {
            title: title.into(),
            episodes: vec![3],
            absolute: true,
            alternatives: vec![lain_api::ReleaseIdentity {
                title: "Show".into(),
                season: 2,
                episodes: vec![3],
                ..Default::default()
            }],
            ..Default::default()
        };
        let take = |m: &Monitored, r: &Release, all: &[Monitored]| covers(m, &settle_for(m, r.clone(), &claimed(all)));
        let both = [show.clone(), Monitored { library_id: "elsewhere".into(), ..sequel.clone() }];
        for title in ["Show 2", "Show II"] {
            let r = read(title);
            assert_eq!(take(&show, &r, &both), Vec::<Unit>::new(), "{title} is the sequel's");
            assert_eq!(take(&sequel, &r, &both), vec![u(0, 3)], "{title}");
            assert!(settle_for(&show, r, &claimed(&both)).alternatives.is_empty(), "the advice is dropped");
        }
        assert_eq!(take(&show, &read("Show 2"), std::slice::from_ref(&show)), vec![u(0, 15)], "nobody claims it");
    }

    #[test]
    fn season_map_converts_both_ways() {
        let v = [ss(1, 1), ss(2, 13), ss(3, 25)];
        let m = SeasonMap(&v);
        assert_eq!(m.to_absolute(2, 1), Some(13));
        assert_eq!(m.to_seasonal(26), Some((3, 2)));
        assert_eq!(m.season_length(2), Some(12));
        assert_eq!(m.season_length(3), None);
        assert_eq!(m.to_absolute(9, 1), None);
        assert!(SeasonMap(&[ss(1, 1), ss(2, 1)]).validate().is_err());
    }

    #[test]
    fn monitored_validate() {
        let ok = Monitored { library_id: "lib".into(), from: 1, ..mon("anime", "Show", NUMBER_ABSOLUTE) };
        validate(ok).unwrap();
        let lib = |m: Monitored| Monitored { library_id: "lib".into(), ..m };
        let bads = [
            lib(mon("anime", "", NUMBER_ABSOLUTE)),
            lib(mon("anime", "Show", "lunar")),
            lib(mon("movie", "Film", NUMBER_ABSOLUTE)),
            lib(mon("manga", "Frieren", NUMBER_SEASONAL)),
            Monitored { from: 10, to: 5, ..lib(mon("anime", "Show", NUMBER_ABSOLUTE)) },
            Monitored {
                seasons: vec![lain_api::acquire::SeasonWant { season: 0, from: 1, to: 0 }],
                ..lib(mon("series", "Show", NUMBER_SEASONAL))
            },
        ];
        for bad in bads {
            assert_eq!(validate(bad.clone()).map_err(|e| e.code).unwrap_err(), CODE_INVALID, "accepted {bad:?}");
        }
        for (kind, want) in [
            ("movie", NUMBER_MOVIE),
            ("series", NUMBER_SEASONAL),
            ("anime", NUMBER_ABSOLUTE),
            ("manga", NUMBER_CHAPTER),
            ("comic", NUMBER_CHAPTER),
        ] {
            let m = validate(lib(mon(kind, "X", ""))).unwrap();
            assert_eq!(m.numbering, want, "{kind}");
        }
        assert_eq!(validate(mon("anime", "X", "")).unwrap_err().code, CODE_NO_LIBRARY);
    }

    #[test]
    fn wanted_absolute() {
        let mut m =
            Monitored { aliases: vec!["Show Alt".into()], from: 1, to: 6, ..mon("anime", "Show", NUMBER_ABSOLUTE) };
        let items = vec![
            item("Show", 0, 1, "/l/Show/[Fansub-A] Show - 01 [720p].mkv"),
            item("show", 0, 2, "/l/Show/[Fansub-A] Show - 02 [1080p].mkv"),
            item("Show Alt", 0, 4, "/l/Show/Show Alt - 04.mkv"),
            item("Other", 0, 3, "/l/Other/Other - 03.mkv"),
            CatalogItem { missing: true, ..item("Show", 0, 5, "/l/Show/x05.mkv") },
        ];
        let p = default_profile();
        let w = wanted(&m, &items, &p, &quality("anime"));
        assert_eq!(w.missing, vec![u(0, 3), u(0, 5), u(0, 6)]);
        assert_eq!(w.open_from, None);
        assert_eq!(w.upgrades.len(), 2, "{:?}", w.upgrades);
        assert_eq!((w.upgrades[0].unit, w.upgrades[0].quality.as_str()), (u(0, 1), "720p"));
        assert_eq!(w.upgrades[1].unit, u(0, 4));
        m.to = 0;
        let w = wanted(&m, &items, &p, &quality("anime"));
        assert_eq!(w.missing, vec![u(0, 3)]);
        assert_eq!(w.open_from, Some(u(0, 5)));
    }

    #[test]
    fn wanted_uses_the_metadata_episode_count() {
        let mut m = Monitored { from: 1, metadata_episodes: 5, ..mon("anime", "Show", NUMBER_ABSOLUTE) };
        let items = vec![item("Show", 0, 1, "/l/a.mkv"), item("Show", 0, 2, "/l/b.mkv")];
        let p = default_profile();
        let w = wanted(&m, &items, &p, &quality("anime"));
        assert_eq!(w.missing, vec![u(0, 3), u(0, 4), u(0, 5)]);
        assert_eq!(w.open_from, Some(u(0, 6)));
        m.to = 3;
        let w = wanted(&m, &items, &p, &quality("anime"));
        assert_eq!((w.missing, w.open_from), (vec![u(0, 3)], None));
    }

    #[test]
    fn wanted_seasonal_and_movie() {
        use lain_api::acquire::SeasonWant;
        let m = Monitored {
            seasons: vec![SeasonWant { season: 1, from: 1, to: 3 }, SeasonWant { season: 2, from: 1, to: 0 }],
            ..mon("series", "Show", NUMBER_SEASONAL)
        };
        let items = vec![
            item("Show", 1, 1, "/l/Show/Season 01/Show - S01E01.mkv"),
            item("Show", 1, 3, "/l/Show/Season 01/Show - S01E03.mkv"),
            item("Show", 2, 2, "/l/Show/Season 02/Show - S02E02.mkv"),
        ];
        let p = default_profile();
        let w = wanted(&m, &items, &p, &quality("anime"));
        assert_eq!(w.missing, vec![u(1, 2), u(2, 1)]);
        assert_eq!(open_seasons(&w), BTreeMap::from([(2, 3)]));
        let film = mon("movie", "Film", NUMBER_MOVIE);
        assert_eq!(wanted(&film, &[], &p, &quality("movie")).missing, vec![u(0, 1)]);
        let have = [item("Film", 0, 0, "/l/Film (2001)/Film (2001).mkv")];
        assert!(wanted(&film, &have, &p, &quality("movie")).missing.is_empty());
        // Go's JSON: integer map keys as strings.
        assert_eq!(serde_json::to_value(&w).unwrap()["open_seasons"], serde_json::json!({"2": 3}));
    }

    #[test]
    fn covers_releases() {
        let map = vec![ss(1, 1), ss(2, 13), ss(3, 25)];
        let anime = Monitored {
            aliases: vec!["Shou".into()],
            season_map: map.clone(),
            ..mon("anime", "Show", NUMBER_ABSOLUTE)
        };
        let seq = |a: i64, b: i64| (a..=b).map(|i| u(0, i)).collect::<Vec<_>>();
        let series = mon("series", "Show", NUMBER_SEASONAL);
        let film = Monitored { year: 2001, ..mon("movie", "Film", NUMBER_MOVIE) };
        let cases: Vec<(Monitored, &str, Vec<Unit>)> = vec![
            (anime.clone(), "[Fansub-A] Show - 14 [1080p]", vec![u(0, 14)]),
            (anime.clone(), "[Fansub-A] Shou - 01-03 [1080p]", vec![u(0, 1), u(0, 2), u(0, 3)]),
            (anime.clone(), "Show S02E03 1080p", vec![u(0, 15)]),
            (anime.clone(), "Show S02 1080p BluRay", seq(13, 24)),
            (anime.clone(), "Show S03 1080p BluRay", vec![]),
            (anime.clone(), "Other Show - 14 [1080p]", vec![]),
            (Monitored { season_map: map, ..series.clone() }, "[Fansub-A] Show - 14 [1080p]", vec![u(2, 2)]),
            (series.clone(), "Show.S01E05E06.1080p", vec![u(1, 5), u(1, 6)]),
            (series, "Show S04 Complete 1080p", vec![u(4, 0)]),
            (mon("manga", "Frieren", NUMBER_CHAPTER), "Frieren v03 c025.cbz", vec![u(0, 25)]),
            (mon("manga", "Frieren", NUMBER_VOLUME), "Frieren v03.cbz", vec![u(0, 3)]),
            (film.clone(), "Film 2001 1080p BluRay", vec![u(0, 1)]),
            (film, "Film 1998 1080p BluRay", vec![]),
        ];
        for (m, name, want) in cases {
            assert_eq!(covers(&m, &parse(name, &m.kind)), want, "{} / {name:?}", m.numbering);
        }
    }
}
