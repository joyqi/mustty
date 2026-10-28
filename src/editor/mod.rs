//! Editor：Document + Cursor + History + Focus + 寄存器，以及由正文派生的两个缓存。
//!
//! 正文唯一写入口是 `Editor::replace`：它同时让 WrapCache 与 HighlightCache 中受影响的逻辑行失效。
//! 输入、删除、粘贴、剪切、寄存器粘贴、undo、redo 全部经由它；`doc` 与两个缓存都归 Editor 所有，
//! 外部只能通过 `doc()` 读、通过 `view()` 惰性计算缓存。

pub mod cursor;
pub mod document;
pub mod history;

use std::ops::Range;
use std::path::PathBuf;
use std::time::Instant;

use ropey::Rope;
use unicode_segmentation::UnicodeSegmentation;

use crate::action::{Action, Motion, Target as MoveTarget};
use crate::markdown::HighlightCache;
use crate::text::count::count_words;
use crate::text::grapheme::{next_boundary, prev_boundary};
use crate::text::wrap::{WrapCache, line_text};
use cursor::Cursor;
use document::{DiskStamp, Document};
use history::{Edit, EditKind, History, Snapshot, Target};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Title,
    Body,
}

/// 内部寄存器。行级（`DeleteLine` 产生）粘贴到下一行，字符级粘贴到光标处。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Register {
    pub text: String,
    pub linewise: bool,
}

/// `apply` 需要 App 处理的副作用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    None,
    /// 复制 / 剪切了文本，App 负责写系统剪贴板（OSC 52）
    Copied(String),
}

pub struct Editor {
    doc: Document,
    wrap: WrapCache,
    highlight: HighlightCache,
    pub cursor: Cursor,
    /// 标题内的 char 索引。
    pub title_cursor: usize,
    pub focus: Focus,
    history: History,
    saved_revision: u64,
    register: Option<Register>,
    /// 正文字数，在 `replace` 里按受影响的行增量维护（词不跨行，逐行之和即全文字数）
    body_words: usize,
}

impl Editor {
    pub fn new(doc: Document, wrap_width: u16, tab_width: u16) -> Self {
        let saved_revision = doc.revision();
        let body_words = count_words(&doc.body().to_string());
        Self {
            doc,
            wrap: WrapCache::new(wrap_width, tab_width),
            highlight: HighlightCache::default(),
            cursor: Cursor::default(),
            title_cursor: 0,
            focus: Focus::Body,
            history: History::default(),
            saved_revision,
            register: None,
            body_words,
        }
    }

    /// 标题 + 正文的字数。
    pub fn word_count(&self) -> usize {
        count_words(self.doc.title_text()) + self.body_words
    }

    pub fn doc(&self) -> &Document {
        &self.doc
    }

    /// 渲染 / 滚动用：正文只读，缓存可惰性填充（不能借此写正文）。
    pub fn view(&mut self) -> (&Document, &mut WrapCache, &mut HighlightCache) {
        (&self.doc, &mut self.wrap, &mut self.highlight)
    }

    pub fn register(&self) -> Option<&Register> {
        self.register.as_ref()
    }

    /// 正文中的非空选区。
    pub fn selection(&self) -> Option<Range<usize>> {
        self.cursor
            .selection()
            .filter(|_| self.focus == Focus::Body)
    }

    pub fn is_dirty(&self) -> bool {
        self.doc.revision() != self.saved_revision
    }

    /// 保存成功：记录路径与磁盘信息；保存总是结束当前合并组。
    pub fn mark_saved(&mut self, path: PathBuf, stamp: DiskStamp) {
        self.doc.meta.path = Some(path);
        self.doc.meta.stamp = Some(stamp);
        self.saved_revision = self.doc.revision();
        self.history.seal();
    }

    /// 执行一个编辑 / 移动动作。`page` 为 PageUp/PageDown 的视觉行数。
    /// 保存、退出、提示行由 App 处理；M4 的动作在此忽略。
    pub fn apply(&mut self, action: &Action, page: isize, now: Instant) -> Effect {
        match action {
            Action::Undo => self.undo(),
            Action::Redo => self.redo(),
            &Action::MoveTo(target, extend) => {
                self.history.seal();
                self.move_to(target, extend);
                return Effect::None;
            }
            Action::Move(..) | Action::SelectAll | Action::Cancel => self.history.seal(),
            _ => {}
        }
        let focus = self.focus;
        let effect = match focus {
            Focus::Title => self.apply_title(action, now),
            Focus::Body => self.apply_body(action, page, now),
        };
        if self.focus != focus {
            self.history.seal();
        }
        effect
    }

    fn apply_body(&mut self, action: &Action, page: isize, now: Instant) -> Effect {
        let rope = self.doc.body();
        let head = self.cursor.head;
        let sel = self.cursor.selection();
        let at_cursor = sel.clone().unwrap_or(head..head);
        match action {
            Action::Insert(s) => {
                let single = sel.is_none() && s.graphemes(true).count() == 1;
                let kind = if single {
                    EditKind::Insert
                } else {
                    EditKind::Other
                };
                self.change(Target::Body, at_cursor, s, kind, now);
            }
            Action::Paste(s) => {
                let s = s.replace("\r\n", "\n").replace('\r', "\n");
                self.change(Target::Body, at_cursor, &s, EditKind::Other, now);
            }
            Action::Newline => self.change(Target::Body, at_cursor, "\n", EditKind::Other, now),
            Action::DeleteBack | Action::DeleteForward | Action::DeleteWordBack
                if sel.is_some() =>
            {
                self.change(Target::Body, at_cursor, "", EditKind::Other, now);
            }
            Action::DeleteBack if head > 0 => {
                let prev = Cursor::prev(rope, head);
                self.change(Target::Body, prev..head, "", EditKind::DeleteBack, now);
            }
            Action::DeleteForward if head < rope.len_chars() => {
                let next = Cursor::next(rope, head);
                self.change(Target::Body, head..next, "", EditKind::DeleteForward, now);
            }
            Action::DeleteWordBack if head > 0 => {
                let start = Cursor::word_left(rope, head);
                self.change(Target::Body, start..head, "", EditKind::Other, now);
            }
            Action::Move(motion, extend) => self.motion(*motion, *extend, page),
            Action::SelectAll => {
                let end = rope.len_chars();
                self.cursor.set(end);
                self.cursor.anchor = Some(0);
            }
            Action::Cancel => self.cursor.anchor = None,
            Action::Copy | Action::Cut => {
                let Some(range) = sel else {
                    return Effect::None;
                };
                let text = rope.slice(range.clone()).to_string();
                self.register = Some(Register {
                    text: text.clone(),
                    linewise: false,
                });
                if *action == Action::Cut {
                    self.change(Target::Body, range, "", EditKind::Other, now);
                }
                return Effect::Copied(text);
            }
            Action::PasteRegister => self.paste_register(now),
            Action::DeleteLine => self.delete_line(now),
            Action::OpenLineBelow => {
                let line = rope.char_to_line(head);
                let end = rope.line_to_char(line) + line_text(rope, line).chars().count();
                self.change(Target::Body, end..end, "\n", EditKind::Other, now);
            }
            _ => {}
        }
        Effect::None
    }

    fn motion(&mut self, motion: Motion, extend: bool, page: isize) {
        let rope = self.doc.body();
        let wrap = &mut self.wrap;
        let head = self.cursor.head;
        // 不扩展时，←/→ 先把选区收拢到对应一端
        if let (false, Some(sel)) = (extend, self.cursor.selection()) {
            match motion {
                Motion::Left => return self.cursor.set(sel.start),
                Motion::Right => return self.cursor.set(sel.end),
                _ => {}
            }
        }
        let anchor = extend.then(|| self.cursor.anchor.unwrap_or(head));
        let c = &mut self.cursor;
        match motion {
            Motion::Left => c.move_left(rope, wrap),
            Motion::Right => c.move_right(rope, wrap),
            Motion::Up => {
                let pos = c.visual(rope, wrap);
                if (pos.line, pos.row) == (0, 0) && !extend {
                    self.focus = Focus::Title;
                    self.title_cursor = self.doc.title_text().chars().count();
                } else {
                    c.move_vertical(rope, wrap, -1);
                }
            }
            Motion::Down => c.move_vertical(rope, wrap, 1),
            Motion::PageUp => c.move_vertical(rope, wrap, -page),
            Motion::PageDown => c.move_vertical(rope, wrap, page),
            Motion::LineStart => c.move_row_start(rope, wrap),
            Motion::LineEnd => c.move_row_end(rope, wrap),
            Motion::DocStart => c.set(0),
            Motion::DocEnd => c.set(rope.len_chars()),
            Motion::WordLeft => c.set(Cursor::word_left(rope, head)),
            Motion::WordRight => c.set(Cursor::word_right(rope, head)),
            Motion::WordEnd => c.set(Cursor::word_end(rope, head)),
        }
        self.cursor.anchor = anchor;
    }

    /// 鼠标定位：正文中按 `extend` 保留 / 建立选区；标题中只移动标题光标。
    fn move_to(&mut self, target: MoveTarget, extend: bool) {
        match target {
            MoveTarget::Body(at) => {
                let from_body = self.focus == Focus::Body;
                self.focus = Focus::Body;
                let anchor =
                    (extend && from_body).then(|| self.cursor.anchor.unwrap_or(self.cursor.head));
                self.cursor.set(at.min(self.doc.body().len_chars()));
                self.cursor.anchor = anchor;
            }
            MoveTarget::Title(at) => {
                self.focus = Focus::Title;
                self.title_cursor = at.min(self.doc.title_text().chars().count());
            }
        }
    }

    fn paste_register(&mut self, now: Instant) {
        let Some(reg) = self.register.clone() else {
            return;
        };
        let rope = self.doc.body();
        let head = self.cursor.head;
        if let Some(sel) = self.cursor.selection() {
            return self.change(Target::Body, sel, &reg.text, EditKind::Other, now);
        }
        if !reg.linewise {
            return self.change(Target::Body, head..head, &reg.text, EditKind::Other, now);
        }
        let line = rope.char_to_line(head);
        // 光标放在粘贴出的那一行行首
        let (at, text, head) = if line + 1 < rope.len_lines() {
            let at = rope.line_to_char(line + 1);
            (at, reg.text.clone(), at)
        } else {
            // 最后一行之后没有行首可插：先补一个换行
            let body = reg.text.strip_suffix('\n').unwrap_or(&reg.text);
            let at = rope.len_chars();
            (at, format!("\n{body}"), at + 1)
        };
        self.change_to(Target::Body, at..at, &text, head, EditKind::Other, now);
    }

    /// 剪切整个逻辑行进寄存器（行级）；最后一行连同它前面的换行一起删除，不留空行。
    fn delete_line(&mut self, now: Instant) {
        let rope = self.doc.body();
        let line = rope.char_to_line(self.cursor.head);
        let start = rope.line_to_char(line);
        let text = format!("{}\n", line_text(rope, line));
        let (range, head_after) = if line + 1 < rope.len_lines() {
            (start..rope.line_to_char(line + 1), start)
        } else if line > 0 {
            (start - 1..rope.len_chars(), rope.line_to_char(line - 1))
        } else {
            (0..rope.len_chars(), 0)
        };
        self.register = Some(Register {
            text,
            linewise: true,
        });
        self.change_to(Target::Body, range, "", head_after, EditKind::Other, now);
    }

    fn apply_title(&mut self, action: &Action, now: Instant) -> Effect {
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
                self.change(Target::Title, at..at, &s, kind, now);
            }
            Action::PasteRegister => {
                if let Some(reg) = &self.register {
                    let s = reg.text.trim_end_matches('\n').replace('\n', " ");
                    self.change(Target::Title, at..at, &s, EditKind::Other, now);
                }
            }
            Action::DeleteBack if at > 0 => {
                let prev = prev_boundary(text, at);
                self.change(Target::Title, prev..at, "", EditKind::DeleteBack, now);
            }
            Action::DeleteForward if at < len => {
                let next = next_boundary(text, at);
                self.change(Target::Title, at..next, "", EditKind::DeleteForward, now);
            }
            Action::DeleteWordBack if at > 0 => {
                let start = Cursor::word_left(&Rope::from_str(text), at);
                self.change(Target::Title, start..at, "", EditKind::Other, now);
            }
            Action::Newline | Action::Move(Motion::Down | Motion::DocStart, _) => {
                self.focus = Focus::Body;
                self.cursor.set(0);
            }
            Action::Move(Motion::DocEnd, _) => {
                self.focus = Focus::Body;
                self.cursor.set(self.doc.body().len_chars());
            }
            Action::SelectAll => {
                self.focus = Focus::Body;
                return self.apply_body(action, 0, now);
            }
            Action::Move(Motion::Left, _) => self.title_cursor = prev_boundary(text, at),
            Action::Move(Motion::Right, _) => self.title_cursor = next_boundary(text, at),
            Action::Move(Motion::WordLeft, _) => {
                self.title_cursor = Cursor::word_left(&Rope::from_str(text), at)
            }
            Action::Move(Motion::WordRight, _) => {
                self.title_cursor = Cursor::word_right(&Rope::from_str(text), at)
            }
            Action::Move(Motion::LineStart, _) => self.title_cursor = 0,
            Action::Move(Motion::LineEnd, _) => self.title_cursor = len,
            _ => {}
        }
        Effect::None
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

    /// 一次用户编辑，光标放到插入内容之后。
    fn change(
        &mut self,
        target: Target,
        range: Range<usize>,
        text: &str,
        kind: EditKind,
        now: Instant,
    ) {
        let head = range.start + text.chars().count();
        self.change_to(target, range, text, head, kind, now);
    }

    /// 一次用户编辑：写入、把光标放到 `head`、记入 History。
    fn change_to(
        &mut self,
        target: Target,
        range: Range<usize>,
        text: &str,
        head: usize,
        kind: EditKind,
        now: Instant,
    ) {
        let before = self.snapshot();
        let deleted = self.write(target, range.clone(), text);
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

    fn undo(&mut self) {
        let Some(tx) = self.history.undo() else {
            return;
        };
        for e in tx.edits.iter().rev() {
            self.write(
                e.target,
                e.at..e.at + e.inserted.chars().count(),
                &e.deleted,
            );
        }
        self.restore(tx.before);
    }

    fn redo(&mut self) {
        let Some(tx) = self.history.redo() else {
            return;
        };
        for e in &tx.edits {
            self.write(
                e.target,
                e.at..e.at + e.deleted.chars().count(),
                &e.inserted,
            );
        }
        self.restore(tx.after);
    }

    /// 替换正文或标题的 char 区间，返回被删除的文本。
    fn write(&mut self, target: Target, range: Range<usize>, text: &str) -> String {
        match target {
            Target::Body => {
                let deleted = self.doc.body().slice(range.clone()).to_string();
                self.replace(range, text);
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

    /// 正文唯一写入口：写正文，并让两个缓存中受影响的逻辑行失效。
    fn replace(&mut self, range: Range<usize>, text: &str) {
        let rope = self.doc.body();
        let first = rope.char_to_line(range.start);
        let lines = first..rope.char_to_line(range.end) + 1;
        let before = rope.len_lines();
        let words = |rope: &Rope, lines: Range<usize>| -> usize {
            lines.map(|l| count_words(&line_text(rope, l))).sum()
        };
        let old_words = words(rope, lines.clone());
        self.doc.replace(range, text);
        let rope = self.doc.body();
        let after = rope.len_lines();
        let inserted = after as isize - before as isize;
        let new_end = first + (lines.len() as isize + inserted) as usize;
        self.body_words = self.body_words + words(rope, first..new_end) - old_words;
        self.wrap.invalidate(lines.clone(), inserted);
        let rescanned = self.highlight.invalidate(rope, lines.clone(), inserted);
        // 编辑点附近（含前后各一行）的缓存必须与重新计算的结果一致
        let near = first.saturating_sub(1)..(new_end.max(first + rescanned) + 1).min(after);
        debug_assert!(
            self.wrap.is_consistent(rope, near.clone()),
            "WrapCache 与正文不一致"
        );
        debug_assert!(
            self.highlight.is_consistent_in(rope, near),
            "HighlightCache 与正文不一致"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use document::FileMeta;
    use proptest::prelude::*;
    use std::time::{Duration, SystemTime};

    fn editor(raw: &str) -> Editor {
        Editor::new(Document::parse(raw, FileMeta::lf()), 20, 4)
    }

    fn run(ed: &mut Editor, actions: &[Action]) -> Effect {
        let now = Instant::now();
        let mut last = Effect::None;
        for a in actions {
            last = ed.apply(a, 5, now);
        }
        last
    }

    fn ins(s: &str) -> Action {
        Action::Insert(s.into())
    }

    fn mv(m: Motion) -> Action {
        Action::Move(m, false)
    }

    fn ext(m: Motion) -> Action {
        Action::Move(m, true)
    }

    fn body(ed: &Editor) -> String {
        ed.doc().body().to_string()
    }

    fn stamp() -> DiskStamp {
        DiskStamp {
            mtime: SystemTime::now(),
            len: 0,
        }
    }

    #[test]
    fn insert_and_delete_by_grapheme() {
        let mut ed = editor("");
        let clusters = ["e\u{301}", "👨\u{200d}👩\u{200d}👧", "🇨🇳", "中"];
        for c in clusters {
            // IME / 终端可能逐个 char 送达
            for ch in c.chars() {
                run(&mut ed, &[ins(&ch.to_string())]);
            }
        }
        assert_eq!(body(&ed), clusters.concat());
        assert!(ed.is_dirty());
        run(&mut ed, &[mv(Motion::Left), Action::DeleteBack]);
        assert_eq!(body(&ed), "e\u{301}👨\u{200d}👩\u{200d}👧中");
        run(&mut ed, &[Action::DeleteBack]);
        assert_eq!(body(&ed), "e\u{301}中");
        run(&mut ed, &[Action::DeleteForward]);
        assert_eq!(body(&ed), "e\u{301}");
        run(&mut ed, &[Action::DeleteBack, Action::DeleteBack]);
        assert_eq!(body(&ed), "");
    }

    #[test]
    fn newline_and_join_lines() {
        let mut ed = editor("ab");
        run(&mut ed, &[mv(Motion::Right), Action::Newline]);
        assert_eq!(body(&ed), "a\nb");
        run(&mut ed, &[Action::DeleteBack]);
        assert_eq!((body(&ed).as_str(), ed.cursor.head), ("ab", 1));
    }

    #[test]
    fn title_focus_round_trip() {
        let mut ed = editor("# 标题\n\n正文");
        run(&mut ed, &[mv(Motion::Up)]);
        assert_eq!((ed.focus, ed.title_cursor), (Focus::Title, 2));
        run(&mut ed, &[Action::DeleteBack, ins("目\n录")]);
        assert_eq!(ed.doc().title_text(), "标目 录");
        run(&mut ed, &[Action::Newline]);
        assert_eq!((ed.focus, ed.cursor.head), (Focus::Body, 0));
        assert_eq!(ed.doc().serialize(), "# 标目 录\n\n正文");
        run(&mut ed, &[mv(Motion::Up), mv(Motion::Down)]);
        assert_eq!(ed.focus, Focus::Body);
    }

    #[test]
    fn unbound_title_becomes_bound() {
        let mut ed = editor("正文\n");
        run(&mut ed, &[mv(Motion::Up), ins("T")]);
        assert_eq!(ed.doc().serialize(), "# T\n\n正文\n");
    }

    #[test]
    fn undo_three_words_then_redo() {
        let mut ed = editor("");
        let t0 = Instant::now();
        for (i, c) in "hello world foo".chars().enumerate() {
            ed.apply(
                &ins(&c.to_string()),
                5,
                t0 + Duration::from_millis(i as u64 * 80),
            );
        }
        let mut seen = vec![];
        for _ in 0..4 {
            run(&mut ed, &[Action::Undo]);
            seen.push((body(&ed), ed.cursor.head));
        }
        let want = [("hello world ", 12), ("hello ", 6), ("", 0), ("", 0)];
        assert_eq!(seen, want.map(|(s, h)| (s.to_owned(), h)));
        run(&mut ed, &[Action::Redo, Action::Redo]);
        assert_eq!((body(&ed), ed.cursor.head), ("hello world ".into(), 12));
    }

    #[test]
    fn undo_to_save_point_clears_dirty() {
        let mut ed = editor("abc");
        let t0 = Instant::now();
        ed.apply(&ins("x"), 5, t0);
        ed.mark_saved("a.md".into(), stamp());
        ed.apply(&ins("y"), 5, t0);
        assert!(ed.is_dirty());
        run(&mut ed, &[Action::Undo]);
        assert!(!ed.is_dirty(), "回到保存点");
        run(&mut ed, &[Action::Undo]);
        assert!(ed.is_dirty());
        run(&mut ed, &[Action::Redo]);
        assert!(!ed.is_dirty());
        // 撤销后另做一次编辑：版本号不能与保存点相撞
        run(&mut ed, &[Action::Undo, ins("z")]);
        assert!(ed.is_dirty());
    }

    #[test]
    fn title_edits_are_undoable() {
        let mut ed = editor("正文");
        run(&mut ed, &[mv(Motion::Up), ins("新"), ins("题")]);
        run(&mut ed, &[Action::Newline, ins("！")]);
        assert_eq!(ed.doc().serialize(), "# 新题\n\n！正文");
        run(&mut ed, &[Action::Undo]);
        assert_eq!((ed.focus, body(&ed).as_str()), (Focus::Body, "正文"));
        run(&mut ed, &[Action::Undo]);
        assert_eq!((ed.focus, ed.title_cursor), (Focus::Title, 0));
        assert_eq!(ed.doc().serialize(), "正文");
        assert!(!ed.is_dirty());
    }

    #[test]
    fn paste_is_one_transaction_and_normalized() {
        let mut ed = editor("[]");
        let text: String = (0..5000).map(|i| format!("第 {i} 行\r\n")).collect();
        run(&mut ed, &[mv(Motion::Right), Action::Paste(text + "a\rb")]);
        assert_eq!(ed.doc().body().len_lines(), 5002);
        assert!(!body(&ed).contains('\r'));
        run(&mut ed, &[Action::Undo]);
        assert_eq!((body(&ed).as_str(), ed.cursor.head), ("[]", 1));
    }

    #[test]
    fn shift_motions_extend_selection() {
        let mut ed = editor("hello world\nsecond");
        run(&mut ed, &[ext(Motion::WordRight), ext(Motion::WordRight)]);
        assert_eq!(ed.selection(), Some(0..11));
        run(&mut ed, &[ext(Motion::Down)]);
        assert_eq!(ed.selection(), Some(0..18));
        run(&mut ed, &[ext(Motion::LineStart)]);
        assert_eq!(ed.selection(), Some(0..12));
        // ← 不扩展：收拢到选区起点；Esc 取消
        run(&mut ed, &[mv(Motion::Left)]);
        assert_eq!((ed.selection(), ed.cursor.head), (None, 0));
        run(&mut ed, &[ext(Motion::Right), Action::Cancel]);
        assert_eq!(ed.selection(), None);
        // Shift+↑ 在首行不进入标题
        run(&mut ed, &[ext(Motion::Up)]);
        assert_eq!(ed.focus, Focus::Body);
    }

    #[test]
    fn typing_replaces_selection_and_undo_restores_it() {
        let mut ed = editor("abc def");
        run(&mut ed, &[ext(Motion::WordRight)]);
        run(&mut ed, &[ins("X")]);
        assert_eq!(body(&ed), "Xdef");
        run(&mut ed, &[Action::Undo]);
        assert_eq!(
            (body(&ed).as_str(), ed.selection()),
            ("abc def", Some(0..4))
        );
        run(&mut ed, &[Action::DeleteBack]);
        assert_eq!(body(&ed), "def");
    }

    #[test]
    fn select_all_copy_cut_paste_register() {
        let mut ed = editor("# T\n\n中文 text");
        assert_eq!(run(&mut ed, &[Action::Copy]), Effect::None, "无选区不复制");
        let copied = run(&mut ed, &[Action::SelectAll, Action::Copy]);
        assert_eq!(copied, Effect::Copied("中文 text".into()));
        run(&mut ed, &[mv(Motion::DocEnd), Action::PasteRegister]);
        assert_eq!(body(&ed), "中文 text中文 text");
        run(&mut ed, &[mv(Motion::DocStart), ext(Motion::WordRight)]);
        assert_eq!(run(&mut ed, &[Action::Cut]), Effect::Copied("中文 ".into()));
        assert_eq!(body(&ed), "text中文 text");
        assert_eq!(
            ed.register(),
            Some(&Register {
                text: "中文 ".into(),
                linewise: false
            })
        );
        // 剪切可撤销
        run(&mut ed, &[Action::Undo]);
        assert_eq!(body(&ed), "中文 text中文 text");
        // 标题里粘贴：单行
        run(
            &mut ed,
            &[mv(Motion::Left), mv(Motion::Up), Action::PasteRegister],
        );
        assert_eq!(ed.doc().title_text(), "T中文 ");
    }

    #[test]
    fn delete_line_and_linewise_paste() {
        let mut ed = editor("one\ntwo\nthree");
        run(&mut ed, &[Action::DeleteLine]);
        assert_eq!((body(&ed).as_str(), ed.cursor.head), ("two\nthree", 0));
        assert_eq!(
            ed.register(),
            Some(&Register {
                text: "one\n".into(),
                linewise: true
            })
        );
        // 行级寄存器粘贴到下一行
        run(&mut ed, &[Action::PasteRegister]);
        assert_eq!((body(&ed).as_str(), ed.cursor.head), ("two\none\nthree", 4));
        // 最后一行（无末尾换行）：连同前面的换行删掉，光标回到上一行
        run(&mut ed, &[mv(Motion::DocEnd), Action::DeleteLine]);
        assert_eq!((body(&ed).as_str(), ed.cursor.head), ("two\none", 4));
        // 在最后一行粘贴：补一个换行
        run(&mut ed, &[Action::PasteRegister]);
        assert_eq!((body(&ed).as_str(), ed.cursor.head), ("two\none\nthree", 8));
        // 唯一一行
        let mut single = editor("only");
        run(&mut single, &[Action::DeleteLine]);
        assert_eq!(body(&single), "");
        run(&mut single, &[Action::Undo]);
        assert_eq!(body(&single), "only");
    }

    #[test]
    fn open_line_below_is_one_undo() {
        let mut ed = editor("ab\ncd");
        run(&mut ed, &[mv(Motion::Right), Action::OpenLineBelow]);
        assert_eq!((body(&ed).as_str(), ed.cursor.head), ("ab\n\ncd", 3));
        run(&mut ed, &[ins("x"), Action::Undo]);
        run(&mut ed, &[Action::Undo]);
        assert_eq!((body(&ed).as_str(), ed.cursor.head), ("ab\ncd", 1));
    }

    #[test]
    fn delete_word_back() {
        let mut ed = editor("hello 中文写作 world");
        run(&mut ed, &[mv(Motion::DocEnd), Action::DeleteWordBack]);
        assert_eq!(body(&ed), "hello 中文写作 ");
        run(&mut ed, &[Action::DeleteWordBack]);
        assert_eq!(body(&ed), "hello ");
    }

    #[test]
    fn word_end_motion_in_mixed_text() {
        let mut ed = editor("ab 中文，cd");
        let stops: Vec<usize> = (0..4)
            .map(|_| {
                run(&mut ed, &[mv(Motion::WordEnd)]);
                ed.cursor.head
            })
            .collect();
        assert_eq!(stops, vec![1, 4, 5, 7]);
    }

    #[derive(Debug, Clone)]
    enum Op {
        Act(Action),
        Paste(String),
        Insert(String),
    }

    fn op() -> impl Strategy<Value = Op> {
        let motions = [
            Motion::Left,
            Motion::Right,
            Motion::Up,
            Motion::Down,
            Motion::LineEnd,
            Motion::DocEnd,
            Motion::WordLeft,
            Motion::WordRight,
            Motion::WordEnd,
        ];
        let simple = [
            Action::DeleteBack,
            Action::DeleteForward,
            Action::DeleteWordBack,
            Action::Undo,
            Action::Redo,
            Action::SelectAll,
            Action::Copy,
            Action::Cut,
            Action::PasteRegister,
            Action::DeleteLine,
            Action::OpenLineBelow,
            Action::Newline,
            Action::Cancel,
        ];
        prop_oneof![
            "[a 中`*\n]|e\u{301}".prop_map(Op::Insert),
            "[ab 中*`#>\n]{0,30}".prop_map(Op::Paste),
            prop::sample::select(simple.to_vec()).prop_map(Op::Act),
            (prop::sample::select(motions.to_vec()), any::<bool>())
                .prop_map(|(m, e)| Op::Act(Action::Move(m, e))),
            Just(Op::Paste("```\n".into())),
        ]
    }

    proptest! {
        /// 任意编辑 / 粘贴 / 剪切 / 寄存器 / undo / redo 序列后：两个缓存都与重新计算的结果一致，
        /// 光标在文档内；全部撤销回到原文。
        #[test]
        fn caches_stay_consistent(ops in prop::collection::vec(op(), 1..60)) {
            let mut ed = editor("一二三四五六七八九十 **hello** world\n```\n代码\n```\n第二行\n");
            let original = body(&ed);
            let t0 = Instant::now();
            for (i, o) in ops.iter().enumerate() {
                // 每步间隔 2 s：保证每步独立成组，便于全部撤销
                let now = t0 + Duration::from_secs(2 * i as u64);
                let action = match o {
                    Op::Act(a) => a.clone(),
                    Op::Paste(s) => Action::Paste(s.clone()),
                    Op::Insert(s) => Action::Insert(s.clone()),
                };
                if ed.focus == Focus::Title {
                    ed.apply(&Action::Newline, 5, now);
                }
                ed.apply(&action, 5, now);
                let caret = ed.cursor;
                let words = ed.word_count();
                let (doc, wrap, hl) = ed.view();
                let rope = doc.body();
                // 像渲染一样惰性填充末行附近，覆盖缓存扩展路径
                hl.ranges(rope, rope.len_lines() - 1);
                wrap.rows(rope, rope.len_lines() - 1);
                prop_assert!(wrap.is_consistent(rope, 0..rope.len_lines()));
                prop_assert!(hl.is_consistent(rope));
                prop_assert!(caret.head <= rope.len_chars());
                let full = count_words(doc.title_text()) + count_words(&rope.to_string());
                prop_assert_eq!(words, full);
                prop_assert!(caret.anchor.is_none_or(|a| a <= rope.len_chars()));
            }
            for _ in 0..ops.len() {
                ed.apply(&Action::Undo, 5, t0);
            }
            prop_assert_eq!(body(&ed), original);
            prop_assert!(!ed.is_dirty());
        }
    }
}
