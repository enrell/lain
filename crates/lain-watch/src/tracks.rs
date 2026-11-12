//! Which audio and subtitle tracks a viewer's preferences pick (D-280).
//!
//! The one rule every player follows; the web player
//! (`web/src/lib/player/track-language.ts`), the Android app
//! (`domain/TrackChoice.kt`) and the mpv script
//! (`assets/mpv_progress.lua`) carry the same rule in their languages,
//! and their tests use the same cases.
//!
//! - Audio: a track in the audio language, else the file's default (the
//!   track flagged default, else the first).
//! - Subtitles, by mode:
//!   - `auto`: a track in the subtitle language when the audio that plays
//!     is not in it (a viewer who reads Portuguese needs no Portuguese
//!     subtitles over Portuguese audio, and does over Japanese audio);
//!     none otherwise. With no subtitle language, or no language tag on
//!     any audio or subtitle track, the player's own default stands
//!     (D-071's rule for untagged media).
//!   - `always`: a track in the subtitle language even over audio in it;
//!     with no subtitle language, the file's default subtitle track.
//!   - `forced`: only a track flagged forced: in the subtitle language,
//!     else in the language of the audio that plays, else an untagged one.
//!   - `off`: none.
//!
//! A full track is preferred over a forced one in the same language
//! wherever full subtitles are wanted.

use lain_api::{MediaStream, TrackPreferences};

/// ISO 639-2 bibliographic codes and their terminology twins.
const LANGUAGE_ALIASES: [(&str, &str); 16] = [
    ("fre", "fra"),
    ("ger", "deu"),
    ("chi", "zho"),
    ("dut", "nld"),
    ("gre", "ell"),
    ("rum", "ron"),
    ("cze", "ces"),
    ("slo", "slk"),
    ("per", "fas"),
    ("may", "msa"),
    ("alb", "sqi"),
    ("arm", "hye"),
    ("baq", "eus"),
    ("bur", "mya"),
    ("ice", "isl"),
    ("mac", "mkd"),
];

fn alias(code: &str) -> String {
    LANGUAGE_ALIASES.iter().find(|(b, _)| *b == code).map(|(_, t)| t.to_string()).unwrap_or_else(|| code.to_string())
}

/// A stream language tag matches a preference (ISO 639-2 bibliographic
/// codes folded to terminology ones; a region suffix is ignored).
pub fn same_language(actual: &str, preferred: &str) -> bool {
    let a = alias(&actual.split('-').next().unwrap_or("").to_ascii_lowercase());
    let p = alias(&preferred.to_ascii_lowercase());
    !a.is_empty() && a == p
}

/// The subtitle side of a choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subtitle {
    /// Leave the player's own default alone.
    Keep,
    /// No subtitles.
    Off,
    /// This stream (its `index`).
    Stream(i64),
}

/// What [`choose`] picked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice {
    /// The audio stream to play; `None` when the file has none.
    pub audio: Option<i64>,
    /// The audio is in the preferred audio language (not the default
    /// standing in for it).
    pub audio_preferred: bool,
    pub subtitle: Subtitle,
}

fn is(s: &MediaStream, kind: &str) -> bool {
    s.kind == kind
}

/// The file's default audio: the stream flagged default, else the first.
pub fn default_audio(streams: &[MediaStream]) -> Option<&MediaStream> {
    streams.iter().find(|s| is(s, "audio") && s.default).or_else(|| streams.iter().find(|s| is(s, "audio")))
}

/// Applies `prefs` to a file's streams (see the module docs).
pub fn choose(streams: &[MediaStream], prefs: &TrackPreferences) -> Choice {
    let wanted = (!prefs.audio_language.is_empty())
        .then(|| streams.iter().find(|s| is(s, "audio") && same_language(&s.language, &prefs.audio_language)))
        .flatten();
    let audio = wanted.or_else(|| default_audio(streams));
    let subtitle = choose_subtitle(streams, prefs, audio.map(|a| a.language.as_str()).unwrap_or(""));
    Choice { audio: audio.map(|a| a.index), audio_preferred: wanted.is_some(), subtitle }
}

fn choose_subtitle(streams: &[MediaStream], prefs: &TrackPreferences, audio_language: &str) -> Subtitle {
    let subs: Vec<&MediaStream> = streams.iter().filter(|s| is(s, "subtitle")).collect();
    let lang = prefs.subtitle_language.as_str();
    // A full track in the language, else a forced one in it.
    let in_language = |l: &str| {
        subs.iter()
            .find(|s| !s.forced && same_language(&s.language, l))
            .or_else(|| subs.iter().find(|s| same_language(&s.language, l)))
            .map(|s| Subtitle::Stream(s.index))
    };
    match prefs.subtitle_mode.as_str() {
        "off" => Subtitle::Off,
        "forced" => {
            let forced: Vec<&&MediaStream> = subs.iter().filter(|s| s.forced).collect();
            let by = |l: &str| forced.iter().find(|s| !l.is_empty() && same_language(&s.language, l));
            by(lang)
                .or_else(|| by(audio_language))
                .or_else(|| forced.iter().find(|s| s.language.is_empty() || s.language == "und"))
                .map(|s| Subtitle::Stream(s.index))
                .unwrap_or(Subtitle::Off)
        }
        "always" => {
            if lang.is_empty() {
                let full = || subs.iter().filter(|s| !s.forced);
                return full()
                    .find(|s| s.default)
                    .or_else(|| full().next())
                    .map(|s| Subtitle::Stream(s.index))
                    .unwrap_or(Subtitle::Off);
            }
            in_language(lang).unwrap_or(if tagged(streams) { Subtitle::Off } else { Subtitle::Keep })
        }
        // "auto", and anything unknown reads as it.
        _ => {
            if lang.is_empty() || !tagged(streams) {
                return Subtitle::Keep;
            }
            if same_language(audio_language, lang) {
                return Subtitle::Off;
            }
            in_language(lang).unwrap_or(Subtitle::Off)
        }
    }
}

/// Some audio or subtitle track says its language: without that a
/// language preference can only guess (D-071).
fn tagged(streams: &[MediaStream]) -> bool {
    streams.iter().any(|s| (is(s, "audio") || is(s, "subtitle")) && !s.language.is_empty() && s.language != "und")
}

/// The audio stream a transcode should take when the client named none
/// (D-280): the preferred-language track when the file's default is
/// another one; `None` keeps the default (and the session keys made
/// before D-280).
pub fn preferred_audio_stream(streams: &[MediaStream], prefs: &TrackPreferences) -> Option<i64> {
    let c = choose(streams, prefs);
    let default = default_audio(streams).map(|a| a.index);
    (c.audio_preferred && c.audio != default).then_some(c.audio).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(index: i64, kind: &str, lang: &str) -> MediaStream {
        MediaStream { index, kind: kind.into(), language: lang.into(), ..Default::default() }
    }

    fn forced(mut m: MediaStream) -> MediaStream {
        m.forced = true;
        m
    }

    fn default(mut m: MediaStream) -> MediaStream {
        m.default = true;
        m
    }

    fn p(audio: &str, subtitle: &str, mode: &str) -> TrackPreferences {
        TrackPreferences {
            audio_language: audio.into(),
            subtitle_language: subtitle.into(),
            subtitle_mode: mode.into(),
        }
    }

    fn pick(streams: &[MediaStream], prefs: TrackPreferences) -> (Option<i64>, Subtitle) {
        let c = choose(streams, &prefs);
        (c.audio, c.subtitle)
    }

    /// An anime release: English dub first and default, Japanese second,
    /// signs, then full Portuguese and English subtitles.
    fn anime() -> Vec<MediaStream> {
        vec![
            s(0, "video", ""),
            default(s(1, "audio", "eng")),
            s(2, "audio", "jpn"),
            forced(s(3, "subtitle", "eng")),
            s(4, "subtitle", "por"),
            s(5, "subtitle", "eng"),
        ]
    }

    /// A film dubbed in Portuguese, with English subtitles only.
    fn dubbed() -> Vec<MediaStream> {
        vec![s(0, "video", ""), s(1, "audio", "por"), s(2, "subtitle", "eng")]
    }

    #[test]
    fn the_owners_case_japanese_audio_portuguese_subtitles() {
        let owner = || p("jpn", "por", "auto");
        assert_eq!(pick(&anime(), owner()), (Some(2), Subtitle::Stream(4)));
        assert_eq!(pick(&dubbed(), owner()), (Some(1), Subtitle::Off), "Portuguese audio needs no subtitles");
        // English subtitles when the release has no Portuguese ones.
        assert_eq!(
            pick(&anime(), p("jpn", "eng", "auto")),
            (Some(2), Subtitle::Stream(5)),
            "the full track, not signs"
        );
    }

    #[test]
    fn one_language_is_d071() {
        let por = || TrackPreferences::legacy("por");
        assert_eq!(pick(&dubbed(), por()), (Some(1), Subtitle::Off), "matching audio, subtitles off");
        let jp = vec![s(1, "audio", "jpn"), s(2, "subtitle", "por")];
        assert_eq!(pick(&jp, por()), (Some(1), Subtitle::Stream(2)), "else subtitles in it");
        let none = vec![s(1, "audio", "jpn"), s(2, "subtitle", "eng")];
        assert_eq!(pick(&none, por()), (Some(1), Subtitle::Off), "tagged with no match: off");
        let untagged = vec![s(1, "audio", ""), s(2, "subtitle", "und")];
        assert_eq!(pick(&untagged, por()), (Some(1), Subtitle::Keep), "untagged keeps the player's default");
        assert_eq!(pick(&anime(), p("", "", "auto")), (Some(1), Subtitle::Keep), "no preference at all");
    }

    #[test]
    fn modes() {
        // Always: over audio in the language too; with no language, the default track.
        assert_eq!(pick(&dubbed(), p("", "eng", "always")), (Some(1), Subtitle::Stream(2)));
        assert_eq!(pick(&dubbed(), p("", "por", "always")), (Some(1), Subtitle::Off));
        let flagged = vec![
            s(1, "audio", "jpn"),
            forced(s(2, "subtitle", "eng")),
            s(3, "subtitle", "eng"),
            default(s(4, "subtitle", "spa")),
        ];
        assert_eq!(pick(&flagged, p("", "", "always")), (Some(1), Subtitle::Stream(4)));
        // Forced: the signs track in the subtitle language, else the audio's.
        assert_eq!(pick(&anime(), p("", "eng", "forced")), (Some(1), Subtitle::Stream(3)));
        assert_eq!(pick(&anime(), p("", "", "forced")), (Some(1), Subtitle::Stream(3)), "signs for the English audio");
        assert_eq!(pick(&anime(), p("jpn", "por", "forced")), (Some(2), Subtitle::Off), "no forced track fits");
        let untagged = vec![s(1, "audio", "jpn"), forced(s(2, "subtitle", ""))];
        assert_eq!(pick(&untagged, p("", "por", "forced")), (Some(1), Subtitle::Stream(2)));
        // Off is off, whatever the file says.
        assert_eq!(pick(&anime(), p("jpn", "por", "off")), (Some(2), Subtitle::Off));
    }

    #[test]
    fn audio_falls_back_to_the_files_default() {
        assert_eq!(choose(&anime(), &p("kor", "", "auto")).audio, Some(1));
        assert!(!choose(&anime(), &p("kor", "", "auto")).audio_preferred);
        assert!(choose(&anime(), &p("jpn", "", "auto")).audio_preferred);
        assert_eq!(choose(&[s(0, "video", "")], &p("jpn", "", "auto")).audio, None);
        // A transcode takes the preferred track only when it is not the default anyway.
        assert_eq!(preferred_audio_stream(&anime(), &p("jpn", "", "auto")), Some(2));
        assert_eq!(preferred_audio_stream(&anime(), &p("eng", "", "auto")), None);
        assert_eq!(preferred_audio_stream(&anime(), &p("kor", "", "auto")), None);
        assert_eq!(preferred_audio_stream(&anime(), &p("", "", "auto")), None);
    }

    #[test]
    fn languages_fold_bibliographic_codes_and_regions() {
        assert!(same_language("fre", "fra") && same_language("pt-BR", "pt") && !same_language("", ""));
        assert_eq!(
            pick(&[s(1, "audio", "jpn"), s(2, "subtitle", "fre")], p("", "fra", "auto")),
            (Some(1), Subtitle::Stream(2))
        );
    }
}
