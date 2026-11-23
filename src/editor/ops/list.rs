//! Markdown lists. A line is a list item when the highlighter marks a list marker at its start, so
//! lines in fenced code, rules and quoted lines never count; `markdown::list_marker` then splits
//! the marker into its parts. Nesting compares indent widths: an item's parent is the nearest
//! item above it with a smaller indent.

use std::ops::Range;

use ropey::Rope;

use super::{Change, Ctx, Splice};
use crate::action::Action;
use crate::markdown::{HighlightCache, ListMarker, MarkerKind, Token, list_marker};
use crate::text::wrap::line_text;

/// Enter, Tab, Shift+Tab, Backspace and vim `o` / `O` on a list item.
pub fn intent(action: &Action, cx: &mut Ctx) -> Option<Change> {
    let line = cx.rope.char_to_line(cx.head);
    let m = item_at(cx.rope, cx.marks, line)?;
    let start = cx.rope.line_to_char(line);
    let col = cx.head - start;
    let text: Vec<char> = line_text(cx.rope, line).chars().collect();
    match action {
        Action::Newline if col >= m.end => {
            if text[m.end..].iter().all(|c| c.is_whitespace()) {
                return Some(end_item(cx, line, &m, text.len()));
            }
            // The new item takes the text after the cursor, minus the spaces right after it
            let gap = text[col..].iter().take_while(|&&c| c == ' ').count();
            let insert = format!("\n{}{}", " ".repeat(m.indent), next_marker(&m));
            Some(Change {
                head: cx.head + insert.chars().count(),
                splices: vec![Splice {
                    range: cx.head..cx.head + gap,
                    text: insert,
                }],
            })
        }
        _ => None,
    }
}

/// Enter on an empty item: a nested one moves out a level, a top-level one is cleared, ending
/// the list.
fn end_item(cx: &mut Ctx, line: usize, m: &ListMarker, len: usize) -> Change {
    if m.indent > 0 {
        return outdent(cx, line, m);
    }
    let start = cx.rope.line_to_char(line);
    Change {
        splices: vec![Splice {
            range: start..start + len,
            text: String::new(),
        }],
        head: start,
    }
}

/// Moves the item and its children out to the parent's indent.
fn outdent(cx: &mut Ctx, line: usize, m: &ListMarker) -> Change {
    let target = parent(cx.rope, cx.marks, line, m.indent).map_or(0, |(_, p)| p.indent);
    move_item(cx, line, m, target)
}

/// Moves the item on `line` and its subtree to `target` indent. The item joins the list already
/// at that level next to it, taking its marker; without one it starts a new list with its own
/// kind of marker. Its task box stays as it is.
fn move_item(cx: &mut Ctx, line: usize, m: &ListMarker, target: usize) -> Change {
    let kind = match prev_sibling(cx.rope, cx.marks, line, target) {
        Some((_, near)) => following(near.kind),
        None => match m.kind {
            MarkerKind::Ordered { delim, .. } => MarkerKind::Ordered { number: 1, delim },
            bullet => bullet,
        },
    };
    let start = cx.rope.line_to_char(line);
    let old = m.indent + m.width;
    let prefix = format!("{}{}", " ".repeat(target), marker_text(kind));
    let col = cx.head - start;
    let head = start + prefix.chars().count() + col.saturating_sub(old);
    let mut splices = vec![Splice {
        range: start..start + old,
        text: prefix,
    }];
    for l in line + 1..subtree_end(cx.rope, line, m.indent) {
        if line_text(cx.rope, l).trim().is_empty() {
            continue;
        }
        let at = cx.rope.line_to_char(l);
        splices.push(if target > m.indent {
            Splice {
                range: at..at,
                text: " ".repeat(target - m.indent),
            }
        } else {
            Splice {
                range: at..at + m.indent - target,
                text: String::new(),
            }
        });
    }
    Change { splices, head }
}

/// The marker kind of the item after one of `kind`.
fn following(kind: MarkerKind) -> MarkerKind {
    match kind {
        MarkerKind::Ordered { number, delim } => MarkerKind::Ordered {
            number: number.saturating_add(1),
            delim,
        },
        bullet => bullet,
    }
}

fn marker_text(kind: MarkerKind) -> String {
    match kind {
        MarkerKind::Bullet(c) => format!("{c} "),
        MarkerKind::Ordered { number, delim } => format!("{number}{delim} "),
    }
}

/// The marker for a new item after `m`: the same bullet or the next number, and an unchecked box
/// if `m` has a box.
fn next_marker(m: &ListMarker) -> String {
    let task = if m.task.is_some() { "[ ] " } else { "" };
    format!("{}{task}", marker_text(following(m.kind)))
}

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

/// Fixes the numbers of every ordered list in the blocks that meet `lines`. A block runs from
/// the first item above to the last line before a line that is not an item, indented text, or a
/// blank line followed by either. Each level counts on its own from the number its first item
/// has; a change of bullet or delimiter starts a new list.
pub fn renumber(rope: &Rope, marks: &mut HighlightCache, lines: Range<usize>) -> Vec<Splice> {
    struct Level {
        indent: usize,
        kind: MarkerKind,
        next: u32,
    }
    let start_of = |m: &ListMarker| Level {
        indent: m.indent,
        kind: m.kind,
        next: match m.kind {
            MarkerKind::Ordered { number, .. } => number.saturating_add(1),
            MarkerKind::Bullet(_) => 0,
        },
    };
    let n = rope.len_lines();
    let mut l = lines.start.min(n - 1);
    // Back to the block's start, when the first line is in one
    let mut first = l;
    while first < n && classify(rope, marks, first) == Line::Blank {
        first += 1;
    }
    if first < n && classify(rope, marks, first) != Line::Other {
        while l > 0 && classify(rope, marks, l - 1) != Line::Other {
            l -= 1;
        }
    }
    let mut stack: Vec<Level> = Vec::new();
    let mut out = Vec::new();
    while l < n {
        let class = classify(rope, marks, l);
        let ends = match class {
            Line::Other => true,
            Line::Indented => false,
            Line::Blank => {
                // Blank lines carry the block on only if it resumes after them
                let next = (l + 1..n).find(|&k| classify(rope, marks, k) != Line::Blank);
                match next {
                    Some(k) if classify(rope, marks, k) != Line::Other => {
                        l = k;
                        continue;
                    }
                    _ => true,
                }
            }
            Line::Item(m) => {
                while stack.last().is_some_and(|t| t.indent > m.indent) {
                    stack.pop();
                }
                match stack.last_mut() {
                    Some(top) if top.indent == m.indent && same_list(top.kind, m.kind) => {
                        if let MarkerKind::Ordered { number, .. } = m.kind {
                            if number != top.next {
                                let at = rope.line_to_char(l) + m.indent;
                                out.push(Splice {
                                    range: at..at + m.width - 2,
                                    text: top.next.to_string(),
                                });
                            }
                            top.next = top.next.saturating_add(1);
                        }
                    }
                    Some(top) if top.indent == m.indent => *top = start_of(&m),
                    _ => stack.push(start_of(&m)),
                }
                false
            }
        };
        if ends {
            stack.clear();
            if l + 1 >= lines.end {
                break;
            }
        }
        l += 1;
    }
    out
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

    /// Runs `action` on `before`, where `|` marks the cursor, and returns the text with the new
    /// cursor marked, or None when the key keeps its plain behaviour.
    fn key(before: &str, action: Action) -> Option<String> {
        let head = before
            .find('|')
            .map(|b| before[..b].chars().count())
            .unwrap();
        let mut rope = Rope::from_str(&before.replacen('|', "", 1));
        let mut marks = HighlightCache::default();
        let mut cx = Ctx {
            rope: &rope,
            marks: &mut marks,
            head,
            tab_width: 4,
        };
        let mut change = intent(&action, &mut cx)?;
        change
            .splices
            .sort_by_key(|s| std::cmp::Reverse(s.range.start));
        for s in change.splices {
            rope.remove(s.range.clone());
            rope.insert(s.range.start, &s.text);
        }
        rope.insert_char(change.head, '|');
        Some(rope.to_string())
    }

    fn each(action: Action, cases: &[(&str, Option<&str>)]) {
        for (before, want) in cases {
            let got = key(before, action.clone());
            assert_eq!(got.as_deref(), *want, "{action:?} on {before:?}");
        }
    }

    #[test]
    fn enter_continues_splits_and_ends_items() {
        each(
            Action::Newline,
            &[
                // At the end: a new item with the next marker
                ("- a|", Some("- a\n- |")),
                ("* a|\n* b", Some("* a\n* |\n* b")),
                ("+ a|", Some("+ a\n+ |")),
                ("3. foo|", Some("3. foo\n4. |")),
                ("9) foo|", Some("9) foo\n10) |")),
                ("  - a|", Some("  - a\n  - |")),
                // A checkbox continues unchecked
                ("- [x] done|", Some("- [x] done\n- [ ] |")),
                ("1. [ ] todo|", Some("1. [ ] todo\n2. [ ] |")),
                // In the middle: split, the space after the cursor goes
                ("3. foo| bar", Some("3. foo\n4. |bar")),
                ("- a|b", Some("- a\n- |b")),
                ("- |ab", Some("- \n- |ab")),
                // An empty top-level item ends the list
                ("- a\n- |", Some("- a\n|")),
                ("- a\n- [ ] |", Some("- a\n|")),
                ("1. a\n2. |  ", Some("1. a\n|")),
                // An empty nested item moves out a level, taking the parent list's marker
                ("- a\n    - |", Some("- a\n- |")),
                ("1. a\n    - |", Some("1. a\n2. |")),
                ("- a\n    - b\n        - |", Some("- a\n    - b\n    - |")),
                ("- a\n    - [ ] |", Some("- a\n- [ ] |")),
                // Before or inside the marker, and outside lists: plain Enter
                ("|- a", None),
                ("-| a", None),
                ("- |[ ] a", None),
                ("para|", None),
                ("```\n- a|\n```", None),
                ("> - a|", None),
            ],
        );
    }

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

    /// Renumbers the blocks meeting `lines` of `before`.
    fn renumbered(before: &str, lines: Range<usize>) -> String {
        let mut rope = Rope::from_str(before);
        let mut marks = HighlightCache::default();
        let mut splices = renumber(&rope, &mut marks, lines);
        splices.sort_by_key(|s| std::cmp::Reverse(s.range.start));
        for s in splices {
            rope.remove(s.range.clone());
            rope.insert(s.range.start, &s.text);
        }
        rope.to_string()
    }

    #[test]
    fn renumbering() {
        let cases = [
            // Counts up from the first item, whatever it is
            ("1. a\n1. b\n1. c", "1. a\n2. b\n3. c"),
            ("3. a\n1. b\n7. c", "3. a\n4. b\n5. c"),
            ("1) a\n5) b", "1) a\n2) b"),
            ("8. a\n9. b\n9. c", "8. a\n9. b\n10. c"),
            // Each level on its own; the outer count resumes after a sublist
            (
                "1. a\n    1. x\n    5. y\n4. b\n    - z\n9. c",
                "1. a\n    1. x\n    2. y\n2. b\n    - z\n3. c",
            ),
            // A different delimiter or bullet starts a new list
            ("1. a\n2. b\n5) c\n9) d", "1. a\n2. b\n5) c\n6) d"),
            ("1. a\n- b\n4. c", "1. a\n- b\n4. c"),
            // Loose lists: blank lines and continuation text stay in the block
            ("1. a\n\n   more\n\n1. b", "1. a\n\n   more\n\n2. b"),
            // A paragraph ends the block; the next list counts from its own first number
            (
                "1. a\n1. b\n\npara\n\n4. c\n1. d",
                "1. a\n2. b\n\npara\n\n4. c\n5. d",
            ),
            // A fenced block inside an item belongs to it
            (
                "1. a\n   ```\n   1. code\n   ```\n1. b",
                "1. a\n   ```\n   1. code\n   ```\n2. b",
            ),
            // A fence at column 0 ends the list
            (
                "1. a\n```\n1. code\n```\n1. b",
                "1. a\n```\n1. code\n```\n1. b",
            ),
        ];
        for (before, want) in cases {
            let last = before.lines().count();
            assert_eq!(renumbered(before, 0..last), want, "{before:?}");
        }
        // Only the blocks that meet the lines change
        let text = "1. a\n1. b\n\npara\n\n1. c\n1. d";
        assert_eq!(renumbered(text, 6..7), "1. a\n1. b\n\npara\n\n1. c\n2. d");
        assert_eq!(renumbered(text, 3..4), text);
        assert_eq!(
            renumbered("1. a\n1. b\n\n\npara", 3..4),
            "1. a\n1. b\n\n\npara"
        );
        // A blank line that the block resumes after counts on from above
        assert_eq!(renumbered("1. a\n\n1. b", 1..2), "1. a\n\n2. b");
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
