//! status bar 的状态：消息与「保存为: 」提示行。

use std::path::Path;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Color, Style};
use ropey::Rope;

use crate::editor::document::TitleField;
use crate::i18n::{t, t_args};
use crate::terminal::strip_controls;
use crate::text::count::thousands;
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
    /// 预填 `input`，光标放在末尾。
    pub fn with_input(input: String) -> Self {
        Self {
            cursor: input.chars().count(),
            input,
            confirm_overwrite: false,
        }
    }

    pub fn label(&self) -> &'static str {
        if self.confirm_overwrite {
            t("prompt-overwrite")
        } else {
            t("prompt-save-as")
        }
    }

    /// 粘贴进单行输入：换行转为空格。
    pub fn paste(&mut self, s: &str) {
        let s = s.replace("\r\n", " ").replace(['\r', '\n'], " ");
        let at = self
            .input
            .char_indices()
            .nth(self.cursor)
            .map_or(self.input.len(), |(b, _)| b);
        self.input.insert_str(at, &s);
        self.cursor += s.chars().count();
        self.confirm_overwrite = false;
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

/// 文件名后的保存状态标记。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveMark {
    /// 有未保存修改：`*`，默认前景色
    Dirty,
    /// 内容与磁盘一致（打开的已有文件未改动、手动或自动保存过）：绿色 `●`
    Saved,
}

impl SaveMark {
    /// `on_disk`：文件在磁盘上存在且记录过它的状态（打开已有文件或保存过）。
    /// 空 Untitled、尚未创建的新文件在未修改时不显示标记。
    pub fn of(dirty: bool, on_disk: bool) -> Option<Self> {
        if dirty {
            Some(Self::Dirty)
        } else if on_disk {
            Some(Self::Saved)
        } else {
            None
        }
    }

    pub fn symbol(self) -> &'static str {
        match self {
            Self::Dirty => "*",
            // East Asian Width 为 Ambiguous，按 narrow（1 列）计
            Self::Saved => "\u{25CF}",
        }
    }

    /// 都不 DIM，与 DIM 的文件名形成对比。
    pub fn style(self) -> Style {
        match self {
            Self::Dirty => Style::default(),
            Self::Saved => Style::default().fg(Color::Green),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageKind {
    /// 到点自动消失
    Info { expires: Instant },
    /// 保持到下一次按键
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub text: String,
    pub kind: MessageKind,
}

#[derive(Debug, Default)]
pub struct StatusModel {
    /// 覆盖左段
    pub message: Option<Message>,
    pub prompt: Option<Prompt>,
    /// 自动保存因外部修改被放弃；常驻提示，直到保存成功
    pub externally_modified: bool,
    /// 显示的字数（标题 + 正文）
    pub words: usize,
    /// 启动键位提示的截止时刻
    pub hint_until: Option<Instant>,
}

impl StatusModel {
    pub fn info(&mut self, text: impl Into<String>, now: Instant, ttl: Duration) {
        let kind = MessageKind::Info { expires: now + ttl };
        self.message = Some(Message {
            text: text.into(),
            kind,
        });
    }

    pub fn error(&mut self, text: impl Into<String>) {
        let kind = MessageKind::Error;
        self.message = Some(Message {
            text: text.into(),
            kind,
        });
    }

    /// 有按键时调用：错误类消息到此为止。
    pub fn on_input(&mut self) {
        if self
            .message
            .as_ref()
            .is_some_and(|m| m.kind == MessageKind::Error)
        {
            self.message = None;
        }
    }

    pub fn expiry(&self) -> Option<Instant> {
        match self.message.as_ref()?.kind {
            MessageKind::Info { expires } => Some(expires),
            MessageKind::Error => None,
        }
    }

    /// 清掉已过期的消息；返回是否有变化。
    pub fn expire(&mut self, now: Instant) -> bool {
        let expired = self.expiry().is_some_and(|t| now >= t);
        if expired {
            self.message = None;
        }
        expired
    }

    pub fn words_label(&self) -> String {
        let n = thousands(self.words);
        // 英文单复数：只有这一处，用两条 key 解决，不引入复数规则
        if self.words == 1 {
            t_args("status-words-one", &[("n", &n)])
        } else {
            t_args("status-words-other", &[("n", &n)])
        }
    }

    /// basename；无路径时为 `Untitled`（按界面语言）。
    /// 终端窗口标题：非空的 H1 标题 > 文件名（含扩展名）> `Untitled`；控制字符已去掉。
    pub fn window_title(title: &TitleField, path: Option<&Path>) -> String {
        match title {
            TitleField::Bound { text, .. } => {
                let text = strip_controls(text);
                if text.trim().is_empty() {
                    strip_controls(&Self::file_name(path))
                } else {
                    text
                }
            }
            TitleField::Unbound => strip_controls(&Self::file_name(path)),
        }
    }

    pub fn file_name(path: Option<&Path>) -> String {
        match path {
            Some(p) => p.file_name().map_or_else(
                || p.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            ),
            None => t("status-untitled").into(),
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
    fn window_title_prefers_h1_then_file_name() {
        crate::i18n::init_for_test("en");
        let h1 = |s: &str| TitleField::Bound {
            text: s.into(),
            sep: "\n".into(),
        };
        let path = Path::new("/home/me/drafts/notes.md");
        let wt = StatusModel::window_title;
        assert_eq!(wt(&h1("春天的信"), Some(path)), "春天的信");
        assert_eq!(wt(&h1("春天的信"), None), "春天的信");
        assert_eq!(wt(&TitleField::Unbound, Some(path)), "notes.md");
        assert_eq!(wt(&h1(""), Some(path)), "notes.md", "空 H1 退回文件名");
        assert_eq!(wt(&h1("  "), Some(path)), "notes.md");
        assert_eq!(wt(&TitleField::Unbound, None), "Untitled");
        assert_eq!(wt(&h1(""), None), "Untitled");
        // 控制字符：BEL / ESC 会结束或注入序列，一律去掉
        assert_eq!(wt(&h1("a\x07b\x1b]0;pwned\x07"), None), "ab]0;pwned");
        assert_eq!(
            wt(&h1("\x1b\x07"), Some(path)),
            "notes.md",
            "只剩控制字符等于空"
        );
        assert_eq!(
            wt(&TitleField::Unbound, Some(Path::new("evil\x1b[2J.md"))),
            "evil[2J.md"
        );
    }

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
        p.paste("x\r\ny");
        assert_eq!(p.input, "a.mdx y");
    }

    #[test]
    fn save_mark_states() {
        assert_eq!(SaveMark::of(true, true), Some(SaveMark::Dirty));
        assert_eq!(
            SaveMark::of(true, false),
            Some(SaveMark::Dirty),
            "Untitled 有修改也是星号"
        );
        assert_eq!(SaveMark::of(false, true), Some(SaveMark::Saved));
        assert_eq!(
            SaveMark::of(false, false),
            None,
            "空 Untitled / 未创建的新文件"
        );
        assert_eq!(SaveMark::Dirty.symbol(), "*");
        assert_eq!(SaveMark::Saved.symbol(), "●");
        assert_eq!(SaveMark::Dirty.style(), Style::default());
        assert_eq!(SaveMark::Saved.style(), Style::default().fg(Color::Green));
    }

    #[test]
    fn messages_expire_or_wait_for_input() {
        let t0 = Instant::now();
        let mut s = StatusModel::default();
        s.info("Typewriter 模式：开", t0, Duration::from_secs(2));
        s.on_input();
        assert!(s.message.is_some(), "info 不因按键消失");
        assert!(!s.expire(t0 + Duration::from_millis(1999)));
        assert!(s.expire(t0 + Duration::from_secs(2)));
        s.error("保存失败");
        assert_eq!(s.expiry(), None);
        s.on_input();
        assert!(s.message.is_none());
    }
}
