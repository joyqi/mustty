//! 设置面板：表驱动的 11 个配置项与面板状态机。渲染见 `view::settings`。
//!
//! 面板只读写 `Config` 的副本；每次调整由 App 立刻 `apply_config`，关闭时决定保留还是回滚。

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::config::{
    AUTOSAVE_IDLE_RANGE, CodeColor, Config, SCROLLOFF_RANGE, TAB_WIDTH_RANGE, WIDTH_RANGE,
};
use crate::i18n::{self, t};

/// PageUp / PageDown 一次调整的步数。
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
    Lang,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Toggle,
    /// `unit` 是已翻译的单位
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
    /// `i18n::AUTO` 或语言标签
    Lang(&'static str),
}

/// 语言项的选项：Auto，然后是 `i18n::LOCALES` 里的每一门语言。新增译文后自动多一项。
pub fn lang_options() -> Vec<&'static str> {
    std::iter::once(i18n::AUTO).chain(i18n::tags()).collect()
}

fn range<T: Into<i64>>((lo, hi): (T, T)) -> (i64, i64) {
    (lo.into(), hi.into())
}

impl Field {
    /// 显示顺序。
    pub const ALL: [Field; 11] = [
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
        Self::Lang,
    ];

    /// 配置文件里的键名（与 `config::KEYS` 一致）。
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
            Self::Lang => "lang",
        }
    }

    pub fn label(self) -> &'static str {
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
            Self::Lang => t("settings-lang"),
        }
    }

    /// 选中时说明行显示的一句话。
    pub fn help(self) -> &'static str {
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
            Self::Lang => t("settings-lang-help"),
        }
    }

    /// 范围复用 config.rs 的 `*_RANGE`，与配置文件的钳制一致。
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
            Self::CodeColor | Self::Lang => Kind::Choice,
            Self::Vim | Self::Mouse | Self::Autosave | Self::Typewriter | Self::FocusMode => {
                Kind::Toggle
            }
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
            Self::Lang => Value::Lang(c.lang),
        }
    }

    /// 写入一项；数字钳制到范围内，类型不符的值忽略。
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
            (Self::Lang, Value::Lang(lang)) => {
                // 没有译文的标签不接受（类型对，只是不在选项里）
                if lang_options().contains(&lang) {
                    c.lang = lang;
                }
            }
            _ => debug_assert!(false, "{self:?} 不接受 {v:?}"),
        }
    }

    /// 调整 `steps` 步后的值：开关翻转；数字按步进走、到边界停住；颜色循环。
    pub fn adjust(self, c: &Config, steps: i64) -> Value {
        match (self.get(c), self.kind()) {
            (Value::Bool(b), _) => Value::Bool(!b),
            (Value::Int(i), Kind::Int { min, max, step, .. }) => {
                Value::Int((i + steps * step).clamp(min, max))
            }
            (Value::Color(color), _) => Value::Color(cycle(&CodeColor::ALL, color, steps)),
            (Value::Lang(lang), _) => Value::Lang(cycle(&lang_options(), lang, steps)),
            (v, _) => v,
        }
    }
}

/// 单选项循环一步（方向取 `steps` 的符号）。
fn cycle<T: Copy + PartialEq>(options: &[T], current: T, steps: i64) -> T {
    let at = options.iter().position(|&x| x == current).unwrap_or(0) as i64;
    let n = options.len() as i64;
    options[(at + steps.signum()).rem_euclid(n) as usize]
}

const AUTOSAVE_IDLE_RANGE_I64: (i64, i64) =
    (AUTOSAVE_IDLE_RANGE.0 as i64, AUTOSAVE_IDLE_RANGE.1 as i64);

/// 颜色名（已翻译）。
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

/// 面板对一个按键的处理结果，由 App 执行。
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// 吞掉，什么都不做（未绑定的键、按键释放）
    Ignored,
    /// 选中项移动了
    Moved,
    /// 某一项变了：立即应用这份配置
    Changed(Config),
    /// Enter：保留当前值、写回文件、关闭
    Save,
    /// Esc / Ctrl+P：恢复快照、关闭
    Cancel,
    /// Ctrl+Q：先按 Cancel 回滚，再走正常的退出流程
    Quit,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// `Field::ALL` 里的下标
    pub selected: usize,
    /// 列表滚动到的第一行（正文区放不下 10 行时）
    pub scroll: usize,
    /// 打开面板时的配置：Esc 回滚到它，Enter 只写与它不同的键
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

    /// 首尾不循环。
    fn select(&mut self, delta: isize) -> Outcome {
        let last = Field::ALL.len() - 1;
        let next = self.selected.saturating_add_signed(delta).min(last);
        if next == self.selected {
            return Outcome::Ignored;
        }
        self.selected = next;
        Outcome::Moved
    }

    /// 列表只有 `visible` 行可用时，让选中项保持可见；返回第一行的下标。
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
            "顺序与键名都与 config::KEYS 一致"
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
            Value::Lang("zh-CN"),
        ];
        assert_eq!(values.len(), Field::ALL.len());
        for (f, v) in Field::ALL.into_iter().zip(values) {
            assert_ne!(f.get(&c), v, "{f:?}：测试值要与默认值不同");
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
            assert_eq!(f.adjust(&c, 1), Value::Int(max), "{f:?} 到上界不再变");
            for _ in 0..200 {
                let v = f.adjust(&c, -1);
                f.set(&mut c, v);
            }
            assert_eq!(f.adjust(&c, -1), Value::Int(min), "{f:?} 到下界不再变");
        }
        let mut c = Config::default();
        Field::Width.set(&mut c, Value::Int(999));
        assert_eq!(c.width, 120, "set 也钳制");
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
        assert_eq!(seen.first(), seen.last(), "8 次回到起点");
        let mut distinct = seen[..8].to_vec();
        distinct.sort_by_key(|x| CodeColor::ALL.iter().position(|y| y == x));
        distinct.dedup();
        assert_eq!(distinct, CodeColor::ALL.to_vec());
        c.code_color = CodeColor::Black;
        assert_eq!(
            Field::CodeColor.adjust(&c, -1),
            Value::Color(CodeColor::White),
            "向左从第一个绕到最后一个"
        );
    }

    #[test]
    fn lang_options_follow_locales() {
        // 选项从 LOCALES 动态生成：Auto 在前，其余按表的顺序
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
            "Auto → English → 简体中文 → Auto"
        );
        assert_eq!(Field::Lang.adjust(&c, -1), Value::Lang("zh-CN"));
        Field::Lang.set(&mut c, Value::Lang("ja"));
        assert_eq!(c.lang, "auto", "没有译文的标签不接受");
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
        // j / k 不绑定
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
        assert_eq!(s.handle_key(&ctrl('s'), &c), Outcome::Ignored, "吞掉");
        // width：← 减 2，PageDown 加 20
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
            "Space 只切换开关"
        );
        // vim：Space / → 翻转
        s.handle_key(&press(KeyCode::Down), &c);
        let Outcome::Changed(vim) = s.handle_key(&press(KeyCode::Char(' ')), &c) else {
            panic!()
        };
        assert!(vim.vim);
        // 到边界后不再产生 Changed
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
        assert_eq!(s.follow(20), 0, "放得下时从头显示");
    }
}
