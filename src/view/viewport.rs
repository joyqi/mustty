//! Viewport: scrolloff scrolling and typewriter mode.

use ropey::Rope;

use crate::editor::cursor::Affinity;
use crate::text::wrap::{VisualPos, WrapCache};

pub const SCROLLOFF: u16 = 3;

/// Top of the viewport: visual row `row` of logical line `line`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct ViewTop {
    pub line: usize,
    pub row: usize,
}

#[derive(Debug)]
pub struct Viewport {
    pub top: ViewTop,
    /// Rows of padding above the body in typewriter mode at the start of the document (effectively a negative ViewTop)
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
    /// Normal mode: scroll as little as possible to keep the cursor scrolloff rows from the top and
    /// bottom edges, never past the end of the document.
    /// Typewriter mode: the cursor's visual row stays at `body_h / 2`, with padding above the start of the document.
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
        // An edit may have removed or shortened the old top line
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

    /// Effective scrolloff: `(body_h - 1) / 2` when the body is shorter than 8 rows, otherwise the configured value capped at that.
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

    /// Keep the top of the viewport on the same text when the wrap width changes.
    pub fn rewrap(&mut self, width: u16, wrap: &mut WrapCache, rope: &Rope) {
        self.reflow(wrap, rope, |w| w.set_width(width));
    }

    /// Rewrap after a tab width change, likewise anchoring the top of the viewport to the same text.
    pub fn retab(&mut self, tab_width: u16, wrap: &mut WrapCache, rope: &Rope) {
        self.reflow(wrap, rope, |w| w.set_tab_width(tab_width));
    }

    fn reflow(&mut self, wrap: &mut WrapCache, rope: &Rope, change: impl FnOnce(&mut WrapCache)) {
        let row = wrap.rows(rope, self.top.line)[self.top.row];
        let anchor = rope.line_to_char(self.top.line) + row.start;
        change(wrap);
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
        let mut wrap = WrapCache::new(20, 4);
        let mut vp = Viewport::default();
        vp.follow(at(10), 10, &mut wrap, &rope);
        assert_eq!(vp.top.line, 4); // cursor on row 6, 3 rows from the bottom edge
        vp.follow(at(7), 10, &mut wrap, &rope);
        assert_eq!(vp.top.line, 4);
        vp.follow(at(6), 10, &mut wrap, &rope);
        assert_eq!(vp.top.line, 3); // fewer than 3 rows from the top edge
    }

    #[test]
    fn does_not_scroll_past_end() {
        let rope = doc(20); // 21 logical lines (with a trailing empty line)
        let mut wrap = WrapCache::new(20, 4);
        let mut vp = Viewport::default();
        vp.follow(at(20), 10, &mut wrap, &rope);
        assert_eq!(vp.top.line, 11);
    }

    #[test]
    fn small_body_centers_scrolloff() {
        let rope = doc(100);
        let mut wrap = WrapCache::new(20, 4);
        let mut vp = Viewport::default();
        vp.follow(at(50), 5, &mut wrap, &rope);
        assert_eq!(vp.top.line, 48);
    }

    #[test]
    fn configured_scrolloff() {
        let rope = doc(100);
        let mut wrap = WrapCache::new(20, 4);
        let mut vp = Viewport {
            scrolloff: 0,
            ..Viewport::default()
        };
        vp.follow(at(9), 10, &mut wrap, &rope);
        assert_eq!(
            vp.top.line, 0,
            "scrolloff 0: the cursor may reach the last row"
        );
        vp.scrolloff = 100; // too large: capped at (body_h - 1) / 2 = 4
        vp.follow(at(9), 10, &mut wrap, &rope);
        assert_eq!(vp.top.line, 4);
    }

    #[test]
    fn typewriter_keeps_cursor_on_middle_row() {
        let rope = doc(100);
        let mut wrap = WrapCache::new(20, 4);
        let mut vp = Viewport {
            typewriter: true,
            ..Viewport::default()
        };
        // Start of the document: pad above so line 0 still sits on the middle row (body_h = 10 → row 5)
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
        // No bounce at the end of the document either: blank space below is allowed
        assert_eq!(vp.top.line, 95);
        // Turning it off restores normal scrolloff behavior and clears the padding
        vp.typewriter = false;
        vp.follow(at(0), 10, &mut wrap, &rope);
        assert_eq!((vp.top.line, vp.top_padding), (0, 0));
    }

    #[test]
    fn rewrap_keeps_anchor_text() {
        let rope = Rope::from_str(&"word ".repeat(40));
        let mut wrap = WrapCache::new(20, 4);
        let mut vp = Viewport {
            top: ViewTop { line: 0, row: 3 },
            ..Viewport::default()
        };
        vp.rewrap(40, &mut wrap, &rope);
        assert_eq!(vp.top, ViewTop { line: 0, row: 1 });
    }
}
