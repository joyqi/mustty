//! Cursor, selection anchor, and movement by grapheme, visual row, and word.

use std::ops::Range;

use ropey::Rope;

use crate::text::grapheme::{graphemes, next_boundary, prev_boundary};
use crate::text::wrap::{VisualPos, WrapCache, line_text};

/// Which row shows a char index at a soft wrap: the start of the next visual row by default, the end of this row after `End`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Affinity {
    Upstream,
    #[default]
    Downstream,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cursor {
    /// Global char index, always on a grapheme boundary.
    pub head: usize,
    /// When `Some`, `[min(anchor, head), max)` is the selection.
    pub anchor: Option<usize>,
    /// Visual column kept across vertical moves; cleared by horizontal moves and edits.
    pub preferred_col: Option<u16>,
    pub affinity: Affinity,
}

impl Cursor {
    /// Move to `head`, clearing the selection, preferred_col, and affinity.
    pub fn set(&mut self, head: usize) {
        *self = Cursor {
            head,
            ..Cursor::default()
        };
    }

    /// The selection, if non-empty.
    pub fn selection(&self) -> Option<Range<usize>> {
        let anchor = self.anchor.filter(|&a| a != self.head)?;
        Some(anchor.min(self.head)..anchor.max(self.head))
    }

    pub fn visual(&self, rope: &Rope, wrap: &mut WrapCache) -> VisualPos {
        wrap.char_to_visual(rope, self.head, self.affinity)
    }

    /// The previous grapheme boundary; at line start, the end of the previous line (before `\n`).
    pub fn prev(rope: &Rope, head: usize) -> usize {
        let line = rope.char_to_line(head);
        let start = rope.line_to_char(line);
        if head > start {
            start + prev_boundary(&line_text(rope, line), head - start)
        } else {
            head.saturating_sub(1)
        }
    }

    /// The next grapheme boundary; at line end, the start of the next line.
    pub fn next(rope: &Rope, head: usize) -> usize {
        let line = rope.char_to_line(head);
        let start = rope.line_to_char(line);
        let text = line_text(rope, line);
        if head - start < text.chars().count() {
            start + next_boundary(&text, head - start)
        } else {
            (head + 1).min(rope.len_chars())
        }
    }

    /// ←: at the start of a soft-wrapped row, first move to the end of the previous visual row (same index, Upstream).
    pub fn move_left(&mut self, rope: &Rope, wrap: &mut WrapCache) {
        let pos = self.visual(rope, wrap);
        let row = wrap.rows(rope, pos.line)[pos.row];
        let at_row_start = self.head == rope.line_to_char(pos.line) + row.start;
        if self.affinity == Affinity::Downstream && pos.row > 0 && at_row_start {
            self.set(self.head);
            self.affinity = Affinity::Upstream;
        } else {
            self.set(Self::prev(rope, self.head));
        }
    }

    /// →: at the end of a soft-wrapped row (Upstream), first move to the start of the next visual row (same index).
    pub fn move_right(&mut self, rope: &Rope, wrap: &mut WrapCache) {
        let pos = self.visual(rope, wrap);
        let rows = wrap.rows(rope, pos.line);
        let at_row_end = self.head == rope.line_to_char(pos.line) + rows[pos.row].end;
        if self.affinity == Affinity::Upstream && pos.row + 1 < rows.len() && at_row_end {
            self.set(self.head);
        } else {
            self.set(Self::next(rope, self.head));
        }
    }

    /// Move `delta` visual rows up or down, keeping `preferred_col`.
    pub fn move_vertical(&mut self, rope: &Rope, wrap: &mut WrapCache, delta: isize) {
        let pos = self.visual(rope, wrap);
        let col = *self.preferred_col.get_or_insert(pos.col);
        let (line, row) = wrap.step_rows(rope, (pos.line, pos.row), delta);
        if (line, row) != (pos.line, pos.row) {
            self.head = wrap.visual_to_char(rope, VisualPos { line, row, col });
            self.affinity = Affinity::Downstream;
        }
    }

    /// Start of the visual row.
    pub fn move_row_start(&mut self, rope: &Rope, wrap: &mut WrapCache) {
        let pos = self.visual(rope, wrap);
        let start = wrap.rows(rope, pos.line)[pos.row].start;
        self.set(rope.line_to_char(pos.line) + start);
    }

    /// After the last grapheme of the visual row, shown at the end of this row.
    pub fn move_row_end(&mut self, rope: &Rope, wrap: &mut WrapCache) {
        let pos = self.visual(rope, wrap);
        let end = wrap.rows(rope, pos.line)[pos.row].end;
        self.set(rope.line_to_char(pos.line) + end);
        self.affinity = Affinity::Upstream;
    }

    /// Start of the next word (Ctrl+→): skip the current word and the whitespace after it; at line end, jump to the first word of the next line.
    pub fn word_right(rope: &Rope, head: usize) -> usize {
        let (line, start, text) = locate(rope, head);
        match Words::new(&text).right(head - start) {
            Some(p) => start + p,
            None if line + 1 < rope.len_lines() => {
                let next = rope.line_to_char(line + 1);
                next + Words::new(&line_text(rope, line + 1)).skip_space(0)
            }
            None => head,
        }
    }

    /// Start of the previous word (Ctrl+←); at line start, go to the end of the previous line.
    pub fn word_left(rope: &Rope, head: usize) -> usize {
        let (_, start, text) = locate(rope, head);
        match Words::new(&text).left(head - start) {
            Some(p) => start + p,
            None => head.saturating_sub(1),
        }
    }

    /// Last grapheme of the current or next word (vim `e`); if no word follows on this line, look at the next line.
    pub fn word_end(rope: &Rope, head: usize) -> usize {
        let (line, start, text) = locate(rope, head);
        if let Some(p) = Words::new(&text).end(head - start, true) {
            return start + p;
        }
        if line + 1 < rope.len_lines() {
            let next = rope.line_to_char(line + 1);
            if let Some(p) = Words::new(&line_text(rope, line + 1)).end(0, false) {
                return next + p;
            }
        }
        head
    }
}

fn locate(rope: &Rope, head: usize) -> (usize, usize, String) {
    let line = rope.char_to_line(head);
    (line, rope.line_to_char(line), line_text(rope, line))
}

/// Word segmentation: a run of graphemes of the same class is one word. A CJK run is one word (as in VS Code),
/// alphanumerics and `_` form words, and every other punctuation mark is a word of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Space,
    Cjk,
    Word,
    Punct,
}

fn class(g: &str) -> Class {
    let c = g.chars().next().unwrap_or(' ');
    let cjk = matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0xAC00..=0xD7AF | 0x20000..=0x2FFFF);
    if c.is_whitespace() {
        Class::Space
    } else if cjk {
        Class::Cjk
    } else if c.is_alphanumeric() || c == '_' {
        Class::Word
    } else {
        Class::Punct
    }
}

/// The graphemes of one logical line and their classes; positions are char offsets within the line.
struct Words {
    starts: Vec<usize>,
    classes: Vec<Class>,
    len: usize,
}

impl Words {
    fn new(line: &str) -> Self {
        let (starts, classes) = graphemes(line).map(|(s, g)| (s, class(g))).unzip();
        Self {
            starts,
            classes,
            len: line.chars().count(),
        }
    }

    fn pos(&self, i: usize) -> usize {
        self.starts.get(i).copied().unwrap_or(self.len)
    }

    fn index(&self, at: usize) -> usize {
        self.starts
            .iter()
            .position(|&s| s >= at)
            .unwrap_or(self.starts.len())
    }

    fn skip_space(&self, at: usize) -> usize {
        let mut i = self.index(at);
        while self.classes.get(i) == Some(&Class::Space) {
            i += 1;
        }
        self.pos(i)
    }

    fn right(&self, at: usize) -> Option<usize> {
        let mut i = self.index(at);
        let c = *self.classes.get(i)?;
        if c != Class::Space {
            while self.classes.get(i) == Some(&c) {
                i += 1;
            }
        }
        Some(self.skip_space(self.pos(i)))
    }

    fn left(&self, at: usize) -> Option<usize> {
        let mut i = self.index(at);
        if i == 0 {
            return None;
        }
        while i > 0 && self.classes[i - 1] == Class::Space {
            i -= 1;
        }
        if i > 0 {
            let c = self.classes[i - 1];
            while i > 0 && self.classes[i - 1] == c {
                i -= 1;
            }
        }
        Some(self.pos(i))
    }

    fn end(&self, at: usize, advance: bool) -> Option<usize> {
        let mut i = self.index(at) + usize::from(advance);
        while self.classes.get(i) == Some(&Class::Space) {
            i += 1;
        }
        let c = *self.classes.get(i)?;
        while self.classes.get(i + 1) == Some(&c) {
            i += 1;
        }
        Some(self.pos(i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn horizontal_crosses_lines_by_grapheme() {
        let rope = Rope::from_str("e\u{301}🇨🇳\n中");
        let mut wrap = WrapCache::new(20, 4);
        let mut c = Cursor::default();
        let mut stops = vec![];
        for _ in 0..6 {
            c.move_right(&rope, &mut wrap);
            stops.push(c.head);
        }
        assert_eq!(stops, vec![2, 4, 5, 6, 6, 6]);
        for want in [5, 4, 2, 0, 0] {
            c.move_left(&rope, &mut wrap);
            assert_eq!(c.head, want);
        }
    }

    #[test]
    fn vertical_keeps_preferred_col() {
        // line 0 "abcdef", line 1 "中文", line 2 "abcdef"
        let rope = Rope::from_str("abcdef\n中文\nabcdef");
        let mut wrap = WrapCache::new(20, 4);
        let mut c = Cursor {
            head: 3,
            ..Cursor::default()
        };
        c.move_vertical(&rope, &mut wrap, 1);
        // column 3 lands on the second cell of 「文」 → snaps to the left of 「文」
        assert_eq!(c.head, 7 + 1);
        c.move_vertical(&rope, &mut wrap, 1);
        assert_eq!(c.head, 10 + 3);
        c.move_vertical(&rope, &mut wrap, 5);
        assert_eq!(c.head, 13);
    }

    #[test]
    fn end_key_is_upstream_at_soft_wrap() {
        // width 6: visual row 0 「一二三」 fills exactly, visual row 1 is 「四五」
        let rope = Rope::from_str("一二三四五");
        let mut wrap = WrapCache::new(6, 4);
        let mut c = Cursor::default();
        c.move_row_end(&rope, &mut wrap);
        assert_eq!((c.head, c.affinity), (3, Affinity::Upstream));
        let pos = c.visual(&rope, &mut wrap);
        assert_eq!((pos.row, pos.col), (0, 6));
        // Home from the Upstream end of row 0 goes to the start of row 0, not row 1
        c.move_row_start(&rope, &mut wrap);
        assert_eq!(c.head, 0);
        // End on the last row: line end, no ambiguity
        c.set(4);
        c.move_row_end(&rope, &mut wrap);
        assert_eq!((c.head, c.visual(&rope, &mut wrap).row), (5, 1));
    }

    #[test]
    fn left_right_flip_affinity_at_soft_wrap() {
        let rope = Rope::from_str("一二三四五");
        let mut wrap = WrapCache::new(6, 4);
        let mut c = Cursor::default();
        c.move_row_end(&rope, &mut wrap); // (0, 6) Upstream
        c.move_right(&rope, &mut wrap);
        // same index moves to the start of the next visual row, without skipping the position before 「四」
        assert_eq!((c.head, c.affinity), (3, Affinity::Downstream));
        assert_eq!(c.visual(&rope, &mut wrap).row, 1);
        c.move_left(&rope, &mut wrap);
        assert_eq!((c.head, c.affinity), (3, Affinity::Upstream));
        c.move_left(&rope, &mut wrap);
        assert_eq!((c.head, c.affinity), (2, Affinity::Downstream));
    }

    #[test]
    fn vertical_from_upstream_end() {
        let rope = Rope::from_str("一二三四五");
        let mut wrap = WrapCache::new(6, 4);
        let mut c = Cursor::default();
        c.move_row_end(&rope, &mut wrap);
        c.move_vertical(&rope, &mut wrap, 1);
        // preferred_col = 6 is past the end of row 1 (the last row) → line end
        assert_eq!((c.head, c.affinity), (5, Affinity::Downstream));
    }

    #[test]
    fn word_motions() {
        //            0123456789012345678
        let line = "foo_bar, 中文写作  x\nnext word";
        let rope = Rope::from_str(line);
        let walk = |f: fn(&Rope, usize) -> usize, from: usize, n: usize| {
            let mut at = from;
            (0..n)
                .map(|_| {
                    at = f(&rope, at);
                    at
                })
                .collect::<Vec<_>>()
        };
        // foo_bar|,| |中文写作|  |x|\n|next
        assert_eq!(walk(Cursor::word_right, 0, 5), vec![7, 9, 15, 16, 17]);
        assert_eq!(walk(Cursor::word_right, 16, 2), vec![17, 22]);
        assert_eq!(walk(Cursor::word_left, 22, 5), vec![17, 16, 15, 9, 7]);
        assert_eq!(walk(Cursor::word_left, 7, 2), vec![0, 0]);
        // vim e: the last character of the current or next word
        assert_eq!(walk(Cursor::word_end, 0, 5), vec![6, 7, 12, 15, 20]);
    }

    #[test]
    fn selection_range_is_ordered_and_nonempty() {
        let c = Cursor {
            head: 2,
            anchor: Some(5),
            ..Cursor::default()
        };
        assert_eq!(c.selection(), Some(2..5));
        let empty = Cursor {
            head: 2,
            anchor: Some(2),
            ..Cursor::default()
        };
        assert_eq!(empty.selection(), None);
    }
}
