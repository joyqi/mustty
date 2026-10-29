//! The title, body and status bar widgets. They write the Buffer directly, with no border and no background.

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

/// Write one row grapheme by grapheme using mustty's width rules: skip the first `scroll` columns and clip anything past `max_w`.
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

/// Like `draw_text`, but the style depends on each grapheme's char offset within the line.
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
            // ratatui gives tabs, control characters and zero-width characters no cells; pad with spaces to keep columns aligned
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

/// Horizontal scroll for single-line input within width `w`: 0 when it fits (including the cursor at the end), otherwise keep the cursor as close to the center as possible.
pub fn hscroll(text_w: u16, cursor_col: u16, w: u16) -> u16 {
    if text_w < w {
        0
    } else {
        cursor_col.saturating_sub(w / 2).min(text_w + 1 - w)
    }
}

/// `(left padding, horizontal scroll)` for the title within width `w`: centered when it fits,
/// otherwise scrolled to keep the cursor centered. Rendering stores this in the `ScreenMap`, so the
/// title text and mouse clicks are both mapped from the same result.
pub fn title_placement(text: &str, cursor_col: Option<u16>, w: u16, tab: u16) -> (u16, u16) {
    let tw = text_width(text, tab);
    let pad = if tw < w { (w - tw) / 2 } else { 0 };
    (pad, hscroll(tw, cursor_col.unwrap_or(0), w))
}

pub struct TitleWidget<'a> {
    pub text: &'a str,
    pub style: Style,
    /// Placement from the `ScreenMap`
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
    /// Rows of top padding in typewriter mode at the start of the document
    pub top_padding: u16,
    /// Document-wide char range
    pub selection: Option<Range<usize>>,
    /// Focus mode: logical line range of the current paragraph; lines outside it are DIM
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

/// Layer a line's style ranges in order into one style per char (outer first, inner overrides).
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
    /// vim mode indicator and its style, leftmost and followed by one space (styled with `left_style`). Kept when truncating
    pub mode: Option<(&'a str, Style)>,
    /// File name, or a transient message
    pub left: &'a str,
    pub left_style: Style,
    /// Save-state mark after the file name: text and style. Separated from the name by one space and counted in the left segment's width
    pub mark: Option<(&'a str, Style)>,
    /// `1,284 words`
    pub words: &'a str,
    /// `line:col` or `Title`
    pub position: &'a str,
    /// Startup key hint: replaces the right segment while it is shown
    pub hint: Option<&'a str>,
    pub tab_width: u16,
}

impl Widget for StatusWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let dim = Style::from(Modifier::DIM);
        let tab = self.tab_width;
        let mark_w = self.mark.map_or(0, |(m, _)| 1 + text_width(m, tab));
        let mode_w = self.mode.map_or(0, |(m, _)| text_width(m, tab) + 1);
        let left_w = mode_w + text_width(self.left, tab) + mark_w;
        // When space runs out, drop the position first, then the word count, and finally truncate the file
        // name from the left (the mark stays right of the truncated name)
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
            None => truncate_left(self.left, area.width.saturating_sub(mark_w + mode_w), tab),
        };
        if let Some((mode, style)) = self.mode {
            let w = text_width(mode, tab).min(area.width);
            draw_text(buf, area.x, area.y, w, mode, style, 0, tab);
            if w < area.width {
                draw_text(buf, area.x + w, area.y, 1, " ", self.left_style, 0, tab);
            }
        }
        let name_x = area.x + mode_w.min(area.width);
        let name_w = text_width(&left, tab).min(area.right() - name_x);
        draw_text(buf, name_x, area.y, name_w, &left, self.left_style, 0, tab);
        if let Some((mark, style)) = self.mark {
            let x = name_x + name_w + 1;
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

/// The whole status bar turns into a "Save as: " input.
pub struct PromptWidget<'a> {
    pub label: &'a str,
    pub input: &'a str,
    pub cursor_col: u16,
    pub tab_width: u16,
}

impl PromptWidget<'_> {
    /// Cursor column relative to the left end of the status bar.
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

/// Truncate from the left, as in `…tes.md`, so the display width is at most `max_w`.
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
        // Doesn't fit: scroll around the cursor, never past text width + the end-of-line cursor
        assert_eq!(t("abcdefghijklmnop", Some(12)), (0, 7));
        assert_eq!(t("abcdefghijklmnop", Some(16)), (0, 7));
        assert_eq!(t("abcdefghijklmnop", Some(2)), (0, 0));
    }
}
