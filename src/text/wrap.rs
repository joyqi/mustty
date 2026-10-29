//! Soft wrap: UAX #14 break opportunities + hanging whitespace + forced breaks for overlong words.

use std::ops::Range;

use ropey::Rope;
use unicode_linebreak::{BreakOpportunity, linebreaks};

use super::grapheme::{grapheme_width, graphemes, is_whitespace};
use crate::editor::cursor::Affinity;

/// One visual row, spanning char offsets `[start, end)` within the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisualRow {
    pub start: usize,
    pub end: usize,
    /// Display width of this row (excluding hanging whitespace).
    pub width: u16,
}

/// Visual row `row` of logical line `line`, at column `col`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisualPos {
    pub line: usize,
    pub row: usize,
    pub col: u16,
}

pub fn wrap_line(line: &str, width: u16, tab_width: u16) -> Vec<VisualRow> {
    let width = width.max(1);
    let gs: Vec<(usize, &str)> = graphemes(line).collect();
    let total = line.chars().count();

    let mut is_opp = vec![false; total + 1];
    let mut char_bytes = line
        .char_indices()
        .map(|(b, _)| b)
        .chain(std::iter::once(line.len()))
        .enumerate();
    for (byte, opp) in linebreaks(line) {
        if opp == BreakOpportunity::Allowed {
            for (c, b) in char_bytes.by_ref() {
                if b == byte {
                    is_opp[c] = true;
                    break;
                }
            }
        }
    }

    let char_at = |gi: usize| gs.get(gi).map_or(total, |&(c, _)| c);
    let mut rows = Vec::new();
    let (mut start, mut i, mut col) = (0, 0, 0u16);
    // Latest break opportunity in this visual row: (grapheme index, row width if broken here)
    let mut last_opp: Option<(usize, u16)> = None;
    let mut hanging = false;
    while i < gs.len() {
        let (c, g) = gs[i];
        if i > start && is_opp[c] {
            last_opp = Some((i, col));
        }
        let w = grapheme_width(g, col, tab_width);
        if !hanging && col + w <= width {
            col += w;
            i += 1;
            continue;
        }
        if is_whitespace(g) {
            hanging = true;
            i += 1;
            continue;
        }
        let (at, row_width) = match last_opp {
            Some(opp) => opp,
            // A single grapheme wider than the row has to take a row of its own
            None if i == start => {
                col += w;
                i += 1;
                continue;
            }
            None => (i, col),
        };
        rows.push(VisualRow {
            start: char_at(start),
            end: char_at(at),
            width: row_width,
        });
        (start, i, col) = (at, at, 0);
        last_opp = None;
        hanging = false;
    }
    rows.push(VisualRow {
        start: char_at(start),
        end: total,
        width: col,
    });
    rows
}

/// Char offset within the line → `(row, col)`. At a soft wrap point, `Downstream` places the offset
/// at the start of the next visual row and `Upstream` at the end of the previous one.
pub fn char_to_visual(
    line: &str,
    rows: &[VisualRow],
    idx: usize,
    aff: Affinity,
    tab_width: u16,
) -> (usize, u16) {
    let upstream = match aff {
        Affinity::Upstream => rows.iter().position(|r| r.start < idx && idx <= r.end),
        Affinity::Downstream => None,
    };
    let row = upstream.unwrap_or_else(|| rows.iter().rposition(|r| r.start <= idx).unwrap_or(0));
    let start = rows[row].start;
    let mut col = 0;
    for (c, g) in graphemes(line) {
        if c >= idx {
            break;
        }
        if c >= start {
            col += grapheme_width(g, col, tab_width);
        }
    }
    (row, col)
}

/// `(row, col)` → char offset within the line. A column inside a wide character snaps to its left
/// edge. Past the row end, the last row yields the line end and other rows yield their last
/// grapheme.
pub fn visual_to_char(
    line: &str,
    rows: &[VisualRow],
    row: usize,
    col: u16,
    tab_width: u16,
) -> usize {
    let row = row.min(rows.len() - 1);
    let r = rows[row];
    let mut c_col = 0;
    let mut last = r.start;
    for (c, g) in graphemes(line)
        .skip_while(|&(c, _)| c < r.start)
        .take_while(|&(c, _)| c < r.end)
    {
        let w = grapheme_width(g, c_col, tab_width);
        if col < c_col + w {
            return c;
        }
        last = c;
        c_col += w;
    }
    if row + 1 == rows.len() { r.end } else { last }
}

/// Text of a logical line (without the trailing `\n`).
pub fn line_text(rope: &Rope, line: usize) -> String {
    let mut s = rope.line(line).to_string();
    if s.ends_with('\n') {
        s.pop();
    }
    s
}

/// Lazily caches `wrap_line` results per logical line.
pub struct WrapCache {
    width: u16,
    /// Display width of a Tab (the tab_width setting); both wrapping and rendering read it here
    tab_width: u16,
    lines: Vec<Option<Vec<VisualRow>>>,
}

impl WrapCache {
    pub fn new(width: u16, tab_width: u16) -> Self {
        Self {
            width,
            tab_width,
            lines: Vec::new(),
        }
    }

    pub fn width(&self) -> u16 {
        self.width
    }

    pub fn tab_width(&self) -> u16 {
        self.tab_width
    }

    pub fn set_width(&mut self, width: u16) {
        if width != self.width {
            self.width = width;
            self.lines.fill(None);
        }
    }

    /// Changes the Tab width. Any line's wrap may change, so the whole cache is cleared (the
    /// settings panel can change tab_width at runtime).
    pub fn set_tab_width(&mut self, tab_width: u16) {
        if tab_width != self.tab_width {
            self.tab_width = tab_width;
            self.lines.fill(None);
        }
    }

    /// Marks lines dirty after an edit: `lines` are the affected logical lines before the edit,
    /// `inserted` is the net change in line count.
    pub fn invalidate(&mut self, lines: Range<usize>, inserted: isize) {
        if lines.end > self.lines.len() {
            self.lines.clear();
            return;
        }
        let new_len = (lines.len() as isize + inserted).max(0) as usize;
        self.lines.splice(lines, std::iter::repeat_n(None, new_len));
    }

    /// Every cached line in `lines` matches a fresh wrap, and the slot count matches the document's
    /// line count.
    pub fn is_consistent(&self, rope: &Rope, lines: Range<usize>) -> bool {
        if self.lines.is_empty() {
            return true;
        }
        self.lines.len() == rope.len_lines()
            && lines.into_iter().all(|l| match &self.lines[l] {
                Some(rows) => *rows == wrap_line(&line_text(rope, l), self.width, self.tab_width),
                None => true,
            })
    }

    pub fn rows(&mut self, rope: &Rope, line: usize) -> &[VisualRow] {
        // Rebuild everything when the slot count doesn't match the document (first use, or
        // invalidate couldn't locate the edit)
        if self.lines.len() != rope.len_lines() {
            self.lines = vec![None; rope.len_lines()];
        }
        let (width, tab) = (self.width, self.tab_width);
        self.lines[line].get_or_insert_with(|| wrap_line(&line_text(rope, line), width, tab))
    }

    pub fn char_to_visual(&mut self, rope: &Rope, idx: usize, aff: Affinity) -> VisualPos {
        let line = rope.char_to_line(idx);
        let text = line_text(rope, line);
        let local = idx - rope.line_to_char(line);
        let tab = self.tab_width;
        let (row, col) = char_to_visual(&text, self.rows(rope, line), local, aff, tab);
        VisualPos { line, row, col }
    }

    pub fn visual_to_char(&mut self, rope: &Rope, pos: VisualPos) -> usize {
        let text = line_text(rope, pos.line);
        let tab = self.tab_width;
        rope.line_to_char(pos.line)
            + visual_to_char(&text, self.rows(rope, pos.line), pos.row, pos.col, tab)
    }

    /// Moves `delta` visual rows from visual row `(line, row)`, stopping at document edges.
    pub fn step_rows(&mut self, rope: &Rope, from: (usize, usize), delta: isize) -> (usize, usize) {
        let (mut line, mut row) = from;
        for _ in 0..delta.unsigned_abs() {
            if delta > 0 {
                if row + 1 < self.rows(rope, line).len() {
                    row += 1;
                } else if line + 1 < rope.len_lines() {
                    (line, row) = (line + 1, 0);
                } else {
                    break;
                }
            } else if row > 0 {
                row -= 1;
            } else if line > 0 {
                line -= 1;
                row = self.rows(rope, line).len() - 1;
            } else {
                break;
            }
        }
        (line, row)
    }

    /// Number of visual rows from visual row `from` to `to` (negative when `to` comes first).
    pub fn rows_between(&mut self, rope: &Rope, from: (usize, usize), to: (usize, usize)) -> isize {
        if to < from {
            return -self.rows_between(rope, to, from);
        }
        let skipped: usize = (from.0..to.0).map(|l| self.rows(rope, l).len()).sum();
        (skipped + to.1) as isize - from.1 as isize
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::grapheme::{graphemes, next_boundary};
    use proptest::prelude::*;

    fn spans(line: &str, width: u16) -> Vec<(usize, usize, u16)> {
        wrap_line(line, width, 4)
            .into_iter()
            .map(|r| (r.start, r.end, r.width))
            .collect()
    }

    #[test]
    fn table() {
        type Case<'a> = (&'a str, &'a str, u16, &'a [(usize, usize, u16)]);
        let cases: &[Case] = &[
            ("empty", "", 10, &[(0, 0, 0)]),
            (
                "english",
                "hello world foo",
                11,
                &[(0, 12, 11), (12, 15, 3)],
            ),
            ("chinese", "一二三四五六", 6, &[(0, 3, 6), (3, 6, 6)]),
            ("mixed", "Hello世界", 7, &[(0, 6, 7), (6, 7, 2)]),
            // Kinsoku: "，" is not allowed at line start
            ("no comma at head", "一二三，四", 6, &[(0, 2, 4), (2, 5, 6)]),
            ("no period at head", "一二三。", 6, &[(0, 2, 4), (2, 4, 4)]),
            // Kinsoku: "「" is not allowed at line end
            ("no open at tail", "一二「三」", 6, &[(0, 2, 4), (2, 5, 6)]),
            (
                "long word forced",
                "aaaaaaaaaaaaaaaaaaaaaaaa",
                10,
                &[(0, 10, 10), (10, 20, 10), (20, 24, 4)],
            ),
            ("hanging spaces", "abc     def", 5, &[(0, 8, 5), (8, 11, 3)]),
            ("tab", "a\tb", 8, &[(0, 3, 5)]),
            ("tab hangs", "abcd\tx", 4, &[(0, 5, 4), (5, 6, 1)]),
            (
                "emoji zwj",
                "👨\u{200d}👩\u{200d}👧👨\u{200d}👩\u{200d}👧",
                3,
                &[(0, 5, 2), (5, 10, 2)],
            ),
            ("width 1", "a中b", 1, &[(0, 1, 1), (1, 2, 2), (2, 3, 1)]),
        ];
        for (name, line, width, want) in cases {
            assert_eq!(spans(line, *width), *want, "{name}");
        }
    }

    #[test]
    fn configured_tab_width_drives_wrap_and_width() {
        use crate::text::grapheme::grapheme_width;
        assert_eq!(grapheme_width("\t", 0, 8), 8);
        assert_eq!(grapheme_width("\t", 3, 8), 5);
        assert_eq!(wrap_line("a\tb", 20, 8)[0].width, 9);
        assert_eq!(grapheme_width("\t", 5, 0), 1, "0 is treated as 1");
        // Explicit parameter: different tab widths in one process don't interfere
        assert_eq!(wrap_line("a\tb", 20, 2)[0].width, 3);
        assert_eq!(wrap_line("a\tb", 20, 8)[0].width, 9);
        let mut wide = WrapCache::new(20, 8);
        let mut narrow = WrapCache::new(20, 2);
        let rope = Rope::from_str("\tx");
        assert_eq!(wide.char_to_visual(&rope, 1, Affinity::Downstream).col, 8);
        assert_eq!(narrow.char_to_visual(&rope, 1, Affinity::Downstream).col, 2);
        assert_eq!(
            wide.visual_to_char(
                &rope,
                VisualPos {
                    line: 0,
                    row: 0,
                    col: 7
                }
            ),
            0
        );
    }

    #[test]
    fn long_url_stays_within_width() {
        let line = "见 https://example.com/a/very/long/path/that/does/not/fit?query=1 结束";
        for r in wrap_line(line, 20, 4) {
            assert!(r.width <= 20, "{r:?}");
        }
    }

    #[test]
    fn mapping_snaps_wide_char_left() {
        let rows = wrap_line("中文", 10, 4);
        assert_eq!(visual_to_char("中文", &rows, 0, 1, 4), 0);
        assert_eq!(visual_to_char("中文", &rows, 0, 3, 4), 1);
        assert_eq!(visual_to_char("中文", &rows, 0, 9, 4), 2);
    }

    #[test]
    fn mapping_wrap_point_is_downstream() {
        let line = "hello world foo";
        let rows = wrap_line(line, 11, 4);
        assert_eq!(
            char_to_visual(line, &rows, 12, Affinity::Downstream, 4),
            (1, 0)
        );
        assert_eq!(
            char_to_visual(line, &rows, 15, Affinity::Downstream, 4),
            (1, 3)
        );
        // Past the end of a non-last row: stop on the last grapheme
        assert_eq!(visual_to_char(line, &rows, 0, 40, 4), 11);
    }

    #[test]
    fn mapping_affinity_at_row_boundaries() {
        use Affinity::{Downstream, Upstream};
        // A row filled exactly: for the same offset, Upstream sits at this row's end (column 6),
        // Downstream at the start of the next row
        let full = "一二三四五六";
        let rows = wrap_line(full, 6, 4);
        assert_eq!(char_to_visual(full, &rows, 3, Upstream, 4), (0, 6));
        assert_eq!(char_to_visual(full, &rows, 3, Downstream, 4), (1, 0));
        // After hanging whitespace
        let line = "hello world foo";
        let rows = wrap_line(line, 11, 4);
        assert_eq!(char_to_visual(line, &rows, 12, Upstream, 4), (0, 12));
        // Off boundaries both affinities agree; line start, line end and empty lines are unaffected
        assert_eq!(char_to_visual(line, &rows, 5, Upstream, 4), (0, 5));
        assert_eq!(char_to_visual(line, &rows, 0, Upstream, 4), (0, 0));
        assert_eq!(char_to_visual(line, &rows, 15, Upstream, 4), (1, 3));
        assert_eq!(
            char_to_visual("", &wrap_line("", 6, 4), 0, Upstream, 4),
            (0, 0)
        );
    }

    #[test]
    fn invalidate_tracks_line_count() {
        let mut rope = Rope::from_str("a\nb\nc");
        let mut wrap = WrapCache::new(10, 4);
        for l in 0..3 {
            wrap.rows(&rope, l);
        }
        rope.insert(1, "x\ny\nz"); // line 0 splits into 3 lines
        wrap.invalidate(0..1, 2);
        assert_eq!(wrap.lines.len(), 5);
        assert!(wrap.lines[0].is_none() && wrap.lines[2].is_none());
        assert!(wrap.lines[3].is_some() && wrap.lines[4].is_some());
        assert_eq!(wrap.rows(&rope, 2)[0].end, 1);
    }

    fn boundaries(line: &str) -> Vec<usize> {
        let total = line.chars().count();
        let mut b = vec![0];
        while *b.last().unwrap() < total {
            b.push(next_boundary(line, *b.last().unwrap()));
        }
        b
    }

    const TEXT: &str = "[a-z ,.!中文字，。「」（）\t]|e\u{301}|👍|👨\u{200d}👩\u{200d}👧|🇨🇳";

    proptest! {
        #[test]
        fn rows_are_well_formed(parts in prop::collection::vec(TEXT, 0..80), width in 1u16..30) {
            let line: String = parts.concat();
            let rows = wrap_line(&line, width, 4);
            let bounds = boundaries(&line);
            prop_assert_eq!(rows[0].start, 0);
            prop_assert_eq!(rows.last().unwrap().end, line.chars().count());
            for w in rows.windows(2) {
                prop_assert_eq!(w[0].end, w[1].start);
            }
            for r in &rows {
                prop_assert!(bounds.contains(&r.start) && bounds.contains(&r.end));
                // A row with a single grapheme (ignoring hanging whitespace) may exceed the width
                let seg: String = line.chars().skip(r.start).take(r.end - r.start).collect();
                let single = graphemes(seg.trim_end()).count() <= 1;
                prop_assert!(r.width <= width || single, "{:?}", r);
            }
        }

        #[test]
        fn mapping_round_trips(parts in prop::collection::vec(TEXT, 0..80), width in 10u16..80) {
            let line: String = parts.concat();
            let rows = wrap_line(&line, width, 4);
            for idx in boundaries(&line) {
                let (row, col) = char_to_visual(&line, &rows, idx, Affinity::Downstream, 4);
                prop_assert_eq!(visual_to_char(&line, &rows, row, col, 4), idx);
            }
        }
    }
}
