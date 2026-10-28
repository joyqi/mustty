//! 键位帮助的渲染：只占正文区，不画边框、不设背景。
//! 两列：左列键位（不翻译），右列说明；vim 下的组标题单独一行（BOLD）。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Widget;
use unicode_width::UnicodeWidthStr;

use crate::help::Line;
use crate::i18n::t_id;

pub struct HelpWidget<'a> {
    pub lines: &'a [Line],
    /// 第一行显示的下标
    pub scroll: usize,
}

impl Widget for HelpWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // 键位列按最长的一条对齐，再留两格；最多占一半宽度，其余给说明
        let keys_w = self
            .lines
            .iter()
            .filter_map(|l| match l {
                Line::Entry(keys, _) => Some(keys.width() as u16),
                _ => None,
            })
            .max()
            .unwrap_or(0)
            .saturating_add(2)
            .min(area.width / 2);
        let rows = self
            .lines
            .iter()
            .skip(self.scroll)
            .take(area.height as usize);
        for (dy, line) in rows.enumerate() {
            let y = area.y + dy as u16;
            match line {
                Line::Blank => {}
                Line::Title(id) => {
                    let style = Style::from(Modifier::BOLD);
                    buf.set_stringn(area.x, y, t_id(id), area.width as usize, style);
                }
                Line::Entry(keys, id) => {
                    let w = keys_w.saturating_sub(1) as usize;
                    buf.set_stringn(area.x, y, keys, w, Style::default());
                    let x = area.x + keys_w;
                    let rest = area.right().saturating_sub(x) as usize;
                    buf.set_stringn(x, y, t_id(id), rest, Style::from(Modifier::DIM));
                }
            }
        }
    }
}
