//! Editing semantics, decoupled from keys. All variants were fixed in M1 and M4 adds or removes none;
//! the only deliberate revisions are six: `OpenSettings` for the settings panel, `OpenHelp` for
//! the help panel, `Tab`, find (`OpenFind`, `FindNext`, `FindPrev`), the list keys (`BackTab`,
//! `OpenLineAbove`), which are named after keys too: what they do depends on the line, which the
//! Editor decides, and `OpenLink`. `Editor::apply` ignores variants that M1 left unbound or unimplemented.

use crate::input::KeyEvent;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Insert(String),
    /// Tab key: inserts tab_width spaces in the body; in the title it moves to the body (like Enter).
    /// Not expanded into `Insert` in the keymap, or the title couldn't tell Tab from typed spaces.
    Tab,
    /// Shift+Tab: moves a list item out a level; does nothing elsewhere
    BackTab,
    Paste(String),
    Newline,
    DeleteBack,
    DeleteForward,
    DeleteWordBack,
    /// `bool`: whether to extend the selection (M3)
    Move(Motion, bool),
    Undo,
    Redo,
    Save,
    Quit,
    Cancel,
    SelectAll,
    Copy,
    Cut,
    /// Ctrl+V / vim `p`: paste the internal register (a linewise register pastes onto the next line)
    PasteRegister,
    /// vim `dd`: cut the whole logical line into the register (linewise)
    DeleteLine,
    /// vim `o`: move to the end of the logical line and insert `\n`, as one Transaction
    OpenLineBelow,
    /// vim `O`: insert `\n` at the start of the logical line and stay on the new line above
    OpenLineAbove,
    ToggleTypewriter,
    ToggleFocusMode,
    /// Ctrl+P: open the settings panel; consumed by App
    OpenSettings,
    /// Ctrl+G (nano's ^G help) / `?` in vim Normal mode: open the help panel; consumed by App
    OpenHelp,
    /// Ctrl+L / vim `gx`, Normal-mode `Enter`: open the link under the cursor in the browser;
    /// consumed by App
    OpenLink,
    /// Ctrl+F: open the find bar; consumed by App
    OpenFind,
    /// Ctrl+N / vim `n`: jump to the next match of the find query, wrapping; consumed by App
    FindNext,
    /// Ctrl+B / vim `N`: jump to the previous match, wrapping; consumed by App
    FindPrev,
    /// A key while a status-line input (the save prompt or the find bar) is open
    PromptInput(KeyEvent),
    /// Mouse click / drag: jump straight to a char index; `bool` is whether to extend the selection
    MoveTo(Target, bool),
    /// Mouse wheel: consumed by App / Viewport, never reaches `Editor::apply`
    ScrollView(i32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    LineStart,
    LineEnd,
    PageUp,
    PageDown,
    DocStart,
    DocEnd,
    /// Previous word start (Ctrl+← / vim `b`)
    WordLeft,
    /// Next word start (Ctrl+→ / vim `w`)
    WordRight,
    /// End of the current or next word (bound only to vim `e`)
    WordEnd,
}

/// Mouse target: a document-wide char index in the body, or a char index within the title.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Body(usize),
    Title(usize),
}
