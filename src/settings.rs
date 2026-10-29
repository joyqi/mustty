//! Settings panel: 13 table-driven options and the panel state machine. Rendering lives in `view::settings`.
//!
//! The panel only reads and writes a copy of `Config`. App calls `apply_config` after every change,
//! and decides on close whether to keep it or roll back.

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::config::{
    AUTOSAVE_IDLE_RANGE, CodeColor, Config, CursorShape, SCROLLOFF_RANGE, TAB_WIDTH_RANGE,
    WIDTH_RANGE,
};
use crate::i18n::{self, t};

/// Steps per PageUp / PageDown.
pub const PAGE_STEPS: i64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Width,
    Vim,
    Mouse,
    Autosave,
    AutosaveIdle,
    Typewriter,
    FocusMode,
    Scrolloff,
    TabWidth,
    CodeColor,
    Cursor,
    CursorBlink,
    Lang,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Toggle,
    /// `unit` is the translated unit
    Int {
        min: i64,
        max: i64,
        step: i64,
        unit: &'static str,
    },
    Choice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Color(CodeColor),
    Cursor(CursorShape),
    /// `i18n::AUTO` or a locale tag
    Lang(&'static str),
}

/// Choices for the Language item: Auto, then every language in `i18n::LOCALES`. A new catalog adds a choice automatically.
pub fn lang_options() -> Vec<&'static str> {
    std::iter::once(i18n::AUTO).chain(i18n::tags()).collect()
}

fn range<T: Into<i64>>((lo, hi): (T, T)) -> (i64, i64) {
    (lo.into(), hi.into())
}

impl Field {
    /// Display order.
    pub const ALL: [Field; 13] = [
        Self::Width,
        Self::Vim,
        Self::Mouse,
        Self::Autosave,
        Self::AutosaveIdle,
        Self::Typewriter,
        Self::FocusMode,
        Self::Scrolloff,
        Self::TabWidth,
        Self::CodeColor,
        Self::Cursor,
        Self::CursorBlink,
        Self::Lang,
    ];

    /// Key name in the config file (matches `config::KEYS`).
    pub fn key(self) -> &'static str {
        match self {
            Self::Width => "width",
            Self::Vim => "vim",
            Self::Mouse => "mouse",
            Self::Autosave => "autosave",
            Self::AutosaveIdle => "autosave_idle_ms",
            Self::Typewriter => "typewriter",
            Self::FocusMode => "focus_mode",
            Self::Scrolloff => "scrolloff",
            Self::TabWidth => "tab_width",
            Self::CodeColor => "code_color",
            Self::Cursor => "cursor",
            Self::CursorBlink => "cursor_blink",
            Self::Lang => "lang",
        }
    }

    /// Label in the panel. With vim on, the two cursor items only affect Insert mode, and the label says so.
    /// It is read from the current `config.vim` at render time, so toggling vim in the panel takes effect immediately.
    pub fn label(self, vim: bool) -> &'static str {
        match self {
            Self::Width => t("settings-width"),
            Self::Vim => t("settings-vim"),
            Self::Mouse => t("settings-mouse"),
            Self::Autosave => t("settings-autosave"),
            Self::AutosaveIdle => t("settings-autosave-idle"),
            Self::Typewriter => t("settings-typewriter"),
            Self::FocusMode => t("settings-focus-mode"),
            Self::Scrolloff => t("settings-scrolloff"),
            Self::TabWidth => t("settings-tab-width"),
            Self::CodeColor => t("settings-code-color"),
            Self::Cursor if vim => t("settings-cursor-vim"),
            Self::CursorBlink if vim => t("settings-cursor-blink-vim"),
            Self::Cursor => t("settings-cursor"),
            Self::CursorBlink => t("settings-cursor-blink"),
            Self::Lang => t("settings-lang"),
        }
    }

    /// The sentence shown in the help line when selected; `vim` works as in [`Field::label`].
    pub fn help(self, vim: bool) -> &'static str {
        match self {
            Self::Width => t("settings-width-help"),
            Self::Vim => t("settings-vim-help"),
            Self::Mouse => t("settings-mouse-help"),
            Self::Autosave => t("settings-autosave-help"),
            Self::AutosaveIdle => t("settings-autosave-idle-help"),
            Self::Typewriter => t("settings-typewriter-help"),
            Self::FocusMode => t("settings-focus-mode-help"),
            Self::Scrolloff => t("settings-scrolloff-help"),
            Self::TabWidth => t("settings-tab-width-help"),
            Self::CodeColor => t("settings-code-color-help"),
            Self::Cursor if vim => t("settings-cursor-help-vim"),
            Self::CursorBlink if vim => t("settings-cursor-blink-help-vim"),
            Self::Cursor => t("settings-cursor-help"),
            Self::CursorBlink => t("settings-cursor-blink-help"),
            Self::Lang => t("settings-lang-help"),
        }
    }

    /// Ranges reuse the `*_RANGE` constants from config.rs, so they match the config file's clamping.
    pub fn kind(self) -> Kind {
        let int = |(min, max), step, unit| Kind::Int {
            min,
            max,
            step,
            unit,
        };
        match self {
            Self::Width => int(range(WIDTH_RANGE), 2, t("settings-unit-columns")),
            Self::AutosaveIdle => int(range(AUTOSAVE_IDLE_RANGE_I64), 500, t("settings-unit-ms")),
            Self::Scrolloff => int(range(SCROLLOFF_RANGE), 1, t("settings-unit-lines")),
            Self::TabWidth => int(range(TAB_WIDTH_RANGE), 1, t("settings-unit-spaces")),
            Self::CodeColor | Self::Cursor | Self::Lang => Kind::Choice,
            Self::Vim
            | Self::Mouse
            | Self::Autosave
            | Self::Typewriter
            | Self::FocusMode
            | Self::CursorBlink => Kind::Toggle,
        }
    }

    pub fn get(self, c: &Config) -> Value {
        match self {
            Self::Width => Value::Int(c.width.into()),
            Self::Vim => Value::Bool(c.vim),
            Self::Mouse => Value::Bool(c.mouse),
            Self::Autosave => Value::Bool(c.autosave),
            Self::AutosaveIdle => Value::Int(c.autosave_idle.as_millis() as i64),
            Self::Typewriter => Value::Bool(c.typewriter),
            Self::FocusMode => Value::Bool(c.focus_mode),
            Self::Scrolloff => Value::Int(c.scrolloff.into()),
            Self::TabWidth => Value::Int(c.tab_width.into()),
            Self::CodeColor => Value::Color(c.code_color),
            Self::Cursor => Value::Cursor(c.cursor),
            Self::CursorBlink => Value::Bool(c.cursor_blink),
            Self::Lang => Value::Lang(c.lang),
        }
    }

    /// Writes one option. Numbers are clamped to range; values of the wrong type are ignored.
    pub fn set(self, c: &mut Config, v: Value) {
        let clamp = |i: i64| match self.kind() {
            Kind::Int { min, max, .. } => i.clamp(min, max),
            _ => i,
        };
        match (self, v) {
            (Self::Width, Value::Int(i)) => c.width = clamp(i) as u16,
            (Self::Vim, Value::Bool(b)) => c.vim = b,
            (Self::Mouse, Value::Bool(b)) => c.mouse = b,
            (Self::Autosave, Value::Bool(b)) => c.autosave = b,
            (Self::AutosaveIdle, Value::Int(i)) => {
                c.autosave_idle = Duration::from_millis(clamp(i) as u64)
            }
            (Self::Typewriter, Value::Bool(b)) => c.typewriter = b,
            (Self::FocusMode, Value::Bool(b)) => c.focus_mode = b,
            (Self::Scrolloff, Value::Int(i)) => c.scrolloff = clamp(i) as u16,
            (Self::TabWidth, Value::Int(i)) => c.tab_width = clamp(i) as u16,
            (Self::CodeColor, Value::Color(color)) => c.code_color = color,
            (Self::Cursor, Value::Cursor(shape)) => c.cursor = shape,
            (Self::CursorBlink, Value::Bool(b)) => c.cursor_blink = b,
            (Self::Lang, Value::Lang(lang)) => {
                // Reject tags without a catalog (right type, just not among the choices)
                if lang_options().contains(&lang) {
                    c.lang = lang;
                }
            }
            _ => debug_assert!(false, "{self:?} doesn't accept {v:?}"),
        }
    }

    /// The value after `steps` adjustments: toggles flip, numbers move by step and stop at the bounds, colors cycle.
    pub fn adjust(self, c: &Config, steps: i64) -> Value {
        match (self.get(c), self.kind()) {
            (Value::Bool(b), _) => Value::Bool(!b),
            (Value::Int(i), Kind::Int { min, max, step, .. }) => {
                Value::Int((i + steps * step).clamp(min, max))
            }
            (Value::Color(color), _) => Value::Color(cycle(&CodeColor::ALL, color, steps)),
            (Value::Cursor(shape), _) => Value::Cursor(cycle(&CursorShape::ALL, shape, steps)),
            (Value::Lang(lang), _) => Value::Lang(cycle(&lang_options(), lang, steps)),
            (v, _) => v,
        }
    }
}

/// Cycles a choice by one step (direction is the sign of `steps`).
fn cycle<T: Copy + PartialEq>(options: &[T], current: T, steps: i64) -> T {
    let at = options.iter().position(|&x| x == current).unwrap_or(0) as i64;
    let n = options.len() as i64;
    options[(at + steps.signum()).rem_euclid(n) as usize]
}

const AUTOSAVE_IDLE_RANGE_I64: (i64, i64) =
    (AUTOSAVE_IDLE_RANGE.0 as i64, AUTOSAVE_IDLE_RANGE.1 as i64);

/// Color name (translated).
pub fn color_name(c: CodeColor) -> &'static str {
    match c {
        CodeColor::Black => t("color-black"),
        CodeColor::Red => t("color-red"),
        CodeColor::Green => t("color-green"),
        CodeColor::Yellow => t("color-yellow"),
        CodeColor::Blue => t("color-blue"),
        CodeColor::Magenta => t("color-magenta"),
        CodeColor::Cyan => t("color-cyan"),
        CodeColor::White => t("color-white"),
    }
}

/// Cursor shape name (translated).
pub fn cursor_name(c: CursorShape) -> &'static str {
    match c {
        CursorShape::Bar => t("cursor-bar"),
        CursorShape::Block => t("cursor-block"),
        CursorShape::Underline => t("cursor-underline"),
    }
}

/// What the panel made of a key press; App carries it out.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Swallowed, nothing happens (unbound keys, key releases)
    Ignored,
    /// The selection moved
    Moved,
    /// An option changed: apply this config right away
    Changed(Config),
    /// Enter: keep the current values, write them back to the file, close
    Save,
    /// Esc / Ctrl+P: restore the snapshot, close
    Cancel,
    /// Ctrl+Q: roll back as Cancel does, then go through the normal quit flow
    Quit,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Index into `Field::ALL`
    pub selected: usize,
    /// First row the list is scrolled to (when the body area can't fit 10 rows)
    pub scroll: usize,
    /// Config at the time the panel opened: Esc rolls back to it, Enter writes only keys that differ from it
    pub snapshot: Config,
}

impl Settings {
    pub fn open(current: &Config) -> Self {
        Self {
            selected: 0,
            scroll: 0,
            snapshot: current.clone(),
        }
    }

    pub fn field(&self) -> Field {
        Field::ALL[self.selected]
    }

    pub fn handle_key(&mut self, key: &KeyEvent, current: &Config) -> Outcome {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return Outcome::Ignored;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let field = self.field();
        let adjust = |steps: i64| {
            let mut c = current.clone();
            field.set(&mut c, field.adjust(current, steps));
            if c == *current {
                Outcome::Ignored
            } else {
                Outcome::Changed(c)
            }
        };
        match key.code {
            KeyCode::Char('q' | 'Q') if ctrl => Outcome::Quit,
            KeyCode::Char('p' | 'P') if ctrl => Outcome::Cancel,
            KeyCode::Esc => Outcome::Cancel,
            KeyCode::Enter => Outcome::Save,
            KeyCode::Up | KeyCode::BackTab => self.select(-1),
            KeyCode::Down | KeyCode::Tab => self.select(1),
            KeyCode::Left => adjust(-1),
            KeyCode::Right => adjust(1),
            KeyCode::Char(' ') if field.kind() == Kind::Toggle => adjust(1),
            KeyCode::PageUp if matches!(field.kind(), Kind::Int { .. }) => adjust(-PAGE_STEPS),
            KeyCode::PageDown if matches!(field.kind(), Kind::Int { .. }) => adjust(PAGE_STEPS),
            _ => Outcome::Ignored,
        }
    }

    /// Doesn't wrap at either end.
    fn select(&mut self, delta: isize) -> Outcome {
        let last = Field::ALL.len() - 1;
        let next = self.selected.saturating_add_signed(delta).min(last);
        if next == self.selected {
            return Outcome::Ignored;
        }
        self.selected = next;
        Outcome::Moved
    }

    /// Keeps the selection visible when the list has only `visible` rows; returns the index of the first row.
    pub fn follow(&mut self, visible: usize) -> usize {
        let visible = visible.max(1);
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + visible {
            self.scroll = self.selected + 1 - visible;
        }
        self.scroll = self.scroll.min(Field::ALL.len().saturating_sub(visible));
        self.scroll
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn every_field_round_trips_and_keys_match_config() {
        assert_eq!(
            Field::ALL.map(Field::key),
            crate::config::KEYS,
            "order and key names match config::KEYS"
        );
        let mut c = Config::default();
        let values = [
            Value::Int(90),
            Value::Bool(true),
            Value::Bool(true),
            Value::Bool(false),
            Value::Int(4500),
            Value::Bool(true),
            Value::Bool(true),
            Value::Int(7),
            Value::Int(8),
            Value::Color(CodeColor::Magenta),
            Value::Cursor(CursorShape::Underline),
            Value::Bool(true),
            Value::Lang("zh-CN"),
        ];
        assert_eq!(values.len(), Field::ALL.len());
        for (f, v) in Field::ALL.into_iter().zip(values) {
            assert_ne!(
                f.get(&c),
                v,
                "{f:?}: the test value must differ from the default"
            );
            f.set(&mut c, v);
            assert_eq!(f.get(&c), v, "{f:?}");
        }
        assert_eq!(
            crate::config::changed_keys(&Config::default(), &c),
            crate::config::KEYS.to_vec()
        );
    }

    #[test]
    fn numbers_stop_at_bounds() {
        let cases = [
            (Field::Width, 40, 120, 2),
            (Field::AutosaveIdle, 500, 60_000, 500),
            (Field::Scrolloff, 0, 10, 1),
            (Field::TabWidth, 1, 16, 1),
        ];
        for (f, min, max, step) in cases {
            let mut c = Config::default();
            assert_eq!(
                f.kind(),
                Kind::Int {
                    min,
                    max,
                    step,
                    unit: match f.kind() {
                        Kind::Int { unit, .. } => unit,
                        _ => unreachable!(),
                    }
                }
            );
            for _ in 0..200 {
                let v = f.adjust(&c, PAGE_STEPS);
                f.set(&mut c, v);
            }
            assert_eq!(f.get(&c), Value::Int(max), "{f:?}");
            assert_eq!(
                f.adjust(&c, 1),
                Value::Int(max),
                "{f:?} stops changing at the upper bound"
            );
            for _ in 0..200 {
                let v = f.adjust(&c, -1);
                f.set(&mut c, v);
            }
            assert_eq!(
                f.adjust(&c, -1),
                Value::Int(min),
                "{f:?} stops changing at the lower bound"
            );
        }
        let mut c = Config::default();
        Field::Width.set(&mut c, Value::Int(999));
        assert_eq!(c.width, 120, "set clamps too");
    }

    #[test]
    fn colors_cycle_both_ways() {
        let mut c = Config::default();
        let mut seen = vec![c.code_color];
        for _ in 0..8 {
            let v = Field::CodeColor.adjust(&c, 1);
            Field::CodeColor.set(&mut c, v);
            seen.push(c.code_color);
        }
        assert_eq!(seen.first(), seen.last(), "back to the start after 8 steps");
        let mut distinct = seen[..8].to_vec();
        distinct.sort_by_key(|x| CodeColor::ALL.iter().position(|y| y == x));
        distinct.dedup();
        assert_eq!(distinct, CodeColor::ALL.to_vec());
        c.code_color = CodeColor::Black;
        assert_eq!(
            Field::CodeColor.adjust(&c, -1),
            Value::Color(CodeColor::White),
            "going left wraps from the first to the last"
        );
    }

    #[test]
    fn cursor_labels_follow_vim() {
        crate::i18n::init_for_test("en");
        assert_eq!(Field::Cursor.label(false), t("settings-cursor"));
        assert_eq!(Field::Cursor.label(true), t("settings-cursor-vim"));
        assert_eq!(Field::CursorBlink.label(false), t("settings-cursor-blink"));
        assert_eq!(
            Field::CursorBlink.label(true),
            t("settings-cursor-blink-vim")
        );
        assert_eq!(Field::Cursor.help(false), t("settings-cursor-help"));
        assert_eq!(Field::Cursor.help(true), t("settings-cursor-help-vim"));
        assert_eq!(
            Field::CursorBlink.help(false),
            t("settings-cursor-blink-help")
        );
        assert_eq!(
            Field::CursorBlink.help(true),
            t("settings-cursor-blink-help-vim")
        );
        for f in [Field::Cursor, Field::CursorBlink] {
            assert_ne!(f.label(false), f.label(true));
            assert_ne!(f.help(false), f.help(true));
            assert!(
                !f.help(false).contains("vim"),
                "help doesn't mention vim when vim is off"
            );
        }
        // Other items don't depend on vim
        for f in Field::ALL {
            if !matches!(f, Field::Cursor | Field::CursorBlink) {
                assert_eq!(f.label(false), f.label(true));
                assert_eq!(f.help(false), f.help(true));
            }
        }
    }

    #[test]
    fn cursor_cycles_and_blink_toggles() {
        assert_eq!(Field::Cursor.kind(), Kind::Choice);
        assert_eq!(Field::CursorBlink.kind(), Kind::Toggle);
        let mut c = Config::default();
        let mut seen = vec![c.cursor];
        for _ in 0..3 {
            let v = Field::Cursor.adjust(&c, 1);
            Field::Cursor.set(&mut c, v);
            seen.push(c.cursor);
        }
        assert_eq!(
            seen,
            vec![
                CursorShape::Bar,
                CursorShape::Block,
                CursorShape::Underline,
                CursorShape::Bar
            ],
            "bar → block → underline → bar"
        );
        assert_eq!(
            Field::Cursor.adjust(&c, -1),
            Value::Cursor(CursorShape::Underline),
            "going left wraps from the first to the last"
        );
        // In the panel: on cursor, → cycles; on cursor_blink, Space toggles
        let mut s = Settings::open(&c);
        while s.field() != Field::Cursor {
            s.handle_key(&press(KeyCode::Down), &c);
        }
        let Outcome::Changed(block) = s.handle_key(&press(KeyCode::Right), &c) else {
            panic!()
        };
        assert_eq!(block.cursor, CursorShape::Block);
        assert_eq!(
            s.handle_key(&press(KeyCode::Char(' ')), &c),
            Outcome::Ignored,
            "Space only flips toggles"
        );
        s.handle_key(&press(KeyCode::Down), &c);
        assert_eq!(s.field(), Field::CursorBlink);
        let Outcome::Changed(blink) = s.handle_key(&press(KeyCode::Char(' ')), &c) else {
            panic!()
        };
        assert!(blink.cursor_blink);
        let Outcome::Changed(steady) = s.handle_key(&press(KeyCode::Right), &blink) else {
            panic!()
        };
        assert!(!steady.cursor_blink);
    }

    #[test]
    fn lang_options_follow_locales() {
        // Choices are built from LOCALES: Auto first, the rest in table order
        let want: Vec<&str> = std::iter::once("auto")
            .chain(i18n::LOCALES.iter().map(|(t, _)| *t))
            .collect();
        assert_eq!(lang_options(), want);
        assert_eq!(lang_options(), vec!["auto", "en", "zh-CN"]);
        let mut c = Config::default();
        let mut seen = vec![c.lang];
        for _ in 0..want.len() {
            let v = Field::Lang.adjust(&c, 1);
            Field::Lang.set(&mut c, v);
            seen.push(c.lang);
        }
        assert_eq!(
            seen,
            vec!["auto", "en", "zh-CN", "auto"],
            "Auto → English → Simplified Chinese → Auto"
        );
        assert_eq!(Field::Lang.adjust(&c, -1), Value::Lang("zh-CN"));
        Field::Lang.set(&mut c, Value::Lang("ja"));
        assert_eq!(c.lang, "auto", "tags without a catalog are rejected");
        assert_eq!(Field::Lang.kind(), Kind::Choice);
    }

    #[test]
    fn navigation_does_not_wrap() {
        let c = Config::default();
        let mut s = Settings::open(&c);
        assert_eq!(s.handle_key(&press(KeyCode::Up), &c), Outcome::Ignored);
        assert_eq!(s.handle_key(&press(KeyCode::BackTab), &c), Outcome::Ignored);
        assert_eq!(s.selected, 0);
        for _ in 0..20 {
            s.handle_key(&press(KeyCode::Tab), &c);
        }
        assert_eq!(s.selected, Field::ALL.len() - 1);
        assert_eq!(s.handle_key(&press(KeyCode::Down), &c), Outcome::Ignored);
        assert_eq!(s.handle_key(&press(KeyCode::Up), &c), Outcome::Moved);
        assert_eq!(s.selected, Field::ALL.len() - 2);
        // j / k are not bound
        assert_eq!(
            s.handle_key(&press(KeyCode::Char('j')), &c),
            Outcome::Ignored
        );
        assert_eq!(s.selected, Field::ALL.len() - 2);
    }

    #[test]
    fn keys_produce_outcomes() {
        let c = Config::default();
        let mut s = Settings::open(&c);
        let ctrl = |ch| KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL);
        assert_eq!(s.handle_key(&press(KeyCode::Enter), &c), Outcome::Save);
        assert_eq!(s.handle_key(&press(KeyCode::Esc), &c), Outcome::Cancel);
        assert_eq!(s.handle_key(&ctrl('p'), &c), Outcome::Cancel);
        assert_eq!(s.handle_key(&ctrl('q'), &c), Outcome::Quit);
        assert_eq!(s.handle_key(&ctrl('s'), &c), Outcome::Ignored, "swallowed");
        // width: ← subtracts 2, PageDown adds 20
        let Outcome::Changed(narrow) = s.handle_key(&press(KeyCode::Left), &c) else {
            panic!()
        };
        assert_eq!(narrow.width, 70);
        let Outcome::Changed(wide) = s.handle_key(&press(KeyCode::PageDown), &c) else {
            panic!()
        };
        assert_eq!(wide.width, 92);
        assert_eq!(
            s.handle_key(&press(KeyCode::Char(' ')), &c),
            Outcome::Ignored,
            "Space only flips toggles"
        );
        // vim: Space / → flips it
        s.handle_key(&press(KeyCode::Down), &c);
        let Outcome::Changed(vim) = s.handle_key(&press(KeyCode::Char(' ')), &c) else {
            panic!()
        };
        assert!(vim.vim);
        // No more Changed once at the bound
        let max = Config {
            width: 120,
            ..Config::default()
        };
        let mut s = Settings::open(&max);
        assert_eq!(s.handle_key(&press(KeyCode::Right), &max), Outcome::Ignored);
    }

    #[test]
    fn follow_keeps_selection_visible() {
        let mut s = Settings::open(&Config::default());
        assert_eq!(s.follow(4), 0);
        s.selected = 6;
        assert_eq!(s.follow(4), 3);
        s.selected = 9;
        assert_eq!(s.follow(4), 6);
        s.selected = 4;
        assert_eq!(s.follow(4), 4);
        assert_eq!(s.follow(20), 0, "shows from the top when everything fits");
    }
}
