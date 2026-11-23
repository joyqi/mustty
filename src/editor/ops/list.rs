//! Markdown lists. A line is a list item when the highlighter marks a list marker at its start, so
//! lines in fenced code, rules and quoted lines never count; `markdown::list_marker` then splits
//! the marker into its parts. Nesting compares indent widths: an item's parent is the nearest
//! item above it with a smaller indent.

use std::ops::Range;

use ropey::Rope;

use crate::markdown::{HighlightCache, ListMarker, MarkerKind, Token, list_marker};
use crate::text::wrap::line_text;

/// How a line takes part in a list block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    Item(ListMarker),
    /// Indented with spaces and not an item: an item's continuation text or nested code
    Indented,
    /// Empty or only whitespace
    Blank,
    /// Anything else ends a list block
    Other,
}

/// The list item on `line`, if it is one.
pub fn item_at(rope: &Rope, marks: &mut HighlightCache, line: usize) -> Option<ListMarker> {
    let at_start = marks
        .ranges(rope, line)
        .iter()
        .any(|r| r.token == Token::ListMark && r.start == 0);
    if !at_start {
        return None;
    }
    let chars: Vec<char> = line_text(rope, line).chars().collect();
    list_marker(&chars, 0)
}

pub fn classify(rope: &Rope, marks: &mut HighlightCache, line: usize) -> Line {
    if let Some(m) = item_at(rope, marks, line) {
        return Line::Item(m);
    }
    let text = line_text(rope, line);
    if text.trim().is_empty() {
        Line::Blank
    } else if text.starts_with(' ') {
        Line::Indented
    } else {
        Line::Other
    }
}

/// The lines' classes, in order.
pub fn shape(rope: &Rope, marks: &mut HighlightCache, lines: Range<usize>) -> Vec<Line> {
    lines.map(|l| classify(rope, marks, l)).collect()
}

/// Leading spaces of a line.
fn indent_of(rope: &Rope, line: usize) -> usize {
    rope.line(line).chars().take_while(|&c| c == ' ').count()
}

/// Walks up from the line above `line` to the first item whose indent `stop` accepts, passing
/// deeper items, continuation text and blank lines; any other line ends the search.
fn item_above(
    rope: &Rope,
    marks: &mut HighlightCache,
    line: usize,
    stop: impl Fn(usize) -> bool,
) -> Option<(usize, ListMarker)> {
    for l in (0..line).rev() {
        match classify(rope, marks, l) {
            Line::Item(m) if stop(m.indent) => return Some((l, m)),
            Line::Other => return None,
            _ => {}
        }
    }
    None
}

/// The previous item at exactly `indent` under the same parent. For a line about to move to
/// `indent`, this is the item it will follow in that level's list.
pub fn prev_sibling(
    rope: &Rope,
    marks: &mut HighlightCache,
    line: usize,
    indent: usize,
) -> Option<(usize, ListMarker)> {
    item_above(rope, marks, line, |i| i <= indent).filter(|(_, m)| m.indent == indent)
}

/// The nearest item above with a smaller indent.
pub fn parent(
    rope: &Rope,
    marks: &mut HighlightCache,
    line: usize,
    indent: usize,
) -> Option<(usize, ListMarker)> {
    item_above(rope, marks, line, |i| i < indent)
}

/// The end (exclusive) of the item on `line` with its children: the following lines indented
/// deeper than `indent`, and blank lines between them.
pub fn subtree_end(rope: &Rope, line: usize, indent: usize) -> usize {
    let n = rope.len_lines();
    let mut end = line + 1;
    let mut l = end;
    while l < n {
        if line_text(rope, l).trim().is_empty() {
            l += 1;
            continue;
        }
        if indent_of(rope, l) <= indent {
            break;
        }
        l += 1;
        end = l;
    }
    end
}

/// Two markers belong to the same list: the same bullet, or ordered with the same delimiter.
pub fn same_list(a: MarkerKind, b: MarkerKind) -> bool {
    match (a, b) {
        (MarkerKind::Bullet(x), MarkerKind::Bullet(y)) => x == y,
        (MarkerKind::Ordered { delim: x, .. }, MarkerKind::Ordered { delim: y, .. }) => x == y,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str) -> (Rope, HighlightCache) {
        (Rope::from_str(text), HighlightCache::default())
    }

    #[test]
    fn items_need_a_marker_at_the_line_start() {
        let text = "- a\n  1. b\n```\n- code\n```\n> - quoted\n* * *\n- [x] done\npara\n    text\n";
        let (rope, mut hl) = doc(text);
        let kinds: Vec<Line> = (0..rope.len_lines())
            .map(|l| classify(&rope, &mut hl, l))
            .collect();
        let item = |l: usize| matches!(kinds[l], Line::Item(_));
        assert!(item(0) && item(1) && item(7));
        assert_eq!(kinds[2], Line::Other, "fence line");
        assert_eq!(kinds[3], Line::Other, "inside a fence");
        assert_eq!(kinds[5], Line::Other, "quoted item");
        assert_eq!(kinds[6], Line::Other, "rule");
        assert_eq!(kinds[8], Line::Other);
        assert_eq!(kinds[9], Line::Indented);
        assert_eq!(kinds[10], Line::Blank);
        let Line::Item(m) = kinds[1] else {
            unreachable!()
        };
        assert_eq!((m.indent, m.width), (2, 3));
    }

    #[test]
    fn siblings_parents_and_subtrees() {
        let text = "- a\n    - b\n\n      more b\n    - c\n- d\n\npara\n- e\n";
        let (rope, mut hl) = doc(text);
        let line = |found: Option<(usize, ListMarker)>| found.map(|(l, _)| l);
        assert_eq!(line(prev_sibling(&rope, &mut hl, 4, 4)), Some(1));
        assert_eq!(
            line(prev_sibling(&rope, &mut hl, 1, 4)),
            None,
            "first child"
        );
        assert_eq!(line(prev_sibling(&rope, &mut hl, 5, 0)), Some(0));
        assert_eq!(
            line(prev_sibling(&rope, &mut hl, 8, 0)),
            None,
            "para ends it"
        );
        assert_eq!(line(parent(&rope, &mut hl, 4, 4)), Some(0));
        assert_eq!(line(parent(&rope, &mut hl, 0, 0)), None);
        assert_eq!(subtree_end(&rope, 0, 0), 5, "children, blank and more text");
        assert_eq!(subtree_end(&rope, 1, 4), 4);
        assert_eq!(
            subtree_end(&rope, 5, 0),
            6,
            "a trailing blank line stays out"
        );
        assert_eq!(
            line(prev_sibling(&rope, &mut hl, 5, 4)),
            Some(4),
            "a level to join"
        );
        assert!(same_list(MarkerKind::Bullet('-'), MarkerKind::Bullet('-')));
        assert!(!same_list(MarkerKind::Bullet('-'), MarkerKind::Bullet('*')));
    }
}
