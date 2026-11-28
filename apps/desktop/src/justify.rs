//! Justified paragraphs for the book reader: GPUI only wraps text
//! ragged, so a paragraph the book sets in `text-align: justify` is broken
//! into lines here, from the text system's measurement of each word, and
//! every line but the last is drawn with its words spread to both edges.
//! This module is the measuring-free part: words, line breaks and which
//! lines are spread, so it is tested with made-up widths.

use std::ops::Range;

/// A line narrower than this share of the column stays start-aligned:
/// spreading it would open holes wider than the words.
pub const MIN_FILL: f32 = 0.6;

/// A word of a paragraph: its byte range in the text, and whether a hard
/// break (`<br>`) follows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Word {
    pub range: Range<usize>,
    pub hard_break: bool,
}

/// One laid-out line: the words it holds (indices into the word list) and
/// whether it is spread to both edges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub words: Range<usize>,
    pub justify: bool,
}

/// The words of `text`, split at the spaces a line may break at. A
/// no-break space (U+00A0) joins its neighbours; a newline is a hard
/// break after the word before it.
pub fn words(text: &str) -> Vec<Word> {
    let mut out: Vec<Word> = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in text.char_indices() {
        let space = c.is_whitespace() && c != '\u{a0}' && c != '\u{202f}' && c != '\u{2007}';
        if space {
            if let Some(s) = start.take() {
                out.push(Word { range: s..i, hard_break: false });
            }
            if c == '\n'
                && let Some(w) = out.last_mut()
            {
                w.hard_break = true;
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(s) = start {
        out.push(Word { range: s..text.len(), hard_break: false });
    }
    out
}

/// Greedy line breaking: as many words per line as fit in `width`, one
/// `space` between neighbours. A word wider than the line gets a line of
/// its own. A line is spread unless it is the paragraph's last, ends at a
/// hard break, holds a single word or fills less than `MIN_FILL` of the
/// width.
pub fn break_lines(widths: &[f32], hard: &[bool], space: f32, width: f32) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut used = 0f32;
    let close = |lines: &mut Vec<Line>, r: Range<usize>, used: f32, last: bool| {
        let justify = !last && r.len() > 1 && used >= width * MIN_FILL;
        lines.push(Line { words: r, justify });
    };
    for (i, &w) in widths.iter().enumerate() {
        if i > start && used + space + w > width {
            close(&mut lines, start..i, used, false);
            start = i;
            used = 0.;
        }
        used = if i == start { w } else { used + space + w };
        if hard.get(i).copied().unwrap_or(false) {
            close(&mut lines, start..i + 1, used, true);
            start = i + 1;
            used = 0.;
        }
    }
    if start < widths.len() {
        close(&mut lines, start..widths.len(), used, true);
    }
    lines
}

/// The parts of style runs (given by their lengths) that cover `range` of
/// the text: (run index, length) pairs, in order.
pub fn slice_runs(lens: &[usize], range: Range<usize>) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut at = 0;
    for (i, &len) in lens.iter().enumerate() {
        let (s, e) = (at.max(range.start), (at + len).min(range.end));
        if s < e {
            out.push((i, e - s));
        }
        at += len;
        if at >= range.end {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts<'a>(t: &'a str, w: &[Word]) -> Vec<&'a str> {
        w.iter().map(|w| &t[w.range.clone()]).collect()
    }

    #[test]
    fn words_split_at_breaking_spaces() {
        let t = "  One\u{a0}two  thr\u{e9}e\nfour ";
        let w = words(t);
        assert_eq!(texts(t, &w), ["One\u{a0}two", "thr\u{e9}e", "four"]);
        assert_eq!(w.iter().map(|w| w.hard_break).collect::<Vec<_>>(), [false, true, false]);
        assert!(words("").is_empty());
        assert!(words(" \n ").is_empty());
    }

    #[test]
    fn lines_fill_greedily_and_the_last_stays_ragged() {
        // Ten wide words, a 1-wide space, a 32-wide line: three per line.
        let widths = [10.; 7];
        let lines = break_lines(&widths, &[false; 7], 1., 32.);
        assert_eq!(
            lines,
            [
                Line { words: 0..3, justify: true },
                Line { words: 3..6, justify: true },
                Line { words: 6..7, justify: false },
            ]
        );
        // Exactly full fits.
        assert_eq!(break_lines(&[10., 10.], &[false; 2], 2., 22.).len(), 1);
    }

    #[test]
    fn single_words_short_lines_and_hard_breaks_are_not_spread() {
        // A word wider than the line gets its own unjustified line.
        let lines = break_lines(&[5., 50., 5., 5.], &[false; 4], 1., 20.);
        assert_eq!(lines[0], Line { words: 0..1, justify: false });
        assert_eq!(lines[1], Line { words: 1..2, justify: false });
        assert_eq!(lines[2], Line { words: 2..4, justify: false });
        // A line pushed early by a long next word fills under MIN_FILL.
        let lines = break_lines(&[4., 4., 95.], &[false; 3], 1., 100.);
        assert_eq!(lines[0], Line { words: 0..2, justify: false });
        // A hard break ends a ragged line and starts a new one.
        let lines = break_lines(&[10., 10., 10., 10., 10.], &[false, true, false, false, false], 1., 32.);
        assert_eq!(lines, [Line { words: 0..2, justify: false }, Line { words: 2..5, justify: false },]);
        assert!(break_lines(&[], &[], 1., 10.).is_empty());
    }

    #[test]
    fn runs_slice_to_a_range() {
        let lens = [3, 4, 5];
        assert_eq!(slice_runs(&lens, 0..3), [(0, 3)]);
        assert_eq!(slice_runs(&lens, 2..8), [(0, 1), (1, 4), (2, 1)]);
        assert_eq!(slice_runs(&lens, 7..12), [(2, 5)]);
        assert_eq!(slice_runs(&lens, 5..5), []);
        assert_eq!(slice_runs(&[0, 2], 0..2), [(1, 2)]);
    }
}
