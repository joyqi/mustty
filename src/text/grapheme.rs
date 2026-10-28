//! grapheme cluster 边界与显示宽度。所有偏移均为行内 char 偏移。

use std::sync::atomic::{AtomicU16, Ordering};

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Tab 的显示宽度。换行与渲染必须用同一个值，所以是进程级设置，启动时按配置设一次。
static TAB_WIDTH: AtomicU16 = AtomicU16::new(4);

pub fn set_tab_width(width: u16) {
    TAB_WIDTH.store(width.max(1), Ordering::Relaxed);
}

pub fn tab_width() -> u16 {
    TAB_WIDTH.load(Ordering::Relaxed)
}

/// 依次产出 `(char 起点, grapheme)`。
pub fn graphemes(s: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut chars = 0;
    s.graphemes(true).map(move |g| {
        let start = chars;
        chars += g.chars().count();
        (start, g)
    })
}

/// `char_idx` 之前最近的 grapheme 边界；已在行首则返回 0。
pub fn prev_boundary(s: &str, char_idx: usize) -> usize {
    graphemes(s)
        .map(|(start, _)| start)
        .take_while(|&start| start < char_idx)
        .last()
        .unwrap_or(0)
}

/// `char_idx` 之后最近的 grapheme 边界；已在行尾则返回行的 char 长度。
pub fn next_boundary(s: &str, char_idx: usize) -> usize {
    graphemes(s)
        .map(|(start, g)| start + g.chars().count())
        .find(|&end| end > char_idx)
        .unwrap_or_else(|| s.chars().count())
}

/// grapheme 在第 `col` 列开始时的显示宽度：Tab 对齐到 `tab_width()`，其余至少 1。
pub fn grapheme_width(g: &str, col: u16) -> u16 {
    if g == "\t" {
        let tab = tab_width();
        tab - col % tab
    } else {
        (g.width() as u16).max(1)
    }
}

/// 软换行时可以悬挂在行尾的空白。
pub fn is_whitespace(g: &str) -> bool {
    g.chars().all(char::is_whitespace)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundaries_step_whole_clusters() {
        let s = "ae\u{301}👨\u{200d}👩\u{200d}👧🇨🇳中";
        let mut stops = vec![0];
        while let Some(&last) = stops.last() {
            let next = next_boundary(s, last);
            if next == last {
                break;
            }
            stops.push(next);
        }
        assert_eq!(stops, vec![0, 1, 3, 8, 10, 11]);
        for w in stops.windows(2) {
            assert_eq!(prev_boundary(s, w[1]), w[0]);
        }
        assert_eq!(prev_boundary(s, 0), 0);
    }

    #[test]
    fn widths() {
        assert_eq!(grapheme_width("a", 0), 1);
        assert_eq!(grapheme_width("中", 0), 2);
        assert_eq!(grapheme_width("\t", 0), 4);
        assert_eq!(grapheme_width("\t", 3), 1);
        assert_eq!(grapheme_width("\u{200b}", 0), 1);
    }
}
