//! 光标、选区锚点与 grapheme / 视觉行 / 词级移动（方案 §4.2）。

use std::ops::Range;

use ropey::Rope;

use crate::text::grapheme::{graphemes, next_boundary, prev_boundary};
use crate::text::wrap::{VisualPos, WrapCache, line_text};

/// 软换行处同一个 char 索引显示在哪一行：默认下一视觉行行首，`End` 键之后为本行行尾。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Affinity {
    Upstream,
    #[default]
    Downstream,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cursor {
    /// 全局 char 索引，永远落在 grapheme 边界。
    pub head: usize,
    /// `Some` 时 `[min(anchor, head), max)` 为选区。
    pub anchor: Option<usize>,
    /// 上下移动时保持的视觉列，水平移动 / 编辑后清空。
    pub preferred_col: Option<u16>,
    pub affinity: Affinity,
}

impl Cursor {
    /// 放到 `head`，清空选区、preferred_col 与 affinity。
    pub fn set(&mut self, head: usize) {
        *self = Cursor {
            head,
            ..Cursor::default()
        };
    }

    /// 非空选区。
    pub fn selection(&self) -> Option<Range<usize>> {
        let anchor = self.anchor.filter(|&a| a != self.head)?;
        Some(anchor.min(self.head)..anchor.max(self.head))
    }

    pub fn visual(&self, rope: &Rope, wrap: &mut WrapCache) -> VisualPos {
        wrap.char_to_visual(rope, self.head, self.affinity)
    }

    /// 前一个 grapheme 边界；行首则为上一行行尾（`\n` 之前）。
    pub fn prev(rope: &Rope, head: usize) -> usize {
        let line = rope.char_to_line(head);
        let start = rope.line_to_char(line);
        if head > start {
            start + prev_boundary(&line_text(rope, line), head - start)
        } else {
            head.saturating_sub(1)
        }
    }

    /// 后一个 grapheme 边界；行尾则为下一行行首。
    pub fn next(rope: &Rope, head: usize) -> usize {
        let line = rope.char_to_line(head);
        let start = rope.line_to_char(line);
        let text = line_text(rope, line);
        if head - start < text.chars().count() {
            start + next_boundary(&text, head - start)
        } else {
            (head + 1).min(rope.len_chars())
        }
    }

    /// ←：在软换行的下一行行首时，先移到上一视觉行行尾（同一索引，Upstream）。
    pub fn move_left(&mut self, rope: &Rope, wrap: &mut WrapCache) {
        let pos = self.visual(rope, wrap);
        let row = wrap.rows(rope, pos.line)[pos.row];
        let at_row_start = self.head == rope.line_to_char(pos.line) + row.start;
        if self.affinity == Affinity::Downstream && pos.row > 0 && at_row_start {
            self.set(self.head);
            self.affinity = Affinity::Upstream;
        } else {
            self.set(Self::prev(rope, self.head));
        }
    }

    /// →：在软换行的本行行尾（Upstream）时，先移到下一视觉行行首（同一索引）。
    pub fn move_right(&mut self, rope: &Rope, wrap: &mut WrapCache) {
        let pos = self.visual(rope, wrap);
        let rows = wrap.rows(rope, pos.line);
        let at_row_end = self.head == rope.line_to_char(pos.line) + rows[pos.row].end;
        if self.affinity == Affinity::Upstream && pos.row + 1 < rows.len() && at_row_end {
            self.set(self.head);
        } else {
            self.set(Self::next(rope, self.head));
        }
    }

    /// 按视觉行上下移动 `delta` 行，保持 `preferred_col`。
    pub fn move_vertical(&mut self, rope: &Rope, wrap: &mut WrapCache, delta: isize) {
        let pos = self.visual(rope, wrap);
        let col = *self.preferred_col.get_or_insert(pos.col);
        let (line, row) = wrap.step_rows(rope, (pos.line, pos.row), delta);
        if (line, row) != (pos.line, pos.row) {
            self.head = wrap.visual_to_char(rope, VisualPos { line, row, col });
            self.affinity = Affinity::Downstream;
        }
    }

    /// 视觉行行首。
    pub fn move_row_start(&mut self, rope: &Rope, wrap: &mut WrapCache) {
        let pos = self.visual(rope, wrap);
        let start = wrap.rows(rope, pos.line)[pos.row].start;
        self.set(rope.line_to_char(pos.line) + start);
    }

    /// 视觉行最后一个 grapheme 之后，显示在本行末尾。
    pub fn move_row_end(&mut self, rope: &Rope, wrap: &mut WrapCache) {
        let pos = self.visual(rope, wrap);
        let end = wrap.rows(rope, pos.line)[pos.row].end;
        self.set(rope.line_to_char(pos.line) + end);
        self.affinity = Affinity::Upstream;
    }

    /// 下一个词首（Ctrl+→）：跳过当前词与其后的空白；在行尾时跳到下一行第一个词首。
    pub fn word_right(rope: &Rope, head: usize) -> usize {
        let (line, start, text) = locate(rope, head);
        match Words::new(&text).right(head - start) {
            Some(p) => start + p,
            None if line + 1 < rope.len_lines() => {
                let next = rope.line_to_char(line + 1);
                next + Words::new(&line_text(rope, line + 1)).skip_space(0)
            }
            None => head,
        }
    }

    /// 上一个词首（Ctrl+←）；在行首时到上一行行尾。
    pub fn word_left(rope: &Rope, head: usize) -> usize {
        let (_, start, text) = locate(rope, head);
        match Words::new(&text).left(head - start) {
            Some(p) => start + p,
            None => head.saturating_sub(1),
        }
    }

    /// 当前或下一个词的最后一个 grapheme（vim `e`）；本行之后没有词时看下一行。
    pub fn word_end(rope: &Rope, head: usize) -> usize {
        let (line, start, text) = locate(rope, head);
        if let Some(p) = Words::new(&text).end(head - start, true) {
            return start + p;
        }
        if line + 1 < rope.len_lines() {
            let next = rope.line_to_char(line + 1);
            if let Some(p) = Words::new(&line_text(rope, line + 1)).end(0, false) {
                return next + p;
            }
        }
        head
    }
}

fn locate(rope: &Rope, head: usize) -> (usize, usize, String) {
    let line = rope.char_to_line(head);
    (line, rope.line_to_char(line), line_text(rope, line))
}

/// 词的划分：连续的同类 grapheme 为一个词。CJK 连续成一个词（与 VS Code 一致），
/// 字母数字与 `_` 成词，其他标点各自成词。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Space,
    Cjk,
    Word,
    Punct,
}

fn class(g: &str) -> Class {
    let c = g.chars().next().unwrap_or(' ');
    let cjk = matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0xAC00..=0xD7AF | 0x20000..=0x2FFFF);
    if c.is_whitespace() {
        Class::Space
    } else if cjk {
        Class::Cjk
    } else if c.is_alphanumeric() || c == '_' {
        Class::Word
    } else {
        Class::Punct
    }
}

/// 一个逻辑行内的 grapheme 与类别；位置都是行内 char 偏移。
struct Words {
    starts: Vec<usize>,
    classes: Vec<Class>,
    len: usize,
}

impl Words {
    fn new(line: &str) -> Self {
        let (starts, classes) = graphemes(line).map(|(s, g)| (s, class(g))).unzip();
        Self {
            starts,
            classes,
            len: line.chars().count(),
        }
    }

    fn pos(&self, i: usize) -> usize {
        self.starts.get(i).copied().unwrap_or(self.len)
    }

    fn index(&self, at: usize) -> usize {
        self.starts
            .iter()
            .position(|&s| s >= at)
            .unwrap_or(self.starts.len())
    }

    fn skip_space(&self, at: usize) -> usize {
        let mut i = self.index(at);
        while self.classes.get(i) == Some(&Class::Space) {
            i += 1;
        }
        self.pos(i)
    }

    fn right(&self, at: usize) -> Option<usize> {
        let mut i = self.index(at);
        let c = *self.classes.get(i)?;
        if c != Class::Space {
            while self.classes.get(i) == Some(&c) {
                i += 1;
            }
        }
        Some(self.skip_space(self.pos(i)))
    }

    fn left(&self, at: usize) -> Option<usize> {
        let mut i = self.index(at);
        if i == 0 {
            return None;
        }
        while i > 0 && self.classes[i - 1] == Class::Space {
            i -= 1;
        }
        if i > 0 {
            let c = self.classes[i - 1];
            while i > 0 && self.classes[i - 1] == c {
                i -= 1;
            }
        }
        Some(self.pos(i))
    }

    fn end(&self, at: usize, advance: bool) -> Option<usize> {
        let mut i = self.index(at) + usize::from(advance);
        while self.classes.get(i) == Some(&Class::Space) {
            i += 1;
        }
        let c = *self.classes.get(i)?;
        while self.classes.get(i + 1) == Some(&c) {
            i += 1;
        }
        Some(self.pos(i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn horizontal_crosses_lines_by_grapheme() {
        let rope = Rope::from_str("e\u{301}🇨🇳\n中");
        let mut wrap = WrapCache::new(20, 4);
        let mut c = Cursor::default();
        let mut stops = vec![];
        for _ in 0..6 {
            c.move_right(&rope, &mut wrap);
            stops.push(c.head);
        }
        assert_eq!(stops, vec![2, 4, 5, 6, 6, 6]);
        for want in [5, 4, 2, 0, 0] {
            c.move_left(&rope, &mut wrap);
            assert_eq!(c.head, want);
        }
    }

    #[test]
    fn vertical_keeps_preferred_col() {
        // 第 0 行 "abcdef"，第 1 行 "中文"，第 2 行 "abcdef"
        let rope = Rope::from_str("abcdef\n中文\nabcdef");
        let mut wrap = WrapCache::new(20, 4);
        let mut c = Cursor {
            head: 3,
            ..Cursor::default()
        };
        c.move_vertical(&rope, &mut wrap, 1);
        // 第 3 列落在「文」的第二格 → 吸附到「文」左侧
        assert_eq!(c.head, 7 + 1);
        c.move_vertical(&rope, &mut wrap, 1);
        assert_eq!(c.head, 10 + 3);
        c.move_vertical(&rope, &mut wrap, 5);
        assert_eq!(c.head, 13);
    }

    #[test]
    fn end_key_is_upstream_at_soft_wrap() {
        // 宽 6：第 0 视觉行「一二三」恰好写满，第 1 视觉行「四五」
        let rope = Rope::from_str("一二三四五");
        let mut wrap = WrapCache::new(6, 4);
        let mut c = Cursor::default();
        c.move_row_end(&rope, &mut wrap);
        assert_eq!((c.head, c.affinity), (3, Affinity::Upstream));
        let pos = c.visual(&rope, &mut wrap);
        assert_eq!((pos.row, pos.col), (0, 6));
        // 从第 0 行的 Upstream 行尾按 Home：回到第 0 行行首，而不是第 1 行行首
        c.move_row_start(&rope, &mut wrap);
        assert_eq!(c.head, 0);
        // 末行 End：行尾，无歧义
        c.set(4);
        c.move_row_end(&rope, &mut wrap);
        assert_eq!((c.head, c.visual(&rope, &mut wrap).row), (5, 1));
    }

    #[test]
    fn left_right_flip_affinity_at_soft_wrap() {
        let rope = Rope::from_str("一二三四五");
        let mut wrap = WrapCache::new(6, 4);
        let mut c = Cursor::default();
        c.move_row_end(&rope, &mut wrap); // (0, 6) Upstream
        c.move_right(&rope, &mut wrap);
        // 同一索引换到下一视觉行行首，不跳过「四」之前的位置
        assert_eq!((c.head, c.affinity), (3, Affinity::Downstream));
        assert_eq!(c.visual(&rope, &mut wrap).row, 1);
        c.move_left(&rope, &mut wrap);
        assert_eq!((c.head, c.affinity), (3, Affinity::Upstream));
        c.move_left(&rope, &mut wrap);
        assert_eq!((c.head, c.affinity), (2, Affinity::Downstream));
    }

    #[test]
    fn vertical_from_upstream_end() {
        let rope = Rope::from_str("一二三四五");
        let mut wrap = WrapCache::new(6, 4);
        let mut c = Cursor::default();
        c.move_row_end(&rope, &mut wrap);
        c.move_vertical(&rope, &mut wrap, 1);
        // preferred_col = 6 超出第 1 行（末行）→ 行尾
        assert_eq!((c.head, c.affinity), (5, Affinity::Downstream));
    }

    #[test]
    fn word_motions() {
        //            0123456789012345678
        let line = "foo_bar, 中文写作  x\nnext word";
        let rope = Rope::from_str(line);
        let walk = |f: fn(&Rope, usize) -> usize, from: usize, n: usize| {
            let mut at = from;
            (0..n)
                .map(|_| {
                    at = f(&rope, at);
                    at
                })
                .collect::<Vec<_>>()
        };
        // foo_bar|,| |中文写作|  |x|\n|next
        assert_eq!(walk(Cursor::word_right, 0, 5), vec![7, 9, 15, 16, 17]);
        assert_eq!(walk(Cursor::word_right, 16, 2), vec![17, 22]);
        assert_eq!(walk(Cursor::word_left, 22, 5), vec![17, 16, 15, 9, 7]);
        assert_eq!(walk(Cursor::word_left, 7, 2), vec![0, 0]);
        // vim e：当前或下一个词的最后一个字
        assert_eq!(walk(Cursor::word_end, 0, 5), vec![6, 7, 12, 15, 20]);
    }

    #[test]
    fn selection_range_is_ordered_and_nonempty() {
        let c = Cursor {
            head: 2,
            anchor: Some(5),
            ..Cursor::default()
        };
        assert_eq!(c.selection(), Some(2..5));
        let empty = Cursor {
            head: 2,
            anchor: Some(2),
            ..Cursor::default()
        };
        assert_eq!(empty.selection(), None);
    }
}
