//! 标题 / 正文 / status bar 三个 Widget，直接写 Buffer，不画边框、不设背景。

use std::ops::Range;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::Widget;
use ropey::Rope;
use unicode_width::UnicodeWidthStr;

use crate::markdown::{HighlightCache, StyledRange};
use crate::text::grapheme::{grapheme_width, graphemes};
use crate::text::wrap::{WrapCache, line_text};
use crate::view::theme;
use crate::view::viewport::ViewTop;

/// 按 mustty 的宽度规则逐 grapheme 写入一行：跳过前 `scroll` 列，超出 `max_w` 的部分裁剪。
#[allow(clippy::too_many_arguments)]
fn draw_text(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    max_w: u16,
    text: &str,
    style: Style,
    scroll: u16,
    tab: u16,
) {
    draw_styled(buf, x, y, max_w, text, scroll, tab, |_| style);
}

/// 同 `draw_text`，样式按 grapheme 的行内 char 偏移决定。
#[allow(clippy::too_many_arguments)]
fn draw_styled(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    max_w: u16,
    text: &str,
    scroll: u16,
    tab: u16,
    style_at: impl Fn(usize) -> Style,
) {
    let mut col = 0;
    for (c, g) in graphemes(text) {
        let w = grapheme_width(g, col, tab);
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

pub fn text_width(text: &str, tab: u16) -> u16 {
    graphemes(text).fold(0, |col, (_, g)| col + grapheme_width(g, col, tab))
}

/// 单行输入在宽度 `w` 内的水平滚动量：放得下（含行尾光标）时为 0，否则让光标尽量居中。
pub fn hscroll(text_w: u16, cursor_col: u16, w: u16) -> u16 {
    if text_w < w {
        0
    } else {
        cursor_col.saturating_sub(w / 2).min(text_w + 1 - w)
    }
}

/// 标题在宽度 `w` 内的 `(左侧留白, 水平滚动量)`：放得下时居中，否则以光标为中心水平滚动。
/// 渲染用它填 `ScreenMap`，标题文字与鼠标点击都按这一份结果换算。
pub fn title_placement(text: &str, cursor_col: Option<u16>, w: u16, tab: u16) -> (u16, u16) {
    let tw = text_width(text, tab);
    let pad = if tw < w { (w - tw) / 2 } else { 0 };
    (pad, hscroll(tw, cursor_col.unwrap_or(0), w))
}

pub struct TitleWidget<'a> {
    pub text: &'a str,
    pub style: Style,
    /// 来自 `ScreenMap` 的摆放
    pub pad: u16,
    pub scroll: u16,
    pub tab_width: u16,
}

impl Widget for TitleWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let pad = self.pad.min(area.width);
        draw_text(
            buf,
            area.x + pad,
            area.y,
            area.width - pad,
            self.text,
            self.style,
            self.scroll,
            self.tab_width,
        );
    }
}

pub struct BodyWidget<'a> {
    pub rope: &'a Rope,
    pub wrap: &'a mut WrapCache,
    pub highlight: &'a mut HighlightCache,
    pub top: ViewTop,
    /// Typewriter 模式在文档开头时的上方留白行数
    pub top_padding: u16,
    /// 全局 char 区间
    pub selection: Option<Range<usize>>,
    /// Focus 模式：当前段落的逻辑行区间，之外的行 DIM
    pub focus: Option<Range<usize>>,
    pub code_color: Color,
}

impl Widget for BodyWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let (mut line, mut row) = (self.top.line, self.top.row);
        let tab = self.wrap.tab_width();
        let code = self.code_color;
        let dimmed = |line: usize| self.focus.as_ref().is_some_and(|f| !f.contains(&line));
        let styles_of = |hl: &mut HighlightCache, line: usize, len: usize| {
            let mut styles = line_styles(hl.ranges(self.rope, line), len, code);
            if dimmed(line) {
                styles
                    .iter_mut()
                    .for_each(|s| *s = s.add_modifier(Modifier::DIM));
            }
            styles
        };
        let mut text = line_text(self.rope, line);
        let mut styles = styles_of(self.highlight, line, text.chars().count());
        let first = area
            .top()
            .saturating_add(self.top_padding)
            .min(area.bottom());
        for y in first..area.bottom() {
            let rows = self.wrap.rows(self.rope, line);
            let r = rows[row];
            let last_row = row + 1 == rows.len();
            let seg: String = text.chars().skip(r.start).take(r.end - r.start).collect();
            let line_start = self.rope.line_to_char(line);
            let sel = self.selection.clone();
            draw_styled(buf, area.x, y, area.width, &seg, 0, tab, |c| {
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
                styles = styles_of(self.highlight, line, text.chars().count());
            } else {
                break;
            }
        }
    }
}

/// 把一行的样式区间按顺序叠加成每个 char 的样式（外层在前，内层覆盖）。
fn line_styles(ranges: &[StyledRange], len: usize, code: Color) -> Vec<Style> {
    let mut styles = vec![Style::default(); len];
    for r in ranges {
        for s in &mut styles[r.start.min(len)..r.end.min(len)] {
            *s = s.patch(theme::style(r.token, code));
        }
    }
    styles
}

pub struct StatusWidget<'a> {
    /// 文件名（可带模式标识），或临时消息
    pub left: &'a str,
    pub left_style: Style,
    /// 文件名后的保存状态标记：文字与样式。与文件名之间隔一个空格，计入左段宽度
    pub mark: Option<(&'a str, Style)>,
    /// `1,284 字`
    pub words: &'a str,
    /// `行:列` 或 `标题`
    pub position: &'a str,
    /// 启动键位提示：在提示期内取代右段
    pub hint: Option<&'a str>,
    pub tab_width: u16,
}

impl Widget for StatusWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let dim = Style::from(Modifier::DIM);
        let tab = self.tab_width;
        let mark_w = self.mark.map_or(0, |(m, _)| 1 + text_width(m, tab));
        let left_w = text_width(self.left, tab) + mark_w;
        // 宽度不足时先省略位置，再省略字数，最后从左截断文件名（标记保留在截断后的文件名右侧）
        let full = format!("{} · {}", self.words, self.position);
        let candidates = match self.hint {
            Some(hint) => vec![hint],
            None => vec![full.as_str(), self.words],
        };
        let right = candidates
            .into_iter()
            .find(|r| left_w + 1 + text_width(r, tab) <= area.width);
        let left = match right {
            Some(right) => {
                let right_w = text_width(right, tab);
                draw_text(
                    buf,
                    area.right() - right_w,
                    area.y,
                    right_w,
                    right,
                    dim,
                    0,
                    tab,
                );
                self.left.to_owned()
            }
            None => truncate_left(self.left, area.width.saturating_sub(mark_w), tab),
        };
        let name_w = text_width(&left, tab);
        draw_text(buf, area.x, area.y, name_w, &left, self.left_style, 0, tab);
        if let Some((mark, style)) = self.mark {
            let x = area.x + name_w + 1;
            draw_text(
                buf,
                x,
                area.y,
                area.right().saturating_sub(x),
                mark,
                style,
                0,
                tab,
            );
        }
    }
}

/// status bar 整行变为「保存为: 」输入。
pub struct PromptWidget<'a> {
    pub label: &'a str,
    pub input: &'a str,
    pub cursor_col: u16,
    pub tab_width: u16,
}

impl PromptWidget<'_> {
    /// 光标相对 status bar 左端的列。
    pub fn cursor_x(&self, w: u16) -> u16 {
        let tab = self.tab_width;
        let label_w = text_width(self.label, tab).min(w);
        let avail = w - label_w;
        label_w + self.cursor_col - hscroll(text_width(self.input, tab), self.cursor_col, avail)
    }
}

impl Widget for PromptWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let tab = self.tab_width;
        let label_w = text_width(self.label, tab).min(area.width);
        let avail = area.width - label_w;
        let scroll = hscroll(text_width(self.input, tab), self.cursor_col, avail);
        let style = Style::default();
        draw_text(buf, area.x, area.y, label_w, self.label, style, 0, tab);
        draw_text(
            buf,
            area.x + label_w,
            area.y,
            avail,
            self.input,
            style,
            scroll,
            tab,
        );
    }
}

/// 从左侧截断为 `…tes.md`，使显示宽度不超过 `max_w`。
fn truncate_left(text: &str, max_w: u16, tab: u16) -> String {
    if text_width(text, tab) <= max_w {
        return text.to_owned();
    }
    let mut kept = Vec::new();
    let mut w = 1; // "…"
    for (_, g) in graphemes(text).collect::<Vec<_>>().into_iter().rev() {
        let gw = grapheme_width(g, 0, tab);
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
        assert_eq!(truncate_left("notes.md", 8, 4), "notes.md");
        assert_eq!(truncate_left("notes.md", 7, 4), "…tes.md");
        assert_eq!(truncate_left("中文名.md", 6, 4), "…名.md");
        assert_eq!(truncate_left("long-name.md •", 8, 4), "…me.md •");
    }

    #[test]
    fn title_centers_then_scrolls() {
        let t = |text, cursor_col| title_placement(text, cursor_col, 10, 4);
        assert_eq!(t("标题", None), (3, 0));
        // 放不下：以光标为中心滚动，且不超过「文本宽 + 行尾光标」
        assert_eq!(t("abcdefghijklmnop", Some(12)), (0, 7));
        assert_eq!(t("abcdefghijklmnop", Some(16)), (0, 7));
        assert_eq!(t("abcdefghijklmnop", Some(2)), (0, 0));
    }
}
