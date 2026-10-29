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

/// 设置面板打开时硬件光标的位置：说明行上离右边界两格的固定列。
/// 选中行整行反显，光标压在反显区边缘时形状看不清；说明行是普通背景，
/// 说明文字也在这一列之前截断，光标总是落在空白处，形状清晰可辨。
/// 列固定不随选中项变，上下选时光标不会左右跳。正文区不到两列时退到最后一列；
/// 没有说明行（只剩一行高）时为 None。
pub fn cursor_position(area: Rect) -> Option<(u16, u16)> {
    let (_, help_y) = list_rows(area.height);
    let dy = help_y?;
    if area.width == 0 {
        return None;
    }
    Some((cursor_column(area), area.y + dy))
}

/// 预览光标所在列：`area.right() - 2`，窄到放不下时退到 `area.right() - 1`。
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
            // 标签与值之间至少留一格
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
            // 说明在预览光标那一列之前截断，光标格总是空白
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
            // 代码颜色：说明后面放一个着色的样例
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
        // 13 项：放得下时列表、空一行、说明
        assert_eq!(list_rows(20), (13, Some(14)));
        assert_eq!(list_rows(15), (13, Some(14)));
        assert_eq!(list_rows(14), (13, Some(13)));
        assert_eq!(list_rows(13), (12, Some(12)));
        assert_eq!(list_rows(6), (5, Some(5)));
        assert_eq!(list_rows(1), (1, None));
    }

    /// 说明行那一行里，`x` 处及右边全是空白（没有被说明文字或样例占用）。
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
            // 在说明行上，不在列表行里；列表行的选中行整行反显
            assert_eq!(y, help_y);
            assert!(y >= area.y + rows as u16, "不在列表行里");
            assert!(area.contains((x, y).into()));
            let sel_y = area.y + selected as u16;
            assert!(buf[(area.x, sel_y)].modifier.contains(Modifier::REVERSED));
            assert!(
                buf[(area.right() - 1, sel_y)]
                    .modifier
                    .contains(Modifier::REVERSED)
            );
            assert!(!buf[(x, y)].modifier.contains(Modifier::REVERSED));
            assert!(blank_from(&buf, area, x, y), "光标格是空白: {selected}");
        }
        // 短终端：说明固定在最后一行，列表滚动，光标跟着说明行
        let short = Rect::new(4, 2, 72, 6);
        assert_eq!(cursor_position(short), Some((short.right() - 2, 2 + 5)));
        // 只剩一行时没有说明行，也就不放光标（那一行是反显的选中行）
        assert_eq!(cursor_position(Rect::new(4, 2, 72, 1)), None);
    }

    #[test]
    fn cursor_stays_inside_narrow_areas() {
        crate::i18n::init_for_test("en");
        let c = Config::default();
        for width in 0..=6u16 {
            let area = Rect::new(3, 1, width, 20);
            let Some((x, y)) = cursor_position(area) else {
                assert_eq!(width, 0, "只有零宽时才没有光标");
                continue;
            };
            assert!(area.contains((x, y).into()), "宽 {width} 越界: {x}");
            let want = if width >= 2 {
                area.right() - 2
            } else {
                area.right() - 1
            };
            assert_eq!(x, want, "宽 {width}");
            // 说明文字占不满：光标那一格及右边都是空白（宽 1 时说明本身就放不下）
            let mut buf = Buffer::empty(area);
            SettingsWidget {
                config: &c,
                selected: 0,
                scroll: 0,
            }
            .render(area, &mut buf);
            if width >= 2 {
                assert!(blank_from(&buf, area, x, y), "宽 {width}");
            }
        }
    }
}
