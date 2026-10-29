//! 设置面板的渲染：只占正文区，不画边框、不设背景。
//! 标签靠左、值靠右；选中行整行反显（与文本选区同一种样式）；底部一行说明。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Widget;
use unicode_width::UnicodeWidthStr;

use crate::config::Config;
use crate::i18n::{self, t};
use crate::settings::{Field, Kind, Value, color_name, cursor_name};
use crate::view::theme;

/// 正文区高 `height` 行时列表可用的行数与说明行的位置（相对正文区顶端）。
/// 放得下时列表 10 行、空一行、说明；不够 11 行时说明固定在最后一行，列表滚动。
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
    /// 第一行显示的 `Field::ALL` 下标
    pub scroll: usize,
}

/// 一项的值，拆成带样式的片段。
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
        // Auto 后面用 DIM 标出实际生效的语言；语言名用各自的母语写法，不翻译
        (Value::Cursor(c), _) => vec![(format!("‹ {} ›", cursor_name(c)), plain)],
        (Value::Lang(i18n::AUTO), _) => vec![
            (format!("‹ {} ›", t("settings-lang-auto")), plain),
            (format!("  ({})", i18n::current()), dim),
        ],
        (Value::Lang(tag), _) => vec![(format!("‹ {} ›", i18n::native_name(tag)), plain)],
        (Value::Int(i), _) => vec![(i.to_string(), plain)],
    }
}

/// 一项的值占几列。
fn value_width(spans: &[(String, Style)]) -> u16 {
    spans.iter().map(|(s, _)| s.width() as u16).sum()
}

/// 选中行的值末尾（`‹ bar ›` 的 `›` 之后）：面板打开时硬件光标停在这里，上下选、左右改时一直可见，
/// 改光标形状能立刻看到效果。选中行不在可见范围内时为 None。
pub fn cursor_position(
    area: Rect,
    config: &Config,
    selected: usize,
    scroll: usize,
) -> Option<(u16, u16)> {
    let (rows, _) = list_rows(area.height);
    if selected < scroll || selected >= scroll + rows {
        return None;
    }
    let w = value_width(&value_spans(Field::ALL[selected], config));
    let x = area.right().saturating_sub(w).max(area.x) + w;
    Some((x.min(area.right()), area.y + (selected - scroll) as u16))
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
            // 标签与值之间至少留一格
            put(
                buf,
                area.x,
                y,
                value_x.saturating_sub(1),
                field.label(),
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
            let x = put(
                buf,
                area.x,
                y,
                right,
                field.help(),
                Style::from(Modifier::DIM),
            );
            // 代码颜色：说明后面放一个着色的样例
            if field == Field::CodeColor {
                let sample = Style::default().fg(self.config.code_color.color());
                put(buf, x + 2, y, right, "`code`", sample);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_fit_or_scroll() {
        // 13 项：放得下时列表、空一行、说明
        assert_eq!(list_rows(20), (13, Some(14)));
        assert_eq!(list_rows(15), (13, Some(14)));
        assert_eq!(list_rows(14), (13, Some(13)));
        assert_eq!(list_rows(13), (12, Some(12)));
        assert_eq!(list_rows(6), (5, Some(5)));
        assert_eq!(list_rows(1), (1, None));
    }

    #[test]
    fn cursor_sits_after_the_selected_value() {
        crate::i18n::init_for_test("en");
        let area = Rect::new(4, 2, 72, 20);
        let c = Config::default();
        // cursor 一项：`‹ bar ›` 右对齐，光标在 `›` 之后（正文区右边界）
        let at = Field::ALL.iter().position(|&f| f == Field::Cursor).unwrap();
        assert_eq!(
            cursor_position(area, &c, at, 0),
            Some((area.right(), 2 + at as u16))
        );
        // 滚动后按可见行算；滚出去了就没有
        assert_eq!(cursor_position(area, &c, 3, 2), Some((area.right(), 3)));
        assert_eq!(cursor_position(area, &c, 1, 2), None);
        let short = Rect::new(4, 2, 72, 4);
        assert_eq!(cursor_position(short, &c, 3, 0), None, "只有 3 行列表");
        // 值比正文区还宽时，光标停在被截断的末尾
        let narrow = Rect::new(0, 0, 4, 20);
        assert_eq!(cursor_position(narrow, &c, at, 0), Some((4, at as u16)));
    }
}
