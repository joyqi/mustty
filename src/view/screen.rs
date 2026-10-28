//! 渲染层输出的「屏幕坐标 ↔ 文档坐标」换算。渲染用它摆放标题文字与光标，鼠标只用它查表，
//! 两边不再各算一遍居中、留白与水平滚动。

use ratatui::layout::{Position, Rect};
use ropey::Rope;

use crate::text::grapheme::{grapheme_width, graphemes};
use crate::text::wrap::{VisualPos, WrapCache};
use crate::view::viewport::{ViewTop, Viewport};

/// 标题行的摆放：文字从 `rect.x + pad` 开始画，跳过前 `scroll` 列。
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
    /// 实际生效的 scrolloff：拖出正文上下边缘时，取这条边界上的行
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

    /// 视口在两次渲染之间滚动（拖拽自动滚动）后同步，供同一批事件继续查表。
    pub fn follow(&mut self, vp: &Viewport) {
        self.top = vp.top;
        self.top_padding = vp.top_padding;
    }

    /// 正文视觉位置 → 屏幕坐标；不在视口内为 None。写满一行的行尾、悬挂空白之后，
    /// 光标放在正文右侧第一格（不超过 `right_limit`），不与最后一个字重叠。
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

    /// 屏幕坐标 → 正文 char 索引；正文区域之外为 None。
    pub fn body_char(&self, p: Position, wrap: &mut WrapCache, rope: &Rope) -> Option<usize> {
        self.body
            .contains(p)
            .then(|| self.body_char_at(p, wrap, rope))
    }

    /// 正文区域内（调用方保证）的坐标 → char 索引。宽字符右半格吸附左侧；
    /// 上方留白映射到文档首行，文档末尾以下映射到最后一个视觉行；超出行尾取行尾。
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

    /// 拖拽位置 → char 索引：钳制到正文区域；拖出上下边缘时取 scrolloff 边界上的那一行，
    /// 这样视口只按自动滚动的节奏移动，不会再被 scrolloff 额外推动。
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

    /// 是否拖出了正文上下边缘（需要自动滚动），以及方向。
    pub fn drag_overflow(&self, p: Position) -> Option<isize> {
        if p.y < self.body.y {
            Some(-1)
        } else if p.y >= self.body.bottom() {
            Some(1)
        } else {
            None
        }
    }

    /// 标题文本第 `col` 列 → 屏幕坐标（渲染放标题光标用）。
    pub fn title_screen(&self, col: u16) -> Option<Position> {
        let t = self.title?;
        Some(Position::new(
            t.rect.x + t.pad + col.saturating_sub(t.scroll),
            t.rect.y,
        ))
    }

    /// 屏幕坐标 → 标题 char 索引：扣除居中留白、加上水平滚动量；宽字符右半格吸附左侧，
    /// 文字左侧取开头、右侧取末尾。不在标题行为 None。
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
        // 正文区域 x = 4, y = 2，宽 6：第 0 行「一二三」|「四a」，第 1 行 "hello"
        let rope = Rope::from_str("一二三四a\nhello\n");
        let mut wrap = WrapCache::new(6, 4);
        let body = Rect::new(4, 2, 6, 5);
        let mut m = map(body, ViewTop::default(), 0, None);
        let mut hit = |m: &ScreenMap, x, y| m.body_char(Position::new(x, y), &mut wrap, &rope);
        assert_eq!(hit(&m, 4, 2), Some(0));
        // 宽字符：左半格与右半格都落在「二」上
        assert_eq!(hit(&m, 6, 2), Some(1));
        assert_eq!(hit(&m, 7, 2), Some(1));
        // 软换行后的第二个视觉行
        assert_eq!(hit(&m, 4, 3), Some(3));
        assert_eq!(hit(&m, 6, 3), Some(4));
        // 超出行尾：逻辑行的末行取行尾
        assert_eq!(hit(&m, 9, 3), Some(5));
        assert_eq!(hit(&m, 9, 4), Some(11));
        // 文档末尾以下（区域内）：最后一个视觉行（末尾空行）
        assert_eq!(hit(&m, 5, 6), Some(12));
        // 正文区域之外
        for (x, y) in [(3, 2), (10, 2), (4, 1), (4, 7)] {
            assert_eq!(hit(&m, x, y), None, "({x},{y})");
        }
        // 视口偏移：顶部是第 0 行的第 1 个视觉行
        m.top = ViewTop { line: 0, row: 1 };
        assert_eq!(hit(&m, 4, 2), Some(3));
        assert_eq!(hit(&m, 5, 3), Some(7));
        // Typewriter 上方留白：映射到文档首行
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
        // 视口之上的行不在屏幕上（上方留白只出现在文档开头，这里不留白）
        let m = map(Rect::new(4, 2, 6, 5), ViewTop { line: 0, row: 1 }, 0, None);
        let pos = wrap.char_to_visual(&rope, 0, Affinity::Downstream);
        assert_eq!(m.body_screen(pos, &mut wrap, &rope, 80), None);
    }

    #[test]
    fn title_click_with_centering_and_horizontal_scroll() {
        let rect = Rect::new(2, 1, 10, 1);
        // 居中：「标题」宽 4，留白 3，文字从 x = 5 开始
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
        assert_eq!(t(&m, 6, "标题"), Some(0), "右半格吸附左侧");
        assert_eq!(t(&m, 7, "标题"), Some(1));
        assert_eq!(t(&m, 3, "标题"), Some(0), "文字左侧 → 开头");
        assert_eq!(t(&m, 11, "标题"), Some(2), "文字右侧 → 末尾");
        assert_eq!(
            m.title_char(Position::new(5, 2), "标题", 4),
            None,
            "不在标题行"
        );
        // 超宽标题水平滚动了 7 列：屏幕第 0 列对应文字第 7 列
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
