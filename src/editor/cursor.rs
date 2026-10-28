//! 光标与 grapheme / 视觉行级移动（方案 §4.2）。

use ropey::Rope;

use crate::text::grapheme::{next_boundary, prev_boundary};
use crate::text::wrap::{VisualPos, WrapCache, line_text};

/// 软换行处同一个 char 索引显示在哪一行：默认下一视觉行行首，`End` 键之后为本行行尾。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Affinity {
    Upstream,
    #[default]
    Downstream,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cursor {
    /// 全局 char 索引，永远落在 grapheme 边界。
    pub head: usize,
    /// 上下移动时保持的视觉列，水平移动 / 编辑后清空。
    pub preferred_col: Option<u16>,
    pub affinity: Affinity,
}

impl Cursor {
    /// 放到 `head`，清空 preferred_col 与 affinity。
    pub fn set(&mut self, head: usize) {
        *self = Cursor {
            head,
            ..Cursor::default()
        };
    }

    pub fn visual(&self, rope: &Rope, wrap: &mut WrapCache) -> VisualPos {
        wrap.char_to_visual(rope, self.head, self.affinity)
    }

    /// 前一个 grapheme 边界；行首则为上一行行尾（`\n` 之前）。
    pub fn prev(rope: &Rope, head: usize) -> usize {
        let line = rope.char_to_line(head);
        let start = rope.line_to_char(line);
        if head > start {
            start + prev_boundary(&line_text(rope, line), head - start)
        } else {
            head.saturating_sub(1)
        }
    }

    /// 后一个 grapheme 边界；行尾则为下一行行首。
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

    pub fn move_left(&mut self, rope: &Rope) {
        self.set(Self::prev(rope, self.head));
    }

    pub fn move_right(&mut self, rope: &Rope) {
        self.set(Self::next(rope, self.head));
    }

    /// 按视觉行上下移动 `delta` 行，保持 `preferred_col`。
    pub fn move_vertical(&mut self, rope: &Rope, wrap: &mut WrapCache, delta: isize) {
        let pos = self.visual(rope, wrap);
        let col = *self.preferred_col.get_or_insert(pos.col);
        let (line, row) = wrap.step_rows(rope, (pos.line, pos.row), delta);
        if (line, row) != (pos.line, pos.row) {
            self.head = wrap.visual_to_char(rope, VisualPos { line, row, col });
            self.affinity = Affinity::Downstream;
        }
    }

    /// 视觉行行首。
    pub fn move_row_start(&mut self, rope: &Rope, wrap: &mut WrapCache) {
        let pos = self.visual(rope, wrap);
        let start = wrap.rows(rope, pos.line)[pos.row].start;
        self.set(rope.line_to_char(pos.line) + start);
    }

    /// 视觉行最后一个 grapheme 之后，显示在本行末尾。
    pub fn move_row_end(&mut self, rope: &Rope, wrap: &mut WrapCache) {
        let pos = self.visual(rope, wrap);
        let end = wrap.rows(rope, pos.line)[pos.row].end;
        self.set(rope.line_to_char(pos.line) + end);
        self.affinity = Affinity::Upstream;
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
            ..Cursor::default()
        };
        c.move_vertical(&rope, &mut wrap, 1);
        // 第 3 列落在「文」的第二格 → 吸附到「文」左侧
        assert_eq!(c.head, 7 + 1);
        c.move_vertical(&rope, &mut wrap, 1);
        assert_eq!(c.head, 10 + 3);
        c.move_vertical(&rope, &mut wrap, 5);
        assert_eq!(c.head, 13);
    }

    #[test]
    fn end_key_is_upstream_at_soft_wrap() {
        // 宽 6：第 0 视觉行「一二三」恰好写满，第 1 视觉行「四五」
        let rope = Rope::from_str("一二三四五");
        let mut wrap = WrapCache::new(6);
        let mut c = Cursor::default();
        c.move_row_end(&rope, &mut wrap);
        assert_eq!((c.head, c.affinity), (3, Affinity::Upstream));
        let pos = c.visual(&rope, &mut wrap);
        assert_eq!((pos.row, pos.col), (0, 6));
        // 同一索引的默认显示在下一行行首
        c.move_right(&rope);
        c.move_left(&rope);
        assert_eq!(c.head, 3);
        let pos = c.visual(&rope, &mut wrap);
        assert_eq!((pos.row, pos.col), (1, 0));
        // 从第 0 行的 Upstream 行尾按 Home：回到第 0 行行首，而不是第 1 行行首
        c.set(0);
        c.move_row_end(&rope, &mut wrap);
        c.move_row_start(&rope, &mut wrap);
        assert_eq!(c.head, 0);
        // 末行 End：行尾，无歧义
        c.set(4);
        c.move_row_end(&rope, &mut wrap);
        assert_eq!(c.visual(&rope, &mut wrap).row, 1);
        assert_eq!(c.head, 5);
    }

    #[test]
    fn vertical_from_upstream_end() {
        let rope = Rope::from_str("一二三四五");
        let mut wrap = WrapCache::new(6);
        let mut c = Cursor::default();
        c.move_row_end(&rope, &mut wrap);
        c.move_vertical(&rope, &mut wrap, 1);
        // preferred_col = 6 超出第 1 行（末行）→ 行尾
        assert_eq!((c.head, c.affinity), (5, Affinity::Downstream));
    }
}
