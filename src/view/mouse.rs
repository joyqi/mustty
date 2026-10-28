//! 鼠标（方案 §10.5，仅 `mouse = true` 时调用）：屏幕坐标 → `Action::MoveTo` / `ScrollView`。
//! 坐标换算只查渲染层给出的 `ScreenMap`，不自己重算布局；与键盘共用 Action，不另开编辑路径。

use std::time::{Duration, Instant};

use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Position;
use ropey::Rope;

use crate::action::{Action, Target};
use crate::text::wrap::WrapCache;
use crate::view::screen::ScreenMap;

/// 拖到正文上下边缘之外时，每隔这么久滚动一行。
pub const DRAG_SCROLL_INTERVAL: Duration = Duration::from_millis(50);
/// 滚轮一格滚动的视觉行数。
pub const WHEEL_ROWS: i32 = 3;

/// 左键在正文里按下后的拖拽状态。`deadline` 只在拖出正文上下边缘时设置。
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
        // 修饰键 + 点击（终端约定 Shift 拖拽为原生选择）不处理
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

    /// 与渲染相同的方式构造 ScreenMap：40×12，标题 y = 1，正文 x = 2、y = 3、宽 36、高 6，status y = 10。
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

    /// 第 0 行 20 个汉字：第 0 视觉行 18 个，第 1 视觉行 2 个；第 1 行行尾有空白，其后一个悬挂
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
            ((2, 3), body(0)),        // 正文首行行首
            ((4, 3), body(1)),        // 宽字符左半格
            ((5, 3), body(1)),        // 宽字符右半格 → 吸附左侧
            ((37, 3), body(17)),      // 第 0 视觉行最后一格
            ((2, 4), body(18)),       // 软换行的第 2 个视觉行
            ((20, 4), body(20)),      // 行尾右侧空白 → 行尾
            ((37, 5), body(21 + 35)), // 第 1 行行尾的空白（其后的第 3 个空格悬挂）
            ((5, 7), body(63 + 3)),   // 末行 "end" 行尾
            ((5, 8), body(66)),       // 文档末尾以下（区域内）→ 最后一个视觉行
            ((0, 3), None),           // 左留白
            ((38, 3), None),          // 右留白
            ((10, 10), None),         // status bar
            ((10, 0), None),          // 上留白
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
        assert_eq!(click(&vp, 4, 3), body(1), "留白区映射到文档首行");
        assert_eq!(click(&vp, 4, 5), body(1));
        assert_eq!(click(&vp, 4, 6), body(19));
    }

    #[test]
    fn click_title_accounts_for_centering() {
        // 「标题」宽 4，正文宽 36 → 左侧留白 16，标题从 x = 18 开始
        let vp = Viewport::default();
        let title = |i| Some(Action::MoveTo(Target::Title(i), false));
        assert_eq!(click(&vp, 18, 1), title(0));
        assert_eq!(click(&vp, 19, 1), title(0));
        assert_eq!(click(&vp, 20, 1), title(1));
        assert_eq!(click(&vp, 30, 1), title(2), "标题右侧 → 末尾");
        assert_eq!(click(&vp, 3, 1), title(0), "标题左侧 → 开头");
    }

    #[test]
    fn click_title_while_horizontally_scrolled() {
        // 60 个字母的标题比 36 列宽：焦点在标题、光标在末尾时，渲染把它滚动了 60 + 1 - 36 = 25 列
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
        // 屏幕第 0 列（x = 2）显示的是第 25 个字母
        assert_eq!(hit(2), Some(Action::MoveTo(Target::Title(25), false)));
        assert_eq!(hit(12), Some(Action::MoveTo(Target::Title(35), false)));
        assert_eq!(hit(36), Some(Action::MoveTo(Target::Title(59), false)));
        assert_eq!(
            hit(37),
            Some(Action::MoveTo(Target::Title(60), false)),
            "最后一格：行尾光标位置"
        );
        // 渲染放标题光标用的是同一个换算
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
        // 没有在正文里按下：拖拽忽略
        assert_eq!(send(DRAG, 5, 4, &mut drag), None);
        send(DOWN, 2, 3, &mut drag);
        assert_eq!(
            send(DRAG, 2, 4, &mut drag),
            Some(Action::MoveTo(Target::Body(18), true))
        );
        assert_eq!(drag.unwrap().deadline, None);
        // 拖到正文下方：取 scrolloff 边界行（正文高 6 → 边距 2 → 第 4 行 "tail"），设置 50 ms deadline
        assert_eq!(
            send(DRAG, 2, 11, &mut drag),
            Some(Action::MoveTo(Target::Body(21 + 37), true))
        );
        assert_eq!(drag.unwrap().deadline, Some(t0 + DRAG_SCROLL_INTERVAL));
        // 回到视口内：清除 deadline；松开：结束拖拽
        send(DRAG, 2, 4, &mut drag);
        assert_eq!(drag.unwrap().deadline, None);
        assert_eq!(
            send(MouseEventKind::Up(MouseButton::Left), 2, 4, &mut drag),
            None
        );
        assert_eq!(drag, None);
        // 滚轮
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
        /// 正文内容范围内的任意点击：得到的索引回到同一视觉行，列偏差小于该处 grapheme 的宽度。
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
                return Err(TestCaseError::fail("正文内点击必须得到 MoveTo"));
            };
            let (line, row) = wrap.step_rows(&rope, (0, 0), y as isize);
            if wrap.rows_between(&rope, (0, 0), (line, row)) != y as isize {
                return Ok(()); // 点在文档末尾以下
            }
            let r = wrap.rows(&rope, line)[row];
            if x >= r.width {
                return Ok(()); // 行尾右侧
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
