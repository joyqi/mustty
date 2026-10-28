//! Editor：Document + Cursor + History + Focus 的组合与编辑 API。
//!
//! 正文唯一写入口是 `Editor::replace`：它同时让 WrapCache 中受影响的逻辑行失效。
//! 输入、删除、粘贴、undo、redo 全部经由它；`doc` 字段私有，外部只能读。

pub mod cursor;
pub mod document;
pub mod history;

use std::ops::Range;
use std::path::PathBuf;
use std::time::{Instant, SystemTime};

use unicode_segmentation::UnicodeSegmentation;

use crate::action::{Action, Motion};
use crate::text::grapheme::{next_boundary, prev_boundary};
use crate::text::wrap::WrapCache;
use cursor::Cursor;
use document::Document;
use history::{Edit, EditKind, History, Snapshot, Target};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Title,
    Body,
}

pub struct Editor {
    doc: Document,
    pub cursor: Cursor,
    /// 标题内的 char 索引。
    pub title_cursor: usize,
    pub focus: Focus,
    history: History,
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
            history: History::default(),
            saved_revision,
        }
    }

    pub fn doc(&self) -> &Document {
        &self.doc
    }

    pub fn is_dirty(&self) -> bool {
        self.doc.revision() != self.saved_revision
    }

    /// 保存成功：记录路径与 mtime；保存总是结束当前合并组。
    pub fn mark_saved(&mut self, path: PathBuf, mtime: SystemTime) {
        self.doc.meta.path = Some(path);
        self.doc.meta.mtime = Some(mtime);
        self.saved_revision = self.doc.revision();
        self.history.seal();
    }

    /// 执行一个编辑 / 移动动作。`page` 为 PageUp/PageDown 的视觉行数。
    /// 保存、退出、提示行由 App 处理；M3+ 的动作在此忽略。
    pub fn apply(&mut self, action: &Action, wrap: &mut WrapCache, page: isize, now: Instant) {
        match action {
            Action::Undo => return self.undo(wrap),
            Action::Redo => return self.redo(wrap),
            Action::Move(..) => self.history.seal(),
            _ => {}
        }
        let focus = self.focus;
        match focus {
            Focus::Title => self.apply_title(action, wrap, now),
            Focus::Body => self.apply_body(action, wrap, page, now),
        }
        if self.focus != focus {
            self.history.seal();
        }
    }

    fn apply_body(&mut self, action: &Action, wrap: &mut WrapCache, page: isize, now: Instant) {
        let rope = self.doc.body();
        let head = self.cursor.head;
        match action {
            Action::Insert(s) => {
                let kind = if s.graphemes(true).count() == 1 {
                    EditKind::Insert
                } else {
                    EditKind::Other
                };
                self.change(Target::Body, head..head, s, kind, wrap, now);
            }
            Action::Paste(s) => {
                let s = s.replace("\r\n", "\n").replace('\r', "\n");
                self.change(Target::Body, head..head, &s, EditKind::Other, wrap, now);
            }
            Action::Newline => {
                self.change(Target::Body, head..head, "\n", EditKind::Other, wrap, now)
            }
            Action::DeleteBack if head > 0 => {
                let prev = Cursor::prev(rope, head);
                self.change(
                    Target::Body,
                    prev..head,
                    "",
                    EditKind::DeleteBack,
                    wrap,
                    now,
                );
            }
            Action::DeleteForward if head < rope.len_chars() => {
                let next = Cursor::next(rope, head);
                self.change(
                    Target::Body,
                    head..next,
                    "",
                    EditKind::DeleteForward,
                    wrap,
                    now,
                );
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

    fn apply_title(&mut self, action: &Action, wrap: &mut WrapCache, now: Instant) {
        let text = self.doc.title_text();
        let len = text.chars().count();
        let at = self.title_cursor;
        match action {
            // 标题是单行
            Action::Insert(s) | Action::Paste(s) => {
                let s = s.replace("\r\n", " ").replace(['\r', '\n'], " ");
                let kind = match action {
                    Action::Insert(_) if s.graphemes(true).count() == 1 => EditKind::Insert,
                    _ => EditKind::Other,
                };
                self.change(Target::Title, at..at, &s, kind, wrap, now);
            }
            Action::DeleteBack if at > 0 => {
                let prev = prev_boundary(text, at);
                self.change(Target::Title, prev..at, "", EditKind::DeleteBack, wrap, now);
            }
            Action::DeleteForward if at < len => {
                let next = next_boundary(text, at);
                self.change(
                    Target::Title,
                    at..next,
                    "",
                    EditKind::DeleteForward,
                    wrap,
                    now,
                );
            }
            Action::Newline | Action::Move(Motion::Down | Motion::DocStart, _) => {
                self.focus = Focus::Body;
                self.cursor.set(0);
            }
            Action::Move(Motion::DocEnd, _) => {
                self.focus = Focus::Body;
                self.cursor.set(self.doc.body().len_chars());
            }
            Action::Move(Motion::Left, _) => self.title_cursor = prev_boundary(text, at),
            Action::Move(Motion::Right, _) => self.title_cursor = next_boundary(text, at),
            Action::Move(Motion::LineStart, _) => self.title_cursor = 0,
            Action::Move(Motion::LineEnd, _) => self.title_cursor = len,
            _ => {}
        }
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            focus: self.focus,
            cursor: self.cursor,
            title_cursor: self.title_cursor,
            revision: self.doc.revision(),
        }
    }

    fn restore(&mut self, s: Snapshot) {
        self.focus = s.focus;
        self.cursor = s.cursor;
        self.title_cursor = s.title_cursor;
        self.doc.set_revision(s.revision);
    }

    /// 一次用户编辑：写入、把光标放到插入内容之后、记入 History。
    fn change(
        &mut self,
        target: Target,
        range: Range<usize>,
        text: &str,
        kind: EditKind,
        wrap: &mut WrapCache,
        now: Instant,
    ) {
        let before = self.snapshot();
        let deleted = self.write(target, range.clone(), text, wrap);
        let head = range.start + text.chars().count();
        match target {
            Target::Body => self.cursor.set(head),
            Target::Title => self.title_cursor = head,
        }
        let edit = Edit {
            target,
            at: range.start,
            deleted,
            inserted: text.to_owned(),
        };
        self.history
            .record(edit, kind, before, self.snapshot(), now);
    }

    fn undo(&mut self, wrap: &mut WrapCache) {
        let Some(tx) = self.history.undo() else {
            return;
        };
        for e in tx.edits.iter().rev() {
            self.write(
                e.target,
                e.at..e.at + e.inserted.chars().count(),
                &e.deleted,
                wrap,
            );
        }
        self.restore(tx.before);
    }

    fn redo(&mut self, wrap: &mut WrapCache) {
        let Some(tx) = self.history.redo() else {
            return;
        };
        for e in &tx.edits {
            self.write(
                e.target,
                e.at..e.at + e.deleted.chars().count(),
                &e.inserted,
                wrap,
            );
        }
        self.restore(tx.after);
    }

    /// 替换正文或标题的 char 区间，返回被删除的文本。
    fn write(
        &mut self,
        target: Target,
        range: Range<usize>,
        text: &str,
        wrap: &mut WrapCache,
    ) -> String {
        match target {
            Target::Body => {
                let deleted = self.doc.body().slice(range.clone()).to_string();
                self.replace(range, text, wrap);
                deleted
            }
            Target::Title => {
                let title = self.doc.title_text();
                let byte = |i: usize| title.char_indices().nth(i).map_or(title.len(), |(b, _)| b);
                let (start, end) = (byte(range.start), byte(range.end));
                let deleted = title[start..end].to_owned();
                let new = [&title[..start], text, &title[end..]].concat();
                self.doc.set_title(new);
                deleted
            }
        }
    }

    /// 正文唯一写入口：写正文并让受影响的逻辑行换行缓存失效。
    fn replace(&mut self, range: Range<usize>, text: &str, wrap: &mut WrapCache) {
        let rope = self.doc.body();
        let first = rope.char_to_line(range.start);
        let lines = first..rope.char_to_line(range.end) + 1;
        let before = rope.len_lines();
        self.doc.replace(range, text);
        let after = self.doc.body().len_lines();
        wrap.invalidate(lines.clone(), after as isize - before as isize);
        // 编辑点附近（含前后各一行）的缓存必须与重新换行的结果一致
        let touched =
            first.saturating_sub(1)..(first + lines.len() + after - before + 1).min(after);
        debug_assert!(
            wrap.is_consistent(self.doc.body(), touched),
            "WrapCache 与正文不一致"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use document::FileMeta;
    use proptest::prelude::*;
    use std::time::Duration;

    fn editor(raw: &str) -> (Editor, WrapCache) {
        let doc = Document::parse(raw, FileMeta::new(None));
        (Editor::new(doc), WrapCache::new(20))
    }

    fn run(ed: &mut Editor, wrap: &mut WrapCache, actions: &[Action]) {
        let now = Instant::now();
        for a in actions {
            ed.apply(a, wrap, 5, now);
        }
    }

    fn ins(s: &str) -> Action {
        Action::Insert(s.into())
    }

    fn body(ed: &Editor) -> String {
        ed.doc().body().to_string()
    }

    #[test]
    fn insert_and_delete_by_grapheme() {
        let (mut ed, mut wrap) = editor("");
        let clusters = ["e\u{301}", "👨\u{200d}👩\u{200d}👧", "🇨🇳", "中"];
        for c in clusters {
            // IME / 终端可能逐个 char 送达
            for ch in c.chars() {
                run(&mut ed, &mut wrap, &[ins(&ch.to_string())]);
            }
        }
        assert_eq!(body(&ed), clusters.concat());
        assert!(ed.is_dirty());
        run(
            &mut ed,
            &mut wrap,
            &[Action::Move(Motion::Left, false), Action::DeleteBack],
        );
        assert_eq!(body(&ed), "e\u{301}👨\u{200d}👩\u{200d}👧中");
        run(&mut ed, &mut wrap, &[Action::DeleteBack]);
        assert_eq!(body(&ed), "e\u{301}中");
        run(&mut ed, &mut wrap, &[Action::DeleteForward]);
        assert_eq!(body(&ed), "e\u{301}");
        run(
            &mut ed,
            &mut wrap,
            &[Action::DeleteBack, Action::DeleteBack],
        );
        assert_eq!(body(&ed), "");
    }

    #[test]
    fn newline_and_join_lines() {
        let (mut ed, mut wrap) = editor("ab");
        run(
            &mut ed,
            &mut wrap,
            &[Action::Move(Motion::Right, false), Action::Newline],
        );
        assert_eq!(body(&ed), "a\nb");
        assert_eq!(wrap.rows(ed.doc().body(), 1)[0].end, 1);
        run(&mut ed, &mut wrap, &[Action::DeleteBack]);
        assert_eq!(body(&ed), "ab");
        assert_eq!(ed.cursor.head, 1);
        assert_eq!(wrap.rows(ed.doc().body(), 0)[0].end, 2);
    }

    #[test]
    fn title_focus_round_trip() {
        let (mut ed, mut wrap) = editor("# 标题\n\n正文");
        let up = Action::Move(Motion::Up, false);
        run(&mut ed, &mut wrap, std::slice::from_ref(&up));
        assert_eq!((ed.focus, ed.title_cursor), (Focus::Title, 2));
        run(&mut ed, &mut wrap, &[Action::DeleteBack, ins("目\n录")]);
        assert_eq!(ed.doc().title_text(), "标目 录");
        run(&mut ed, &mut wrap, &[Action::Newline]);
        assert_eq!((ed.focus, ed.cursor.head), (Focus::Body, 0));
        assert_eq!(ed.doc().serialize(), "# 标目 录\n\n正文");
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
        assert_eq!(ed.doc().serialize(), "# T\n\n正文\n");
    }

    #[test]
    fn undo_three_words_then_redo() {
        let (mut ed, mut wrap) = editor("");
        let t0 = Instant::now();
        for (i, c) in "hello world foo".chars().enumerate() {
            ed.apply(
                &ins(&c.to_string()),
                &mut wrap,
                5,
                t0 + Duration::from_millis(i as u64 * 80),
            );
        }
        let mut seen = vec![];
        for _ in 0..4 {
            run(&mut ed, &mut wrap, &[Action::Undo]);
            seen.push((body(&ed), ed.cursor.head));
        }
        let want = [("hello world ", 12), ("hello ", 6), ("", 0), ("", 0)];
        assert_eq!(seen, want.map(|(s, h)| (s.to_owned(), h)));
        run(&mut ed, &mut wrap, &[Action::Redo, Action::Redo]);
        assert_eq!((body(&ed), ed.cursor.head), ("hello world ".into(), 12));
    }

    #[test]
    fn undo_to_save_point_clears_dirty() {
        let (mut ed, mut wrap) = editor("abc");
        let t0 = Instant::now();
        ed.apply(&ins("x"), &mut wrap, 5, t0);
        ed.mark_saved("a.md".into(), SystemTime::now());
        ed.apply(&ins("y"), &mut wrap, 5, t0);
        assert!(ed.is_dirty());
        run(&mut ed, &mut wrap, &[Action::Undo]);
        assert!(!ed.is_dirty(), "回到保存点");
        run(&mut ed, &mut wrap, &[Action::Undo]);
        assert!(ed.is_dirty());
        run(&mut ed, &mut wrap, &[Action::Redo]);
        assert!(!ed.is_dirty());
        // 撤销后另做一次编辑：版本号不能与保存点相撞
        run(&mut ed, &mut wrap, &[Action::Undo, ins("z")]);
        assert!(ed.is_dirty());
    }

    #[test]
    fn title_edits_are_undoable() {
        let (mut ed, mut wrap) = editor("正文");
        run(
            &mut ed,
            &mut wrap,
            &[Action::Move(Motion::Up, false), ins("新"), ins("题")],
        );
        run(&mut ed, &mut wrap, &[Action::Newline, ins("！")]);
        assert_eq!(ed.doc().serialize(), "# 新题\n\n！正文");
        run(&mut ed, &mut wrap, &[Action::Undo]);
        assert_eq!((ed.focus, body(&ed).as_str()), (Focus::Body, "正文"));
        run(&mut ed, &mut wrap, &[Action::Undo]);
        // 标题两个字是一组；撤销后焦点回到标题，文件与原来一致
        assert_eq!((ed.focus, ed.title_cursor), (Focus::Title, 0));
        assert_eq!(ed.doc().serialize(), "正文");
        assert!(!ed.is_dirty());
    }

    #[test]
    fn paste_is_one_transaction_and_normalized() {
        let (mut ed, mut wrap) = editor("[]");
        let text: String = (0..5000).map(|i| format!("第 {i} 行\r\n")).collect();
        run(
            &mut ed,
            &mut wrap,
            &[
                Action::Move(Motion::Right, false),
                Action::Paste(text + "a\rb"),
            ],
        );
        assert_eq!(ed.doc().body().len_lines(), 5002);
        assert!(!body(&ed).contains('\r'));
        assert!(wrap.is_consistent(ed.doc().body(), 0..5002));
        run(&mut ed, &mut wrap, &[Action::Undo]);
        assert_eq!(body(&ed), "[]");
        assert_eq!(ed.cursor.head, 1);
    }

    #[derive(Debug, Clone)]
    enum Op {
        Insert(String),
        Paste(String),
        Back,
        Forward,
        Move(u8),
        Undo,
        Redo,
    }

    fn op() -> impl Strategy<Value = Op> {
        prop_oneof![
            "[a 中\n]|e\u{301}".prop_map(Op::Insert),
            "[ab 中文\n]{0,30}".prop_map(Op::Paste),
            Just(Op::Back),
            Just(Op::Forward),
            (0u8..6).prop_map(Op::Move),
            Just(Op::Undo),
            Just(Op::Redo),
        ]
    }

    proptest! {
        /// 任意编辑 / 粘贴 / undo / redo 序列后：WrapCache 与重新换行的结果一致，光标在文档内；
        /// 全部撤销回到原文。
        #[test]
        fn wrap_cache_stays_consistent(ops in prop::collection::vec(op(), 1..60)) {
            let (mut ed, mut wrap) = editor("一二三四五六七八九十 hello world\n第二行\n");
            let original = body(&ed);
            let t0 = Instant::now();
            for (i, o) in ops.iter().enumerate() {
                // 每步间隔 2 s：保证每步独立成组，便于全部撤销
                let now = t0 + Duration::from_secs(2 * i as u64);
                let motions = [Motion::Left, Motion::Right, Motion::Up, Motion::Down, Motion::LineEnd, Motion::DocEnd];
                let action = match o {
                    Op::Insert(s) => Action::Insert(s.clone()),
                    Op::Paste(s) => Action::Paste(s.clone()),
                    Op::Back => Action::DeleteBack,
                    Op::Forward => Action::DeleteForward,
                    Op::Move(m) => Action::Move(motions[*m as usize], false),
                    Op::Undo => Action::Undo,
                    Op::Redo => Action::Redo,
                };
                if ed.focus == Focus::Title {
                    ed.apply(&Action::Newline, &mut wrap, 5, now);
                }
                ed.apply(&action, &mut wrap, 5, now);
                let rope = ed.doc().body();
                prop_assert!(wrap.is_consistent(rope, 0..rope.len_lines()));
                prop_assert!(ed.cursor.head <= rope.len_chars());
                ed.cursor.visual(rope, &mut wrap);
            }
            for _ in 0..ops.len() {
                ed.apply(&Action::Undo, &mut wrap, 5, t0);
            }
            prop_assert_eq!(body(&ed), original);
            prop_assert!(!ed.is_dirty());
        }
    }
}
