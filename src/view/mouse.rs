//! Mouse input (only called when `mouse = true`): screen coordinates → `Action::MoveTo` / `ScrollView`.
//! Coordinates are looked up in the `ScreenMap` produced by rendering, never recomputed from the
//! layout here. The mouse reuses the keyboard's Actions rather than opening a separate editing path.

use std::time::{Duration, Instant};

use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Position;
use ropey::Rope;

use crate::action::{Action, Target};
use crate::text::wrap::WrapCache;
use crate::view::screen::ScreenMap;

/// While a drag is past the top or bottom edge of the body, scroll one row per interval.
pub const DRAG_SCROLL_INTERVAL: Duration = Duration::from_millis(50);
/// Visual rows scrolled per wheel notch.
pub const WHEEL_ROWS: i32 = 3;

/// Drag state after a left press inside the body. `deadline` is set only while the drag is past the body's top or bottom edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DragState {
    pub last: Position,
    pub deadline: Option<Instant>,
}

pub fn map_mouse(
    ev: &MouseEvent,
    map: &ScreenMap,
    wrap: &mut WrapCache,
    rope: &Rope,
    title: &str,
    drag: &mut Option<DragState>,
    now: Instant,
) -> Option<Action> {
    let pos = Position::new(ev.column, ev.row);
    match ev.kind {
        // Ignore clicks with modifiers (by terminal convention, Shift-drag is native selection)
        _ if ev.modifiers != KeyModifiers::NONE => None,
        MouseEventKind::Down(MouseButton::Left) => {
            *drag = None;
            if let Some(at) = map.body_char(pos, wrap, rope) {
                *drag = Some(DragState {
                    last: pos,
                    deadline: None,
                });
                return Some(Action::MoveTo(Target::Body(at), false));
            }
            let at = map.title_char(pos, title, wrap.tab_width())?;
            Some(Action::MoveTo(Target::Title(at), false))
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            let state = drag.as_mut()?;
            state.last = pos;
            state.deadline = map.drag_overflow(pos).map(|_| now + DRAG_SCROLL_INTERVAL);
            Some(Action::MoveTo(
                Target::Body(map.drag_char(pos, wrap, rope)),
                true,
            ))
        }
        MouseEventKind::Up(MouseButton::Left) => {
            *drag = None;
            None
        }
        MouseEventKind::ScrollUp => Some(Action::ScrollView(-WHEEL_ROWS)),
        MouseEventKind::ScrollDown => Some(Action::ScrollView(WHEEL_ROWS)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::cursor::Affinity;
    use crate::text::grapheme::{grapheme_width, graphemes};
    use crate::view::layout::{MEASURE, compute_layout};
    use crate::view::screen::TitleMap;
    use crate::view::viewport::{ViewTop, Viewport};
    use crate::view::widgets::title_placement;
    use proptest::prelude::*;
    use ratatui::layout::Rect;

    fn ev(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    const DOWN: MouseEventKind = MouseEventKind::Down(MouseButton::Left);
    const DRAG: MouseEventKind = MouseEventKind::Drag(MouseButton::Left);
    const TITLE: &str = "标题";

    /// Build the ScreenMap the way rendering does: 40×12, title at y = 1, body at x = 2, y = 3, 36 wide, 6 tall, status bar at y = 10.
    fn screen(vp: &Viewport, title: &str, title_cursor: Option<u16>) -> ScreenMap {
        let layout = compute_layout(Rect::new(0, 0, 40, 12), MEASURE);
        assert_eq!(
            (layout.body, layout.title.map(|r| r.y)),
            (Rect::new(2, 3, 36, 6), Some(1))
        );
        let rect = layout.title.unwrap();
        let (pad, scroll) = title_placement(title, title_cursor, rect.width, 4);
        ScreenMap::new(layout.body, vp, Some(TitleMap { rect, pad, scroll }))
    }

    /// Line 0 has 20 CJK characters: 18 on visual row 0, 2 on visual row 1. Line 1 ends in whitespace, and its last space hangs
    fn doc() -> (Rope, WrapCache) {
        let text = format!("{}\n{}   tail\nend", "汉".repeat(20), "a".repeat(34));
        (Rope::from_str(&text), WrapCache::new(36, 4))
    }

    fn click(vp: &Viewport, x: u16, y: u16) -> Option<Action> {
        let (rope, mut wrap) = doc();
        let map = screen(vp, TITLE, None);
        map_mouse(
            &ev(DOWN, x, y),
            &map,
            &mut wrap,
            &rope,
            TITLE,
            &mut None,
            Instant::now(),
        )
    }

    fn body(i: usize) -> Option<Action> {
        Some(Action::MoveTo(Target::Body(i), false))
    }

    #[test]
    fn click_table() {
        let vp = Viewport::default();
        let cases = [
            ((2, 3), body(0)),        // start of the first body line
            ((4, 3), body(1)),        // left half of a wide char
            ((5, 3), body(1)),        // right half of a wide char → snaps left
            ((37, 3), body(17)),      // last cell of visual row 0
            ((2, 4), body(18)),       // second visual row of a soft wrap
            ((20, 4), body(20)),      // blank space right of the line end → line end
            ((37, 5), body(21 + 35)), // trailing whitespace on line 1 (the third space hangs)
            ((5, 7), body(63 + 3)),   // end of the last line, "end"
            ((5, 8), body(66)), // below the end of the document (inside the area) → last visual row
            ((0, 3), None),     // left margin
            ((38, 3), None),    // right margin
            ((10, 10), None),   // status bar
            ((10, 0), None),    // top margin
        ];
        for ((x, y), want) in cases {
            assert_eq!(click(&vp, x, y), want, "({x},{y})");
        }
    }

    #[test]
    fn click_with_viewport_offset_and_typewriter_padding() {
        let vp = Viewport {
            top: ViewTop { line: 0, row: 1 },
            ..Viewport::default()
        };
        assert_eq!(click(&vp, 2, 3), body(18));
        assert_eq!(click(&vp, 2, 4), body(21));
        let vp = Viewport {
            top_padding: 2,
            ..Viewport::default()
        };
        assert_eq!(
            click(&vp, 4, 3),
            body(1),
            "top padding maps to the first document line"
        );
        assert_eq!(click(&vp, 4, 5), body(1));
        assert_eq!(click(&vp, 4, 6), body(19));
    }

    #[test]
    fn click_title_accounts_for_centering() {
        // "标题" is 4 columns wide and the body 36 → 16 columns of padding, so the title starts at x = 18
        let vp = Viewport::default();
        let title = |i| Some(Action::MoveTo(Target::Title(i), false));
        assert_eq!(click(&vp, 18, 1), title(0));
        assert_eq!(click(&vp, 19, 1), title(0));
        assert_eq!(click(&vp, 20, 1), title(1));
        assert_eq!(click(&vp, 30, 1), title(2), "right of the title → end");
        assert_eq!(click(&vp, 3, 1), title(0), "left of the title → start");
    }

    #[test]
    fn click_title_while_horizontally_scrolled() {
        // A 60-letter title is wider than 36 columns: with focus on the title and the cursor at the end,
        // rendering scrolls it by 60 + 1 - 36 = 25 columns
        let (rope, mut wrap) = doc();
        let long: String = ('a'..='z').cycle().take(60).collect();
        let map = screen(&Viewport::default(), &long, Some(60));
        assert_eq!(map.title.map(|t| (t.pad, t.scroll)), Some((0, 25)));
        let mut hit = |x| {
            map_mouse(
                &ev(DOWN, x, 1),
                &map,
                &mut wrap,
                &rope,
                &long,
                &mut None,
                Instant::now(),
            )
        };
        // Screen column 0 (x = 2) shows letter 25
        assert_eq!(hit(2), Some(Action::MoveTo(Target::Title(25), false)));
        assert_eq!(hit(12), Some(Action::MoveTo(Target::Title(35), false)));
        assert_eq!(hit(36), Some(Action::MoveTo(Target::Title(59), false)));
        assert_eq!(
            hit(37),
            Some(Action::MoveTo(Target::Title(60), false)),
            "last cell: cursor position past the end"
        );
        // Rendering places the title cursor with the same mapping
        assert_eq!(map.title_screen(60), Some(Position::new(37, 1)));
    }

    #[test]
    fn drag_extends_and_schedules_autoscroll() {
        let (rope, mut wrap) = doc();
        let map = screen(&Viewport::default(), "", None);
        let t0 = Instant::now();
        let mut drag = None;
        let mut send = |kind, x, y, drag: &mut Option<DragState>| {
            map_mouse(&ev(kind, x, y), &map, &mut wrap, &rope, "", drag, t0)
        };
        // No press inside the body: the drag is ignored
        assert_eq!(send(DRAG, 5, 4, &mut drag), None);
        send(DOWN, 2, 3, &mut drag);
        assert_eq!(
            send(DRAG, 2, 4, &mut drag),
            Some(Action::MoveTo(Target::Body(18), true))
        );
        assert_eq!(drag.unwrap().deadline, None);
        // Drag below the body: target the scrolloff boundary row (body height 6 → margin 2 → row 4, "tail")
        // and set a 50 ms deadline
        assert_eq!(
            send(DRAG, 2, 11, &mut drag),
            Some(Action::MoveTo(Target::Body(21 + 37), true))
        );
        assert_eq!(drag.unwrap().deadline, Some(t0 + DRAG_SCROLL_INTERVAL));
        // Back inside the viewport: the deadline clears. Release: the drag ends
        send(DRAG, 2, 4, &mut drag);
        assert_eq!(drag.unwrap().deadline, None);
        assert_eq!(
            send(MouseEventKind::Up(MouseButton::Left), 2, 4, &mut drag),
            None
        );
        assert_eq!(drag, None);
        // Wheel
        assert_eq!(
            send(MouseEventKind::ScrollUp, 9, 9, &mut drag),
            Some(Action::ScrollView(-3))
        );
        assert_eq!(
            send(MouseEventKind::ScrollDown, 9, 9, &mut drag),
            Some(Action::ScrollView(3))
        );
    }

    #[test]
    fn modifiers_and_other_buttons_are_ignored() {
        let (rope, mut wrap) = doc();
        let map = screen(&Viewport::default(), "", None);
        let mut shift = ev(DOWN, 2, 3);
        shift.modifiers = KeyModifiers::SHIFT;
        let right = ev(MouseEventKind::Down(MouseButton::Right), 2, 3);
        for e in [shift, right] {
            assert_eq!(
                map_mouse(&e, &map, &mut wrap, &rope, "", &mut None, Instant::now()),
                None
            );
        }
    }

    proptest! {
        /// Any click within the body's content maps to an index on the same visual row, off by less than the width of the grapheme there.
        #[test]
        fn click_round_trips_to_same_row(
            lines in prop::collection::vec("[a 中文，e\u{301}\t]{0,50}", 1..6),
            x in 0u16..36,
            y in 0u16..6,
            tab in 1u16..9,
        ) {
            let rope = Rope::from_str(&lines.join("\n"));
            let mut wrap = WrapCache::new(36, tab);
            let map = screen(&Viewport::default(), "", None);
            let (bx, by) = (map.body.x + x, map.body.y + y);
            let Some(Action::MoveTo(Target::Body(at), false)) =
                map_mouse(&ev(DOWN, bx, by), &map, &mut wrap, &rope, "", &mut None, Instant::now())
            else {
                return Err(TestCaseError::fail("a click inside the body must yield MoveTo"));
            };
            let (line, row) = wrap.step_rows(&rope, (0, 0), y as isize);
            if wrap.rows_between(&rope, (0, 0), (line, row)) != y as isize {
                return Ok(()); // click is below the end of the document
            }
            let r = wrap.rows(&rope, line)[row];
            if x >= r.width {
                return Ok(()); // click is right of the line end
            }
            let pos = wrap.char_to_visual(&rope, at, Affinity::Downstream);
            prop_assert_eq!((pos.line, pos.row), (line, row));
            let g = graphemes(&crate::text::wrap::line_text(&rope, line))
                .find(|&(c, _)| rope.line_to_char(line) + c == at)
                .map_or(1, |(_, g)| grapheme_width(g, pos.col, tab));
            prop_assert!(x >= pos.col && x - pos.col < g, "x={} col={} w={}", x, pos.col, g);
        }
    }
}
