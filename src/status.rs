//! Status bar state: messages and the "Save as: " prompt.

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

/// The single-line "Save as: " input.
#[derive(Debug, Default)]
pub struct Prompt {
    pub input: String,
    /// Char index
    pub cursor: usize,
    /// The target exists; a second Enter confirms the overwrite. Reset when the path changes
    pub confirm_overwrite: bool,
}

impl Prompt {
    /// Prefills `input` with the cursor at the end.
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

    /// Pastes into the single-line input, turning newlines into spaces.
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

    /// Handles one key; returns true if it was Enter.
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

/// Save-state mark shown after the file name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveMark {
    /// Unsaved changes: `*` in the default foreground
    Dirty,
    /// Content matches the disk (an opened file left unchanged, or saved manually or by autosave):
    /// green `●`
    Saved,
}

impl SaveMark {
    /// `on_disk`: the file exists on disk and its state was recorded (an existing file was opened,
    /// or it was saved). An empty untitled document or a not-yet-created new file shows no mark
    /// while unmodified.
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
            // East Asian Width is Ambiguous; we count it as narrow (1 column)
            Self::Saved => "\u{25CF}",
        }
    }

    /// Neither is DIM, so they stand out against the DIM file name.
    pub fn style(self) -> Style {
        match self {
            Self::Dirty => Style::default(),
            Self::Saved => Style::default().fg(Color::Green),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageKind {
    /// Disappears on its own when it expires
    Info { expires: Instant },
    /// Stays until the next key press
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub text: String,
    pub kind: MessageKind,
}

#[derive(Debug, Default)]
pub struct StatusModel {
    /// Overrides the left section
    pub message: Option<Message>,
    pub prompt: Option<Prompt>,
    /// Autosave was abandoned because of an external change; the notice stays until a save succeeds
    pub externally_modified: bool,
    /// The displayed word count (title + body)
    pub words: usize,
    /// When the startup key hint ends
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

    /// Called on key input; error messages end here.
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

    /// Clears an expired message; returns whether anything changed.
    pub fn expire(&mut self, now: Instant) -> bool {
        let expired = self.expiry().is_some_and(|t| now >= t);
        if expired {
            self.message = None;
        }
        expired
    }

    pub fn words_label(&self) -> String {
        let n = thousands(self.words);
        // English plurals: this is the only place that needs them, so two keys beat plural rules
        if self.words == 1 {
            t_args("status-words-one", &[("n", &n)])
        } else {
            t_args("status-words-other", &[("n", &n)])
        }
    }

    /// Terminal window title: a non-empty H1 title > the file name (with extension) > `Untitled`,
    /// with control characters stripped. Unsaved changes add a `* ` prefix (not translated).
    pub fn window_title(title: &TitleField, path: Option<&Path>, dirty: bool) -> String {
        let text = Self::window_title_text(title, path);
        if dirty { format!("* {text}") } else { text }
    }

    fn window_title_text(title: &TitleField, path: Option<&Path>) -> String {
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

    /// The basename, or `Untitled` (in the UI language) when there is no path.
    pub fn file_name(path: Option<&Path>) -> String {
        match path {
            Some(p) => p.file_name().map_or_else(
                || p.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            ),
            None => t("status-untitled").into(),
        }
    }

    /// Line = logical line number, column = grapheme index within the line, both 1-based.
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
        let wt = |t: &TitleField, p| StatusModel::window_title(t, p, false);
        assert_eq!(wt(&h1("春天的信"), Some(path)), "春天的信");
        assert_eq!(wt(&h1("春天的信"), None), "春天的信");
        assert_eq!(wt(&TitleField::Unbound, Some(path)), "notes.md");
        assert_eq!(wt(&h1(""), Some(path)), "notes.md", "empty H1 → file name");
        assert_eq!(wt(&h1("  "), Some(path)), "notes.md");
        assert_eq!(wt(&TitleField::Unbound, None), "Untitled");
        assert_eq!(wt(&h1(""), None), "Untitled");
        // Control characters: BEL / ESC can end or inject a sequence, so strip them all
        assert_eq!(wt(&h1("a\x07b\x1b]0;pwned\x07"), None), "ab]0;pwned");
        assert_eq!(
            wt(&h1("\x1b\x07"), Some(path)),
            "notes.md",
            "only control characters counts as empty"
        );
        assert_eq!(
            wt(&TitleField::Unbound, Some(Path::new("evil\x1b[2J.md"))),
            "evil[2J.md"
        );
    }

    #[test]
    fn window_title_marks_unsaved_changes() {
        crate::i18n::init_for_test("en");
        let h1 = TitleField::Bound {
            text: "春天的信".into(),
            sep: "\n".into(),
        };
        let path = Path::new("/home/me/notes.md");
        let wt = StatusModel::window_title;
        // Three sources (H1 / file name / Untitled): no prefix when clean, `* ` when modified
        assert_eq!(wt(&h1, Some(path), false), "春天的信");
        assert_eq!(wt(&h1, Some(path), true), "* 春天的信");
        assert_eq!(wt(&TitleField::Unbound, Some(path), false), "notes.md");
        assert_eq!(wt(&TitleField::Unbound, Some(path), true), "* notes.md");
        assert_eq!(wt(&TitleField::Unbound, None, false), "Untitled");
        assert_eq!(wt(&TitleField::Unbound, None, true), "* Untitled");
        // The prefix is not translated; control characters are still stripped
        crate::i18n::init_for_test("zh-CN");
        let evil = TitleField::Bound {
            text: "\x1b]0;x\x07".into(),
            sep: "\n".into(),
        };
        assert_eq!(wt(&evil, None, true), "* ]0;x");
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
            "modified untitled also gets the asterisk"
        );
        assert_eq!(SaveMark::of(false, true), Some(SaveMark::Saved));
        assert_eq!(
            SaveMark::of(false, false),
            None,
            "empty untitled / not-yet-created new file"
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
        assert!(s.message.is_some(), "info does not disappear on key input");
        assert!(!s.expire(t0 + Duration::from_millis(1999)));
        assert!(s.expire(t0 + Duration::from_secs(2)));
        s.error("保存失败");
        assert_eq!(s.expiry(), None);
        s.on_input();
        assert!(s.message.is_none());
    }
}
