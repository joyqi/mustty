//! 视口：scrolloff 滚动与 Typewriter 模式（方案 §4.4）。

use ratatui::layout::{Position, Rect};
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

#[derive(Debug)]
pub struct Viewport {
    pub top: ViewTop,
    /// Typewriter 模式在文档开头时，正文上方的留白行数（相当于「负」的 ViewTop）
    pub top_padding: u16,
    pub typewriter: bool,
    pub scrolloff: u16,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            top: ViewTop::default(),
            top_padding: 0,
            typewriter: false,
            scrolloff: SCROLLOFF,
        }
    }
}

impl Viewport {
    /// 普通模式：最少量滚动，使光标与视口上下边保持 scrolloff 行距，不滚过文档末尾。
    /// Typewriter 模式：光标视觉行恒在 `body_h / 2`，文档开头上方留白。
    pub fn follow(&mut self, cursor: VisualPos, body_h: u16, wrap: &mut WrapCache, rope: &Rope) {
        let h = body_h.max(1) as isize;
        let cur = (cursor.line, cursor.row);
        if self.typewriter {
            let mid = h / 2;
            let (line, row) = wrap.step_rows(rope, cur, -mid);
            let above = wrap.rows_between(rope, (line, row), cur);
            self.top = ViewTop { line, row };
            self.top_padding = (mid - above) as u16;
            return;
        }
        self.top_padding = 0;
        let so = self.margin(body_h) as isize;
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

    /// 实际生效的 scrolloff：正文高度 < 8 时为 `(body_h - 1) / 2`，否则为配置值且不超过它。
    pub fn margin(&self, body_h: u16) -> u16 {
        let half = body_h.saturating_sub(1) / 2;
        if body_h < 8 {
            half
        } else {
            self.scrolloff.min(half)
        }
    }

    pub fn scroll(&mut self, delta: isize, wrap: &mut WrapCache, rope: &Rope) {
        let (line, row) = wrap.step_rows(rope, (self.top.line, self.top.row), delta);
        self.top = ViewTop { line, row };
    }

    /// 屏幕坐标 → 正文 char 索引；正文区域之外返回 None。
    pub fn hit_test(
        &self,
        body: Rect,
        x: u16,
        y: u16,
        wrap: &mut WrapCache,
        rope: &Rope,
    ) -> Option<usize> {
        body.contains(Position::new(x, y))
            .then(|| self.char_at(body, x, y, wrap, rope))
    }

    /// 正文区域内（调用方保证）的坐标 → char 索引。宽字符右半格吸附左侧；
    /// 上方留白映射到文档首行，文档末尾以下映射到最后一个视觉行；超出行尾取行尾。
    pub fn char_at(&self, body: Rect, x: u16, y: u16, wrap: &mut WrapCache, rope: &Rope) -> usize {
        let dy = y.saturating_sub(body.y) as isize - self.top_padding as isize;
        let (line, row) = if dy < 0 {
            (0, 0)
        } else {
            wrap.step_rows(rope, (self.top.line, self.top.row), dy)
        };
        let col = x.saturating_sub(body.x);
        wrap.visual_to_char(rope, VisualPos { line, row, col })
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
    fn configured_scrolloff() {
        let rope = doc(100);
        let mut wrap = WrapCache::new(20);
        let mut vp = Viewport {
            scrolloff: 0,
            ..Viewport::default()
        };
        vp.follow(at(9), 10, &mut wrap, &rope);
        assert_eq!(vp.top.line, 0, "scrolloff 0：光标可以到最后一行");
        vp.scrolloff = 100; // 过大时取 (body_h - 1) / 2 = 4
        vp.follow(at(9), 10, &mut wrap, &rope);
        assert_eq!(vp.top.line, 4);
    }

    #[test]
    fn typewriter_keeps_cursor_on_middle_row() {
        let rope = doc(100);
        let mut wrap = WrapCache::new(20);
        let mut vp = Viewport {
            typewriter: true,
            ..Viewport::default()
        };
        // 文档开头：上方留白，使第 0 行仍在中线（body_h = 10 → 第 5 行）
        vp.follow(at(0), 10, &mut wrap, &rope);
        assert_eq!((vp.top.line, vp.top_padding), (0, 5));
        vp.follow(at(2), 10, &mut wrap, &rope);
        assert_eq!((vp.top.line, vp.top_padding), (0, 3));
        for line in [5, 6, 50, 100] {
            vp.follow(at(line), 10, &mut wrap, &rope);
            let screen_row = vp.top_padding as isize
                + wrap.rows_between(&rope, (vp.top.line, vp.top.row), (line, 0));
            assert_eq!(screen_row, 5, "line {line}");
        }
        // 文档末尾也不回弹：下方允许留白
        assert_eq!(vp.top.line, 95);
        // 关闭后回到普通 scrolloff 行为，留白清零
        vp.typewriter = false;
        vp.follow(at(0), 10, &mut wrap, &rope);
        assert_eq!((vp.top.line, vp.top_padding), (0, 0));
    }

    #[test]
    fn screen_to_char_mapping() {
        // 正文区域 x = 4, y = 2，宽 6：第 0 行「一二三」|「四a」，第 1 行 "hello"
        let rope = Rope::from_str("一二三四a\nhello\n");
        let mut wrap = WrapCache::new(6);
        let body = Rect::new(4, 2, 6, 5);
        let mut vp = Viewport::default();
        let hit = |vp: &Viewport, wrap: &mut WrapCache, x, y| vp.hit_test(body, x, y, wrap, &rope);
        assert_eq!(hit(&vp, &mut wrap, 4, 2), Some(0));
        // 宽字符：左半格与右半格都落在「二」上
        assert_eq!(hit(&vp, &mut wrap, 6, 2), Some(1));
        assert_eq!(hit(&vp, &mut wrap, 7, 2), Some(1));
        // 软换行后的第二个视觉行
        assert_eq!(hit(&vp, &mut wrap, 4, 3), Some(3));
        assert_eq!(hit(&vp, &mut wrap, 6, 3), Some(4));
        // 超出行尾：逻辑行的末行取行尾
        assert_eq!(hit(&vp, &mut wrap, 9, 3), Some(5));
        assert_eq!(hit(&vp, &mut wrap, 9, 4), Some(11));
        // 文档末尾以下（区域内）：最后一个视觉行（末尾空行）
        assert_eq!(hit(&vp, &mut wrap, 5, 6), Some(12));
        // 正文区域之外
        for (x, y) in [(3, 2), (10, 2), (4, 1), (4, 7)] {
            assert_eq!(hit(&vp, &mut wrap, x, y), None, "({x},{y})");
        }
        // 视口偏移：顶部是第 0 行的第 1 个视觉行
        vp.top = ViewTop { line: 0, row: 1 };
        assert_eq!(hit(&vp, &mut wrap, 4, 2), Some(3));
        assert_eq!(hit(&vp, &mut wrap, 5, 3), Some(7));
        // Typewriter 上方留白：映射到文档首行
        vp.top = ViewTop::default();
        vp.top_padding = 2;
        assert_eq!(hit(&vp, &mut wrap, 6, 2), Some(1));
        assert_eq!(hit(&vp, &mut wrap, 4, 4), Some(0));
        assert_eq!(hit(&vp, &mut wrap, 4, 5), Some(3));
    }

    #[test]
    fn rewrap_keeps_anchor_text() {
        let rope = Rope::from_str(&"word ".repeat(40));
        let mut wrap = WrapCache::new(20);
        let mut vp = Viewport {
            top: ViewTop { line: 0, row: 3 },
            ..Viewport::default()
        };
        vp.rewrap(40, &mut wrap, &rope);
        assert_eq!(vp.top, ViewTop { line: 0, row: 1 });
    }
}
