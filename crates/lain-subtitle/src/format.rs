//! SRT, WebVTT and ASS/SSA to cues, cues to WebVTT.

/// Bounds on what a subtitle file may hold.
pub const MAX_BYTES: usize = 5 << 20;
pub const MAX_CUES: usize = 20_000;

/// One timed piece of text, in milliseconds. Text may carry WebVTT
/// inline tags; lines are separated by `\n`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cue {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

/// Why a file is not a readable subtitle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError(pub String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "subtitle: unreadable: {}", self.0)
    }
}

impl std::error::Error for ParseError {}

/// Detects the format by content and returns the cues in file order; a
/// file without one cue is an error.
pub fn parse(data: &[u8]) -> Result<Vec<Cue>, ParseError> {
    if data.len() > MAX_BYTES {
        return Err(ParseError(format!("larger than {MAX_BYTES} bytes")));
    }
    let text = decode_text(data).replace("\r\n", "\n").replace('\r', "\n");
    let head = text.trim();
    let cues = if head.starts_with("WEBVTT") {
        parse_timed(&text, true)
    } else if head.to_lowercase().contains("[events]") || head.contains("\nDialogue:") || head.starts_with("Dialogue:")
    {
        parse_ass(&text)
    } else {
        parse_timed(&text, false)
    };
    if cues.is_empty() {
        return Err(ParseError("no cues".into()));
    }
    Ok(cues)
}

/// SRT and WebVTT in one line-based state machine, tolerant of missing
/// blank lines, dot or comma milliseconds, missing hours and stray index
/// lines.
fn parse_timed(text: &str, vtt: bool) -> Vec<Cue> {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out = Vec::new();
    let mut cur: Option<Cue> = None;
    let mut buf: Vec<&str> = Vec::new();
    let flush = |cur: &mut Option<Cue>, buf: &mut Vec<&str>, out: &mut Vec<Cue>| {
        if let Some(mut c) = cur.take() {
            c.text = clean_text(&buf.join("\n"), vtt);
            if !c.text.is_empty() && out.len() < MAX_CUES {
                out.push(c);
            }
        }
        buf.clear();
    };
    let mut skip_block = false;
    for i in 0..lines.len() {
        let line = lines[i].trim();
        if i == 0 && vtt {
            continue;
        }
        if line.is_empty() {
            flush(&mut cur, &mut buf, &mut out);
            skip_block = false;
            continue;
        }
        if skip_block {
            continue;
        }
        if let Some((start, end)) = parse_timing(line) {
            flush(&mut cur, &mut buf, &mut out);
            cur = Some(Cue { start_ms: start, end_ms: end.max(start), text: String::new() });
            continue;
        }
        // A line right before a timing line is a cue identifier.
        if i + 1 < lines.len() && parse_timing(lines[i + 1].trim()).is_some() {
            continue;
        }
        if cur.is_none() {
            if vtt && (line.starts_with("NOTE") || line.starts_with("STYLE") || line.starts_with("REGION")) {
                skip_block = true;
            }
            continue;
        }
        buf.push(line);
    }
    flush(&mut cur, &mut buf, &mut out);
    out
}

/// `start --> end [settings]`.
fn parse_timing(line: &str) -> Option<(i64, i64)> {
    let (a, b) = line.split_once("-->")?;
    let start = parse_clock(a.trim());
    let end = parse_clock(b.split_whitespace().next()?);
    Some((start?, end?))
}

/// `[hh:]mm:ss[.,]fff` (1-3 fraction digits), in milliseconds.
fn parse_clock(s: &str) -> Option<i64> {
    let s = s.replacen(',', ".", 1);
    let (main, frac) = s.split_once('.').unwrap_or((&s, ""));
    let parts: Vec<&str> = main.split(':').collect();
    if parts.len() < 2 || parts.len() > 3 || frac.len() > 3 {
        return None;
    }
    let mut total: i64 = 0;
    for p in parts {
        if p.is_empty() || p.len() > 3 {
            return None;
        }
        total = total * 60 + go_atoi(p)?;
    }
    let mut ms = 0;
    if !frac.is_empty() {
        ms = go_atoi(frac)?;
        for _ in frac.len()..3 {
            ms *= 10;
        }
    }
    if total > 100 * 3600 {
        return None;
    }
    Some(total * 1000 + ms)
}

/// `strconv.Atoi` on short input: an optional sign, then digits; a
/// negative value is refused by the callers.
fn go_atoi(s: &str) -> Option<i64> {
    let digits = s.strip_prefix('+').unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// Drops formatting browsers do not understand and empty lines.
fn clean_text(s: &str, vtt: bool) -> String {
    let s = if vtt { s.to_string() } else { strip_braces(&strip_tag(s, "font")) };
    s.split('\n').map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
}

fn strip_tag(s: &str, tag: &str) -> String {
    let b = s.as_bytes();
    let lower = s.to_ascii_lowercase();
    let lb = lower.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'<' {
            let mut rest = &lb[i + 1..];
            if rest.first() == Some(&b'/') {
                rest = &rest[1..];
            }
            let t = tag.as_bytes();
            if rest.starts_with(t)
                && (rest.len() == t.len() || rest[t.len()] == b'>' || rest[t.len()] == b' ')
                && let Some(end) = b[i..].iter().position(|&c| c == b'>')
            {
                i += end + 1;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn strip_braces(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'{'
            && let Some(end) = b[i..].iter().position(|&c| c == b'}')
            && end > 0
            && b[i..i + end].contains(&b'\\')
        {
            i += end + 1;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The `[Events]` section of ASS/SSA: `Dialogue:` lines laid out by the
/// section's `Format:` line.
fn parse_ass(text: &str) -> Vec<Cue> {
    let mut out = Vec::new();
    let mut fields: Vec<String> =
        ["layer", "start", "end", "style", "name", "marginl", "marginr", "marginv", "effect", "text"]
            .iter()
            .map(|s| s.to_string())
            .collect();
    let mut in_events = false;
    for raw in text.split('\n') {
        let line = raw.trim();
        if line.starts_with('[') {
            in_events = line.eq_ignore_ascii_case("[events]");
            continue;
        }
        if in_events && let Some(f) = line.strip_prefix("Format:") {
            fields = f.split(',').map(|x| x.trim().to_lowercase()).collect();
            continue;
        }
        let Some(body) = line.strip_prefix("Dialogue:") else { continue };
        if out.len() >= MAX_CUES {
            continue;
        }
        let parts: Vec<&str> = body.trim().splitn(fields.len(), ',').collect();
        if parts.len() != fields.len() {
            continue;
        }
        let (mut start, mut end, mut text) = (None, None, "");
        for (i, f) in fields.iter().enumerate() {
            match f.as_str() {
                "start" => start = parse_clock(parts[i].trim()),
                "end" => end = parse_clock(parts[i].trim()),
                "text" => text = parts[i],
                _ => {}
            }
        }
        let (Some(start), Some(end)) = (start, end) else { continue };
        let t = ass_text(text);
        if !t.is_empty() {
            out.push(Cue { start_ms: start, end_ms: end.max(start), text: t });
        }
    }
    out
}

fn ass_text(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut drawing = false;
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'{' {
            let end = b[i..].iter().position(|&c| c == b'}').unwrap_or(b.len() - i - 1);
            let block = &b[i..i + end + 1];
            if let Some(idx) = block.windows(2).position(|w| w == b"\\p")
                && idx + 2 < block.len()
            {
                drawing = block[idx + 2] != b'0';
            }
            i += end + 1;
            continue;
        }
        if b[i] == b'\\' && i + 1 < b.len() && matches!(b[i + 1], b'N' | b'n' | b'h') {
            if !drawing {
                out.push(if b[i + 1] == b'h' { b' ' } else { b'\n' });
            }
            i += 2;
            continue;
        }
        if !drawing {
            out.push(b[i]);
        }
        i += 1;
    }
    clean_text(&String::from_utf8_lossy(&out), true)
}

/// Renders cues as WebVTT; `&` and `-->` in text are escaped so a cue
/// can never break the file's structure.
pub fn webvtt(cues: &[Cue]) -> Vec<u8> {
    let mut b = String::from("WEBVTT\n\n");
    for c in cues {
        b.push_str(&format!("{} --> {}\n", clock(c.start_ms), clock(c.end_ms)));
        let text = c.text.replace('&', "&amp;").replace("-->", "--&gt;");
        let lines: Vec<&str> = text.split('\n').map(str::trim).filter(|l| !l.is_empty()).collect();
        b.push_str(&lines.join("\n"));
        b.push_str("\n\n");
    }
    b.into_bytes()
}

fn clock(ms: i64) -> String {
    let ms = ms.max(0);
    format!("{:02}:{:02}:{:02}.{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

/// Windows-1252's 0x80-0x9F (the rest is Latin-1).
const CP1252: [char; 32] = [
    '€', '\u{FFFD}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{FFFD}', 'Ž', '\u{FFFD}', '\u{FFFD}',
    '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{FFFD}', 'ž', 'Ÿ',
];

/// UTF-8: a UTF-8 BOM is dropped, UTF-16 with a BOM decoded, valid
/// UTF-8 passes and anything else reads as Windows-1252.
pub fn decode_text(b: &[u8]) -> String {
    let b = if let Some(rest) = b.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        rest
    } else if b.len() >= 2 && ((b[0] == 0xFF && b[1] == 0xFE) || (b[0] == 0xFE && b[1] == 0xFF)) {
        let le = b[0] == 0xFF;
        let units: Vec<u16> = b[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| if le { u16::from_le_bytes(*c) } else { u16::from_be_bytes(*c) })
            .collect();
        return char::decode_utf16(units).map(|r| r.unwrap_or('\u{FFFD}')).collect();
    } else {
        b
    };
    match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        Err(_) => {
            b.iter().map(|&c| if (0x80..0xA0).contains(&c) { CP1252[(c - 0x80) as usize] } else { c as char }).collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srt_vtt_and_ass() {
        let srt = b"1\r\n00:00:01,000 --> 00:00:02,500\r\n<font color=\"red\">Hello</font> {\\an8}there\r\n\r\n2\r\n00:03.5 --> 00:04\r\nA & B --> C\r\n";
        let cues = parse(srt).unwrap();
        assert_eq!(cues[0], Cue { start_ms: 1000, end_ms: 2500, text: "Hello there".into() });
        assert_eq!((cues[1].start_ms, cues[1].end_ms), (3500, 4000));
        let out = String::from_utf8(webvtt(&cues)).unwrap();
        assert_eq!(
            out,
            "WEBVTT\n\n00:00:01.000 --> 00:00:02.500\nHello there\n\n00:00:03.500 --> 00:00:04.000\nA &amp; B --&gt; C\n\n"
        );
        let vtt = b"WEBVTT\n\nNOTE skip\nthis\n\nid1\n00:00:05.000 --> 00:00:06.000 align:start\n<i>Hi</i>\n";
        assert_eq!(parse(vtt).unwrap()[0].text, "<i>Hi</i>");
        let ass = b"[Script Info]\nTitle: x\n\n[Events]\nFormat: Layer, Start, End, Style, Text\nDialogue: 0,0:00:01.00,0:00:02.00,Default,{\\i1}One\\NTwo, three\nDialogue: 0,0:00:03.00,0:00:04.00,Default,{\\p1}m 0 0 l 1 1{\\p0}\n";
        let cues = parse(ass).unwrap();
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].text, "One\nTwo, three");
    }

    #[test]
    fn hostile_input_is_refused() {
        assert!(parse(b"").is_err());
        assert!(parse(b"just text").is_err());
        assert!(parse(&vec![b'a'; MAX_BYTES + 1]).is_err());
        assert!(parse(b"00:00:01,000 --> 99999:00:00,000\nx").is_err());
        let _ = parse(b"[Events]\nDialogue: {{{{\\p");
        let _ = parse(b"{\\");
    }

    #[test]
    fn encodings() {
        assert_eq!(decode_text(&[0xEF, 0xBB, 0xBF, b'a']), "a");
        assert_eq!(decode_text(&[0xFF, 0xFE, b'h', 0, b'i', 0]), "hi");
        assert_eq!(decode_text(&[0xE9, 0x80]), "é€");
    }
}
