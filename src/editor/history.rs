//! undo / redo：线性栈 + 合并规则。
//! History 只记录编辑；撤销时由 Editor 经正文唯一写入口回放。

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use unicode_segmentation::UnicodeSegmentation;

use super::Focus;
use super::cursor::Cursor;
use crate::text::grapheme::is_whitespace;

pub const MAX_TRANSACTIONS: usize = 1000;
pub const MERGE_WINDOW: Duration = Duration::from_secs(1);

/// 写入的是正文还是标题；两者的坐标都是各自的 char 索引。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Title,
    Body,
}

/// char 坐标的区间替换：`[at, at + deleted.len)` 被 `inserted` 取代。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub target: Target,
    pub at: usize,
    pub deleted: String,
    pub inserted: String,
}

/// 决定能否与上一个 Transaction 合并。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    /// 单个 grapheme 的插入
    Insert,
    DeleteBack,
    DeleteForward,
    /// 换行、粘贴、多字符插入：总是独立成组
    Other,
}

/// 撤销 / 重做后要恢复的编辑器状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot {
    pub focus: Focus,
    pub cursor: Cursor,
    pub title_cursor: usize,
    pub revision: u64,
}

#[derive(Debug, Clone)]
pub struct Transaction {
    pub edits: Vec<Edit>,
    pub kind: EditKind,
    pub before: Snapshot,
    pub after: Snapshot,
}

#[derive(Debug, Default)]
pub struct History {
    undo: VecDeque<Transaction>,
    redo: Vec<Transaction>,
    last_edit_at: Option<Instant>,
    sealed: bool,
}

impl History {
    pub fn record(
        &mut self,
        edit: Edit,
        kind: EditKind,
        before: Snapshot,
        after: Snapshot,
        now: Instant,
    ) {
        self.redo.clear();
        let recent = self
            .last_edit_at
            .is_some_and(|t| now.saturating_duration_since(t) < MERGE_WINDOW);
        match self.undo.back_mut() {
            Some(tx) if !self.sealed && recent && tx.kind == kind && mergeable(tx, &edit) => {
                tx.edits.push(edit);
                tx.after = after;
            }
            _ => {
                self.undo.push_back(Transaction {
                    edits: vec![edit],
                    kind,
                    before,
                    after,
                });
                if self.undo.len() > MAX_TRANSACTIONS {
                    self.undo.pop_front();
                }
            }
        }
        self.sealed = false;
        self.last_edit_at = Some(now);
    }

    /// 强制结束当前合并组（光标移动、保存、焦点切换）。
    pub fn seal(&mut self) {
        self.sealed = true;
    }

    /// 取出要撤销的 Transaction（已移入 redo 栈）。
    pub fn undo(&mut self) -> Option<Transaction> {
        let tx = self.undo.pop_back()?;
        self.redo.push(tx.clone());
        self.sealed = true;
        Some(tx)
    }

    pub fn redo(&mut self) -> Option<Transaction> {
        let tx = self.redo.pop()?;
        self.undo.push_back(tx.clone());
        self.sealed = true;
        Some(tx)
    }
}

fn mergeable(tx: &Transaction, edit: &Edit) -> bool {
    let Some(last) = tx.edits.last() else {
        return false;
    };
    if last.target != edit.target {
        return false;
    }
    match tx.kind {
        EditKind::Insert => {
            let adjacent = edit.at == last.at + last.inserted.chars().count();
            // 以词为单位：空白之后开始的新词另起一组
            let new_word = last
                .inserted
                .chars()
                .last()
                .is_some_and(char::is_whitespace)
                && !is_whitespace(&edit.inserted);
            adjacent && edit.inserted.graphemes(true).count() == 1 && !new_word
        }
        EditKind::DeleteBack => edit.at + edit.deleted.chars().count() == last.at,
        EditKind::DeleteForward => edit.at == last.at,
        EditKind::Other => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(rev: u64) -> Snapshot {
        Snapshot {
            focus: Focus::Body,
            cursor: Cursor::default(),
            title_cursor: 0,
            revision: rev,
        }
    }

    fn edit(at: usize, deleted: &str, inserted: &str) -> Edit {
        Edit {
            target: Target::Body,
            at,
            deleted: deleted.into(),
            inserted: inserted.into(),
        }
    }

    /// 依次记录 `(edit, kind, 距起点毫秒)`，返回各 Transaction 的 edit 数。
    fn groups(steps: &[(Edit, EditKind, u64)], seal_before: &[usize]) -> Vec<usize> {
        let t0 = Instant::now();
        let mut h = History::default();
        for (i, (e, kind, ms)) in steps.iter().enumerate() {
            if seal_before.contains(&i) {
                h.seal();
            }
            h.record(
                e.clone(),
                *kind,
                snap(0),
                snap(1),
                t0 + Duration::from_millis(*ms),
            );
        }
        h.undo.iter().map(|tx| tx.edits.len()).collect()
    }

    fn typing(text: &str, gap_ms: u64) -> Vec<(Edit, EditKind, u64)> {
        text.chars()
            .enumerate()
            .map(|(i, c)| {
                (
                    edit(i, "", &c.to_string()),
                    EditKind::Insert,
                    i as u64 * gap_ms,
                )
            })
            .collect()
    }

    #[test]
    fn typing_groups_by_word() {
        // "hello " | "world " | "foo"
        assert_eq!(groups(&typing("hello world foo", 50), &[]), vec![6, 6, 3]);
        // 中文没有空白：整段连续输入为一组
        assert_eq!(groups(&typing("中文输入", 50), &[]), vec![4]);
    }

    #[test]
    fn pause_of_one_second_splits() {
        assert_eq!(groups(&typing("abc", 999), &[]), vec![3]);
        assert_eq!(groups(&typing("abc", 1000), &[]), vec![1, 1, 1]);
    }

    #[test]
    fn non_adjacent_or_multi_grapheme_splits() {
        let steps = [
            (edit(0, "", "a"), EditKind::Insert, 0),
            (edit(5, "", "b"), EditKind::Insert, 10),
            (edit(6, "", "e\u{301}"), EditKind::Insert, 20), // 一个 grapheme
            (edit(8, "", "xy"), EditKind::Insert, 30),
        ];
        assert_eq!(groups(&steps, &[]), vec![1, 2, 1]);
    }

    #[test]
    fn seal_and_other_kinds_split() {
        let mut steps = typing("ab", 10);
        steps.push((edit(2, "", "\n"), EditKind::Other, 20));
        steps.push((edit(3, "", "c"), EditKind::Insert, 30));
        steps.push((edit(4, "", "d"), EditKind::Insert, 40));
        // 第 4 步前移动过光标
        assert_eq!(groups(&steps, &[4]), vec![2, 1, 1, 1]);
    }

    #[test]
    fn deletes_merge_per_direction() {
        let steps = [
            (edit(4, "d", ""), EditKind::DeleteBack, 0),
            (edit(3, "c", ""), EditKind::DeleteBack, 10),
            (edit(3, "x", ""), EditKind::DeleteForward, 20), // 方向改变
            (edit(3, "y", ""), EditKind::DeleteForward, 30),
            (edit(1, "a", ""), EditKind::DeleteBack, 40), // 方向再次改变
        ];
        assert_eq!(groups(&steps, &[]), vec![2, 2, 1]);
    }

    #[test]
    fn title_and_body_do_not_merge() {
        let mut title = edit(1, "", "b");
        title.target = Target::Title;
        let steps = [
            (edit(0, "", "a"), EditKind::Insert, 0),
            (title, EditKind::Insert, 10),
        ];
        assert_eq!(groups(&steps, &[]), vec![1, 1]);
    }

    #[test]
    fn redo_cleared_by_new_edit_and_capped() {
        let t0 = Instant::now();
        let mut h = History::default();
        for i in 0..MAX_TRANSACTIONS + 5 {
            h.seal();
            h.record(edit(i, "", "a"), EditKind::Insert, snap(0), snap(1), t0);
        }
        assert_eq!(h.undo.len(), MAX_TRANSACTIONS);
        assert!(h.undo().is_some());
        assert_eq!(h.redo.len(), 1);
        h.record(edit(0, "", "z"), EditKind::Insert, snap(0), snap(1), t0);
        assert!(h.redo.is_empty());
        // undo 之后的输入不并入更早的组
        assert_eq!(h.undo.back().unwrap().edits.len(), 1);
    }
}
