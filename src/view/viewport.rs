//! 视口与 scrolloff 滚动（方案 §4.4）。

use ropey::Rope;

use crate::editor::cursor::Affinity;
use crate::text::wrap::{VisualPos, WrapCache};

pub const SCROLLOFF: u16 = 3;

/// 视口顶部：第 `line` 个逻辑行的第 `row` 个视觉行。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct ViewTop {
    pub line: usize,
    pub row: usize,
}

#[derive(Debug, Default)]
pub struct Viewport {
    pub top: ViewTop,
}

impl Viewport {
    /// 最少量滚动，使光标与视口上下边保持 scrolloff 行距；不滚过文档末尾。
    pub fn follow(&mut self, cursor: VisualPos, body_h: u16, wrap: &mut WrapCache, rope: &Rope) {
        let h = body_h.max(1) as isize;
        let so = if body_h < 8 {
            (h - 1) / 2
        } else {
            SCROLLOFF as isize
        };
        let cur = (cursor.line, cursor.row);
        // 编辑可能让原来的顶部行消失或变短
        let last = rope.len_lines() - 1;
        let top_line = self.top.line.min(last);
        let top = (
            top_line,
            self.top.row.min(wrap.rows(rope, top_line).len() - 1),
        );
        let d = wrap.rows_between(rope, top, cur);
        let new_top = if d < so {
            wrap.step_rows(rope, cur, -so)
        } else if d > h - 1 - so {
            wrap.step_rows(rope, cur, -(h - 1 - so))
        } else {
            top
        };
        let end = (last, wrap.rows(rope, last).len() - 1);
        let max_top = wrap.step_rows(rope, end, -(h - 1));
        let (line, row) = new_top.min(max_top);
        self.top = ViewTop { line, row };
    }

    pub fn scroll(&mut self, delta: isize, wrap: &mut WrapCache, rope: &Rope) {
        let (line, row) = wrap.step_rows(rope, (self.top.line, self.top.row), delta);
        self.top = ViewTop { line, row };
    }

    /// 换行宽度变化前后，让视口顶部仍停在同一段文字上。
    pub fn rewrap(&mut self, width: u16, wrap: &mut WrapCache, rope: &Rope) {
        let row = wrap.rows(rope, self.top.line)[self.top.row];
        let anchor = rope.line_to_char(self.top.line) + row.start;
        wrap.set_width(width);
        let pos = wrap.char_to_visual(rope, anchor, Affinity::Downstream);
        self.top = ViewTop {
            line: pos.line,
            row: pos.row,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(lines: usize) -> Rope {
        Rope::from_str(&(0..lines).map(|i| format!("{i}\n")).collect::<String>())
    }

    fn at(line: usize) -> VisualPos {
        VisualPos {
            line,
            row: 0,
            col: 0,
        }
    }

    #[test]
    fn scrolloff_both_edges() {
        let rope = doc(100);
        let mut wrap = WrapCache::new(20);
        let mut vp = Viewport::default();
        vp.follow(at(10), 10, &mut wrap, &rope);
        assert_eq!(vp.top.line, 4); // 光标在第 6 行，距底边 3 行
        vp.follow(at(7), 10, &mut wrap, &rope);
        assert_eq!(vp.top.line, 4);
        vp.follow(at(6), 10, &mut wrap, &rope);
        assert_eq!(vp.top.line, 3); // 距顶边不足 3 行
    }

    #[test]
    fn does_not_scroll_past_end() {
        let rope = doc(20); // 21 个逻辑行（末尾空行）
        let mut wrap = WrapCache::new(20);
        let mut vp = Viewport::default();
        vp.follow(at(20), 10, &mut wrap, &rope);
        assert_eq!(vp.top.line, 11);
    }

    #[test]
    fn small_body_centers_scrolloff() {
        let rope = doc(100);
        let mut wrap = WrapCache::new(20);
        let mut vp = Viewport::default();
        vp.follow(at(50), 5, &mut wrap, &rope);
        assert_eq!(vp.top.line, 48);
    }

    #[test]
    fn rewrap_keeps_anchor_text() {
        let rope = Rope::from_str(&"word ".repeat(40));
        let mut wrap = WrapCache::new(20);
        let mut vp = Viewport {
            top: ViewTop { line: 0, row: 3 },
        };
        vp.rewrap(40, &mut wrap, &rope);
        assert_eq!(vp.top, ViewTop { line: 0, row: 1 });
    }
}
