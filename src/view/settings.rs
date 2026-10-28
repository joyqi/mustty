//! 设置面板的渲染（方案 §8.6）：只占正文区，不画边框、不设背景。
//! 标签靠左、值靠右；选中行整行反显（与文本选区同一种样式）；底部一行说明。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Widget;
use unicode_width::UnicodeWidthStr;

use crate::config::{Config, PartialConfig};
use crate::i18n::t;
use crate::settings::{Field, Kind, Value, color_name};
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
    pub cli: &'a PartialConfig,
    pub selected: usize,
    /// 第一行显示的 `Field::ALL` 下标
    pub scroll: usize,
}

/// 一项的值，拆成带样式的片段。
fn value_spans(field: Field, config: &Config, from_cli: bool) -> Vec<(String, Style)> {
    let plain = Style::default();
    let dim = Style::from(Modifier::DIM);
    let mut spans = match (field.get(config), field.kind()) {
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
        (Value::Int(i), _) => vec![(i.to_string(), plain)],
    };
    if from_cli {
        spans.push((format!(" {}", t("settings-from-cli")), dim));
    }
    spans
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
            let spans = value_spans(field, self.config, field.from_cli(self.cli));
            let value_w: u16 = spans.iter().map(|(s, _)| s.width() as u16).sum();
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
        assert_eq!(list_rows(20), (10, Some(11)));
        assert_eq!(list_rows(12), (10, Some(11)));
        assert_eq!(list_rows(11), (10, Some(10)));
        assert_eq!(list_rows(6), (5, Some(5)));
        assert_eq!(list_rows(1), (1, None));
    }
}
