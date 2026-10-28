//! KeyEvent → Action（方案 §7）。M1：`vim` 恒为 false，`mode` 恒为 Insert。

use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};

use crate::action::{Action, Motion};

/// 定义见方案 §7.4；M1–M3 只使用 Insert。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal(NormalState),
    Insert,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NormalState {
    /// `'g'` 或 `'d'`
    pub pending: Option<char>,
    pub visual: bool,
}

pub struct Keymap {
    pub vim: bool,
}

impl Keymap {
    /// `mode` 用 `&mut`，是为了让 M4 的模式切换在 keymap 内部完成，不经过 Action。
    pub fn map_event(&self, ev: &Event, _mode: &mut Mode, prompt_active: bool) -> Option<Action> {
        let key = match ev {
            Event::Key(key) => key,
            Event::Paste(s) => return Some(Action::Paste(s.clone())),
            _ => return None,
        };
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return None;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        // Windows 上 AltGr 组合出的字符带 Ctrl+Alt，按普通字符处理
        let altgr = ctrl && key.modifiers.contains(KeyModifiers::ALT);
        if prompt_active {
            return Some(match key.code {
                KeyCode::Esc => Action::Cancel,
                KeyCode::Char('q' | 'Q') if ctrl && !altgr => Action::Quit,
                _ => Action::PromptInput(*key),
            });
        }
        let alt = key.modifiers.contains(KeyModifiers::ALT) && !altgr;
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        // Shift + 移动键扩展选区
        let mv = |m| Some(Action::Move(m, shift));
        match key.code {
            // Alt+B / Alt+F：macOS 终端的 Option 键按词移动
            KeyCode::Char('b' | 'B') if alt => Some(Action::Move(Motion::WordLeft, false)),
            KeyCode::Char('f' | 'F') if alt => Some(Action::Move(Motion::WordRight, false)),
            KeyCode::Char(_) if alt => None,
            KeyCode::Char(c) if !ctrl || altgr => Some(Action::Insert(c.to_string())),
            KeyCode::Char('s' | 'S') => Some(Action::Save),
            KeyCode::Char('q' | 'Q') => Some(Action::Quit),
            KeyCode::Char('z' | 'Z') => Some(Action::Undo),
            KeyCode::Char('y' | 'Y') => Some(Action::Redo),
            KeyCode::Char('a' | 'A') => Some(Action::SelectAll),
            KeyCode::Char('c' | 'C') => Some(Action::Copy),
            KeyCode::Char('x' | 'X') => Some(Action::Cut),
            KeyCode::Char('v' | 'V') => Some(Action::PasteRegister),
            // Unix 终端把 Ctrl+Backspace 发成 0x08，crossterm 报为 Ctrl+H
            KeyCode::Char('w' | 'W' | 'h' | 'H') => Some(Action::DeleteWordBack),
            KeyCode::Backspace if ctrl => Some(Action::DeleteWordBack),
            KeyCode::Enter => Some(Action::Newline),
            KeyCode::Backspace => Some(Action::DeleteBack),
            KeyCode::Delete => Some(Action::DeleteForward),
            KeyCode::Tab => Some(Action::Insert("    ".into())),
            KeyCode::Esc => Some(Action::Cancel),
            KeyCode::Left if ctrl => mv(Motion::WordLeft),
            KeyCode::Right if ctrl => mv(Motion::WordRight),
            KeyCode::Left => mv(Motion::Left),
            KeyCode::Right => mv(Motion::Right),
            KeyCode::Up => mv(Motion::Up),
            KeyCode::Down => mv(Motion::Down),
            KeyCode::Home if ctrl => mv(Motion::DocStart),
            KeyCode::End if ctrl => mv(Motion::DocEnd),
            KeyCode::Home => mv(Motion::LineStart),
            KeyCode::End => mv(Motion::LineEnd),
            KeyCode::PageUp => mv(Motion::PageUp),
            KeyCode::PageDown => mv(Motion::PageDown),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, KeyEventState};

    fn key(code: KeyCode, mods: KeyModifiers) -> Event {
        Event::Key(KeyEvent::new(code, mods))
    }

    #[test]
    fn default_bindings() {
        use KeyCode::*;
        let none = KeyModifiers::NONE;
        let ctrl = KeyModifiers::CONTROL;
        let shift = KeyModifiers::SHIFT;
        let mv = |m| Some(Action::Move(m, false));
        let ext = |m| Some(Action::Move(m, true));
        let ins = |s: &str| Some(Action::Insert(s.into()));
        let cases = [
            (key(Char('a'), none), ins("a")),
            (key(Char('A'), KeyModifiers::SHIFT), ins("A")),
            (key(Char('中'), none), ins("中")),
            (key(Char('@'), ctrl | KeyModifiers::ALT), ins("@")),
            (key(Char('x'), KeyModifiers::ALT), None),
            (key(Char('b'), KeyModifiers::ALT), mv(Motion::WordLeft)),
            (key(Char('f'), KeyModifiers::ALT), mv(Motion::WordRight)),
            (key(Left, ctrl), mv(Motion::WordLeft)),
            (key(Right, ctrl), mv(Motion::WordRight)),
            (key(Left, shift), ext(Motion::Left)),
            (key(Right, shift), ext(Motion::Right)),
            (key(Up, shift), ext(Motion::Up)),
            (key(Down, shift), ext(Motion::Down)),
            (key(Home, shift), ext(Motion::LineStart)),
            (key(End, shift), ext(Motion::LineEnd)),
            (key(Right, ctrl | shift), ext(Motion::WordRight)),
            (key(End, ctrl | shift), ext(Motion::DocEnd)),
            (key(Backspace, ctrl), Some(Action::DeleteWordBack)),
            (key(Char('h'), ctrl), Some(Action::DeleteWordBack)),
            (key(Char('w'), ctrl), Some(Action::DeleteWordBack)),
            (key(Char('a'), ctrl), Some(Action::SelectAll)),
            (key(Char('c'), ctrl), Some(Action::Copy)),
            (key(Char('x'), ctrl), Some(Action::Cut)),
            (key(Char('v'), ctrl), Some(Action::PasteRegister)),
            (key(Enter, none), Some(Action::Newline)),
            (key(Backspace, none), Some(Action::DeleteBack)),
            (key(Delete, none), Some(Action::DeleteForward)),
            (key(Tab, none), ins("    ")),
            (key(Esc, none), Some(Action::Cancel)),
            (key(Left, none), mv(Motion::Left)),
            (key(Right, none), mv(Motion::Right)),
            (key(Up, none), mv(Motion::Up)),
            (key(Down, none), mv(Motion::Down)),
            (key(Home, none), mv(Motion::LineStart)),
            (key(End, none), mv(Motion::LineEnd)),
            (key(PageUp, none), mv(Motion::PageUp)),
            (key(PageDown, none), mv(Motion::PageDown)),
            (key(Home, ctrl), mv(Motion::DocStart)),
            (key(End, ctrl), mv(Motion::DocEnd)),
            (key(Char('s'), ctrl), Some(Action::Save)),
            (key(Char('q'), ctrl), Some(Action::Quit)),
            (key(Char('z'), ctrl), Some(Action::Undo)),
            (key(Char('y'), ctrl), Some(Action::Redo)),
            (
                Event::Paste("a\r\nb".into()),
                Some(Action::Paste("a\r\nb".into())),
            ),
            // M4 的键位尚未绑定
            (key(Char('t'), ctrl), None),
            (key(Char('f'), ctrl), None),
            (key(F(1), none), None),
            (Event::FocusLost, None),
        ];
        let keymap = Keymap { vim: false };
        for (ev, want) in cases {
            let mut mode = Mode::Insert;
            assert_eq!(keymap.map_event(&ev, &mut mode, false), want, "{ev:?}");
            assert_eq!(mode, Mode::Insert, "{ev:?}");
        }
    }

    #[test]
    fn release_is_ignored() {
        let ev = Event::Key(KeyEvent::new_with_kind_and_state(
            KeyCode::Char('a'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
            KeyEventState::NONE,
        ));
        let mut mode = Mode::Insert;
        assert_eq!(Keymap { vim: false }.map_event(&ev, &mut mode, false), None);
        let repeat = Event::Key(KeyEvent::new_with_kind(
            KeyCode::Left,
            KeyModifiers::NONE,
            KeyEventKind::Repeat,
        ));
        assert_eq!(
            Keymap { vim: false }.map_event(&repeat, &mut mode, false),
            Some(Action::Move(Motion::Left, false))
        );
    }

    #[test]
    fn prompt_bindings() {
        let keymap = Keymap { vim: false };
        let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        let cases = [
            (key(KeyCode::Esc, KeyModifiers::NONE), Some(Action::Cancel)),
            (
                key(KeyCode::Char('q'), KeyModifiers::CONTROL),
                Some(Action::Quit),
            ),
            (Event::Key(enter), Some(Action::PromptInput(enter))),
        ];
        for (ev, want) in cases {
            let mut mode = Mode::Insert;
            assert_eq!(keymap.map_event(&ev, &mut mode, true), want, "{ev:?}");
            assert_eq!(mode, Mode::Insert);
        }
    }
}
