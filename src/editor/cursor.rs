//! 光标与只读浏览用的移动（方案 §4.2）。

use ropey::Rope;

use crate::text::grapheme::{next_boundary, prev_boundary};
use crate::text::wrap::{VisualPos, WrapCache, line_text};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cursor {
    /// 全局 char 索引，永远落在 grapheme 边界。
    pub head: usize,
    /// 上下移动时保持的视觉列，水平移动后清空。
    pub preferred_col: Option<u16>,
}

impl Cursor {
    pub fn move_left(&mut self, rope: &Rope) {
        self.preferred_col = None;
        let line = rope.char_to_line(self.head);
        let start = rope.line_to_char(line);
        if self.head > start {
            self.head = start + prev_boundary(&line_text(rope, line), self.head - start);
        } else if line > 0 {
            self.head = start - 1;
        }
    }

    pub fn move_right(&mut self, rope: &Rope) {
        self.preferred_col = None;
        let line = rope.char_to_line(self.head);
        let start = rope.line_to_char(line);
        let text = line_text(rope, line);
        if self.head - start < text.chars().count() {
            self.head = start + next_boundary(&text, self.head - start);
        } else if line + 1 < rope.len_lines() {
            self.head = rope.line_to_char(line + 1);
        }
    }

    /// 按视觉行上下移动 `delta` 行，保持 `preferred_col`。
    pub fn move_vertical(&mut self, rope: &Rope, wrap: &mut WrapCache, delta: isize) {
        let pos = wrap.char_to_visual(rope, self.head);
        let col = *self.preferred_col.get_or_insert(pos.col);
        let (line, row) = wrap.step_rows(rope, (pos.line, pos.row), delta);
        if (line, row) != (pos.line, pos.row) {
            self.head = wrap.visual_to_char(rope, VisualPos { line, row, col });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn horizontal_crosses_lines_by_grapheme() {
        let rope = Rope::from_str("e\u{301}🇨🇳\n中");
        let mut c = Cursor::default();
        let mut stops = vec![];
        for _ in 0..6 {
            c.move_right(&rope);
            stops.push(c.head);
        }
        assert_eq!(stops, vec![2, 4, 5, 6, 6, 6]);
        for want in [5, 4, 2, 0, 0] {
            c.move_left(&rope);
            assert_eq!(c.head, want);
        }
    }

    #[test]
    fn vertical_keeps_preferred_col() {
        // 第 0 行 "abcdef"，第 1 行 "中文"，第 2 行 "abcdef"
        let rope = Rope::from_str("abcdef\n中文\nabcdef");
        let mut wrap = WrapCache::new(20);
        let mut c = Cursor {
            head: 3,
            preferred_col: None,
        };
        c.move_vertical(&rope, &mut wrap, 1);
        // 第 3 列落在「文」的第二格 → 吸附到「文」左侧
        assert_eq!(c.head, 7 + 1);
        c.move_vertical(&rope, &mut wrap, 1);
        assert_eq!(c.head, 10 + 3);
        c.move_vertical(&rope, &mut wrap, 5);
        assert_eq!(c.head, 13);
    }
}
