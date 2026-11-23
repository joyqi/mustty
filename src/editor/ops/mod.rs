//! Smart editing operations. Each one is at most two pure functions over the body: an `intent`
//! that turns a key's Action into edits in context, and a `settle` that adds follow-up edits for
//! the lines an edit touched. Both only produce `Splice`s, which `Editor::commit` writes in the
//! same Transaction as their cause, so one key is one undo step.

pub mod list;

use std::ops::Range;

use ropey::Rope;

use crate::action::Action;
use crate::markdown::HighlightCache;

/// Replace the body's chars in `range` (coordinates before the edit) with `text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Splice {
    pub range: Range<usize>,
    pub text: String,
}

/// What a smart key does instead of its plain behaviour. No splices means the key is swallowed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub splices: Vec<Splice>,
    /// The cursor after the edit
    pub head: usize,
}

/// The body around the cursor, with no selection.
pub struct Ctx<'a> {
    pub rope: &'a Rope,
    pub marks: &'a mut HighlightCache,
    pub head: usize,
    pub tab_width: usize,
}

/// The smart behaviour of `action` at the cursor, or None for the plain one.
pub fn intent(action: &Action, cx: &mut Ctx) -> Option<Change> {
    list::intent(action, cx)
}
