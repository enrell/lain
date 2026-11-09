//! `ComicInfo.xml`, the metadata an archive carries about itself. Only
//! the flat subset lain shows is read: the root element's direct
//! children, as text. Metadata is decoration: anything that does not
//! parse is "no info", never an error that blocks reading.

pub use lain_api::ComicInfo;

/// A `ComicInfo.xml` read is bounded; real files are a few KB.
pub const MAX_INFO_BYTES: u64 = 1 << 20;

/// Field caps keep one hostile archive from bloating every page index.
pub const MAX_FIELD: usize = 256;
pub const MAX_SUMMARY: usize = 4000;
pub const MAX_GENRES: usize = 16;

/// Decodes a `ComicInfo.xml` document; `None` when it does not parse or
/// carries nothing lain shows.
pub fn parse(raw: &[u8]) -> Option<ComicInfo> {
    let doc = String::from_utf8_lossy(raw);
    let fields = children(&doc)?;
    let get = |name: &str| fields.iter().rev().find(|(k, _)| k == name).map(|(_, v)| v.as_str()).unwrap_or("");
    let mut info = ComicInfo {
        title: clean(get("Title"), MAX_FIELD),
        series: clean(get("Series"), MAX_FIELD),
        number: clean(get("Number"), 16),
        volume: bounded(get("Volume"), 0, 9999),
        count: bounded(get("Count"), 0, 99_999),
        summary: clean(get("Summary"), MAX_SUMMARY),
        year: bounded(get("Year"), 1800, 2200),
        writer: clean(get("Writer"), MAX_FIELD),
        artist: clean(get("Penciller"), MAX_FIELD),
        publisher: clean(get("Publisher"), MAX_FIELD),
        language: clean(get("LanguageISO"), 16),
        age_rating: clean(get("AgeRating"), 64),
        ..ComicInfo::default()
    };
    for g in get("Genre").split(',') {
        let g = clean(g, 64);
        if !g.is_empty() && info.genres.len() < MAX_GENRES {
            info.genres.push(g);
        }
    }
    info.direction = match get("Manga").trim().to_lowercase().as_str() {
        "yesandrighttoleft" => "rtl".into(),
        "no" => "ltr".into(),
        _ => String::new(),
    };
    (info != ComicInfo::default()).then_some(info)
}

/// The root element's direct children as (local name, text), in order.
/// `None` when there is no root element or a tag never closes.
fn children(doc: &str) -> Option<Vec<(String, String)>> {
    let mut rest = doc;
    // The prolog: declarations, comments, a doctype.
    loop {
        rest = rest.trim_start();
        if let Some(r) = rest.strip_prefix("<?") {
            rest = &r[r.find("?>")? + 2..];
        } else if let Some(r) = rest.strip_prefix("<!--") {
            rest = &r[r.find("-->")? + 3..];
        } else if let Some(r) = rest.strip_prefix("<!") {
            rest = &r[r.find('>')? + 1..];
        } else {
            break;
        }
    }
    let r = rest.strip_prefix('<')?;
    let end = r.find('>')?;
    let open = &r[..end];
    if open.is_empty() || open.starts_with('/') || !open.starts_with(|c: char| c.is_alphabetic() || c == '_') {
        return None;
    }
    let mut out = Vec::new();
    if open.ends_with('/') {
        return Some(out);
    }
    rest = &r[end + 1..];
    // Depth 1 is the root's content; text is kept for depth-2 elements.
    let mut depth = 1usize;
    let mut current: Option<(String, String)> = None;
    while !rest.is_empty() && depth > 0 {
        let Some(lt) = rest.find('<') else { break };
        if let Some((_, text)) = current.as_mut()
            && depth == 2
        {
            text.push_str(&unescape(&rest[..lt]));
        }
        rest = &rest[lt..];
        if let Some(r) = rest.strip_prefix("<!--") {
            rest = &r[r.find("-->")? + 3..];
        } else if let Some(r) = rest.strip_prefix("<![CDATA[") {
            let e = r.find("]]>")?;
            if let Some((_, text)) = current.as_mut()
                && depth == 2
            {
                text.push_str(&r[..e]);
            }
            rest = &r[e + 3..];
        } else if let Some(r) = rest.strip_prefix("<?") {
            rest = &r[r.find("?>")? + 2..];
        } else if let Some(r) = rest.strip_prefix("</") {
            rest = &r[r.find('>')? + 1..];
            depth -= 1;
            if depth == 1
                && let Some(c) = current.take()
            {
                out.push(c);
            }
        } else {
            let r = &rest[1..];
            let e = r.find('>')?;
            let tag = &r[..e];
            rest = &r[e + 1..];
            let self_closing = tag.ends_with('/');
            let name = tag.trim_end_matches('/').split(|c: char| c.is_whitespace()).next().unwrap_or("");
            let local = name.rsplit(':').next().unwrap_or(name).to_string();
            if depth == 1 {
                if self_closing {
                    out.push((local, String::new()));
                } else {
                    current = Some((local, String::new()));
                }
            }
            if !self_closing {
                depth += 1;
            }
        }
    }
    Some(out)
}

fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(semi) = rest.find(';').filter(|&i| i <= 12) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..semi];
        let ch = match ent {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "apos" => Some('\''),
            "quot" => Some('"'),
            _ => ent
                .strip_prefix("#x")
                .or_else(|| ent.strip_prefix("#X"))
                .map(|h| u32::from_str_radix(h, 16))
                .or_else(|| ent.strip_prefix('#').map(str::parse::<u32>))
                .and_then(Result::ok)
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                // Unknown entities stay literal (a lenient reader).
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Drops control characters and U+FFFD, collapses whitespace runs and
/// caps the length in characters.
pub fn clean(s: &str, max: usize) -> String {
    let kept: String = s.chars().filter(|&c| c != char::REPLACEMENT_CHARACTER && !c.is_control()).collect();
    let joined = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.chars().count() > max { joined.chars().take(max).collect::<String>().trim().to_string() } else { joined }
}

fn bounded(s: &str, lo: i64, hi: i64) -> i64 {
    match s.trim().parse::<i64>() {
        Ok(n) if (lo..=hi).contains(&n) => n,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0"?>
<ComicInfo xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <Title>The Long Road</Title>
  <Series>Frieren</Series>
  <Number>12.5</Number>
  <Volume>3</Volume>
  <Summary>A mage
     walks on.</Summary>
  <Year>2021</Year>
  <Writer>Writer A</Writer>
  <Penciller>Artist B</Penciller>
  <Publisher>Publisher-A</Publisher>
  <Genre>Fantasy, Adventure, ,Drama</Genre>
  <LanguageISO>ja</LanguageISO>
  <Manga>YesAndRightToLeft</Manga>
</ComicInfo>"#;

    #[test]
    fn fields() {
        let want = ComicInfo {
            title: "The Long Road".into(),
            series: "Frieren".into(),
            number: "12.5".into(),
            volume: 3,
            summary: "A mage walks on.".into(),
            year: 2021,
            writer: "Writer A".into(),
            artist: "Artist B".into(),
            publisher: "Publisher-A".into(),
            genres: vec!["Fantasy".into(), "Adventure".into(), "Drama".into()],
            language: "ja".into(),
            direction: "rtl".into(),
            ..ComicInfo::default()
        };
        assert_eq!(parse(SAMPLE.as_bytes()), Some(want));
    }

    #[test]
    fn direction_bounds_and_caps() {
        assert_eq!(parse(b"<ComicInfo><Manga>No</Manga></ComicInfo>").unwrap().direction, "ltr");
        assert_eq!(parse(b"<ComicInfo><Manga>Yes</Manga><Series>S</Series></ComicInfo>").unwrap().direction, "");
        let got = parse(b"<ComicInfo><Year>99999</Year><Volume>-2</Volume><Series>S</Series></ComicInfo>").unwrap();
        assert_eq!((got.year, got.volume), (0, 0));
        let long = "é".repeat(5000);
        let got =
            parse(format!("<ComicInfo><Summary>{long}</Summary><Title>{long}</Title></ComicInfo>").as_bytes()).unwrap();
        assert_eq!((got.summary.chars().count(), got.title.chars().count()), (MAX_SUMMARY, MAX_FIELD));
        let got = parse(b"<ComicInfo><Title>A &amp; B &#x41;<![CDATA[<x>]]></Title><!-- c --></ComicInfo>").unwrap();
        assert_eq!(got.title, "A & B A<x>");
    }

    #[test]
    fn nothing_useful() {
        for doc in ["", "not xml", "<ComicInfo></ComicInfo>", "<ComicInfo><Manga>Yes</Manga></ComicInfo>", "<x/>", "<a"]
        {
            assert_eq!(parse(doc.as_bytes()), None, "{doc:?}");
        }
    }

    #[test]
    fn hostile_input_never_panics_and_stays_clean() {
        let docs = [
            "<ComicInfo><Number>\u{0}\u{1}</Number><Series>é\u{7}x</Series></ComicInfo>",
            "<ComicInfo><Genre>,,,</Genre><Year>-1</Year></ComicInfo>",
            "<ComicInfo><Title>&#xD800;&#99999999;&</Title><Series>s</Series>",
            "<ComicInfo><a><b><c>x</c></b></a></ComicInfo></ComicInfo></x>",
            "<ComicInfo><![CDATA[",
        ];
        for d in docs {
            if let Some(i) = parse(d.as_bytes()) {
                for s in [&i.title, &i.series, &i.number, &i.summary] {
                    assert!(!s.chars().any(char::is_control), "{s:?}");
                }
                assert!(i.genres.len() <= MAX_GENRES);
            }
        }
    }
}
