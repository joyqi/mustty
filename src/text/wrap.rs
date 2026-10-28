//! 软换行（方案 §4.3）：UAX #14 断行机会 + 悬挂空白 + 超长词强制断行。

use std::ops::Range;

use ropey::Rope;
use unicode_linebreak::{BreakOpportunity, linebreaks};

use super::grapheme::{grapheme_width, graphemes, is_whitespace};
use crate::editor::cursor::Affinity;

/// 一个视觉行，区间为行内 char 偏移 `[start, end)`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisualRow {
    pub start: usize,
    pub end: usize,
    /// 本行显示宽度（不含悬挂空白）。
    pub width: u16,
}

/// 第 `line` 个逻辑行的第 `row` 个视觉行、第 `col` 列。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisualPos {
    pub line: usize,
    pub row: usize,
    pub col: u16,
}

pub fn wrap_line(line: &str, width: u16) -> Vec<VisualRow> {
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
    // 当前视觉行内最近的断行机会：(grapheme 下标, 断在此处时的行宽)
    let mut last_opp: Option<(usize, u16)> = None;
    let mut hanging = false;
    while i < gs.len() {
        let (c, g) = gs[i];
        if i > start && is_opp[c] {
            last_opp = Some((i, col));
        }
        let w = grapheme_width(g, col);
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
            // 单个 grapheme 比整行还宽，只能独占一行
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

/// 行内 char 偏移 → `(row, col)`。软换行处的偏移按 `Downstream` 显示在下一视觉行行首，
/// `Upstream` 显示在上一视觉行行尾。
pub fn char_to_visual(line: &str, rows: &[VisualRow], idx: usize, aff: Affinity) -> (usize, u16) {
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
            col += grapheme_width(g, col);
        }
    }
    (row, col)
}

/// `(row, col)` → 行内 char 偏移。列落在宽字符中间时吸附到其左侧；
/// 超出行尾时，末行取行尾，其余行取最后一个 grapheme。
pub fn visual_to_char(line: &str, rows: &[VisualRow], row: usize, col: u16) -> usize {
    let row = row.min(rows.len() - 1);
    let r = rows[row];
    let mut c_col = 0;
    let mut last = r.start;
    for (c, g) in graphemes(line)
        .skip_while(|&(c, _)| c < r.start)
        .take_while(|&(c, _)| c < r.end)
    {
        let w = grapheme_width(g, c_col);
        if col < c_col + w {
            return c;
        }
        last = c;
        c_col += w;
    }
    if row + 1 == rows.len() { r.end } else { last }
}

/// 逻辑行文本（不含末尾 `\n`）。
pub fn line_text(rope: &Rope, line: usize) -> String {
    let mut s = rope.line(line).to_string();
    if s.ends_with('\n') {
        s.pop();
    }
    s
}

/// 按逻辑行惰性缓存 `wrap_line` 的结果。
pub struct WrapCache {
    width: u16,
    lines: Vec<Option<Vec<VisualRow>>>,
}

impl WrapCache {
    pub fn new(width: u16) -> Self {
        Self {
            width,
            lines: Vec::new(),
        }
    }

    pub fn width(&self) -> u16 {
        self.width
    }

    pub fn set_width(&mut self, width: u16) {
        if width != self.width {
            self.width = width;
            self.lines.fill(None);
        }
    }

    /// 编辑后标脏：`lines` 为编辑前受影响的逻辑行，`inserted` 为行数净增量。
    pub fn invalidate(&mut self, lines: Range<usize>, inserted: isize) {
        if lines.end > self.lines.len() {
            self.lines.clear();
            return;
        }
        let new_len = (lines.len() as isize + inserted).max(0) as usize;
        self.lines.splice(lines, std::iter::repeat_n(None, new_len));
    }

    pub fn rows(&mut self, rope: &Rope, line: usize) -> &[VisualRow] {
        // 槽位数与文档对不上（首次使用或 invalidate 无法定位）时整体重建
        if self.lines.len() != rope.len_lines() {
            self.lines = vec![None; rope.len_lines()];
        }
        let width = self.width;
        self.lines[line].get_or_insert_with(|| wrap_line(&line_text(rope, line), width))
    }

    pub fn char_to_visual(&mut self, rope: &Rope, idx: usize, aff: Affinity) -> VisualPos {
        let line = rope.char_to_line(idx);
        let text = line_text(rope, line);
        let local = idx - rope.line_to_char(line);
        let (row, col) = char_to_visual(&text, self.rows(rope, line), local, aff);
        VisualPos { line, row, col }
    }

    pub fn visual_to_char(&mut self, rope: &Rope, pos: VisualPos) -> usize {
        let text = line_text(rope, pos.line);
        rope.line_to_char(pos.line)
            + visual_to_char(&text, self.rows(rope, pos.line), pos.row, pos.col)
    }

    /// 从视觉行 `(line, row)` 前进 `delta` 个视觉行，在文档首尾处停住。
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

    /// 从视觉行 `from` 到 `to` 相隔的视觉行数（`to` 在前时为负）。
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
        wrap_line(line, width)
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
            // 避头：「，」不出现在行首
            ("no comma at head", "一二三，四", 6, &[(0, 2, 4), (2, 5, 6)]),
            ("no period at head", "一二三。", 6, &[(0, 2, 4), (2, 4, 4)]),
            // 避尾：「「」不出现在行尾
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
    fn long_url_stays_within_width() {
        let line = "见 https://example.com/a/very/long/path/that/does/not/fit?query=1 结束";
        for r in wrap_line(line, 20) {
            assert!(r.width <= 20, "{r:?}");
        }
    }

    #[test]
    fn mapping_snaps_wide_char_left() {
        let rows = wrap_line("中文", 10);
        assert_eq!(visual_to_char("中文", &rows, 0, 1), 0);
        assert_eq!(visual_to_char("中文", &rows, 0, 3), 1);
        assert_eq!(visual_to_char("中文", &rows, 0, 9), 2);
    }

    #[test]
    fn mapping_wrap_point_is_downstream() {
        let line = "hello world foo";
        let rows = wrap_line(line, 11);
        assert_eq!(
            char_to_visual(line, &rows, 12, Affinity::Downstream),
            (1, 0)
        );
        assert_eq!(
            char_to_visual(line, &rows, 15, Affinity::Downstream),
            (1, 3)
        );
        // 非末行超出行尾：停在最后一个 grapheme 上
        assert_eq!(visual_to_char(line, &rows, 0, 40), 11);
    }

    #[test]
    fn mapping_affinity_at_row_boundaries() {
        use Affinity::{Downstream, Upstream};
        // 恰好写满一行：同一个偏移，Upstream 在本行行尾（第 6 列），Downstream 在下一行行首
        let full = "一二三四五六";
        let rows = wrap_line(full, 6);
        assert_eq!(char_to_visual(full, &rows, 3, Upstream), (0, 6));
        assert_eq!(char_to_visual(full, &rows, 3, Downstream), (1, 0));
        // 悬挂空白之后
        let line = "hello world foo";
        let rows = wrap_line(line, 11);
        assert_eq!(char_to_visual(line, &rows, 12, Upstream), (0, 12));
        // 非边界处两种 affinity 结果相同；行首、行尾、空行不受影响
        assert_eq!(char_to_visual(line, &rows, 5, Upstream), (0, 5));
        assert_eq!(char_to_visual(line, &rows, 0, Upstream), (0, 0));
        assert_eq!(char_to_visual(line, &rows, 15, Upstream), (1, 3));
        assert_eq!(char_to_visual("", &wrap_line("", 6), 0, Upstream), (0, 0));
    }

    #[test]
    fn invalidate_tracks_line_count() {
        let mut rope = Rope::from_str("a\nb\nc");
        let mut wrap = WrapCache::new(10);
        for l in 0..3 {
            wrap.rows(&rope, l);
        }
        rope.insert(1, "x\ny\nz"); // 第 0 行拆成 3 行
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
            let rows = wrap_line(&line, width);
            let bounds = boundaries(&line);
            prop_assert_eq!(rows[0].start, 0);
            prop_assert_eq!(rows.last().unwrap().end, line.chars().count());
            for w in rows.windows(2) {
                prop_assert_eq!(w[0].end, w[1].start);
            }
            for r in &rows {
                prop_assert!(bounds.contains(&r.start) && bounds.contains(&r.end));
                // 只有一个 grapheme（不计悬挂空白）的行允许超宽
                let seg: String = line.chars().skip(r.start).take(r.end - r.start).collect();
                let single = graphemes(seg.trim_end()).count() <= 1;
                prop_assert!(r.width <= width || single, "{:?}", r);
            }
        }

        #[test]
        fn mapping_round_trips(parts in prop::collection::vec(TEXT, 0..80), width in 10u16..80) {
            let line: String = parts.concat();
            let rows = wrap_line(&line, width);
            for idx in boundaries(&line) {
                let (row, col) = char_to_visual(&line, &rows, idx, Affinity::Downstream);
                prop_assert_eq!(visual_to_char(&line, &rows, row, col), idx);
            }
        }
    }
}
