//! KeyEvent → Action. With `vim` off, `mode` is always Insert; the vim state machine lives only in
//! this file.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::action::{Action, Motion};

/// Editing mode; always Insert when `vim` is off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal(NormalState),
    Insert,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NormalState {
    /// `'g'` or `'d'`
    pub pending: Option<char>,
    pub visual: bool,
}

pub struct Keymap {
    pub vim: bool,
}

impl Keymap {
    /// `mode` is `&mut` so that M4's mode switches happen inside the keymap instead of going
    /// through an Action.
    pub fn map_event(&self, ev: &Event, mode: &mut Mode, prompt_active: bool) -> Option<Action> {
        let key = match ev {
            Event::Key(key) => key,
            Event::Paste(s) => return Some(Action::Paste(s.clone())),
            _ => return None,
        };
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return None;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        // On Windows, AltGr characters arrive with Ctrl+Alt; treat them as plain characters
        let altgr = ctrl && key.modifiers.contains(KeyModifiers::ALT);
        if prompt_active {
            return Some(match key.code {
                KeyCode::Esc => Action::Cancel,
                KeyCode::Char('q' | 'Q') if ctrl && !altgr => Action::Quit,
                _ => Action::PromptInput(*key),
            });
        }
        if !self.vim {
            return default_map(key, false);
        }
        match *mode {
            Mode::Insert if key.code == KeyCode::Esc => {
                *mode = Mode::Normal(NormalState::default());
                Some(Action::Cancel)
            }
            Mode::Insert => default_map(key, false),
            Mode::Normal(state) => {
                let (action, next) = map_normal(key, state);
                *mode = next;
                action
            }
        }
    }
}

/// Default (non-modal) bindings, also used in vim Insert mode. When `extend` is true, every motion
/// extends the selection (Visual mode).
fn default_map(key: &KeyEvent, extend: bool) -> Option<Action> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let altgr = ctrl && key.modifiers.contains(KeyModifiers::ALT);
    let alt = key.modifiers.contains(KeyModifiers::ALT) && !altgr;
    // Shift + a motion key extends the selection
    let shift = extend || key.modifiers.contains(KeyModifiers::SHIFT);
    let mv = |m| Some(Action::Move(m, shift));
    match key.code {
        // Alt+B / Alt+F: word motion via the Option key in macOS terminals
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
        // Unix terminals send Ctrl+Backspace as 0x08, which crossterm reports as Ctrl+H
        KeyCode::Char('w' | 'W' | 'h' | 'H') => Some(Action::DeleteWordBack),
        KeyCode::Char('t' | 'T') => Some(Action::ToggleTypewriter),
        KeyCode::Char('f' | 'F') => Some(Action::ToggleFocusMode),
        KeyCode::Char('p' | 'P') => Some(Action::OpenSettings),
        KeyCode::Char('g' | 'G') => Some(Action::OpenHelp),
        KeyCode::Backspace if ctrl => Some(Action::DeleteWordBack),
        KeyCode::Enter => Some(Action::Newline),
        KeyCode::Backspace => Some(Action::DeleteBack),
        KeyCode::Delete => Some(Action::DeleteForward),
        KeyCode::Tab => Some(Action::Tab),
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

/// vim Normal mode. Returns the action and the new Mode; Visual mode and the g/d prefixes are
/// substates of NormalState.
fn map_normal(key: &KeyEvent, mut st: NormalState) -> (Option<Action>, Mode) {
    use Motion::*;
    let pending = st.pending.take();
    let visual = st.visual;
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let normal = Mode::Normal(st);
    let leave_visual = Mode::Normal(NormalState::default());
    let mv = |m| (Some(Action::Move(m, visual)), normal);
    // Ctrl / Alt combos match the default bindings; Ctrl+R redoes only in Normal mode
    if (ctrl || alt) && !(ctrl && alt) {
        if ctrl && matches!(key.code, KeyCode::Char('r' | 'R')) {
            return (Some(Action::Redo), normal);
        }
        // Normal mode never inserts text, though: combos like Ctrl+Tab or Ctrl+Enter that fall back
        // to plain editing keys are ignored
        let action = default_map(key, visual).filter(|a| {
            !matches!(
                a,
                Action::Insert(_)
                    | Action::Tab
                    | Action::Newline
                    | Action::DeleteBack
                    | Action::DeleteForward
            )
        });
        return (action, normal);
    }
    let KeyCode::Char(c) = key.code else {
        return match key.code {
            KeyCode::Esc if visual => (Some(Action::Cancel), leave_visual),
            KeyCode::Esc => (None, normal),
            KeyCode::Left
            | KeyCode::Right
            | KeyCode::Up
            | KeyCode::Down
            | KeyCode::Home
            | KeyCode::End
            | KeyCode::PageUp
            | KeyCode::PageDown => (default_map(key, visual), normal),
            _ => (None, normal),
        };
    };
    match (pending, c) {
        (Some('g'), 'g') => mv(DocStart),
        (Some('d'), 'd') => (Some(Action::DeleteLine), normal),
        // Invalid sequence: drop the pending operator and ignore the second key too
        (Some(_), _) => (None, normal),
        (None, 'i') => (visual.then_some(Action::Cancel), Mode::Insert),
        (None, 'a') => (Some(Action::Move(Right, false)), Mode::Insert),
        (None, 'o') => (Some(Action::OpenLineBelow), Mode::Insert),
        (None, 'v') if visual => (Some(Action::Cancel), leave_visual),
        (None, 'v') => (
            None,
            Mode::Normal(NormalState {
                pending: None,
                visual: true,
            }),
        ),
        (None, 'h') => mv(Left),
        (None, 'j') => mv(Down),
        (None, 'k') => mv(Up),
        (None, 'l') => mv(Right),
        (None, 'w') => mv(WordRight),
        (None, 'b') => mv(WordLeft),
        (None, 'e') => mv(WordEnd),
        (None, '0') => mv(LineStart),
        (None, '$') => mv(LineEnd),
        (None, 'G') => mv(DocEnd),
        (None, 'g') => (
            None,
            Mode::Normal(NormalState {
                pending: Some('g'),
                visual,
            }),
        ),
        (None, 'x') if visual => (Some(Action::Cut), leave_visual),
        (None, 'x') => (Some(Action::DeleteForward), normal),
        (None, 'd') if visual => (Some(Action::Cut), leave_visual),
        (None, 'd') => (
            None,
            Mode::Normal(NormalState {
                pending: Some('d'),
                visual,
            }),
        ),
        (None, 'y') if visual => (Some(Action::Copy), leave_visual),
        (None, 'p') => (Some(Action::PasteRegister), leave_visual),
        (None, 'u') => (Some(Action::Undo), normal),
        (None, '?') => (Some(Action::OpenHelp), normal),
        // Any other printable character (including CJK committed by an IME) is ignored
        _ => (None, normal),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEventState;
    use proptest::prelude::*;

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
            (key(Tab, none), Some(Action::Tab)),
            (key(BackTab, shift), None), // Shift+Tab is a no-op; never adds spaces to the title
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
            (key(Char('t'), ctrl), Some(Action::ToggleTypewriter)),
            (key(Char('f'), ctrl), Some(Action::ToggleFocusMode)),
            (key(Char('p'), ctrl), Some(Action::OpenSettings)),
            (key(Char('r'), ctrl), None), // redo is vim Normal mode only
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
    fn tab_maps_to_its_own_action() {
        // Editor picks the number of spaces from tab_width (see the editor's tab tests); keymap
        // only reports "Tab pressed"
        let tab = key(KeyCode::Tab, KeyModifiers::NONE);
        let keymap = Keymap { vim: false };
        assert_eq!(
            keymap.map_event(&tab, &mut Mode::Insert, false),
            Some(Action::Tab)
        );
        let vim = Keymap { vim: true };
        assert_eq!(
            vim.map_event(&tab, &mut Mode::Insert, false),
            Some(Action::Tab)
        );
        for code in [KeyCode::Tab, KeyCode::BackTab] {
            for mods in [
                KeyModifiers::NONE,
                KeyModifiers::SHIFT,
                KeyModifiers::CONTROL,
            ] {
                let ev = key(code, mods);
                let got = keymap.map_event(&ev, &mut Mode::Insert, false);
                assert!(!matches!(got, Some(Action::Insert(_))), "{ev:?} → {got:?}");
                let got = vim.map_event(&ev, &mut NOR.clone(), false);
                assert_eq!(got, None, "Normal mode {ev:?}");
            }
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

    // ---- vim ----

    fn normal(pending: Option<char>, visual: bool) -> Mode {
        Mode::Normal(NormalState { pending, visual })
    }

    const NOR: Mode = Mode::Normal(NormalState {
        pending: None,
        visual: false,
    });
    const VIS: Mode = Mode::Normal(NormalState {
        pending: None,
        visual: true,
    });

    fn ch(c: char) -> Event {
        let mods = if c.is_uppercase() || c == '$' {
            KeyModifiers::SHIFT
        } else {
            KeyModifiers::NONE
        };
        key(KeyCode::Char(c), mods)
    }

    /// Feeds events in order starting from `mode` and returns (action, new Mode) for each step.
    fn vim(mode: Mode, events: &[Event], prompt: bool) -> Vec<(Option<Action>, Mode)> {
        let keymap = Keymap { vim: true };
        let mut mode = mode;
        events
            .iter()
            .map(|ev| {
                let a = keymap.map_event(ev, &mut mode, prompt);
                (a, mode)
            })
            .collect()
    }

    fn one(mode: Mode, ev: Event) -> (Option<Action>, Mode) {
        vim(mode, &[ev], false).remove(0)
    }

    #[test]
    fn vim_mode_switches() {
        let esc = key(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(one(Mode::Insert, esc.clone()), (Some(Action::Cancel), NOR));
        assert_eq!(one(NOR, ch('i')), (None, Mode::Insert));
        assert_eq!(
            one(NOR, ch('a')),
            (Some(Action::Move(Motion::Right, false)), Mode::Insert)
        );
        assert_eq!(
            one(NOR, ch('o')),
            (Some(Action::OpenLineBelow), Mode::Insert)
        );
        assert_eq!(one(NOR, ch('v')), (None, VIS));
        assert_eq!(one(VIS, ch('v')), (Some(Action::Cancel), NOR));
        assert_eq!(one(VIS, esc.clone()), (Some(Action::Cancel), NOR));
        assert_eq!(
            one(normal(Some('g'), false), esc.clone()),
            (None, NOR),
            "Esc clears the pending operator"
        );
        assert_eq!(one(NOR, esc), (None, NOR));
    }

    #[test]
    fn vim_normal_table() {
        use Motion::*;
        let m = |motion| Some(Action::Move(motion, false));
        let cases = [
            ('h', m(Left)),
            ('j', m(Down)),
            ('k', m(Up)),
            ('l', m(Right)),
            ('w', m(WordRight)),
            ('b', m(WordLeft)),
            ('e', m(WordEnd)),
            ('0', m(LineStart)),
            ('$', m(LineEnd)),
            ('G', m(DocEnd)),
            ('x', Some(Action::DeleteForward)),
            ('p', Some(Action::PasteRegister)),
            ('u', Some(Action::Undo)),
            ('y', None), // no yy outside Visual mode
            // Other printable characters (including CJK committed by an IME) are ignored
            ('q', None),
            ('中', None),
            ('，', None),
        ];
        for (c, want) in cases {
            assert_eq!(one(NOR, ch(c)), (want, NOR), "{c:?}");
        }
        // Arrow keys and friends match the default bindings
        let arrow = key(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(one(NOR, arrow), (m(Down), NOR));
        let home = key(KeyCode::Home, KeyModifiers::NONE);
        assert_eq!(one(NOR, home), (m(LineStart), NOR));
        // Editing keys have no effect in Normal mode
        for code in [KeyCode::Enter, KeyCode::Backspace, KeyCode::Tab] {
            assert_eq!(
                one(NOR, key(code, KeyModifiers::NONE)),
                (None, NOR),
                "{code:?}"
            );
        }
    }

    #[test]
    fn vim_visual_extends_and_exits_after_operator() {
        use Motion::*;
        assert_eq!(
            one(VIS, ch('w')),
            (Some(Action::Move(WordRight, true)), VIS)
        );
        assert_eq!(one(VIS, ch('j')), (Some(Action::Move(Down, true)), VIS));
        assert_eq!(
            one(VIS, key(KeyCode::Right, KeyModifiers::NONE)),
            (Some(Action::Move(Right, true)), VIS)
        );
        assert_eq!(one(VIS, ch('G')), (Some(Action::Move(DocEnd, true)), VIS));
        assert_eq!(one(VIS, ch('d')), (Some(Action::Cut), NOR));
        assert_eq!(one(VIS, ch('x')), (Some(Action::Cut), NOR));
        assert_eq!(one(VIS, ch('y')), (Some(Action::Copy), NOR));
        assert_eq!(one(VIS, ch('p')), (Some(Action::PasteRegister), NOR));
        // gg extends in Visual mode too
        let seq = vim(VIS, &[ch('g'), ch('g')], false);
        assert_eq!(seq[1], (Some(Action::Move(DocStart, true)), VIS));
    }

    #[test]
    fn vim_pending_sequences() {
        let seq = vim(NOR, &[ch('g'), ch('g')], false);
        assert_eq!(
            seq,
            vec![
                (None, normal(Some('g'), false)),
                (Some(Action::Move(Motion::DocStart, false)), NOR)
            ]
        );
        let seq = vim(NOR, &[ch('d'), ch('d')], false);
        assert_eq!(
            seq,
            vec![
                (None, normal(Some('d'), false)),
                (Some(Action::DeleteLine), NOR)
            ]
        );
        // Invalid sequence: drop the pending operator and ignore the second key too
        for (first, second) in [('g', 'x'), ('d', 'j'), ('g', 'd'), ('d', 'g')] {
            let seq = vim(NOR, &[ch(first), ch(second)], false);
            assert_eq!(seq[1], (None, NOR), "{first}{second}");
        }
        let seq = vim(
            NOR,
            &[ch('g'), key(KeyCode::Down, KeyModifiers::NONE)],
            false,
        );
        assert_eq!(
            seq[1],
            (Some(Action::Move(Motion::Down, false)), NOR),
            "non-char key: only drops the pending operator"
        );
    }

    #[test]
    fn vim_prompt_paste_and_ctrl_keys() {
        // While the prompt is open, Esc only closes the prompt and doesn't switch modes
        let esc = key(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(
            vim(Mode::Insert, std::slice::from_ref(&esc), true)[0],
            (Some(Action::Cancel), Mode::Insert)
        );
        assert_eq!(vim(VIS, &[esc], true)[0], (Some(Action::Cancel), VIS));
        // A terminal paste still inserts in Normal mode
        let paste = Event::Paste("文本".into());
        assert_eq!(one(NOR, paste), (Some(Action::Paste("文本".into())), NOR));
        // Ctrl combos return the same action in both modes (motions extend the selection in Visual)
        let ctrl = KeyModifiers::CONTROL;
        for c in ['s', 'q', 'z', 'y', 'c', 'x', 'v', 't', 'f', 'p', 'a', 'w'] {
            let ev = key(KeyCode::Char(c), ctrl);
            let insert = vim(Mode::Insert, std::slice::from_ref(&ev), false)[0]
                .0
                .clone();
            assert_eq!(one(NOR, ev), (insert, NOR), "Ctrl+{c}");
        }
        // Ctrl+R redoes only in Normal mode
        let ctrl_r = key(KeyCode::Char('r'), ctrl);
        assert_eq!(one(NOR, ctrl_r.clone()), (Some(Action::Redo), NOR));
        assert_eq!(one(Mode::Insert, ctrl_r), (None, Mode::Insert));
        // The settings panel opens from Normal mode too
        let ctrl_p = key(KeyCode::Char('p'), ctrl);
        assert_eq!(one(NOR, ctrl_p), (Some(Action::OpenSettings), NOR));
        // Insert mode is identical to the default bindings
        let typed = one(Mode::Insert, ch('j'));
        assert_eq!(typed, (Some(Action::Insert("j".into())), Mode::Insert));
    }

    const INS: Mode = Mode::Insert;
    const PEND_G: Mode = Mode::Normal(NormalState {
        pending: Some('g'),
        visual: false,
    });
    const PEND_D: Mode = Mode::Normal(NormalState {
        pending: Some('d'),
        visual: false,
    });
    const fn mv(m: Motion, extend: bool) -> Option<Action> {
        Some(Action::Move(m, extend))
    }

    /// The vim bindings, one row each: (key sequence, start state, action of the last key, end
    /// state). `<Esc>` is the Esc key; everything else is char keys. When adding or changing a
    /// binding, update both this table and the help table.
    const VIM_TABLE: &[(&str, Mode, Option<Action>, Mode)] = &[
        // Mode switches
        ("<Esc>", INS, Some(Action::Cancel), NOR),
        ("i", NOR, None, INS),
        ("a", NOR, mv(Motion::Right, false), INS),
        ("o", NOR, Some(Action::OpenLineBelow), INS),
        ("v", NOR, None, VIS),
        ("v", VIS, Some(Action::Cancel), NOR),
        ("<Esc>", VIS, Some(Action::Cancel), NOR),
        ("<Esc>", NOR, None, NOR),
        ("<Esc>", PEND_G, None, NOR),
        // Normal mode actions
        ("h", NOR, mv(Motion::Left, false), NOR),
        ("j", NOR, mv(Motion::Down, false), NOR),
        ("k", NOR, mv(Motion::Up, false), NOR),
        ("l", NOR, mv(Motion::Right, false), NOR),
        ("w", NOR, mv(Motion::WordRight, false), NOR),
        ("b", NOR, mv(Motion::WordLeft, false), NOR),
        ("e", NOR, mv(Motion::WordEnd, false), NOR),
        ("0", NOR, mv(Motion::LineStart, false), NOR),
        ("$", NOR, mv(Motion::LineEnd, false), NOR),
        ("gg", NOR, mv(Motion::DocStart, false), NOR),
        ("G", NOR, mv(Motion::DocEnd, false), NOR),
        ("x", NOR, Some(Action::DeleteForward), NOR),
        ("dd", NOR, Some(Action::DeleteLine), NOR),
        ("p", NOR, Some(Action::PasteRegister), NOR),
        ("u", NOR, Some(Action::Undo), NOR),
        // Visual: motions extend the selection; d / y / x exit afterwards
        ("h", VIS, mv(Motion::Left, true), VIS),
        ("w", VIS, mv(Motion::WordRight, true), VIS),
        ("$", VIS, mv(Motion::LineEnd, true), VIS),
        ("gg", VIS, mv(Motion::DocStart, true), VIS),
        ("G", VIS, mv(Motion::DocEnd, true), VIS),
        ("x", VIS, Some(Action::Cut), NOR),
        ("d", VIS, Some(Action::Cut), NOR),
        ("y", VIS, Some(Action::Copy), NOR),
        // Prefix sequences and invalid sequences
        ("g", NOR, None, PEND_G),
        ("d", NOR, None, PEND_D),
        ("gx", NOR, None, NOR),
        ("dj", NOR, None, NOR),
        // Other printable characters are ignored
        ("z", NOR, None, NOR),
        ("中", NOR, None, NOR),
    ];

    #[test]
    fn vim_table_replays() {
        for (keys, before, want, after) in VIM_TABLE {
            let events: Vec<Event> = if *keys == "<Esc>" {
                vec![key(KeyCode::Esc, KeyModifiers::NONE)]
            } else {
                keys.chars().map(ch).collect()
            };
            let last = vim(*before, &events, false).pop().unwrap();
            assert_eq!(last, (want.clone(), *after), "{keys:?} from {before:?}");
        }
    }

    fn any_event() -> impl Strategy<Value = Event> {
        let codes = prop_oneof![
            any::<char>().prop_map(KeyCode::Char),
            prop::sample::select(vec![
                KeyCode::Esc,
                KeyCode::Enter,
                KeyCode::Backspace,
                KeyCode::Delete,
                KeyCode::Tab,
                KeyCode::Left,
                KeyCode::Right,
                KeyCode::Up,
                KeyCode::Down,
                KeyCode::Home,
                KeyCode::End,
                KeyCode::PageUp,
                KeyCode::PageDown,
                KeyCode::F(12),
            ]),
        ];
        let kinds = prop::sample::select(vec![
            KeyEventKind::Press,
            KeyEventKind::Repeat,
            KeyEventKind::Release,
        ]);
        let mods = (0u8..16).prop_map(KeyModifiers::from_bits_truncate);
        prop_oneof![
            8 => (codes, mods, kinds).prop_map(|(c, m, k)| Event::Key(KeyEvent::new_with_kind(c, m, k))),
            1 => ".{0,8}".prop_map(Event::Paste),
            1 => Just(Event::FocusLost),
            1 => (0u16..200, 0u16..60).prop_map(|(w, h)| Event::Resize(w, h)),
        ]
    }

    proptest! {
        /// With vim off, no event sequence changes the Mode (always Insert).
        #[test]
        fn vim_off_never_changes_mode(events in prop::collection::vec(any_event(), 0..80), prompt: bool) {
            let keymap = Keymap { vim: false };
            let mut mode = Mode::Insert;
            for ev in &events {
                keymap.map_event(ev, &mut mode, prompt);
                prop_assert_eq!(mode, Mode::Insert);
            }
        }

        /// With vim on, no sequence panics, Normal mode never inserts, and the pending operator is
        /// only ever g / d.
        #[test]
        fn vim_on_is_total(events in prop::collection::vec(any_event(), 0..80)) {
            let keymap = Keymap { vim: true };
            let mut mode = Mode::Insert;
            for ev in &events {
                let before = mode;
                let action = keymap.map_event(ev, &mut mode, false);
                if matches!(before, Mode::Normal(_)) {
                    prop_assert!(!matches!(action, Some(Action::Insert(_) | Action::Tab)), "{:?}", ev);
                }
                if let Mode::Normal(s) = mode {
                    prop_assert!(matches!(s.pending, None | Some('g' | 'd')));
                }
            }
        }
    }
}
