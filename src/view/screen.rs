//! Screen ↔ document coordinate mapping, produced by rendering. Rendering uses it to place the title
//! text and the cursor; the mouse only looks things up in it. Centering, padding and horizontal
//! scroll are computed once instead of separately on each side.

use ratatui::layout::{Position, Rect};
use ropey::Rope;

use crate::text::grapheme::{grapheme_width, graphemes};
use crate::text::wrap::{VisualPos, WrapCache};
use crate::view::viewport::{ViewTop, Viewport};

/// Title placement: text is drawn from `rect.x + pad`, skipping the first `scroll` columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TitleMap {
    pub rect: Rect,
    pub pad: u16,
    pub scroll: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenMap {
    pub body: Rect,
    pub top: ViewTop,
    pub top_padding: u16,
    /// Effective scrolloff: a drag past the body's top or bottom edge targets the row on this boundary
    pub margin: u16,
    pub title: Option<TitleMap>,
}

impl ScreenMap {
    pub fn new(body: Rect, vp: &Viewport, title: Option<TitleMap>) -> Self {
        Self {
            body,
            top: vp.top,
            top_padding: vp.top_padding,
            margin: vp.margin(body.height),
            title,
        }
    }

    /// Resync after the viewport scrolls between renders (drag autoscroll), so later events in the same batch still map correctly.
    pub fn follow(&mut self, vp: &Viewport) {
        self.top = vp.top;
        self.top_padding = vp.top_padding;
    }

    /// Body visual position → screen coordinates; None outside the viewport. At the end of a full row,
    /// or past hanging whitespace, the cursor goes in the first cell right of the body (capped by
    /// `right_limit`) so it doesn't overlap the last character.
    pub fn body_screen(
        &self,
        pos: VisualPos,
        wrap: &mut WrapCache,
        rope: &Rope,
        right_limit: u16,
    ) -> Option<Position> {
        let top = (self.top.line, self.top.row);
        let dy = self.top_padding as isize + wrap.rows_between(rope, top, (pos.line, pos.row));
        (0..self.body.height as isize).contains(&dy).then(|| {
            let x = (self.body.x + pos.col.min(self.body.width)).min(right_limit.saturating_sub(1));
            Position::new(x, self.body.y + dy as u16)
        })
    }

    /// Screen coordinates → body char index; None outside the body area.
    pub fn body_char(&self, p: Position, wrap: &mut WrapCache, rope: &Rope) -> Option<usize> {
        self.body
            .contains(p)
            .then(|| self.body_char_at(p, wrap, rope))
    }

    /// Coordinates inside the body area (the caller guarantees this) → char index. The right half of a
    /// wide char snaps left. Top padding maps to the first document line, anything below the end of
    /// the document to the last visual row, and anything past a line end to the line end.
    pub fn body_char_at(&self, p: Position, wrap: &mut WrapCache, rope: &Rope) -> usize {
        let dy = p.y.saturating_sub(self.body.y) as isize - self.top_padding as isize;
        let (line, row) = if dy < 0 {
            (0, 0)
        } else {
            wrap.step_rows(rope, (self.top.line, self.top.row), dy)
        };
        let col = p.x.saturating_sub(self.body.x);
        wrap.visual_to_char(rope, VisualPos { line, row, col })
    }

    /// Drag position → char index, clamped to the body area. Past the top or bottom edge, target the row
    /// on the scrolloff boundary, so the viewport moves only at the autoscroll pace and scrolloff
    /// doesn't push it further.
    pub fn drag_char(&self, p: Position, wrap: &mut WrapCache, rope: &Rope) -> usize {
        let b = self.body;
        let x = p.x.clamp(b.x, b.right() - 1);
        let y = if p.y < b.y {
            b.y + self.margin
        } else if p.y >= b.bottom() {
            b.bottom() - 1 - self.margin
        } else {
            p.y
        };
        self.body_char_at(Position::new(x, y), wrap, rope)
    }

    /// Whether the drag is past the body's top or bottom edge (and needs autoscroll), and in which direction.
    pub fn drag_overflow(&self, p: Position) -> Option<isize> {
        if p.y < self.body.y {
            Some(-1)
        } else if p.y >= self.body.bottom() {
            Some(1)
        } else {
            None
        }
    }

    /// Title text column `col` → screen coordinates (used by rendering to place the title cursor).
    pub fn title_screen(&self, col: u16) -> Option<Position> {
        let t = self.title?;
        Some(Position::new(
            t.rect.x + t.pad + col.saturating_sub(t.scroll),
            t.rect.y,
        ))
    }

    /// Screen coordinates → title char index: subtract the centering padding and add the horizontal
    /// scroll. The right half of a wide char snaps left; left of the text maps to the start, right of
    /// it to the end. None off the title row.
    pub fn title_char(&self, p: Position, title: &str, tab_width: u16) -> Option<usize> {
        let t = self.title.filter(|t| t.rect.contains(p))?;
        let col = p.x.saturating_sub(t.rect.x + t.pad) + t.scroll;
        let mut c_col = 0;
        for (c, g) in graphemes(title) {
            let w = grapheme_width(g, c_col, tab_width);
            if col < c_col + w {
                return Some(c);
            }
            c_col += w;
        }
        Some(title.chars().count())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::cursor::Affinity;

    fn map(body: Rect, top: ViewTop, top_padding: u16, title: Option<TitleMap>) -> ScreenMap {
        ScreenMap {
            body,
            top,
            top_padding,
            margin: 2,
            title,
        }
    }

    #[test]
    fn screen_to_body_char() {
        // Body area at x = 4, y = 2, 6 wide: line 0 wraps as "一二三" | "四a", line 1 is "hello"
        let rope = Rope::from_str("一二三四a\nhello\n");
        let mut wrap = WrapCache::new(6, 4);
        let body = Rect::new(4, 2, 6, 5);
        let mut m = map(body, ViewTop::default(), 0, None);
        let mut hit = |m: &ScreenMap, x, y| m.body_char(Position::new(x, y), &mut wrap, &rope);
        assert_eq!(hit(&m, 4, 2), Some(0));
        // Wide char: both halves land on "二"
        assert_eq!(hit(&m, 6, 2), Some(1));
        assert_eq!(hit(&m, 7, 2), Some(1));
        // Second visual row after a soft wrap
        assert_eq!(hit(&m, 4, 3), Some(3));
        assert_eq!(hit(&m, 6, 3), Some(4));
        // Past the line end: on the last row of a logical line, map to the line end
        assert_eq!(hit(&m, 9, 3), Some(5));
        assert_eq!(hit(&m, 9, 4), Some(11));
        // Below the end of the document (inside the area): the last visual row (the trailing empty line)
        assert_eq!(hit(&m, 5, 6), Some(12));
        // Outside the body area
        for (x, y) in [(3, 2), (10, 2), (4, 1), (4, 7)] {
            assert_eq!(hit(&m, x, y), None, "({x},{y})");
        }
        // Viewport offset: the top is visual row 1 of line 0
        m.top = ViewTop { line: 0, row: 1 };
        assert_eq!(hit(&m, 4, 2), Some(3));
        assert_eq!(hit(&m, 5, 3), Some(7));
        // Typewriter top padding: maps to the first document line
        m.top = ViewTop::default();
        m.top_padding = 2;
        assert_eq!(hit(&m, 6, 2), Some(1));
        assert_eq!(hit(&m, 4, 4), Some(0));
        assert_eq!(hit(&m, 4, 5), Some(3));
    }

    #[test]
    fn body_screen_is_inverse_of_body_char() {
        let rope = Rope::from_str("一二三四a\nhello\n");
        let mut wrap = WrapCache::new(6, 4);
        let m = map(Rect::new(4, 2, 6, 5), ViewTop { line: 0, row: 1 }, 1, None);
        for idx in [3, 4, 5, 6, 8, 11, 12] {
            let pos = wrap.char_to_visual(&rope, idx, Affinity::Downstream);
            let p = m.body_screen(pos, &mut wrap, &rope, 80).unwrap();
            assert_eq!(
                m.body_char(p, &mut wrap, &rope),
                Some(idx),
                "idx {idx} at {p:?}"
            );
        }
        // Rows above the viewport are off screen (top padding only appears at the start of the document, so none here)
        let m = map(Rect::new(4, 2, 6, 5), ViewTop { line: 0, row: 1 }, 0, None);
        let pos = wrap.char_to_visual(&rope, 0, Affinity::Downstream);
        assert_eq!(m.body_screen(pos, &mut wrap, &rope, 80), None);
    }

    #[test]
    fn title_click_with_centering_and_horizontal_scroll() {
        let rect = Rect::new(2, 1, 10, 1);
        // Centered: "标题" is 4 wide, padding 3, so the text starts at x = 5
        let m = map(
            Rect::new(2, 3, 10, 4),
            ViewTop::default(),
            0,
            Some(TitleMap {
                rect,
                pad: 3,
                scroll: 0,
            }),
        );
        let t = |m: &ScreenMap, x, title| m.title_char(Position::new(x, 1), title, 4);
        assert_eq!(t(&m, 5, "标题"), Some(0));
        assert_eq!(t(&m, 6, "标题"), Some(0), "right half snaps left");
        assert_eq!(t(&m, 7, "标题"), Some(1));
        assert_eq!(t(&m, 3, "标题"), Some(0), "left of the text → start");
        assert_eq!(t(&m, 11, "标题"), Some(2), "right of the text → end");
        assert_eq!(
            m.title_char(Position::new(5, 2), "标题", 4),
            None,
            "not on the title row"
        );
        // An overlong title scrolled by 7 columns: screen column 0 shows text column 7
        let text = "abcdefghijklmnop";
        let m = map(
            Rect::new(2, 3, 10, 4),
            ViewTop::default(),
            0,
            Some(TitleMap {
                rect,
                pad: 0,
                scroll: 7,
            }),
        );
        assert_eq!(t(&m, 2, text), Some(7));
        assert_eq!(t(&m, 11, text), Some(16));
        assert_eq!(m.title_screen(16), Some(Position::new(11, 1)));
        assert_eq!(m.title_screen(7), Some(Position::new(2, 1)));
    }

    #[test]
    fn drag_targets_scrolloff_boundary_outside_body() {
        let rope = Rope::from_str(&(0..50).map(|i| format!("{i}\n")).collect::<String>());
        let mut wrap = WrapCache::new(10, 4);
        let m = map(
            Rect::new(0, 5, 10, 8),
            ViewTop { line: 10, row: 0 },
            0,
            None,
        );
        assert_eq!(m.drag_overflow(Position::new(3, 4)), Some(-1));
        assert_eq!(m.drag_overflow(Position::new(3, 13)), Some(1));
        assert_eq!(m.drag_overflow(Position::new(3, 9)), None);
        let line = |i| rope.line_to_char(i);
        assert_eq!(m.drag_char(Position::new(0, 0), &mut wrap, &rope), line(12));
        assert_eq!(
            m.drag_char(Position::new(0, 30), &mut wrap, &rope),
            line(15)
        );
        assert_eq!(
            m.drag_char(Position::new(99, 7), &mut wrap, &rope),
            line(12) + 2
        );
    }
}
