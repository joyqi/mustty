//! 标题 / 正文 / status bar 三个 Widget，直接写 Buffer，不画边框、不设背景。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Widget;
use ropey::Rope;
use unicode_width::UnicodeWidthStr;

use crate::editor::document::TitleField;
use crate::text::grapheme::{grapheme_width, graphemes};
use crate::text::wrap::{WrapCache, line_text};
use crate::view::viewport::ViewTop;

/// 按 mustty 的宽度规则逐 grapheme 写入一行，超出 `max_w` 的部分裁剪；返回写入宽度。
fn draw_text(buf: &mut Buffer, x: u16, y: u16, max_w: u16, text: &str, style: Style) -> u16 {
    let mut col = 0;
    for (_, g) in graphemes(text) {
        let w = grapheme_width(g, col);
        if col + w > max_w {
            break;
        }
        // Tab、控制字符、零宽字符 ratatui 不会占格，用空格补齐以保持列对齐
        if g == "\t" || g.contains(char::is_control) || g.width() == 0 {
            for i in 0..w {
                buf[(x + col + i, y)].set_symbol(" ").set_style(style);
            }
        } else {
            buf.set_stringn(x + col, y, g, w as usize, style);
        }
        col += w;
    }
    col
}

pub fn text_width(text: &str) -> u16 {
    graphemes(text).fold(0, |col, (_, g)| col + grapheme_width(g, col))
}

pub struct TitleWidget<'a> {
    pub title: &'a TitleField,
    /// Unbound 时显示的占位（文件名去扩展名）。
    pub placeholder: &'a str,
}

impl Widget for TitleWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let (text, style) = match self.title {
            TitleField::Bound { text, .. } => (text.as_str(), Modifier::BOLD.into()),
            TitleField::Unbound => (self.placeholder, Modifier::DIM.into()),
        };
        let x = area.x + area.width.saturating_sub(text_width(text)) / 2;
        draw_text(buf, x, area.y, area.right() - x, text, style);
    }
}

pub struct BodyWidget<'a> {
    pub rope: &'a Rope,
    pub wrap: &'a mut WrapCache,
    pub top: ViewTop,
}

impl Widget for BodyWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let (mut line, mut row) = (self.top.line, self.top.row);
        let mut text = line_text(self.rope, line);
        for y in area.top()..area.bottom() {
            let rows = self.wrap.rows(self.rope, line);
            let r = rows[row];
            let last_row = row + 1 == rows.len();
            let seg: String = text.chars().skip(r.start).take(r.end - r.start).collect();
            draw_text(buf, area.x, y, area.width, &seg, Style::default());
            if !last_row {
                row += 1;
            } else if line + 1 < self.rope.len_lines() {
                (line, row) = (line + 1, 0);
                text = line_text(self.rope, line);
            } else {
                break;
            }
        }
    }
}

pub struct StatusWidget<'a> {
    pub file_name: &'a str,
    /// `行:列`
    pub position: &'a str,
}

impl Widget for StatusWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let style = Style::from(Modifier::DIM);
        let (name_w, pos_w) = (text_width(self.file_name), text_width(self.position));
        if name_w + 1 + pos_w <= area.width {
            draw_text(buf, area.x, area.y, name_w, self.file_name, style);
            draw_text(
                buf,
                area.right() - pos_w,
                area.y,
                pos_w,
                self.position,
                style,
            );
        } else {
            let name = truncate_left(self.file_name, area.width);
            draw_text(buf, area.x, area.y, area.width, &name, style);
        }
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
    }
}
