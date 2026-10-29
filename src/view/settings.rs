//! Settings panel rendering: it occupies only the body area, with no border and no background.
//! Labels sit on the left and values on the right. The selected row is reversed across its full
//! width (the same style as a text selection), with a help line at the bottom.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Widget;
use unicode_width::UnicodeWidthStr;

use crate::config::Config;
use crate::i18n::{self, t};
use crate::settings::{Field, Kind, Value, color_name, cursor_name};
use crate::view::theme;

/// For a body area `height` rows tall: the rows available to the list, and the help line's position
/// (relative to the top of the body area). When everything fits: the full list, a blank row, then
/// the help line. Otherwise the help line is pinned to the last row and the list scrolls.
pub fn list_rows(height: u16) -> (usize, Option<u16>) {
    let n = Field::ALL.len() as u16;
    match height {
        0 => (0, None),
        1 => (1, None),
        h if h > n + 1 => (n as usize, Some(n + 1)),
        h => ((h - 1) as usize, Some(h - 1)),
    }
}

pub struct SettingsWidget<'a> {
    pub config: &'a Config,
    pub selected: usize,
    /// Index into `Field::ALL` of the first visible row
    pub scroll: usize,
}

/// A field's value, split into styled spans.
fn value_spans(field: Field, config: &Config) -> Vec<(String, Style)> {
    let plain = Style::default();
    let dim = Style::from(Modifier::DIM);
    match (field.get(config), field.kind()) {
        (Value::Bool(true), _) => vec![(t("settings-on").to_owned(), plain)],
        (Value::Bool(false), _) => vec![(t("settings-off").to_owned(), dim)],
        (Value::Int(i), Kind::Int { unit, .. }) => {
            vec![(format!("‹ {i} ›"), plain), (format!(" {unit}"), dim)]
        }
        (Value::Color(c), _) => vec![
            ("‹ ".to_owned(), plain),
            ("■".to_owned(), Style::default().fg(c.color())),
            (format!(" {} ›", color_name(c)), plain),
        ],
        // After Auto, show the effective language in DIM. Language names are written natively, never translated
        (Value::Cursor(c), _) => vec![(format!("‹ {} ›", cursor_name(c)), plain)],
        (Value::Lang(i18n::AUTO), _) => vec![
            (format!("‹ {} ›", t("settings-lang-auto")), plain),
            (format!("  ({})", i18n::current()), dim),
        ],
        (Value::Lang(tag), _) => vec![(format!("‹ {} ›", i18n::native_name(tag)), plain)],
        (Value::Int(i), _) => vec![(i.to_string(), plain)],
    }
}

/// Display width of a field's value.
fn value_width(spans: &[(String, Style)]) -> u16 {
    spans.iter().map(|(s, _)| s.width() as u16).sum()
}

/// Hardware cursor position while the settings panel is open: a fixed column two cells from the
/// right edge, on the help line. The selected row is reversed, and a cursor on the edge of a reversed
/// area makes its shape hard to see. The help line has a normal background and its text is truncated
/// before this column, so the cursor always sits on a blank cell and its shape stays clear.
/// The column doesn't depend on the selection, so the cursor doesn't jump sideways when moving up
/// and down. A body area narrower than two columns falls back to the last column. None when there is
/// no help line (only one row left).
pub fn cursor_position(area: Rect) -> Option<(u16, u16)> {
    let (_, help_y) = list_rows(area.height);
    let dy = help_y?;
    if area.width == 0 {
        return None;
    }
    Some((cursor_column(area), area.y + dy))
}

/// Column of the preview cursor: `area.right() - 2`, or `area.right() - 1` when the area is too narrow.
fn cursor_column(area: Rect) -> u16 {
    if area.width >= 2 {
        area.right() - 2
    } else {
        area.right().saturating_sub(1)
    }
}

fn put(buf: &mut Buffer, x: u16, y: u16, max_x: u16, text: &str, style: Style) -> u16 {
    let w = max_x.saturating_sub(x) as usize;
    buf.set_stringn(x, y, text, w, style).0
}

impl Widget for SettingsWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let (rows, help_y) = list_rows(area.height);
        let right = area.right();
        for (i, &field) in Field::ALL.iter().enumerate().skip(self.scroll).take(rows) {
            let y = area.y + (i - self.scroll) as u16;
            let selected = i == self.selected;
            let patch = |s: Style| {
                if selected {
                    s.patch(theme::selection())
                } else {
                    s
                }
            };
            if selected {
                for x in area.x..right {
                    buf[(x, y)].set_symbol(" ").set_style(theme::selection());
                }
            }
            let spans = value_spans(field, self.config);
            let value_w = value_width(&spans);
            let value_x = right.saturating_sub(value_w).max(area.x);
            // Keep at least one cell between label and value
            put(
                buf,
                area.x,
                y,
                value_x.saturating_sub(1),
                field.label(self.config.vim),
                patch(Style::default()),
            );
            let mut x = value_x;
            for (text, style) in &spans {
                x = put(buf, x, y, right, text, patch(*style));
            }
        }
        if let Some(dy) = help_y {
            let y = area.y + dy;
            let field = Field::ALL[self.selected];
            // Truncate the help text before the preview cursor's column so the cursor cell stays blank
            let help_right = if area.width >= 2 {
                cursor_column(area)
            } else {
                right
            };
            let x = put(
                buf,
                area.x,
                y,
                help_right,
                field.help(self.config.vim),
                Style::from(Modifier::DIM),
            );
            // Code color: append a colored sample after the help text
            if field == Field::CodeColor {
                let sample = Style::default().fg(self.config.code_color.color());
                put(buf, x + 2, y, help_right, "`code`", sample);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_fit_or_scroll() {
        // 13 fields: when they fit, the list, a blank row, then the help line
        assert_eq!(list_rows(20), (13, Some(14)));
        assert_eq!(list_rows(15), (13, Some(14)));
        assert_eq!(list_rows(14), (13, Some(13)));
        assert_eq!(list_rows(13), (12, Some(12)));
        assert_eq!(list_rows(6), (5, Some(5)));
        assert_eq!(list_rows(1), (1, None));
    }

    /// On the help line, `x` and everything to its right are blank (not taken by help text or the sample).
    fn blank_from(buf: &Buffer, area: Rect, x: u16, y: u16) -> bool {
        (x..area.right()).all(|x| buf[(x, y)].symbol() == " ")
    }

    #[test]
    fn cursor_sits_on_the_help_line_at_a_fixed_column() {
        crate::i18n::init_for_test("en");
        let area = Rect::new(4, 2, 72, 20);
        let c = Config::default();
        let (rows, help_y) = list_rows(area.height);
        let help_y = area.y + help_y.unwrap();
        let expected = Some((area.right() - 2, help_y));
        assert_eq!(cursor_position(area), expected);
        for selected in 0..Field::ALL.len() {
            let mut buf = Buffer::empty(area);
            SettingsWidget {
                config: &c,
                selected,
                scroll: 0,
            }
            .render(area, &mut buf);
            let (x, y) = cursor_position(area).unwrap();
            // On the help line, not a list row; the selected list row is reversed across its full width
            assert_eq!(y, help_y);
            assert!(y >= area.y + rows as u16, "not on a list row");
            assert!(area.contains((x, y).into()));
            let sel_y = area.y + selected as u16;
            assert!(buf[(area.x, sel_y)].modifier.contains(Modifier::REVERSED));
            assert!(
                buf[(area.right() - 1, sel_y)]
                    .modifier
                    .contains(Modifier::REVERSED)
            );
            assert!(!buf[(x, y)].modifier.contains(Modifier::REVERSED));
            assert!(
                blank_from(&buf, area, x, y),
                "cursor cell is blank: {selected}"
            );
        }
        // Short terminal: the help line is pinned to the last row, the list scrolls, and the cursor follows the help line
        let short = Rect::new(4, 2, 72, 6);
        assert_eq!(cursor_position(short), Some((short.right() - 2, 2 + 5)));
        // With only one row there is no help line, so no cursor either (that row is the reversed selected row)
        assert_eq!(cursor_position(Rect::new(4, 2, 72, 1)), None);
    }

    #[test]
    fn cursor_stays_inside_narrow_areas() {
        crate::i18n::init_for_test("en");
        let c = Config::default();
        for width in 0..=6u16 {
            let area = Rect::new(3, 1, width, 20);
            let Some((x, y)) = cursor_position(area) else {
                assert_eq!(width, 0, "only a zero-width area has no cursor");
                continue;
            };
            assert!(
                area.contains((x, y).into()),
                "width {width} out of bounds: {x}"
            );
            let want = if width >= 2 {
                area.right() - 2
            } else {
                area.right() - 1
            };
            assert_eq!(x, want, "width {width}");
            // The help text doesn't fill the row: the cursor cell and everything right of it are blank (at width 1 the help text doesn't fit at all)
            let mut buf = Buffer::empty(area);
            SettingsWidget {
                config: &c,
                selected: 0,
                scroll: 0,
            }
            .render(area, &mut buf);
            if width >= 2 {
                assert!(blank_from(&buf, area, x, y), "width {width}");
            }
        }
    }
}
