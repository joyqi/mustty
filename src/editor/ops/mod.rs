//! Smart editing operations. Each one is at most two pure functions over the body: an `intent`
//! that turns a key's Action into edits in context, and a `settle` that adds follow-up edits for
//! the lines an edit touched. Both only produce `Splice`s, which `Editor::commit` writes in the
//! same Transaction as their cause, so one key is one undo step.

pub mod list;
pub mod task;

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
    list::intent(action, cx).or_else(|| task::intent(action, cx))
}

/// The structure `settle` looks at on some lines, taken before an edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shape {
    lines: Vec<list::Line>,
    /// When the edit removes the first item of a numbered list: the item the list goes on with
    /// (counted in lines from the end of the body, which the edit does not change) and the
    /// number the list started at
    heir: Option<(usize, u32)>,
}

/// `lines` and the chars in `gone` are what the edit is about to replace.
pub fn shape(
    rope: &Rope,
    marks: &mut HighlightCache,
    lines: Range<usize>,
    gone: Range<usize>,
) -> Shape {
    let heir = list::heir(rope, marks, gone).map(|(l, n)| (rope.len_lines() - l, n));
    Shape {
        lines: list::shape(rope, marks, lines),
        heir,
    }
}

/// Follow-up edits for `lines` (after the edit) that were `before` before it: when the edit
/// changed the lists there, renumber the blocks it touched. Coordinates are after the edit.
pub fn settle(
    rope: &Rope,
    marks: &mut HighlightCache,
    lines: Range<usize>,
    before: &Shape,
) -> Vec<Splice> {
    if list::shape(rope, marks, lines.clone()) == before.lines {
        return Vec::new();
    }
    let start = before.heir.map(|(back, n)| (rope.len_lines() - back, n));
    let end = start.map_or(lines.end, |(l, _)| lines.end.max(l + 1));
    list::renumber(rope, marks, lines.start..end, start)
}

/// Where `pos` ends up after `splices` (non-overlapping, in coordinates before them). A position
/// inside a replaced range stays inside the replacement.
pub fn map(pos: usize, splices: &[Splice]) -> usize {
    let mut out = pos;
    for s in splices {
        let len = s.text.chars().count();
        if s.range.end <= pos {
            out = out + len - s.range.len();
        } else if s.range.start < pos {
            out = out - (pos - s.range.start) + (pos - s.range.start).min(len);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_moves_a_position_past_splices() {
        let s = |range: Range<usize>, text: &str| Splice {
            range,
            text: text.into(),
        };
        let fixes = [s(0..1, "10"), s(5..6, ""), s(8..10, "x")];
        assert_eq!(map(0, &fixes), 0);
        assert_eq!(map(1, &fixes), 2);
        assert_eq!(map(5, &fixes), 6);
        assert_eq!(map(7, &fixes), 7);
        assert_eq!(
            map(9, &fixes),
            9,
            "inside a replacement: as far in, up to its length"
        );
        assert_eq!(map(10, &fixes), 9);
        assert_eq!(map(12, &fixes), 11);
    }
}
