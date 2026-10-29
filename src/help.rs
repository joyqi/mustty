//! 键位帮助（Ctrl+G，vim Normal 态 `?`）：内容表与面板状态机。渲染见 `view::help`。
//!
//! 只列特殊的、不容易猜到的键：Enter、Backspace、Tab、打字这类任何编辑器都一样的不列，
//! 方向键 / Home / End / PgUp / PgDn 合并成一条「移动」。
//!
//! 每一条 `Entry` 都写明它在 `Keymap::map_event` 里应当得到什么（`Expect`），由测试逐条喂给
//! keymap 核对；反过来，keymap 能产生的每种 Action（常规输入除外）、Motion 与模式切换也必须
//! 被这张表覆盖。所以帮助里写的「Ctrl+S 保存」由测试保证是真的，新增了快捷键而忘了写进帮助
//! 会让测试失败。

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::action::{Action, Motion};
use crate::keymap::{Mode, NormalState};

/// 一条键位在 keymap 里应当产生的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expect {
    /// 按下这串键：最后一个键返回 `action`（之前的键必须返回 None），按完之后处于 `mode`
    /// （None 表示模式不变）
    Key {
        action: Option<Action>,
        mode: Option<Mode>,
    },
    /// 常规移动的合并项：其中每个键单独按下都得到 `Move(_, extend)`，模式不变
    Moves { extend: bool },
}

impl Expect {
    pub fn action(a: Action) -> Self {
        Self::Key {
            action: Some(a),
            mode: None,
        }
    }

    fn then(self, mode: Mode) -> Self {
        let Self::Key { action, .. } = self else {
            unreachable!("合并的移动项不切换模式");
        };
        Self::Key {
            action,
            mode: Some(mode),
        }
    }

    fn mode(mode: Mode) -> Self {
        Self::Key {
            action: None,
            mode: Some(mode),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// 显示的键位（不翻译），` / ` 分隔几种写法，测试逐个核对。`Expect::Moves` 里空格也分隔，
    /// 且 `↑↓←→` 这样连写的方向键各算一个键
    pub keys: &'static str,
    /// 说明文字的 i18n id
    pub desc_id: &'static str,
    /// 一个：所有写法结果相同；多个：与 ` / ` 分隔的写法一一对应（`Ctrl+Z / Ctrl+Y` 撤销 / 重做）
    pub expect: Vec<Expect>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// 组标题的 i18n id；默认模式只有一组，不显示标题
    pub title_id: Option<&'static str>,
    /// 这组键位在哪个模式下生效（测试从这个模式开始核对）
    pub mode: Mode,
    pub entries: Vec<Entry>,
}

const NORMAL: Mode = Mode::Normal(NormalState {
    pending: None,
    visual: false,
});
const VISUAL: Mode = Mode::Normal(NormalState {
    pending: None,
    visual: true,
});

fn entry(keys: &'static str, desc_id: &'static str, expect: Expect) -> Entry {
    each(keys, desc_id, vec![expect])
}

/// 几种写法各有各的结果，合成一条显示。
fn each(keys: &'static str, desc_id: &'static str, expect: Vec<Expect>) -> Entry {
    Entry {
        keys,
        desc_id,
        expect,
    }
}

fn mv(m: Motion, extend: bool) -> Expect {
    Expect::action(Action::Move(m, extend))
}

/// 默认模式（也是 vim 的 Insert 态）。只有 `Esc` 在两者之间不同。
fn base_entries(vim: bool) -> Vec<Entry> {
    use Action::*;
    use Motion::*;
    let a = Expect::action;
    let esc = if vim {
        entry("Esc", "help-esc-normal", a(Cancel).then(NORMAL))
    } else {
        entry("Esc", "help-esc", a(Cancel))
    };
    vec![
        entry(
            "↑↓←→ Home End PgUp PgDn",
            "help-move",
            Expect::Moves { extend: false },
        ),
        each(
            "Ctrl+← / Ctrl+→",
            "help-word-move",
            vec![mv(WordLeft, false), mv(WordRight, false)],
        ),
        each(
            "Ctrl+Home / Ctrl+End",
            "help-doc-ends",
            vec![mv(DocStart, false), mv(DocEnd, false)],
        ),
        entry("Shift+↑↓←→", "help-extend", Expect::Moves { extend: true }),
        entry(
            "Ctrl+Backspace / Ctrl+W",
            "help-delete-word",
            a(DeleteWordBack),
        ),
        entry("Ctrl+A", "help-select-all", a(SelectAll)),
        each(
            "Ctrl+C / Ctrl+X / Ctrl+V",
            "help-clipboard",
            vec![a(Copy), a(Cut), a(PasteRegister)],
        ),
        each("Ctrl+Z / Ctrl+Y", "help-undo-redo", vec![a(Undo), a(Redo)]),
        esc,
        entry("Ctrl+S", "help-save", a(Save)),
        entry("Ctrl+Q", "help-quit", a(Quit)),
        entry("Ctrl+T", "help-typewriter", a(ToggleTypewriter)),
        entry("Ctrl+F", "help-focus", a(ToggleFocusMode)),
        entry("Ctrl+P", "help-settings", a(OpenSettings)),
        entry("Ctrl+G", "help-help", a(OpenHelp)),
    ]
}

fn normal_entries() -> Vec<Entry> {
    use Action::*;
    use Motion::*;
    let a = Expect::action;
    vec![
        entry("i", "help-vim-insert", Expect::mode(Mode::Insert)),
        entry("a", "help-vim-append", mv(Right, false).then(Mode::Insert)),
        entry(
            "o",
            "help-vim-open-line",
            a(OpenLineBelow).then(Mode::Insert),
        ),
        entry("v", "help-vim-visual", Expect::mode(VISUAL)),
        entry("h", "help-left", mv(Left, false)),
        entry("j", "help-down", mv(Down, false)),
        entry("k", "help-up", mv(Up, false)),
        entry("l", "help-right", mv(Right, false)),
        entry("w", "help-vim-word-next", mv(WordRight, false)),
        entry("b", "help-word-left", mv(WordLeft, false)),
        entry("e", "help-vim-word-end", mv(WordEnd, false)),
        entry("0", "help-line-start", mv(LineStart, false)),
        entry("$", "help-line-end", mv(LineEnd, false)),
        entry("gg", "help-doc-start", mv(DocStart, false)),
        entry("G", "help-doc-end", mv(DocEnd, false)),
        entry("x", "help-delete-forward", a(DeleteForward)),
        entry("dd", "help-vim-cut-line", a(DeleteLine)),
        entry("p", "help-vim-paste", a(PasteRegister)),
        entry("u", "help-undo", a(Undo)),
        entry("Ctrl+R", "help-redo", a(Redo)),
        entry("?", "help-help", a(OpenHelp)),
    ]
}

fn visual_entries() -> Vec<Entry> {
    use Action::*;
    let a = Expect::action;
    vec![
        entry("l", "help-vim-extend", mv(Motion::Right, true)),
        entry("y", "help-copy", a(Copy).then(NORMAL)),
        entry("d / x", "help-cut", a(Cut).then(NORMAL)),
        entry("p", "help-vim-replace", a(PasteRegister).then(NORMAL)),
        entry("v / Esc", "help-vim-leave-visual", a(Cancel).then(NORMAL)),
        entry("i", "help-vim-insert", a(Cancel).then(Mode::Insert)),
    ]
}

/// 帮助的全部内容：默认模式一张表；vim 下分 Insert / Normal / Visual 三组。
pub fn groups(vim: bool) -> Vec<Group> {
    if !vim {
        return vec![Group {
            title_id: None,
            mode: Mode::Insert,
            entries: base_entries(false),
        }];
    }
    vec![
        Group {
            title_id: Some("help-group-insert"),
            mode: Mode::Insert,
            entries: base_entries(true),
        },
        Group {
            title_id: Some("help-group-normal"),
            mode: NORMAL,
            entries: normal_entries(),
        },
        Group {
            title_id: Some("help-group-visual"),
            mode: VISUAL,
            entries: visual_entries(),
        },
    ]
}

/// 面板里的一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    Title(&'static str),
    Entry(&'static str, &'static str),
    Blank,
}

/// 帮助里用到的全部 i18n id（说明与组标题）。它们存在表里、经 `i18n::t_id` 取，
/// i18n 的测试据此确认每个 id 都在 en.toml 里。
pub fn ids() -> Vec<&'static str> {
    [false, true]
        .into_iter()
        .flat_map(lines)
        .filter_map(|l| match l {
            Line::Title(id) | Line::Entry(_, id) => Some(id),
            Line::Blank => None,
        })
        .collect()
}

/// 展开成逐行内容：组之间空一行，组标题单独一行。
pub fn lines(vim: bool) -> Vec<Line> {
    let mut out = Vec::new();
    for (i, g) in groups(vim).into_iter().enumerate() {
        if i > 0 {
            out.push(Line::Blank);
        }
        if let Some(id) = g.title_id {
            out.push(Line::Title(id));
        }
        out.extend(g.entries.iter().map(|e| Line::Entry(e.keys, e.desc_id)));
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Ignored,
    Scrolled,
    Close,
    /// Ctrl+Q：先关面板，再走正常退出流程
    Quit,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Help {
    /// 第一行显示的行号
    pub scroll: usize,
}

impl Help {
    /// `total` 行内容、正文区 `visible` 行高。滚动首尾不循环；一页是 `visible - 2` 行，与正文的
    /// PageUp / PageDown 一致。`?` 只在 vim 下关闭（默认模式里它就是个普通字符）。
    pub fn handle_key(
        &mut self,
        key: &KeyEvent,
        vim: bool,
        total: usize,
        visible: usize,
    ) -> Outcome {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return Outcome::Ignored;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let page = visible.saturating_sub(2).max(1) as isize;
        let max = total.saturating_sub(visible);
        let delta = match key.code {
            KeyCode::Char('q' | 'Q') if ctrl => return Outcome::Quit,
            KeyCode::Char('g' | 'G') if ctrl => return Outcome::Close,
            KeyCode::Esc => return Outcome::Close,
            KeyCode::Char('?') if vim => return Outcome::Close,
            KeyCode::Up => -1,
            KeyCode::Down => 1,
            KeyCode::PageUp => -page,
            KeyCode::PageDown => page,
            KeyCode::Home => -(self.scroll as isize),
            KeyCode::End => max as isize - self.scroll as isize,
            _ => return Outcome::Ignored,
        };
        let next = self.scroll.saturating_add_signed(delta).min(max);
        if next == self.scroll {
            return Outcome::Ignored;
        }
        self.scroll = next;
        Outcome::Scrolled
    }

    /// 窗口变高或内容变短后，滚动量不超过末尾。
    pub fn clamp(&mut self, total: usize, visible: usize) -> usize {
        self.scroll = self.scroll.min(total.saturating_sub(visible));
        self.scroll
    }
}

/// 方向键符号对应的按键，其余字符原样。
fn char_key(c: char) -> KeyCode {
    match c {
        '←' => KeyCode::Left,
        '→' => KeyCode::Right,
        '↑' => KeyCode::Up,
        '↓' => KeyCode::Down,
        c => KeyCode::Char(c),
    }
}

/// 把 `Ctrl+Shift+→`、`PgUp`、`gg`、`Shift+↑↓` 这类写法还原成按键序列。
pub fn parse_keys(keys: &str) -> Vec<KeyEvent> {
    let mut mods = KeyModifiers::NONE;
    let mut rest = keys;
    loop {
        let (m, tail) = match rest.split_once('+') {
            Some(("Ctrl", tail)) if !tail.is_empty() => (KeyModifiers::CONTROL, tail),
            Some(("Shift", tail)) if !tail.is_empty() => (KeyModifiers::SHIFT, tail),
            Some(("Alt", tail)) if !tail.is_empty() => (KeyModifiers::ALT, tail),
            _ => break,
        };
        mods |= m;
        rest = tail;
    }
    let named = match rest {
        "Enter" => Some(KeyCode::Enter),
        "Backspace" => Some(KeyCode::Backspace),
        "Delete" => Some(KeyCode::Delete),
        "Tab" => Some(KeyCode::Tab),
        "Esc" => Some(KeyCode::Esc),
        "Home" => Some(KeyCode::Home),
        "End" => Some(KeyCode::End),
        "PgUp" | "PageUp" => Some(KeyCode::PageUp),
        "PgDn" | "PageDown" => Some(KeyCode::PageDown),
        _ => None,
    };
    match named {
        Some(code) => vec![KeyEvent::new(code, mods)],
        // 修饰键后的字母按小写（Ctrl+S 即 Ctrl+s）；没有修饰键时是逐个字符的序列（gg、dd、$、↑↓）
        None if mods != KeyModifiers::NONE => rest
            .chars()
            .map(|c| KeyEvent::new(char_key(c.to_ascii_lowercase()), mods))
            .collect(),
        None => rest
            .chars()
            .map(|c| KeyEvent::new(char_key(c), KeyModifiers::NONE))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::Keymap;
    use crossterm::event::Event;
    use std::collections::BTreeSet;
    use std::mem::discriminant;

    /// 从 `mode` 开始依次按下 `keys`，返回最后一个键的 Action 与结束时的模式；之前的键必须没有 Action。
    fn press(vim: bool, mode: Mode, keys: &[KeyEvent]) -> (Option<Action>, Mode) {
        let keymap = Keymap { vim };
        let mut mode = mode;
        let mut last = None;
        for (i, k) in keys.iter().enumerate() {
            last = keymap.map_event(&Event::Key(*k), &mut mode, false);
            if i + 1 < keys.len() {
                assert_eq!(last, None, "前缀键 {k:?} 不应产生 Action");
            }
        }
        (last, mode)
    }

    /// `Expect::Moves` 项里的每个键（写法之间用空格或 ` / ` 分隔，`↑↓←→` 各算一个）。
    fn move_keys(keys: &str) -> Vec<KeyEvent> {
        keys.split([' ', '/'])
            .filter(|w| !w.is_empty())
            .flat_map(parse_keys)
            .collect()
    }

    #[test]
    fn every_entry_matches_the_keymap() {
        let mut checked = 0;
        for vim in [false, true] {
            for g in groups(vim) {
                for e in &g.entries {
                    if let [Expect::Moves { extend }] = e.expect[..] {
                        let keys = move_keys(e.keys);
                        assert!(keys.len() >= 4, "{:?}", e.keys);
                        for k in keys {
                            let (action, mode) = press(vim, g.mode, &[k]);
                            assert!(
                                matches!(action, Some(Action::Move(_, x)) if x == extend)
                                    && mode == g.mode,
                                "vim={vim} {:?} 组的 {:?}（{}）里 {k:?} 得到 {action:?} / {mode:?}",
                                g.title_id,
                                e.keys,
                                e.desc_id
                            );
                            checked += 1;
                        }
                        continue;
                    }
                    let alts: Vec<&str> = e.keys.split(" / ").collect();
                    assert!(
                        e.expect.len() == 1 || e.expect.len() == alts.len(),
                        "{:?}：{} 种写法却有 {} 个期望",
                        e.keys,
                        alts.len(),
                        e.expect.len()
                    );
                    for (i, alt) in alts.iter().enumerate() {
                        let Expect::Key {
                            action: want,
                            mode: want_mode,
                        } = &e.expect[i.min(e.expect.len() - 1)]
                        else {
                            panic!("{:?}：Moves 只能单独使用", e.keys);
                        };
                        let keys = parse_keys(alt);
                        assert!(!keys.is_empty(), "{alt:?}");
                        let (action, mode) = press(vim, g.mode, &keys);
                        assert_eq!(
                            (&action, mode),
                            (want, want_mode.unwrap_or(g.mode)),
                            "vim={vim} {:?} 组的 {alt:?}（{}）与 keymap 不符",
                            g.title_id,
                            e.desc_id
                        );
                        checked += 1;
                    }
                }
            }
        }
        assert!(checked > 60, "只核对了 {checked} 条");
    }

    /// keymap 在某个模式下能产生的所有结果：单键（各种修饰键组合）与 g / d 前缀的两键序列。
    fn reachable(vim: bool, start: Mode) -> Vec<(Option<Action>, Mode, Mode)> {
        let mut codes: Vec<KeyCode> = (' '..='~').map(KeyCode::Char).collect();
        codes.extend([
            KeyCode::Enter,
            KeyCode::Backspace,
            KeyCode::Delete,
            KeyCode::Tab,
            KeyCode::BackTab,
            KeyCode::Esc,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Insert,
            KeyCode::F(1),
        ]);
        let mods = [
            KeyModifiers::NONE,
            KeyModifiers::CONTROL,
            KeyModifiers::ALT,
            KeyModifiers::SHIFT,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ];
        let mut out = Vec::new();
        for &code in &codes {
            for &m in &mods {
                let k = KeyEvent::new(code, m);
                let (action, mode) = press(vim, start, &[k]);
                out.push((action, start, mode));
                if matches!(mode, Mode::Normal(s) if s.pending.is_some()) {
                    for &second in &codes {
                        let k2 = KeyEvent::new(second, KeyModifiers::NONE);
                        let (action, end) = press(vim, mode, &[k2]);
                        out.push((action, start, end));
                    }
                }
            }
        }
        out
    }

    /// 任何文本编辑器都一样的常规输入：帮助里不列，反向测试也不要求。没有通配分支，
    /// 新增 Action 变体时必须在这里表态——默认（false）就是要求它出现在帮助里。
    fn conventional(a: &Action) -> bool {
        match a {
            Action::Insert(_)
            | Action::Tab
            | Action::Newline
            | Action::DeleteBack
            | Action::DeleteForward => true,
            // 不由按键产生（粘贴事件、保存提示行、鼠标），出现在这里说明 keymap 变了，要写进帮助
            Action::Paste(_)
            | Action::PromptInput(_)
            | Action::MoveTo(..)
            | Action::ScrollView(_) => false,
            Action::DeleteWordBack
            | Action::Move(..)
            | Action::Undo
            | Action::Redo
            | Action::Save
            | Action::Quit
            | Action::Cancel
            | Action::SelectAll
            | Action::Copy
            | Action::Cut
            | Action::PasteRegister
            | Action::DeleteLine
            | Action::OpenLineBelow
            | Action::ToggleTypewriter
            | Action::ToggleFocusMode
            | Action::OpenSettings
            | Action::OpenHelp => false,
        }
    }

    #[test]
    fn every_binding_appears_in_help() {
        // 帮助覆盖到的 Action 变体、Motion 与模式切换。合并的移动项不逐个写 Motion，
        // 它覆盖的就是其中各键实际得到的 Motion（前一个测试已核对它们都是移动）。
        let mut variants = BTreeSet::new();
        let mut motions = BTreeSet::new();
        let mut switches = BTreeSet::new();
        for vim in [false, true] {
            for g in groups(vim) {
                for e in &g.entries {
                    for x in &e.expect {
                        match x {
                            Expect::Key { action, mode } => {
                                if let Some(a) = action {
                                    variants.insert(format!("{:?}", discriminant(a)));
                                    if let Action::Move(m, _) = a {
                                        motions.insert(format!("{m:?}"));
                                    }
                                }
                                if let Some(m) = mode {
                                    switches.insert(format!("{:?} → {m:?}", g.mode));
                                }
                            }
                            Expect::Moves { .. } => {
                                for k in move_keys(e.keys) {
                                    if let (Some(a @ Action::Move(m, _)), _) =
                                        press(vim, g.mode, &[k])
                                    {
                                        variants.insert(format!("{:?}", discriminant(&a)));
                                        motions.insert(format!("{m:?}"));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        // 常规移动只靠合并项覆盖：默认表（也是 vim 的 Insert 组）里必须有它
        for vim in [false, true] {
            assert!(
                groups(vim)[0]
                    .entries
                    .iter()
                    .any(|e| e.expect == [Expect::Moves { extend: false }]),
                "vim={vim}：缺少覆盖常规移动的合并项"
            );
        }
        let starts = [
            (false, Mode::Insert),
            (true, Mode::Insert),
            (true, NORMAL),
            (true, VISUAL),
        ];
        for (vim, start) in starts {
            for (action, from, to) in reachable(vim, start) {
                if let Some(a) = &action {
                    if !conventional(a) {
                        assert!(
                            variants.contains(&format!("{:?}", discriminant(a))),
                            "vim={vim} {from:?} 下的 {a:?} 没有出现在帮助里"
                        );
                    }
                    if let Action::Move(m, _) = a {
                        assert!(
                            motions.contains(&format!("{m:?}")),
                            "{m:?} 没有被帮助覆盖（单列或合并的移动项）"
                        );
                    }
                }
                // 模式切换（不计 g / d 前缀这种中间状态）
                let settled = !matches!(to, Mode::Normal(s) if s.pending.is_some());
                if settled && to != from {
                    assert!(
                        switches.contains(&format!("{from:?} → {to:?}")),
                        "vim={vim} 的模式切换 {from:?} → {to:?} 没有出现在帮助里（{action:?}）"
                    );
                }
            }
        }
    }

    #[test]
    fn parse_keys_handles_modifiers_names_and_sequences() {
        let k = |c, m| KeyEvent::new(c, m);
        assert_eq!(
            parse_keys("Ctrl+S"),
            vec![k(KeyCode::Char('s'), KeyModifiers::CONTROL)]
        );
        assert_eq!(
            parse_keys("Shift+→"),
            vec![k(KeyCode::Right, KeyModifiers::SHIFT)]
        );
        assert_eq!(
            parse_keys("Ctrl+Home"),
            vec![k(KeyCode::Home, KeyModifiers::CONTROL)]
        );
        assert_eq!(
            parse_keys("gg"),
            vec![
                k(KeyCode::Char('g'), KeyModifiers::NONE),
                k(KeyCode::Char('g'), KeyModifiers::NONE)
            ]
        );
        assert_eq!(
            parse_keys("G"),
            vec![k(KeyCode::Char('G'), KeyModifiers::NONE)]
        );
        assert_eq!(
            parse_keys("$"),
            vec![k(KeyCode::Char('$'), KeyModifiers::NONE)]
        );
        assert_eq!(
            parse_keys("PgDn"),
            vec![k(KeyCode::PageDown, KeyModifiers::NONE)]
        );
        assert_eq!(
            parse_keys("Shift+↑↓"),
            vec![
                k(KeyCode::Up, KeyModifiers::SHIFT),
                k(KeyCode::Down, KeyModifiers::SHIFT)
            ]
        );
    }

    #[test]
    fn default_list_is_short() {
        // 只列特殊快捷键：默认表不超过 15 条，正文 16 行高时一屏放得下
        let plain = lines(false);
        assert!(plain.len() <= 15, "默认表有 {} 条", plain.len());
        // vim 的 Insert 组与默认表同样精简，Normal / Visual 两组照旧全列
        let g = groups(true);
        assert_eq!(g[0].entries.len(), plain.len());
        assert!(g[1].entries.len() > g[0].entries.len());
        // 常规输入不单列
        for vim in [false, true] {
            for e in groups(vim).iter().take(1).flat_map(|g| &g.entries) {
                for gone in ["Enter", "Backspace", "Delete", "Tab", "Home", "PgUp"] {
                    assert!(e.keys != gone, "vim={vim}：{gone} 不该单列（{})", e.desc_id);
                }
            }
        }
    }

    #[test]
    fn lines_group_vim_help() {
        let plain = lines(false);
        assert!(
            plain.iter().all(|l| matches!(l, Line::Entry(..))),
            "默认模式不分组"
        );
        let vim = lines(true);
        let titles: Vec<_> = vim
            .iter()
            .filter_map(|l| match l {
                Line::Title(id) => Some(*id),
                _ => None,
            })
            .collect();
        assert_eq!(
            titles,
            vec![
                "help-group-insert",
                "help-group-normal",
                "help-group-visual"
            ]
        );
        assert_eq!(vim.iter().filter(|l| **l == Line::Blank).count(), 2);
    }

    #[test]
    fn scrolling_stops_at_both_ends() {
        let mut h = Help::default();
        let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
        let (total, visible) = (30, 10);
        assert_eq!(
            h.handle_key(&key(KeyCode::Up), false, total, visible),
            Outcome::Ignored
        );
        assert_eq!(
            h.handle_key(&key(KeyCode::Down), false, total, visible),
            Outcome::Scrolled
        );
        assert_eq!(h.scroll, 1);
        for _ in 0..10 {
            h.handle_key(&key(KeyCode::PageDown), false, total, visible);
        }
        assert_eq!(h.scroll, 20, "停在末尾：最后一行在最下面");
        assert_eq!(
            h.handle_key(&key(KeyCode::Down), false, total, visible),
            Outcome::Ignored
        );
        h.handle_key(&key(KeyCode::PageUp), false, total, visible);
        assert_eq!(h.scroll, 12, "一页 = 正文高度 - 2，与正文的分页一致");
        for _ in 0..5 {
            h.handle_key(&key(KeyCode::PageUp), false, total, visible);
        }
        assert_eq!(h.scroll, 0);
        // Home / End 跳到首尾，已在那里时不再变化
        assert_eq!(
            h.handle_key(&key(KeyCode::End), false, total, visible),
            Outcome::Scrolled
        );
        assert_eq!(h.scroll, 20);
        assert_eq!(
            h.handle_key(&key(KeyCode::End), false, total, visible),
            Outcome::Ignored
        );
        assert_eq!(
            h.handle_key(&key(KeyCode::Home), false, total, visible),
            Outcome::Scrolled
        );
        assert_eq!(h.scroll, 0);
        assert_eq!(
            h.handle_key(&key(KeyCode::Home), false, total, visible),
            Outcome::Ignored
        );
        // 放得下时不滚动
        let mut short = Help::default();
        assert_eq!(
            short.handle_key(&key(KeyCode::Down), false, 5, 10),
            Outcome::Ignored
        );
        // 内容变短 / 窗口变高后钳制
        h.scroll = 20;
        assert_eq!(h.clamp(30, 25), 5);
    }

    #[test]
    fn closing_keys() {
        let mut h = Help::default();
        let ctrl = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
        let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
        assert_eq!(
            h.handle_key(&key(KeyCode::Esc), false, 1, 1),
            Outcome::Close
        );
        assert_eq!(h.handle_key(&ctrl('g'), false, 1, 1), Outcome::Close);
        assert_eq!(h.handle_key(&ctrl('q'), false, 1, 1), Outcome::Quit);
        assert_eq!(
            h.handle_key(&key(KeyCode::Char('?')), true, 1, 1),
            Outcome::Close
        );
        assert_eq!(
            h.handle_key(&key(KeyCode::Char('?')), false, 1, 1),
            Outcome::Ignored,
            "默认模式里 ? 是普通字符：面板吞掉它，不关闭"
        );
        assert_eq!(
            h.handle_key(&ctrl('p'), false, 1, 1),
            Outcome::Ignored,
            "不能从帮助里打开设置"
        );
    }
}
