//! Grapheme cluster boundaries and display width. All offsets are char offsets within a line.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Yields `(char start, grapheme)` in order.
pub fn graphemes(s: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut chars = 0;
    s.graphemes(true).map(move |g| {
        let start = chars;
        chars += g.chars().count();
        (start, g)
    })
}

/// The nearest grapheme boundary before `char_idx`, or 0 at line start.
pub fn prev_boundary(s: &str, char_idx: usize) -> usize {
    graphemes(s)
        .map(|(start, _)| start)
        .take_while(|&start| start < char_idx)
        .last()
        .unwrap_or(0)
}

/// The nearest grapheme boundary after `char_idx`, or the line's char length at line end.
pub fn next_boundary(s: &str, char_idx: usize) -> usize {
    graphemes(s)
        .map(|(start, g)| start + g.chars().count())
        .find(|&end| end > char_idx)
        .unwrap_or_else(|| s.chars().count())
}

/// Display width of a grapheme starting at column `col`. A Tab advances to the next multiple of
/// `tab_width`; anything else is at least 1.
pub fn grapheme_width(g: &str, col: u16, tab_width: u16) -> u16 {
    if g == "\t" {
        let tab = tab_width.max(1);
        tab - col % tab
    } else {
        (g.width() as u16).max(1)
    }
}

/// Whitespace that may hang past the line end during soft wrap.
pub fn is_whitespace(g: &str) -> bool {
    g.chars().all(char::is_whitespace)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundaries_step_whole_clusters() {
        let s = "ae\u{301}👨\u{200d}👩\u{200d}👧🇨🇳中";
        let mut stops = vec![0];
        while let Some(&last) = stops.last() {
            let next = next_boundary(s, last);
            if next == last {
                break;
            }
            stops.push(next);
        }
        assert_eq!(stops, vec![0, 1, 3, 8, 10, 11]);
        for w in stops.windows(2) {
            assert_eq!(prev_boundary(s, w[1]), w[0]);
        }
        assert_eq!(prev_boundary(s, 0), 0);
    }

    #[test]
    fn widths() {
        assert_eq!(grapheme_width("a", 0, 4), 1);
        assert_eq!(grapheme_width("中", 0, 4), 2);
        assert_eq!(grapheme_width("\t", 0, 4), 4);
        assert_eq!(grapheme_width("\t", 3, 4), 1);
        assert_eq!(grapheme_width("\u{200b}", 0, 4), 1);
        // tab_width is an explicit parameter, so the same input always gives the same result
        assert_eq!(grapheme_width("\t", 0, 8), 8);
        assert_eq!(grapheme_width("\t", 3, 8), 5);
        assert_eq!(grapheme_width("\t", 5, 1), 1);
        assert_eq!(grapheme_width("\t", 5, 0), 1, "0 is treated as 1");
    }
}
