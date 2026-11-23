//! Smart editing operations. Each one is at most two pure functions over the body: an `intent`
//! that turns a key's Action into edits in context, and a `settle` that adds follow-up edits for
//! the lines an edit touched. Both only produce `Splice`s, which `Editor::commit` writes in the
//! same Transaction as their cause, so one key is one undo step.

use std::ops::Range;

/// Replace the body's chars in `range` (coordinates before the edit) with `text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Splice {
    pub range: Range<usize>,
    pub text: String,
}
