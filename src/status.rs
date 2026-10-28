//! status bar 的状态：消息与「保存为: 」提示行（方案 §9）。

use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ropey::Rope;

use crate::text::grapheme::{graphemes, next_boundary, prev_boundary};
use crate::text::wrap::line_text;

/// 「保存为: 」单行输入。
#[derive(Debug, Default)]
pub struct Prompt {
    pub input: String,
    /// char 索引
    pub cursor: usize,
    /// 目标已存在，等待再按一次 Enter 确认覆盖；改动路径后复位
    pub confirm_overwrite: bool,
}

impl Prompt {
    pub fn label(&self) -> &'static str {
        if self.confirm_overwrite {
            "文件已存在，Enter 覆盖: "
        } else {
            "保存为: "
        }
    }

    /// 处理一个按键；返回 true 表示按下了 Enter。
    pub fn edit(&mut self, key: KeyEvent) -> bool {
        let byte = |s: &str, i: usize| s.char_indices().nth(i).map_or(s.len(), |(b, _)| b);
        let len = self.input.chars().count();
        match key.code {
            KeyCode::Enter => return true,
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                let at = byte(&self.input, self.cursor);
                self.input.insert(at, c);
                self.cursor += 1;
                self.confirm_overwrite = false;
            }
            KeyCode::Backspace if self.cursor > 0 => {
                let prev = prev_boundary(&self.input, self.cursor);
                let range = byte(&self.input, prev)..byte(&self.input, self.cursor);
                self.input.replace_range(range, "");
                self.cursor = prev;
                self.confirm_overwrite = false;
            }
            KeyCode::Delete if self.cursor < len => {
                let next = next_boundary(&self.input, self.cursor);
                let range = byte(&self.input, self.cursor)..byte(&self.input, next);
                self.input.replace_range(range, "");
                self.confirm_overwrite = false;
            }
            KeyCode::Left => self.cursor = prev_boundary(&self.input, self.cursor),
            KeyCode::Right => self.cursor = next_boundary(&self.input, self.cursor),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = len,
            _ => {}
        }
        false
    }
}

#[derive(Debug, Default)]
pub struct StatusModel {
    /// 覆盖左段，下一次按键时清除。
    pub message: Option<String>,
    pub prompt: Option<Prompt>,
}

impl StatusModel {
    /// basename；无路径时为 `Untitled`。
    pub fn file_name(path: Option<&Path>) -> String {
        match path {
            Some(p) => p.file_name().map_or_else(
                || p.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            ),
            None => "Untitled".into(),
        }
    }

    /// 行 = 逻辑行号，列 = 行内 grapheme 序号，均从 1 起。
    pub fn position(rope: &Rope, head: usize) -> String {
        let line = rope.char_to_line(head);
        let local = head - rope.line_to_char(line);
        let col = graphemes(&line_text(rope, line))
            .take_while(|&(c, _)| c < local)
            .count();
        format!("{}:{}", line + 1, col + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_counts_graphemes() {
        let rope = Rope::from_str("a\n中e\u{301}x");
        assert_eq!(StatusModel::position(&rope, 0), "1:1");
        assert_eq!(StatusModel::position(&rope, 2), "2:1");
        assert_eq!(StatusModel::position(&rope, 5), "2:3");
    }

    #[test]
    fn prompt_editing() {
        let mut p = Prompt::default();
        let k = |c| KeyEvent::new(c, KeyModifiers::NONE);
        for c in "a中.md".chars() {
            assert!(!p.edit(k(KeyCode::Char(c))));
        }
        p.edit(k(KeyCode::Home));
        p.edit(k(KeyCode::Right));
        p.edit(k(KeyCode::Delete));
        p.edit(k(KeyCode::End));
        p.edit(k(KeyCode::Backspace));
        assert_eq!((p.input.as_str(), p.cursor), ("a.m", 3));
        p.confirm_overwrite = true;
        p.edit(k(KeyCode::Char('d')));
        assert!(!p.confirm_overwrite);
        assert!(p.edit(k(KeyCode::Enter)));
    }
}
