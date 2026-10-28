//! 标题 / 正文 / status bar 三个 Widget，直接写 Buffer，不画边框、不设背景。

use std::ops::Range;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Widget;
use ropey::Rope;
use unicode_width::UnicodeWidthStr;

use crate::markdown::{HighlightCache, StyledRange};
use crate::text::grapheme::{grapheme_width, graphemes};
use crate::text::wrap::{WrapCache, line_text};
use crate::view::theme;
use crate::view::viewport::ViewTop;

/// 按 mustty 的宽度规则逐 grapheme 写入一行：跳过前 `scroll` 列，超出 `max_w` 的部分裁剪。
fn draw_text(buf: &mut Buffer, x: u16, y: u16, max_w: u16, text: &str, style: Style, scroll: u16) {
    draw_styled(buf, x, y, max_w, text, scroll, |_| style);
}

/// 同 `draw_text`，样式按 grapheme 的行内 char 偏移决定。
fn draw_styled(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    max_w: u16,
    text: &str,
    scroll: u16,
    style_at: impl Fn(usize) -> Style,
) {
    let mut col = 0;
    for (c, g) in graphemes(text) {
        let w = grapheme_width(g, col);
        if col >= scroll {
            let at = col - scroll;
            if at + w > max_w {
                break;
            }
            let style = style_at(c);
            // Tab、控制字符、零宽字符 ratatui 不会占格，用空格补齐以保持列对齐
            if g == "\t" || g.contains(char::is_control) || g.width() == 0 {
                for i in 0..w {
                    buf[(x + at + i, y)].set_symbol(" ").set_style(style);
                }
            } else {
                buf.set_stringn(x + at, y, g, w as usize, style);
            }
        }
        col += w;
    }
}

pub fn text_width(text: &str) -> u16 {
    graphemes(text).fold(0, |col, (_, g)| col + grapheme_width(g, col))
}

/// 单行输入在宽度 `w` 内的水平滚动量：放得下（含行尾光标）时为 0，否则让光标尽量居中。
pub fn hscroll(text_w: u16, cursor_col: u16, w: u16) -> u16 {
    if text_w < w {
        0
    } else {
        cursor_col.saturating_sub(w / 2).min(text_w + 1 - w)
    }
}

pub struct TitleWidget<'a> {
    pub text: &'a str,
    pub style: Style,
    /// 焦点在标题时的光标列。
    pub cursor_col: Option<u16>,
}

impl TitleWidget<'_> {
    /// `(左侧留白, 水平滚动量)`：放得下时居中，否则以光标为中心水平滚动。
    pub fn placement(&self, w: u16) -> (u16, u16) {
        let tw = text_width(self.text);
        let pad = if tw < w { (w - tw) / 2 } else { 0 };
        (pad, hscroll(tw, self.cursor_col.unwrap_or(0), w))
    }
}

impl Widget for TitleWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let (pad, scroll) = self.placement(area.width);
        draw_text(
            buf,
            area.x + pad,
            area.y,
            area.width - pad,
            self.text,
            self.style,
            scroll,
        );
    }
}

pub struct BodyWidget<'a> {
    pub rope: &'a Rope,
    pub wrap: &'a mut WrapCache,
    pub highlight: &'a mut HighlightCache,
    pub top: ViewTop,
    /// 全局 char 区间
    pub selection: Option<Range<usize>>,
}

impl Widget for BodyWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let (mut line, mut row) = (self.top.line, self.top.row);
        let mut text = line_text(self.rope, line);
        let mut styles = line_styles(self.highlight.ranges(self.rope, line), text.chars().count());
        for y in area.top()..area.bottom() {
            let rows = self.wrap.rows(self.rope, line);
            let r = rows[row];
            let last_row = row + 1 == rows.len();
            let seg: String = text.chars().skip(r.start).take(r.end - r.start).collect();
            let line_start = self.rope.line_to_char(line);
            let sel = self.selection.clone();
            draw_styled(buf, area.x, y, area.width, &seg, 0, |c| {
                let local = r.start + c;
                let style = styles[local];
                match &sel {
                    Some(s) if s.contains(&(line_start + local)) => style.patch(theme::selection()),
                    _ => style,
                }
            });
            if !last_row {
                row += 1;
            } else if line + 1 < self.rope.len_lines() {
                (line, row) = (line + 1, 0);
                text = line_text(self.rope, line);
                styles = line_styles(self.highlight.ranges(self.rope, line), text.chars().count());
            } else {
                break;
            }
        }
    }
}

/// 把一行的样式区间按顺序叠加成每个 char 的样式（外层在前，内层覆盖）。
fn line_styles(ranges: &[StyledRange], len: usize) -> Vec<Style> {
    let mut styles = vec![Style::default(); len];
    for r in ranges {
        for s in &mut styles[r.start.min(len)..r.end.min(len)] {
            *s = s.patch(theme::style(r.token));
        }
    }
    styles
}

pub struct StatusWidget<'a> {
    /// 文件名 + 修改标记，或临时消息
    pub left: &'a str,
    pub left_style: Style,
    /// `1,284 字`
    pub words: &'a str,
    /// `行:列` 或 `标题`
    pub position: &'a str,
}

impl Widget for StatusWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let dim = Style::from(Modifier::DIM);
        let left_w = text_width(self.left);
        // 宽度不足时先省略位置，再省略字数，最后从左截断左段
        let full = format!("{} · {}", self.words, self.position);
        let right = [full.as_str(), self.words]
            .into_iter()
            .find(|r| left_w + 1 + text_width(r) <= area.width);
        match right {
            Some(right) => {
                let right_w = text_width(right);
                draw_text(buf, area.x, area.y, left_w, self.left, self.left_style, 0);
                draw_text(buf, area.right() - right_w, area.y, right_w, right, dim, 0);
            }
            None => {
                let left = truncate_left(self.left, area.width);
                draw_text(buf, area.x, area.y, area.width, &left, self.left_style, 0);
            }
        }
    }
}

/// status bar 整行变为「保存为: 」输入。
pub struct PromptWidget<'a> {
    pub label: &'a str,
    pub input: &'a str,
    pub cursor_col: u16,
}

impl PromptWidget<'_> {
    /// 光标相对 status bar 左端的列。
    pub fn cursor_x(&self, w: u16) -> u16 {
        let label_w = text_width(self.label).min(w);
        let avail = w - label_w;
        label_w + self.cursor_col - hscroll(text_width(self.input), self.cursor_col, avail)
    }
}

impl Widget for PromptWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let label_w = text_width(self.label).min(area.width);
        let avail = area.width - label_w;
        let scroll = hscroll(text_width(self.input), self.cursor_col, avail);
        let style = Style::default();
        draw_text(buf, area.x, area.y, label_w, self.label, style, 0);
        draw_text(
            buf,
            area.x + label_w,
            area.y,
            avail,
            self.input,
            style,
            scroll,
        );
    }
}

/// 从左侧截断为 `…tes.md`，使显示宽度不超过 `max_w`。
fn truncate_left(text: &str, max_w: u16) -> String {
    if text_width(text) <= max_w {
        return text.to_owned();
    }
    let mut kept = Vec::new();
    let mut w = 1; // "…"
    for (_, g) in graphemes(text).collect::<Vec<_>>().into_iter().rev() {
        let gw = grapheme_width(g, 0);
        if w + gw > max_w {
            break;
        }
        w += gw;
        kept.push(g);
    }
    std::iter::once("…").chain(kept.into_iter().rev()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_from_left() {
        assert_eq!(truncate_left("notes.md", 8), "notes.md");
        assert_eq!(truncate_left("notes.md", 7), "…tes.md");
        assert_eq!(truncate_left("中文名.md", 6), "…名.md");
        assert_eq!(truncate_left("long-name.md •", 8), "…me.md •");
    }

    #[test]
    fn title_centers_then_scrolls() {
        let t = |text, cursor_col| TitleWidget {
            text,
            style: Style::default(),
            cursor_col,
        };
        assert_eq!(t("标题", None).placement(10), (3, 0));
        // 放不下：以光标为中心滚动，且不超过「文本宽 + 行尾光标」
        assert_eq!(t("abcdefghijklmnop", Some(12)).placement(10), (0, 7));
        assert_eq!(t("abcdefghijklmnop", Some(16)).placement(10), (0, 7));
        assert_eq!(t("abcdefghijklmnop", Some(2)).placement(10), (0, 0));
    }
}
