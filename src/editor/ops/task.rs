//! Task boxes (`Ctrl+D`). On a task item the box is checked or unchecked; a list item without a
//! box gets one; any other line becomes an unchecked task item. Lines in fenced code or a quote
//! are left alone, like the scanner, which does not count a box there as a task.

use super::{Change, Ctx, Splice};
use crate::action::Action;
use crate::editor::ops::list::item_at;
use crate::markdown::Token;
use crate::text::wrap::line_text;

pub fn intent(action: &Action, cx: &mut Ctx) -> Option<Change> {
    if *action != Action::ToggleTask {
        return None;
    }
    let line = cx.rope.char_to_line(cx.head);
    let start = cx.rope.line_to_char(line);
    let (splice, at_least) = match item_at(cx.rope, cx.marks, line) {
        Some(m) => {
            let at = start + m.indent + m.width;
            let splice = match m.task {
                // The mark between the brackets
                Some(done) => Splice {
                    range: at + 1..at + 2,
                    text: if done { " " } else { "x" }.into(),
                },
                None => Splice {
                    range: at..at,
                    text: "[ ] ".into(),
                },
            };
            (splice, start)
        }
        None if in_code_or_quote(cx, line) => {
            return Some(Change {
                splices: Vec::new(),
                head: cx.head,
            });
        }
        None => {
            // After the line's indent, so an indented line becomes a nested item
            let indent = line_text(cx.rope, line)
                .chars()
                .take_while(|&c| c == ' ')
                .count();
            let at = start + indent;
            let text = "- [ ] ";
            let splice = Splice {
                range: at..at,
                text: text.into(),
            };
            // A cursor in the indent goes after the box; one in the text keeps its place
            (splice, at + text.len())
        }
    };
    let head = super::map(cx.head, std::slice::from_ref(&splice)).max(at_least);
    Some(Change {
        splices: vec![splice],
        head,
    })
}

fn in_code_or_quote(cx: &mut Ctx, line: usize) -> bool {
    cx.marks.ranges(cx.rope, line).iter().any(|r| {
        matches!(
            r.token,
            Token::FenceLine | Token::FencedCode | Token::QuoteMark
        )
    })
}

#[cfg(test)]
mod tests {
    use ropey::Rope;

    use super::*;
    use crate::markdown::HighlightCache;

    /// Runs Ctrl+D on `before`, where `|` marks the cursor, and returns the text with the new
    /// cursor marked.
    fn toggle(before: &str) -> String {
        let head = before[..before.find('|').unwrap()].chars().count();
        let mut rope = Rope::from_str(&before.replacen('|', "", 1));
        let mut marks = HighlightCache::default();
        let mut cx = Ctx {
            rope: &rope,
            marks: &mut marks,
            head,
            tab_width: 4,
        };
        let change = intent(&Action::ToggleTask, &mut cx).expect("Ctrl+D is always handled");
        for s in change.splices.iter().rev() {
            rope.remove(s.range.clone());
            rope.insert(s.range.start, &s.text);
        }
        rope.insert_char(change.head, '|');
        rope.to_string()
    }

    fn each(cases: &[(&str, &str)]) {
        for (before, want) in cases {
            assert_eq!(toggle(before), *want, "Ctrl+D on {before:?}");
        }
    }

    #[test]
    fn toggles_a_task_box() {
        each(&[
            ("- [ ] a|", "- [x] a|"),
            ("- [x] a|", "- [ ] a|"),
            ("- [X] a|", "- [ ] a|"),
            ("|* [ ] a", "|* [x] a"),
            ("+ [ ] a|", "+ [x] a|"),
            ("12. [ ] a|", "12. [x] a|"),
            ("1) [x] |a", "1) [ ] |a"),
            ("- a\n    - [ ] b|\n- c", "- a\n    - [x] b|\n- c"),
            ("- [ ]|", "- [x]|"),
            ("- [|x] a", "- [| ] a"),
        ]);
    }

    #[test]
    fn gives_a_list_item_a_box() {
        each(&[
            ("- a|", "- [ ] a|"),
            ("- |a", "- [ ] |a"),
            ("|- a", "|- [ ] a"),
            ("* a|", "* [ ] a|"),
            ("3. a|", "3. [ ] a|"),
            ("  - nested|", "  - [ ] nested|"),
            ("- |", "- [ ] |"),
            // A bracket pair that is not a box is the item's text
            ("- [y] a|", "- [ ] [y] a|"),
        ]);
    }

    #[test]
    fn makes_any_other_line_a_task_item() {
        each(&[
            ("|", "- [ ] |"),
            ("a\n|\nb", "a\n- [ ] |\nb"),
            ("text|", "- [ ] text|"),
            ("te|xt", "- [ ] te|xt"),
            ("|text", "- [ ] |text"),
            ("  |text", "  - [ ] |text"),
            ("|  text", "  - [ ] |text"),
            ("# Heading|", "- [ ] # Heading|"),
            ("1.x|", "- [ ] 1.x|"),
        ]);
    }

    #[test]
    fn leaves_code_and_quotes_alone() {
        each(&[
            ("```\n- [ ] a|\n```", "```\n- [ ] a|\n```"),
            ("```\ncode|\n```", "```\ncode|\n```"),
            ("```|\n```", "```|\n```"),
            ("~~~\n|\n~~~", "~~~\n|\n~~~"),
            ("> - [ ] a|", "> - [ ] a|"),
            ("> quote|", "> quote|"),
            (">|", ">|"),
        ]);
    }

    #[test]
    fn other_keys_pass_through() {
        let rope = Rope::from_str("- [ ] a");
        let mut marks = HighlightCache::default();
        let mut cx = Ctx {
            rope: &rope,
            marks: &mut marks,
            head: 7,
            tab_width: 4,
        };
        assert_eq!(intent(&Action::Insert("x".into()), &mut cx), None);
    }
}
