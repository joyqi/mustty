//! Input events, independent of the terminal library.
//!
//! The names, fields and constructors mirror `crossterm::event`, so the keymap and the panels read the
//! same whichever host produced the event. Equality also mirrors crossterm: a key event compares
//! after case normalization (`Shift+a` equals `A`). The native host converts with `From`.

use std::hash::{Hash, Hasher};
use std::ops::{BitAnd, BitOr, BitOrAssign};

mod parse;

pub use parse::InputParser;

#[derive(Debug, PartialOrd, PartialEq, Eq, Clone, Hash)]
pub enum Event {
    FocusGained,
    FocusLost,
    Key(KeyEvent),
    Mouse(MouseEvent),
    Paste(String),
    Resize(u16, u16),
}

#[derive(Debug, PartialOrd, Clone, Copy)]
pub struct KeyEvent {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
    pub kind: KeyEventKind,
    pub state: KeyEventState,
}

impl KeyEvent {
    pub const fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        Self::new_with_kind(code, modifiers, KeyEventKind::Press)
    }

    pub const fn new_with_kind(code: KeyCode, modifiers: KeyModifiers, kind: KeyEventKind) -> Self {
        Self::new_with_kind_and_state(code, modifiers, kind, KeyEventState::NONE)
    }

    pub const fn new_with_kind_and_state(
        code: KeyCode,
        modifiers: KeyModifiers,
        kind: KeyEventKind,
        state: KeyEventState,
    ) -> Self {
        Self {
            code,
            modifiers,
            kind,
            state,
        }
    }

    /// SHIFT is present iff the char is uppercase, so `Shift+a` and `A` are the same key
    fn normalize_case(mut self) -> Self {
        let KeyCode::Char(c) = self.code else {
            return self;
        };
        if c.is_ascii_uppercase() {
            self.modifiers.insert(KeyModifiers::SHIFT);
        } else if self.modifiers.contains(KeyModifiers::SHIFT) {
            self.code = KeyCode::Char(c.to_ascii_uppercase());
        }
        self
    }
}

impl From<KeyCode> for KeyEvent {
    fn from(code: KeyCode) -> Self {
        Self::new(code, KeyModifiers::NONE)
    }
}

impl PartialEq for KeyEvent {
    fn eq(&self, other: &Self) -> bool {
        let (a, b) = (self.normalize_case(), other.normalize_case());
        (a.code, a.modifiers, a.kind, a.state) == (b.code, b.modifiers, b.kind, b.state)
    }
}

impl Eq for KeyEvent {}

impl Hash for KeyEvent {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let k = self.normalize_case();
        (k.code, k.modifiers, k.kind, k.state).hash(state);
    }
}

#[derive(Debug, PartialOrd, PartialEq, Eq, Clone, Copy, Hash)]
pub enum KeyCode {
    Backspace,
    Enter,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Tab,
    BackTab,
    Delete,
    Insert,
    F(u8),
    Char(char),
    Null,
    Esc,
    CapsLock,
    ScrollLock,
    NumLock,
    PrintScreen,
    Pause,
    Menu,
    KeypadBegin,
    /// Media and lone modifier keys, which only the kitty keyboard protocol reports and no binding uses
    Other,
}

#[derive(Debug, PartialOrd, PartialEq, Eq, Clone, Copy, Hash)]
pub enum KeyEventKind {
    Press,
    Repeat,
    Release,
}

macro_rules! flags {
    ($(#[$doc:meta])* $name:ident { $($flag:ident = $bit:expr),* $(,)? }) => {
        $(#[$doc])*
        #[derive(PartialOrd, PartialEq, Eq, Clone, Copy, Hash, Default)]
        pub struct $name(u8);

        impl $name {
            pub const NONE: Self = Self(0);
            $(pub const $flag: Self = Self($bit);)*

            pub const fn empty() -> Self {
                Self::NONE
            }

            pub const fn bits(self) -> u8 {
                self.0
            }

            /// Drops bits that aren't a defined flag
            pub const fn from_bits_truncate(bits: u8) -> Self {
                Self(bits & (0 $(| $bit)*))
            }

            pub const fn is_empty(self) -> bool {
                self.0 == 0
            }

            pub const fn contains(self, other: Self) -> bool {
                self.0 & other.0 == other.0
            }

            pub const fn intersects(self, other: Self) -> bool {
                self.0 & other.0 != 0
            }

            pub fn insert(&mut self, other: Self) {
                self.0 |= other.0;
            }

            pub fn remove(&mut self, other: Self) {
                self.0 &= !other.0;
            }
        }

        impl BitOr for $name {
            type Output = Self;
            fn bitor(self, rhs: Self) -> Self {
                Self(self.0 | rhs.0)
            }
        }

        impl BitOrAssign for $name {
            fn bitor_assign(&mut self, rhs: Self) {
                self.0 |= rhs.0;
            }
        }

        impl BitAnd for $name {
            type Output = Self;
            fn bitand(self, rhs: Self) -> Self {
                Self(self.0 & rhs.0)
            }
        }

        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                let names: Vec<&str> = [$((stringify!($flag), Self::$flag)),*]
                    .into_iter()
                    .filter(|&(_, flag)| self.contains(flag))
                    .map(|(name, _)| name)
                    .collect();
                write!(f, "{}({})", stringify!($name), names.join(" | "))
            }
        }
    };
}

flags!(KeyModifiers {
    SHIFT = 0b0000_0001,
    CONTROL = 0b0000_0010,
    ALT = 0b0000_0100,
    SUPER = 0b0000_1000,
    HYPER = 0b0001_0000,
    META = 0b0010_0000,
});

flags!(KeyEventState {
    KEYPAD = 0b0000_0001,
    CAPS_LOCK = 0b0000_0010,
    NUM_LOCK = 0b0000_0100,
});

#[derive(Debug, PartialOrd, PartialEq, Eq, Clone, Copy, Hash)]
pub struct MouseEvent {
    pub kind: MouseEventKind,
    pub column: u16,
    pub row: u16,
    pub modifiers: KeyModifiers,
}

#[derive(Debug, PartialOrd, PartialEq, Eq, Clone, Copy, Hash)]
pub enum MouseEventKind {
    Down(MouseButton),
    Up(MouseButton),
    Drag(MouseButton),
    Moved,
    ScrollDown,
    ScrollUp,
    ScrollLeft,
    ScrollRight,
}

#[derive(Debug, PartialOrd, PartialEq, Eq, Clone, Copy, Hash)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

#[cfg(feature = "native")]
mod from_crossterm {
    use super::*;
    use crossterm::event as ct;

    impl From<ct::Event> for Event {
        fn from(ev: ct::Event) -> Self {
            match ev {
                ct::Event::FocusGained => Self::FocusGained,
                ct::Event::FocusLost => Self::FocusLost,
                ct::Event::Key(k) => Self::Key(k.into()),
                ct::Event::Mouse(m) => Self::Mouse(m.into()),
                ct::Event::Paste(s) => Self::Paste(s),
                ct::Event::Resize(w, h) => Self::Resize(w, h),
            }
        }
    }

    impl From<ct::KeyEvent> for KeyEvent {
        fn from(k: ct::KeyEvent) -> Self {
            Self {
                code: k.code.into(),
                modifiers: k.modifiers.into(),
                kind: match k.kind {
                    ct::KeyEventKind::Press => KeyEventKind::Press,
                    ct::KeyEventKind::Repeat => KeyEventKind::Repeat,
                    ct::KeyEventKind::Release => KeyEventKind::Release,
                },
                state: KeyEventState(k.state.bits()),
            }
        }
    }

    impl From<ct::KeyCode> for KeyCode {
        fn from(code: ct::KeyCode) -> Self {
            use ct::KeyCode as C;
            match code {
                C::Backspace => Self::Backspace,
                C::Enter => Self::Enter,
                C::Left => Self::Left,
                C::Right => Self::Right,
                C::Up => Self::Up,
                C::Down => Self::Down,
                C::Home => Self::Home,
                C::End => Self::End,
                C::PageUp => Self::PageUp,
                C::PageDown => Self::PageDown,
                C::Tab => Self::Tab,
                C::BackTab => Self::BackTab,
                C::Delete => Self::Delete,
                C::Insert => Self::Insert,
                C::F(n) => Self::F(n),
                C::Char(c) => Self::Char(c),
                C::Null => Self::Null,
                C::Esc => Self::Esc,
                C::CapsLock => Self::CapsLock,
                C::ScrollLock => Self::ScrollLock,
                C::NumLock => Self::NumLock,
                C::PrintScreen => Self::PrintScreen,
                C::Pause => Self::Pause,
                C::Menu => Self::Menu,
                C::KeypadBegin => Self::KeypadBegin,
                C::Media(_) | C::Modifier(_) => Self::Other,
            }
        }
    }

    impl From<ct::KeyModifiers> for KeyModifiers {
        fn from(m: ct::KeyModifiers) -> Self {
            Self(m.bits())
        }
    }

    impl From<ct::MouseEvent> for MouseEvent {
        fn from(m: ct::MouseEvent) -> Self {
            Self {
                kind: match m.kind {
                    ct::MouseEventKind::Down(b) => MouseEventKind::Down(b.into()),
                    ct::MouseEventKind::Up(b) => MouseEventKind::Up(b.into()),
                    ct::MouseEventKind::Drag(b) => MouseEventKind::Drag(b.into()),
                    ct::MouseEventKind::Moved => MouseEventKind::Moved,
                    ct::MouseEventKind::ScrollDown => MouseEventKind::ScrollDown,
                    ct::MouseEventKind::ScrollUp => MouseEventKind::ScrollUp,
                    ct::MouseEventKind::ScrollLeft => MouseEventKind::ScrollLeft,
                    ct::MouseEventKind::ScrollRight => MouseEventKind::ScrollRight,
                },
                column: m.column,
                row: m.row,
                modifiers: m.modifiers.into(),
            }
        }
    }

    impl From<ct::MouseButton> for MouseButton {
        fn from(b: ct::MouseButton) -> Self {
            match b {
                ct::MouseButton::Left => Self::Left,
                ct::MouseButton::Right => Self::Right,
                ct::MouseButton::Middle => Self::Middle,
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn modifier_and_state_bits_match_crossterm() {
            // Converting copies the bits, so every flag must sit where crossterm has it
            let pairs = [
                (ct::KeyModifiers::SHIFT, KeyModifiers::SHIFT),
                (ct::KeyModifiers::CONTROL, KeyModifiers::CONTROL),
                (ct::KeyModifiers::ALT, KeyModifiers::ALT),
                (ct::KeyModifiers::SUPER, KeyModifiers::SUPER),
                (ct::KeyModifiers::HYPER, KeyModifiers::HYPER),
                (ct::KeyModifiers::META, KeyModifiers::META),
            ];
            for (theirs, ours) in pairs {
                assert_eq!(KeyModifiers::from(theirs), ours, "{theirs:?}");
            }
            assert_eq!(ct::KeyModifiers::all().bits(), 0b0011_1111);
            let states = [
                (ct::KeyEventState::KEYPAD, KeyEventState::KEYPAD),
                (ct::KeyEventState::CAPS_LOCK, KeyEventState::CAPS_LOCK),
                (ct::KeyEventState::NUM_LOCK, KeyEventState::NUM_LOCK),
            ];
            for (theirs, ours) in states {
                assert_eq!(theirs.bits(), ours.bits(), "{theirs:?}");
            }
            assert_eq!(ct::KeyEventState::all().bits(), 0b0000_0111);
        }

        #[test]
        fn key_equality_normalizes_case_like_crossterm() {
            let pairs = [
                (ct::KeyCode::Char('a'), ct::KeyModifiers::SHIFT),
                (ct::KeyCode::Char('A'), ct::KeyModifiers::NONE),
                (ct::KeyCode::Char('A'), ct::KeyModifiers::SHIFT),
                (ct::KeyCode::Char('a'), ct::KeyModifiers::NONE),
                (ct::KeyCode::Char('中'), ct::KeyModifiers::SHIFT),
                (ct::KeyCode::Enter, ct::KeyModifiers::SHIFT),
            ];
            for a in pairs {
                for b in pairs {
                    let theirs = ct::KeyEvent::new(a.0, a.1) == ct::KeyEvent::new(b.0, b.1);
                    let ours = KeyEvent::from(ct::KeyEvent::new(a.0, a.1))
                        == KeyEvent::from(ct::KeyEvent::new(b.0, b.1));
                    assert_eq!(ours, theirs, "{a:?} vs {b:?}");
                }
            }
        }
    }
}
