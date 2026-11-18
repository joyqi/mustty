//! Text find in the body: the literal smart-case needle, the per-line search, navigation over the
//! matches and the find bar's state machine. The title is never searched.
//!
//! Matches never span lines (the query is a single-line input) and never overlap; both ends sit on
//! grapheme boundaries. Rendering doesn't read the match list: it calls `Needle::line_matches` on
//! each visible line, and the current match is derived from the cursor rather than stored.

use std::borrow::Cow;
use std::ops::Range;

use ropey::Rope;
use unicode_segmentation::GraphemeCursor;

use crate::editor::Focus;
use crate::editor::cursor::Cursor;
use crate::input::{KeyCode, KeyEvent, KeyModifiers};
use crate::status::{LineEdit, LineInput};
use crate::view::viewport::ViewTop;

/// A non-empty query. Smart-case: an all-lowercase query matches any case; one with an uppercase
/// letter matches exactly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Needle {
    /// Already lowercased when `fold`
    text: String,
    fold: bool,
}

/// Lowercases char by char, so every char of the original maps to its own run of the result.
fn fold_char(c: char, out: &mut String) {
    if c.is_ascii() {
        out.push(c.to_ascii_lowercase());
    } else {
        out.extend(c.to_lowercase());
    }
}

impl Needle {
    /// None for an empty query.
    pub fn new(query: &str) -> Option<Self> {
        if query.is_empty() {
            return None;
        }
        let fold = !query.chars().any(char::is_uppercase);
        let text = if fold {
            let mut s = String::with_capacity(query.len());
            query.chars().for_each(|c| fold_char(c, &mut s));
            s
        } else {
            query.to_owned()
        };
        Some(Self { text, fold })
    }

    /// Matches within one line (no `\n`), as char ranges, left to right and non-overlapping.
    pub fn line_matches(&self, line: &str) -> Vec<Range<usize>> {
        let mut out = Vec::new();
        let ascii = line.is_ascii();
        let hay: Cow<str> = match (self.fold, ascii) {
            (false, _) => Cow::Borrowed(line),
            (true, true) => Cow::Owned(line.to_ascii_lowercase()),
            (true, false) => {
                let mut hay = String::with_capacity(line.len());
                line.chars().for_each(|c| fold_char(c, &mut hay));
                Cow::Owned(hay)
            }
        };
        // Most lines have no candidate at all: they cost one substring search
        let Some(first) = hay.find(&self.text) else {
            return out;
        };
        // An ASCII line: bytes are chars, and with no `\n` in a line every char is a cluster
        if ascii {
            let mut from = first;
            while let Some(i) = hay[from..].find(&self.text) {
                let start = from + i;
                from = start + self.text.len();
                out.push(start..from);
            }
            return out;
        }
        // For each char of `line`, plus the end: where it starts in `hay`, and in `line`. Folding
        // may turn one char into several (`İ` → `i̇`), so a match must start and end on one of
        // the `hay` offsets
        let mut starts = Vec::with_capacity(line.len() + 1);
        let mut bytes = Vec::with_capacity(line.len() + 1);
        let mut at = 0;
        for (b, c) in line.char_indices() {
            starts.push(at);
            bytes.push(b);
            at += if self.fold && !c.is_ascii() {
                c.to_lowercase().map(char::len_utf8).sum()
            } else {
                c.len_utf8()
            };
        }
        starts.push(at);
        bytes.push(line.len());
        let char_at = |b: usize| starts.binary_search(&b).ok();
        // The whole line is the chunk, so the cursor never asks for more context
        let on_cluster = |c: usize| {
            GraphemeCursor::new(bytes[c], line.len(), true).is_boundary(line, 0) == Ok(true)
        };
        let mut found = Some(first);
        while let Some(b) = found {
            let end = b + self.text.len();
            let from = match (char_at(b), char_at(end)) {
                (Some(s), Some(e)) if on_cluster(s) && on_cluster(e) => {
                    out.push(s..e);
                    end
                }
                // Not a whole run of chars or clusters: try again one char further on
                _ => b + hay[b..].chars().next().map_or(1, char::len_utf8),
            };
            found = hay[from..].find(&self.text).map(|i| from + i);
        }
        out
    }
}

/// Every match in the body, as document-wide char ranges in ascending order.
pub fn search(needle: &Needle, rope: &Rope) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    for line in rope.lines() {
        let len = line.len_chars();
        let text = Cow::<str>::from(line);
        let text = text.strip_suffix('\n').unwrap_or(&text);
        out.extend(
            needle
                .line_matches(text)
                .into_iter()
                .map(|m| start + m.start..start + m.end),
        );
        start += len;
    }
    out
}

/// Where find was opened; Esc puts all of it back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Origin {
    pub cursor: Cursor,
    pub focus: Focus,
    pub title_cursor: usize,
    pub top: ViewTop,
}

impl Origin {
    /// The body position incremental search starts from: the cursor, or the start of the body
    /// when find was opened from the title.
    pub fn pos(&self) -> usize {
        match self.focus {
            Focus::Body => self.cursor.head,
            Focus::Title => 0,
        }
    }
}

/// The open find bar.
#[derive(Debug)]
pub struct Bar {
    pub line: LineInput,
    pub origin: Origin,
}

/// What a key in the find bar asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Ignored,
    /// The query changed; the needle is already rebuilt
    Edited,
    Next,
    Prev,
    /// Enter: accept if there is a current match
    Accept,
    /// Ctrl+F: same as Esc
    Cancel,
}

/// What the body renders: the needle to search each visible line with, and the cursor position
/// that marks the current match.
#[derive(Debug, Clone, Copy)]
pub struct Marks<'a> {
    pub needle: &'a Needle,
    /// The body cursor (None when the title has focus)
    pub caret: Option<usize>,
}

#[derive(Debug, Default)]
pub struct Find {
    /// Some while the find bar is open
    pub bar: Option<Bar>,
    /// The last accepted query; FindNext / FindPrev use it once the bar is closed
    query: String,
    /// The query in effect: the bar's input while it's open, `query` otherwise
    needle: Option<Needle>,
    /// Every match of `needle` in the body at `revision`, ascending
    matches: Vec<Range<usize>>,
    /// None: `matches` needs recomputing
    revision: Option<u64>,
    /// Whether the matches are highlighted
    shown: bool,
}

impl Find {
    /// Opens the bar with an empty input.
    pub fn open(&mut self, origin: Origin) {
        self.bar = Some(Bar {
            line: LineInput::default(),
            origin,
        });
        self.set_needle(None);
        self.shown = true;
    }

    fn set_needle(&mut self, needle: Option<Needle>) {
        if needle != self.needle {
            self.needle = needle;
            self.revision = None;
        }
    }

    /// One key while the bar is open (see the find bar table in the design).
    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        let Some(bar) = &mut self.bar else {
            return Outcome::Ignored;
        };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL)
            && !key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Char('n' | 'N') if ctrl => return Outcome::Next,
            KeyCode::Char('b' | 'B') if ctrl => return Outcome::Prev,
            KeyCode::Char('f' | 'F') if ctrl => return Outcome::Cancel,
            _ => {}
        }
        match bar.line.edit(key) {
            LineEdit::Submit => Outcome::Accept,
            LineEdit::Changed => {
                let needle = Needle::new(&bar.line.text);
                self.set_needle(needle);
                Outcome::Edited
            }
            LineEdit::Unchanged => Outcome::Ignored,
        }
    }

    /// Pastes into the bar; the caller treats it like `Outcome::Edited`.
    pub fn paste(&mut self, s: &str) {
        if let Some(bar) = &mut self.bar {
            bar.line.paste(s);
            let needle = Needle::new(&bar.line.text);
            self.set_needle(needle);
        }
    }

    /// Recomputes the matches if the needle or the body changed since the last time.
    pub fn sync(&mut self, rope: &Rope, revision: u64) {
        if self.revision == Some(revision) {
            return;
        }
        self.matches = self
            .needle
            .as_ref()
            .map_or_else(Vec::new, |n| search(n, rope));
        self.revision = Some(revision);
    }

    /// The first match starting at or after `pos`, wrapping to the first.
    pub fn first_from(&self, pos: usize) -> Option<usize> {
        let i = self.matches.partition_point(|m| m.start < pos);
        self.matches
            .get(i)
            .or(self.matches.first())
            .map(|m| m.start)
    }

    /// The first match starting after `head`, wrapping to the first.
    pub fn next(&self, head: usize) -> Option<usize> {
        let i = self.matches.partition_point(|m| m.start <= head);
        self.matches
            .get(i)
            .or(self.matches.first())
            .map(|m| m.start)
    }

    /// The last match starting before `head`, wrapping to the last.
    pub fn prev(&self, head: usize) -> Option<usize> {
        let i = self.matches.partition_point(|m| m.start < head);
        i.checked_sub(1)
            .and_then(|i| self.matches.get(i))
            .or(self.matches.last())
            .map(|m| m.start)
    }

    /// 1-based number of the match starting exactly at `head`.
    pub fn position(&self, head: usize) -> Option<usize> {
        self.matches
            .binary_search_by_key(&head, |m| m.start)
            .ok()
            .map(|i| i + 1)
    }

    pub fn len(&self) -> usize {
        self.matches.len()
    }

    pub fn is_empty(&self) -> bool {
        self.matches.is_empty()
    }

    /// Enter: the bar's input becomes the accepted query and the bar closes.
    pub fn accept(&mut self) {
        if let Some(bar) = self.bar.take() {
            self.query = bar.line.text;
        }
    }

    /// Esc: closes the bar, falls back to the accepted query and hides the highlight. Returns
    /// where find was opened.
    pub fn cancel(&mut self) -> Option<Origin> {
        let bar = self.bar.take()?;
        self.set_needle(Needle::new(&self.query));
        self.shown = false;
        Some(bar.origin)
    }

    pub fn show(&mut self) {
        self.shown = true;
    }

    pub fn hide(&mut self) {
        self.shown = false;
    }

    /// The query in effect (the bar's input while it's open).
    pub fn query(&self) -> &str {
        match &self.bar {
            Some(bar) => &bar.line.text,
            None => &self.query,
        }
    }

    /// What the body highlights, if anything.
    pub fn marks(&self, caret: Option<usize>) -> Option<Marks<'_>> {
        let needle = self.needle.as_ref().filter(|_| self.shown)?;
        Some(Marks { needle, caret })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::grapheme::graphemes;
    use crate::text::wrap::line_text;
    use proptest::prelude::*;

    fn find(query: &str, line: &str) -> Vec<Range<usize>> {
        Needle::new(query).unwrap().line_matches(line)
    }

    #[test]
    fn line_matches_literal_and_non_overlapping() {
        assert_eq!(find("ab", "xabyab"), vec![1..3, 4..6]);
        assert_eq!(
            find("aa", "aaa"),
            vec![0..2],
            "non-overlapping, left to right"
        );
        assert_eq!(find("aa", "aaaa"), vec![0..2, 2..4]);
        assert_eq!(find("春天", "一个春天，又一个春天"), vec![2..4, 8..10]);
        assert_eq!(find("天a", "春天a天A"), vec![1..3, 3..5]);
        assert_eq!(find(".", "a.b"), vec![1..2], "no regex");
        assert_eq!(find("x", ""), vec![]);
        assert_eq!(find("abcd", "abc"), vec![], "query longer than the line");
        assert!(Needle::new("").is_none());
    }

    #[test]
    fn smart_case() {
        assert_eq!(find("us", "US us Us"), vec![0..2, 3..5, 6..8]);
        assert_eq!(find("US", "US us Us"), vec![0..2]);
        assert_eq!(find("Us", "US us Us"), vec![6..8]);
        // Non-ASCII case folds too
        assert_eq!(find("éa", "ÉA"), vec![0..2]);
    }

    #[test]
    fn folding_that_changes_length() {
        // İ lowercases to two chars (i + combining dot): matches map back to whole original chars
        assert_eq!(find("i̇x", "İx"), vec![0..2]);
        assert_eq!(find("x", "İx"), vec![1..2]);
        assert_eq!(
            find("i", "İ"),
            vec![],
            "half of a folded char doesn't match"
        );
        assert_eq!(find("ab", "İab"), vec![1..3]);
    }

    #[test]
    fn grapheme_boundaries() {
        // e + combining acute is one cluster: `e` alone doesn't match it
        assert_eq!(find("e", "e\u{301}e"), vec![2..3]);
        assert_eq!(find("e\u{301}", "e\u{301}e"), vec![0..2]);
        // ZWJ family: its members don't match, the whole sequence does
        let family = "👨\u{200d}👩\u{200d}👧";
        assert_eq!(find("👩", family), vec![]);
        assert_eq!(find(family, &format!("a{family}")), vec![1..6]);
        // A failed candidate doesn't hide a valid match that overlaps it
        assert_eq!(
            find("ee", "ee\u{301}e"),
            vec![],
            "e e\u{301}: second e is a cluster start"
        );
        assert_eq!(find("ab", "aab"), vec![1..3]);
    }

    #[test]
    fn search_whole_body() {
        let n = Needle::new("ab").unwrap();
        let rope = Rope::from_str("ab\nxab\n\nab");
        assert_eq!(search(&n, &rope), vec![0..2, 4..6, 8..10]);
        let rope = Rope::from_str("a\nb");
        assert_eq!(search(&n, &rope), vec![], "matches don't span lines");
        assert_eq!(search(&n, &Rope::new()), vec![]);
    }

    fn with_matches(query: &str, body: &str) -> Find {
        let mut f = Find::default();
        f.set_needle(Needle::new(query));
        f.sync(&Rope::from_str(body), 0);
        f
    }

    #[test]
    fn navigation_wraps() {
        // Matches start at 2, 6, 10
        let f = with_matches("ab", "..ab..ab..ab");
        assert_eq!(f.len(), 3);
        for (pos, first, next, prev) in [
            (0, 2, 2, 10), // before the first
            (2, 2, 6, 10), // on the first
            (3, 6, 6, 2),  // inside the first
            (6, 6, 10, 2),
            (10, 10, 2, 6), // on the last: next wraps
            (11, 2, 2, 10), // after the last
        ] {
            assert_eq!(f.first_from(pos), Some(first), "first_from({pos})");
            assert_eq!(f.next(pos), Some(next), "next({pos})");
            assert_eq!(f.prev(pos), Some(prev), "prev({pos})");
        }
        assert_eq!(f.position(6), Some(2));
        assert_eq!(f.position(7), None);
        // A single match comes back to itself
        let one = with_matches("ab", "xab");
        assert_eq!((one.next(1), one.prev(1)), (Some(1), Some(1)));
        // No matches
        let none = with_matches("zz", "ab");
        assert!(none.is_empty());
        assert_eq!(
            (
                none.first_from(0),
                none.next(0),
                none.prev(0),
                none.position(0)
            ),
            (None, None, None, None)
        );
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn origin() -> Origin {
        Origin {
            cursor: Cursor::default(),
            focus: Focus::Body,
            title_cursor: 0,
            top: ViewTop::default(),
        }
    }

    #[test]
    fn bar_keys() {
        let mut f = Find::default();
        assert_eq!(
            f.key(key(KeyCode::Char('a'))),
            Outcome::Ignored,
            "bar closed"
        );
        f.open(origin());
        let cases = [
            (key(KeyCode::Char('a')), Outcome::Edited),
            (key(KeyCode::Char('b')), Outcome::Edited),
            (key(KeyCode::Left), Outcome::Ignored),
            (key(KeyCode::Right), Outcome::Ignored),
            (key(KeyCode::Home), Outcome::Ignored),
            (key(KeyCode::Delete), Outcome::Edited),
            (key(KeyCode::End), Outcome::Ignored),
            (key(KeyCode::Backspace), Outcome::Edited),
            (key(KeyCode::Enter), Outcome::Accept),
            (ctrl('n'), Outcome::Next),
            (ctrl('b'), Outcome::Prev),
            (ctrl('f'), Outcome::Cancel),
            (key(KeyCode::Up), Outcome::Ignored),
            (key(KeyCode::Down), Outcome::Ignored),
            (key(KeyCode::Tab), Outcome::Ignored),
            (ctrl('s'), Outcome::Ignored),
        ];
        for (k, want) in cases {
            assert_eq!(f.key(k), want, "{k:?}");
        }
        assert_eq!(f.query(), "");
        assert!(f.bar.is_some(), "the bar only reports; the App closes it");
    }

    #[test]
    fn accept_and_cancel() {
        let rope = Rope::from_str("one two one");
        let mut f = Find::default();
        f.open(origin());
        f.paste("one");
        f.sync(&rope, 0);
        assert_eq!(f.len(), 2);
        assert!(f.marks(None).is_some(), "shown while the bar is open");
        f.accept();
        assert!(f.bar.is_none());
        assert_eq!(f.query(), "one");
        assert!(f.marks(Some(0)).is_some(), "still shown after Enter");
        // Opened again and cancelled: back to the accepted query, highlight hidden
        f.open(origin());
        assert!(f.marks(None).is_none(), "empty input has no needle");
        f.paste("tw\no");
        assert_eq!(f.query(), "tw o", "newlines become spaces");
        f.sync(&rope, 0);
        assert_eq!(f.len(), 0);
        assert_eq!(f.cancel(), Some(origin()));
        assert_eq!(f.query(), "one");
        assert!(f.marks(None).is_none());
        f.sync(&rope, 0);
        assert_eq!(f.len(), 2, "the accepted query is in effect again");
        f.show();
        assert!(f.marks(None).is_some());
        f.hide();
        assert!(f.marks(None).is_none());
        assert_eq!(f.cancel(), None, "nothing to cancel");
    }

    #[test]
    fn sync_rescans_only_on_change() {
        let mut f = with_matches("a", "a a");
        assert_eq!(f.len(), 2);
        // Same revision: the new text isn't scanned
        f.sync(&Rope::from_str("a a a"), 0);
        assert_eq!(f.len(), 2);
        f.sync(&Rope::from_str("a a a"), 1);
        assert_eq!(f.len(), 3);
        // A new needle forces a rescan even at the same revision
        f.set_needle(Needle::new("aa"));
        f.sync(&Rope::from_str("aa"), 1);
        assert_eq!(f.len(), 1);
    }

    fn fold(s: &str) -> String {
        let mut out = String::new();
        s.chars().for_each(|c| fold_char(c, &mut out));
        out
    }

    fn boundaries(line: &str) -> Vec<usize> {
        let mut b: Vec<usize> = graphemes(line).map(|(c, _)| c).collect();
        b.push(line.chars().count());
        b
    }

    proptest! {
        #[test]
        fn search_is_sound(
            query in prop_oneof![".{1,3}", "[aAİie\u{301}\u{200d}👩]{1,3}"],
            body in prop_oneof![".{0,40}", "[aAİie\u{301}\u{200d}👩\n ]{0,40}"],
        ) {
            let needle = Needle::new(&query).unwrap();
            let rope = Rope::from_str(&body);
            let all = search(&needle, &rope);
            let chars: Vec<char> = body.chars().collect();
            let mut last_end = 0;
            for m in &all {
                prop_assert!(m.start < m.end && m.end <= chars.len());
                prop_assert!(m.start >= last_end, "ascending, non-overlapping");
                last_end = m.end;
                let line = rope.char_to_line(m.start);
                prop_assert_eq!(rope.char_to_line(m.end - 1), line, "within one line");
                let ls = rope.line_to_char(line);
                let lt = line_text(&rope, line);
                let b = boundaries(&lt);
                prop_assert!(b.contains(&(m.start - ls)) && b.contains(&(m.end - ls)));
                let text: String = chars[m.clone()].iter().collect();
                if needle.fold {
                    prop_assert_eq!(fold(&text), fold(&query));
                } else {
                    prop_assert_eq!(&text, &query);
                }
            }
            // A line that is exactly the query always matches at its start
            let own = needle.line_matches(&query);
            prop_assert_eq!(own.first(), Some(&(0..query.chars().count())));
        }

        #[test]
        fn navigation_lands_on_matches_and_cycles(
            body in "[ab\n]{0,30}",
            query in "[ab]{1,2}",
            head in 0usize..40,
        ) {
            let f = with_matches(&query, &body);
            let starts: Vec<usize> = f.matches.iter().map(|m| m.start).collect();
            for got in [f.next(head), f.prev(head), f.first_from(head)].into_iter().flatten() {
                prop_assert!(starts.contains(&got));
            }
            for &m in &starts {
                let mut at = m;
                let mut seen = Vec::new();
                for _ in 0..f.len() {
                    at = f.next(at).unwrap();
                    seen.push(at);
                }
                prop_assert_eq!(at, m, "len() steps of next come back");
                seen.sort_unstable();
                prop_assert_eq!(&seen, &starts, "each match visited once");
                let mut at = m;
                for _ in 0..f.len() {
                    at = f.prev(at).unwrap();
                }
                prop_assert_eq!(at, m);
                prop_assert_eq!(f.prev(f.next(m).unwrap()), Some(m));
            }
        }
    }
}
