//! Editor：Document + Cursor + Focus 的组合与编辑 API。

pub mod cursor;
pub mod document;

use std::ops::Range;

use crate::action::{Action, Motion};
use crate::text::grapheme::{next_boundary, prev_boundary};
use crate::text::wrap::WrapCache;
use cursor::Cursor;
use document::Document;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Title,
    Body,
}

pub struct Editor {
    pub doc: Document,
    pub cursor: Cursor,
    /// 标题内的 char 索引。
    pub title_cursor: usize,
    pub focus: Focus,
    saved_revision: u64,
}

impl Editor {
    pub fn new(doc: Document) -> Self {
        let saved_revision = doc.revision();
        Self {
            doc,
            cursor: Cursor::default(),
            title_cursor: 0,
            focus: Focus::Body,
            saved_revision,
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.doc.revision() != self.saved_revision
    }

    pub fn mark_saved(&mut self) {
        self.saved_revision = self.doc.revision();
    }

    /// 执行一个编辑 / 移动动作。`page` 为 PageUp/PageDown 的视觉行数。
    /// 保存、退出、提示行等由 App 处理；M2+ 的动作在此忽略。
    pub fn apply(&mut self, action: &Action, wrap: &mut WrapCache, page: isize) {
        match self.focus {
            Focus::Title => self.apply_title(action),
            Focus::Body => self.apply_body(action, wrap, page),
        }
    }

    fn apply_body(&mut self, action: &Action, wrap: &mut WrapCache, page: isize) {
        let rope = &self.doc.body;
        let head = self.cursor.head;
        match action {
            Action::Insert(s) => self.insert(s, wrap),
            Action::Newline => self.insert("\n", wrap),
            Action::DeleteBack if head > 0 => {
                let prev = Cursor::prev(rope, head);
                self.replace(prev..head, "", wrap);
                self.cursor.set(prev);
            }
            Action::DeleteForward if head < rope.len_chars() => {
                let next = Cursor::next(rope, head);
                self.replace(head..next, "", wrap);
                self.cursor.set(head);
            }
            Action::Move(motion, _) => match motion {
                Motion::Left => self.cursor.move_left(rope),
                Motion::Right => self.cursor.move_right(rope),
                Motion::Up => {
                    let pos = self.cursor.visual(rope, wrap);
                    if (pos.line, pos.row) == (0, 0) {
                        self.focus = Focus::Title;
                        self.title_cursor = self.doc.title_text().chars().count();
                    } else {
                        self.cursor.move_vertical(rope, wrap, -1);
                    }
                }
                Motion::Down => self.cursor.move_vertical(rope, wrap, 1),
                Motion::PageUp => self.cursor.move_vertical(rope, wrap, -page),
                Motion::PageDown => self.cursor.move_vertical(rope, wrap, page),
                Motion::LineStart => self.cursor.move_row_start(rope, wrap),
                Motion::LineEnd => self.cursor.move_row_end(rope, wrap),
                Motion::DocStart => self.cursor.set(0),
                Motion::DocEnd => self.cursor.set(rope.len_chars()),
                Motion::WordLeft | Motion::WordRight | Motion::WordEnd => {}
            },
            _ => {}
        }
    }

    fn insert(&mut self, s: &str, wrap: &mut WrapCache) {
        let head = self.cursor.head;
        self.replace(head..head, s, wrap);
        self.cursor.set(head + s.chars().count());
    }

    /// 写正文并让受影响的逻辑行换行缓存失效。
    fn replace(&mut self, range: Range<usize>, text: &str, wrap: &mut WrapCache) {
        let rope = &self.doc.body;
        let lines = rope.char_to_line(range.start)..rope.char_to_line(range.end) + 1;
        let before = rope.len_lines();
        self.doc.replace(range, text);
        wrap.invalidate(lines, self.doc.body.len_lines() as isize - before as isize);
    }

    fn apply_title(&mut self, action: &Action) {
        let text = self.doc.title_text();
        let len = text.chars().count();
        let at = self.title_cursor;
        let byte = |i: usize| text.char_indices().nth(i).map_or(text.len(), |(b, _)| b);
        match action {
            Action::Insert(s) => {
                // 标题是单行
                let s = s.replace('\n', " ");
                let mut new = text.to_owned();
                new.insert_str(byte(at), &s);
                self.title_cursor += s.chars().count();
                self.doc.set_title(new);
            }
            Action::DeleteBack if at > 0 => {
                let prev = prev_boundary(text, at);
                let new = [&text[..byte(prev)], &text[byte(at)..]].concat();
                self.title_cursor = prev;
                self.doc.set_title(new);
            }
            Action::DeleteForward if at < len => {
                let next = next_boundary(text, at);
                let new = [&text[..byte(at)], &text[byte(next)..]].concat();
                self.doc.set_title(new);
            }
            Action::Newline | Action::Move(Motion::Down | Motion::DocStart, _) => {
                self.focus = Focus::Body;
                self.cursor.set(0);
            }
            Action::Move(Motion::DocEnd, _) => {
                self.focus = Focus::Body;
                self.cursor.set(self.doc.body.len_chars());
            }
            Action::Move(Motion::Left, _) => self.title_cursor = prev_boundary(text, at),
            Action::Move(Motion::Right, _) => self.title_cursor = next_boundary(text, at),
            Action::Move(Motion::LineStart, _) => self.title_cursor = 0,
            Action::Move(Motion::LineEnd, _) => self.title_cursor = len,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use document::FileMeta;

    fn editor(raw: &str) -> (Editor, WrapCache) {
        let doc = Document::parse(raw, FileMeta::new(None));
        (Editor::new(doc), WrapCache::new(20))
    }

    fn run(ed: &mut Editor, wrap: &mut WrapCache, actions: &[Action]) {
        for a in actions {
            ed.apply(a, wrap, 5);
        }
    }

    fn ins(s: &str) -> Action {
        Action::Insert(s.into())
    }

    #[test]
    fn insert_and_delete_by_grapheme() {
        let (mut ed, mut wrap) = editor("");
        let clusters = ["e\u{301}", "👨\u{200d}👩\u{200d}👧", "🇨🇳", "中"];
        for c in clusters {
            // IME / 终端可能逐个 char 送达
            for ch in c.chars() {
                ed.apply(&ins(&ch.to_string()), &mut wrap, 5);
            }
        }
        assert_eq!(ed.doc.body.to_string(), clusters.concat());
        assert!(ed.is_dirty());
        run(&mut ed, &mut wrap, &[Action::Move(Motion::Left, false)]);
        run(&mut ed, &mut wrap, &[Action::DeleteBack]);
        assert_eq!(ed.doc.body.to_string(), "e\u{301}👨\u{200d}👩\u{200d}👧中");
        run(&mut ed, &mut wrap, &[Action::DeleteBack]);
        assert_eq!(ed.doc.body.to_string(), "e\u{301}中");
        run(&mut ed, &mut wrap, &[Action::DeleteForward]);
        assert_eq!(ed.doc.body.to_string(), "e\u{301}");
        run(
            &mut ed,
            &mut wrap,
            &[Action::DeleteBack, Action::DeleteBack],
        );
        assert_eq!(ed.doc.body.to_string(), "");
    }

    #[test]
    fn newline_and_join_lines() {
        let (mut ed, mut wrap) = editor("ab");
        run(
            &mut ed,
            &mut wrap,
            &[Action::Move(Motion::Right, false), Action::Newline],
        );
        assert_eq!(ed.doc.body.to_string(), "a\nb");
        assert_eq!(wrap.rows(&ed.doc.body, 1)[0].end, 1);
        run(&mut ed, &mut wrap, &[Action::DeleteBack]);
        assert_eq!(ed.doc.body.to_string(), "ab");
        assert_eq!(ed.cursor.head, 1);
        assert_eq!(wrap.rows(&ed.doc.body, 0)[0].end, 2);
    }

    #[test]
    fn title_focus_round_trip() {
        let (mut ed, mut wrap) = editor("# 标题\n\n正文");
        let up = Action::Move(Motion::Up, false);
        ed.apply(&up, &mut wrap, 5);
        assert_eq!((ed.focus, ed.title_cursor), (Focus::Title, 2));
        run(&mut ed, &mut wrap, &[Action::DeleteBack, ins("目\n录")]);
        assert_eq!(ed.doc.title_text(), "标目 录");
        run(&mut ed, &mut wrap, &[Action::Newline]);
        assert_eq!((ed.focus, ed.cursor.head), (Focus::Body, 0));
        assert_eq!(ed.doc.serialize(), "# 标目 录\n\n正文");
        // ↓ 也回到正文
        run(&mut ed, &mut wrap, &[up, Action::Move(Motion::Down, false)]);
        assert_eq!(ed.focus, Focus::Body);
    }

    #[test]
    fn unbound_title_becomes_bound() {
        let (mut ed, mut wrap) = editor("正文\n");
        run(
            &mut ed,
            &mut wrap,
            &[Action::Move(Motion::Up, false), ins("T")],
        );
        assert_eq!(ed.doc.serialize(), "# T\n\n正文\n");
    }

    #[test]
    fn mark_saved_clears_dirty() {
        let (mut ed, mut wrap) = editor("");
        run(&mut ed, &mut wrap, &[ins("x")]);
        ed.mark_saved();
        assert!(!ed.is_dirty());
    }
}
