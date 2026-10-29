//! `~/.mustty.toml`：优先级 配置文件 > 内置默认值，按键逐个合并。
//! 配置写错永远不阻止启动：语法错误整份作废，单个键出错只回退这个键，越界钳制；都只产生警告。

use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::cursor::SetCursorStyle;
use ratatui::style::Color;
use serde::Deserialize;

use crate::i18n::{self, t, t_args};

pub const WIDTH_RANGE: (u16, u16) = (40, 120);
pub const TAB_WIDTH_RANGE: (u16, u16) = (1, 16);
pub const AUTOSAVE_IDLE_RANGE: (u64, u64) = (500, 60_000);
pub const SCROLLOFF_RANGE: (u16, u16) = (0, 10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub width: u16,
    pub vim: bool,
    pub mouse: bool,
    pub autosave: bool,
    pub autosave_idle: Duration,
    pub typewriter: bool,
    pub focus_mode: bool,
    pub scrolloff: u16,
    pub tab_width: u16,
    /// 存选项本身而不是 `Color`：`white` 与 `gray` 都映射到 `Color::Gray`，反推不回来
    pub code_color: CodeColor,
    /// 编辑时的光标形状（vim Normal 态不受它影响，始终是方块）
    pub cursor: CursorShape,
    pub cursor_blink: bool,
    /// 界面语言：`i18n::AUTO` 或 `i18n::LOCALES` 里的标签
    pub lang: &'static str,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            width: 72,
            vim: false,
            mouse: false,
            autosave: true,
            autosave_idle: Duration::from_millis(2000),
            typewriter: false,
            focus_mode: false,
            scrolloff: 3,
            tab_width: 4,
            code_color: CodeColor::Cyan,
            cursor: CursorShape::Bar,
            cursor_blink: false,
            lang: i18n::AUTO,
        }
    }
}

/// 光标形状：`cursor = "bar" | "block" | "underline"`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CursorShape {
    Bar,
    Block,
    Underline,
}

impl CursorShape {
    /// 设置面板里的循环顺序。
    pub const ALL: [CursorShape; 3] = [Self::Bar, Self::Block, Self::Underline];

    /// 配置文件里的写法。
    pub fn name(self) -> &'static str {
        match self {
            Self::Bar => "bar",
            Self::Block => "block",
            Self::Underline => "underline",
        }
    }

    /// 对应的终端光标形状（DECSCUSR）。
    pub fn style(self, blink: bool) -> SetCursorStyle {
        match (self, blink) {
            (Self::Bar, false) => SetCursorStyle::SteadyBar,
            (Self::Bar, true) => SetCursorStyle::BlinkingBar,
            (Self::Block, false) => SetCursorStyle::SteadyBlock,
            (Self::Block, true) => SetCursorStyle::BlinkingBlock,
            (Self::Underline, false) => SetCursorStyle::SteadyUnderScore,
            (Self::Underline, true) => SetCursorStyle::BlinkingUnderScore,
        }
    }
}

/// 8 个标准 ANSI 前景色（SGR 30–37）。ratatui 把 SGR 37 叫 `Color::Gray`，
/// 所以 `white`（以及需求里写的 `gray`）都映射到 `Color::Gray`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CodeColor {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    #[serde(alias = "gray")]
    White,
}

impl CodeColor {
    /// 设置面板里的循环顺序 = SGR 30–37 的顺序。
    pub const ALL: [CodeColor; 8] = [
        Self::Black,
        Self::Red,
        Self::Green,
        Self::Yellow,
        Self::Blue,
        Self::Magenta,
        Self::Cyan,
        Self::White,
    ];

    /// 配置文件里的写法。
    pub fn name(self) -> &'static str {
        match self {
            Self::Black => "black",
            Self::Red => "red",
            Self::Green => "green",
            Self::Yellow => "yellow",
            Self::Blue => "blue",
            Self::Magenta => "magenta",
            Self::Cyan => "cyan",
            Self::White => "white",
        }
    }

    pub fn color(self) -> Color {
        match self {
            Self::Black => Color::Black,
            Self::Red => Color::Red,
            Self::Green => Color::Green,
            Self::Yellow => Color::Yellow,
            Self::Blue => Color::Blue,
            Self::Magenta => Color::Magenta,
            Self::Cyan => Color::Cyan,
            Self::White => Color::Gray,
        }
    }
}

/// 配置文件这一层：未给出（或写错）的键为 None，合并时保持默认值。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PartialConfig {
    pub width: Option<u16>,
    pub vim: Option<bool>,
    pub mouse: Option<bool>,
    pub autosave: Option<bool>,
    pub autosave_idle_ms: Option<u64>,
    pub typewriter: Option<bool>,
    pub focus_mode: Option<bool>,
    pub scrolloff: Option<u16>,
    pub tab_width: Option<u16>,
    pub code_color: Option<CodeColor>,
    pub cursor: Option<CursorShape>,
    pub cursor_blink: Option<bool>,
    pub lang: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigWarning {
    /// 出问题的键；整份文件的问题（读不了、语法错误）为 None
    pub key: Option<&'static str>,
    pub message: String,
}

impl ConfigWarning {
    fn file(message: String) -> Self {
        Self { key: None, message }
    }
}

pub const KEYS: [&str; 13] = [
    "width",
    "vim",
    "mouse",
    "autosave",
    "autosave_idle_ms",
    "typewriter",
    "focus_mode",
    "scrolloff",
    "tab_width",
    "code_color",
    "cursor",
    "cursor_blink",
    "lang",
];

impl PartialConfig {
    /// 永不失败：语法错误 → 空配置 + 带行号的警告；未知键丢弃。
    pub fn from_toml_str(s: &str) -> (Self, Vec<ConfigWarning>) {
        let table: toml::Table = match s.parse() {
            Ok(t) => t,
            Err(e) => {
                let line = e
                    .span()
                    .map_or(1, |r| s[..r.start.min(s.len())].matches('\n').count() + 1);
                let msg = t_args("config-syntax-error", &[("line", &line.to_string())]);
                return (Self::default(), vec![ConfigWarning::file(msg)]);
            }
        };
        let mut warnings = Vec::new();
        let mut p = Self::default();
        for (key, value) in &table {
            // 未知键（含其他表）直接忽略，便于以后加键或与其他工具共用文件
            let Some(&key) = KEYS.iter().find(|&&k| k == key) else {
                continue;
            };
            let mut warn = |message: String| {
                warnings.push(ConfigWarning {
                    key: Some(key),
                    message,
                })
            };
            let int = |range: (i64, i64), warn: &mut dyn FnMut(String)| -> Option<i64> {
                let Some(v) = value.as_integer() else {
                    warn(t_args("config-expected-integer", &[("key", key)]));
                    return None;
                };
                let c = v.clamp(range.0, range.1);
                if c != v {
                    warn(format!("{key}={v} → {c}"));
                }
                Some(c)
            };
            let boolean = |warn: &mut dyn FnMut(String)| -> Option<bool> {
                let b = value.as_bool();
                if b.is_none() {
                    warn(t_args("config-expected-bool", &[("key", key)]));
                }
                b
            };
            let range = |(lo, hi): (u16, u16)| (lo as i64, hi as i64);
            match key {
                "width" => p.width = int(range(WIDTH_RANGE), &mut warn).map(|v| v as u16),
                "autosave_idle_ms" => {
                    let r = (AUTOSAVE_IDLE_RANGE.0 as i64, AUTOSAVE_IDLE_RANGE.1 as i64);
                    p.autosave_idle_ms = int(r, &mut warn).map(|v| v as u64);
                }
                "scrolloff" => {
                    p.scrolloff = int(range(SCROLLOFF_RANGE), &mut warn).map(|v| v as u16)
                }
                "tab_width" => {
                    p.tab_width = int(range(TAB_WIDTH_RANGE), &mut warn).map(|v| v as u16)
                }
                "vim" => p.vim = boolean(&mut warn),
                "mouse" => p.mouse = boolean(&mut warn),
                "autosave" => p.autosave = boolean(&mut warn),
                "typewriter" => p.typewriter = boolean(&mut warn),
                "focus_mode" => p.focus_mode = boolean(&mut warn),
                "cursor_blink" => p.cursor_blink = boolean(&mut warn),
                "cursor" => {
                    // 大小写不敏感
                    p.cursor = value.as_str().and_then(|s| {
                        CursorShape::ALL
                            .into_iter()
                            .find(|c| c.name().eq_ignore_ascii_case(s))
                    });
                    if p.cursor.is_none() {
                        let value = value.to_string();
                        warn(t_args("config-invalid-cursor", &[("value", &value)]));
                    }
                }
                "code_color" => {
                    // 大小写不敏感
                    let lowered = value
                        .as_str()
                        .map(|s| toml::Value::String(s.to_lowercase()));
                    p.code_color = lowered.and_then(|v| v.try_into().ok());
                    if p.code_color.is_none() {
                        let value = value.to_string();
                        warn(t_args("config-invalid-color", &[("value", &value)]));
                    }
                }
                "lang" => {
                    // auto 或有译文的语言标签，大小写不敏感
                    p.lang = value.as_str().and_then(|s| {
                        if s.eq_ignore_ascii_case(i18n::AUTO) {
                            Some(i18n::AUTO)
                        } else {
                            i18n::known_tag(s)
                        }
                    });
                    if p.lang.is_none() {
                        let choices = std::iter::once(i18n::AUTO)
                            .chain(i18n::tags())
                            .collect::<Vec<_>>()
                            .join(" ");
                        let value = value.to_string();
                        warn(t_args(
                            "config-invalid-lang",
                            &[("value", &value), ("choices", &choices)],
                        ));
                    }
                }
                _ => unreachable!("KEYS 与 match 分支一一对应"),
            }
        }
        (p, warnings)
    }
}

impl Config {
    /// 用配置文件这一层覆盖当前值（数值在 `from_toml_str` 里已经钳制过）。
    pub fn merge(mut self, p: &PartialConfig) -> Self {
        self.width = p.width.unwrap_or(self.width);
        self.vim = p.vim.unwrap_or(self.vim);
        self.mouse = p.mouse.unwrap_or(self.mouse);
        self.autosave = p.autosave.unwrap_or(self.autosave);
        if let Some(ms) = p.autosave_idle_ms {
            self.autosave_idle = Duration::from_millis(ms);
        }
        self.typewriter = p.typewriter.unwrap_or(self.typewriter);
        self.focus_mode = p.focus_mode.unwrap_or(self.focus_mode);
        self.scrolloff = p.scrolloff.unwrap_or(self.scrolloff);
        self.tab_width = p.tab_width.unwrap_or(self.tab_width);
        self.code_color = p.code_color.unwrap_or(self.code_color);
        self.cursor = p.cursor.unwrap_or(self.cursor);
        self.cursor_blink = p.cursor_blink.unwrap_or(self.cursor_blink);
        self.lang = p.lang.unwrap_or(self.lang);
        self
    }
}

pub fn path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".mustty.toml"))
}

/// 读 `~/.mustty.toml` 并按 默认 ← 文件 合并。
pub fn load() -> (Config, Vec<ConfigWarning>) {
    let (file, warnings) = match path() {
        Some(p) => read_file(&p),
        None => (PartialConfig::default(), vec![]),
    };
    (Config::default().merge(&file), warnings)
}

fn read_file(path: &Path) -> (PartialConfig, Vec<ConfigWarning>) {
    match std::fs::read_to_string(path) {
        Ok(text) => PartialConfig::from_toml_str(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (PartialConfig::default(), vec![]),
        Err(e) => {
            let msg = t_args("config-unreadable", &[("error", &e.to_string())]);
            (PartialConfig::default(), vec![ConfigWarning::file(msg)])
        }
    }
}

/// 一个键在配置文件里的取值。
#[derive(Debug, Clone, PartialEq, Eq)]
enum Scalar {
    Int(i64),
    Bool(bool),
    Str(&'static str),
}

impl Config {
    fn scalar(&self, key: &str) -> Scalar {
        match key {
            "width" => Scalar::Int(self.width.into()),
            "vim" => Scalar::Bool(self.vim),
            "mouse" => Scalar::Bool(self.mouse),
            "autosave" => Scalar::Bool(self.autosave),
            "autosave_idle_ms" => Scalar::Int(self.autosave_idle.as_millis() as i64),
            "typewriter" => Scalar::Bool(self.typewriter),
            "focus_mode" => Scalar::Bool(self.focus_mode),
            "scrolloff" => Scalar::Int(self.scrolloff.into()),
            "tab_width" => Scalar::Int(self.tab_width.into()),
            "code_color" => Scalar::Str(self.code_color.name()),
            "cursor" => Scalar::Str(self.cursor.name()),
            "cursor_blink" => Scalar::Bool(self.cursor_blink),
            "lang" => Scalar::Str(self.lang),
            _ => unreachable!("不在 KEYS 里的键 {key}"),
        }
    }
}

/// 两份配置之间取值不同的键，按 `KEYS` 的顺序。
pub fn changed_keys(old: &Config, new: &Config) -> Vec<&'static str> {
    KEYS.into_iter()
        .filter(|k| old.scalar(k) != new.scalar(k))
        .collect()
}

#[derive(Debug)]
pub enum WriteBackError {
    /// 现有文件有语法错误：拒绝写入，不尝试修复用户的文件
    Syntax,
    Io(std::io::Error),
}

/// 把 `old` → `new` 之间变化的键写回配置文件。
///
/// 写入时重新读取文件，在最新内容上只改这些键：已有键原地改值（保留注释与键顺序），
/// 新键追加到顶层末尾，未知键和其他表不动。符号链接解析到真实目标后再原子替换，链接本身保留。
pub fn write_back(path: &Path, old: &Config, new: &Config) -> Result<(), WriteBackError> {
    let keys = changed_keys(old, new);
    if keys.is_empty() {
        return Ok(());
    }
    let not_found = |e: &std::io::Error| e.kind() == std::io::ErrorKind::NotFound;
    let target = match std::fs::canonicalize(path) {
        Ok(p) => p,
        Err(e) if not_found(&e) => path.to_path_buf(),
        Err(e) => return Err(WriteBackError::Io(e)),
    };
    let text = match std::fs::read_to_string(&target) {
        Ok(text) => text,
        Err(e) if not_found(&e) => String::new(),
        Err(e) => return Err(WriteBackError::Io(e)),
    };
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|_| WriteBackError::Syntax)?;
    for key in keys {
        let value: toml_edit::Value = match new.scalar(key) {
            Scalar::Int(i) => i.into(),
            Scalar::Bool(b) => b.into(),
            Scalar::Str(s) => s.into(),
        };
        match doc.get_mut(key).and_then(toml_edit::Item::as_value_mut) {
            Some(slot) => {
                // 换值不换装饰：等号两侧的空白与行尾注释原样保留
                let decor = slot.decor().clone();
                *slot = value;
                *slot.decor_mut() = decor;
            }
            None => {
                doc.insert(key, toml_edit::Item::Value(value));
            }
        }
    }
    crate::io::file::atomic_write(&target, doc.to_string().as_bytes()).map_err(WriteBackError::Io)
}

/// 把所有警告汇总成一行，用于 status bar 与 stderr。
pub fn summarize(warnings: &[ConfigWarning]) -> Option<String> {
    if warnings.is_empty() {
        return None;
    }
    let parts: Vec<&str> = warnings.iter().map(|w| w.message.as_str()).collect();
    let list = parts.join(t("config-separator"));
    if warnings.iter().all(|w| w.key.is_some()) {
        Some(t_args("config-corrected", &[("list", &list)]))
    } else {
        Some(list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn parse(text: &str) -> (Config, Vec<ConfigWarning>) {
        let (p, w) = PartialConfig::from_toml_str(text);
        (Config::default().merge(&p), w)
    }

    fn keys(w: &[ConfigWarning]) -> Vec<Option<&'static str>> {
        w.iter().map(|w| w.key).collect()
    }

    #[test]
    fn empty_or_missing_file_uses_defaults() {
        assert_eq!(parse(""), (Config::default(), vec![]));
        let dir = tempfile::tempdir().unwrap();
        let (p, w) = read_file(&dir.path().join(".mustty.toml"));
        assert_eq!((p, w), (PartialConfig::default(), vec![]));
    }

    #[test]
    fn all_keys_parse() {
        let text = r#"
            width = 64
            vim = true
            mouse = true
            autosave = false
            autosave_idle_ms = 5000
            typewriter = true
            focus_mode = true
            scrolloff = 5
            tab_width = 2
            code_color = "yellow"
            cursor = "underline"
            cursor_blink = true
            lang = "zh-CN"
        "#;
        let want = Config {
            width: 64,
            vim: true,
            mouse: true,
            autosave: false,
            autosave_idle: Duration::from_millis(5000),
            typewriter: true,
            focus_mode: true,
            scrolloff: 5,
            tab_width: 2,
            code_color: CodeColor::Yellow,
            cursor: CursorShape::Underline,
            cursor_blink: true,
            lang: "zh-CN",
        };
        assert_eq!(parse(text), (want, vec![]));
    }

    #[test]
    fn lang_accepts_auto_and_known_tags_only() {
        crate::i18n::init_for_test("zh-CN");
        assert_eq!(Config::default().lang, "auto");
        for (text, want) in [
            ("lang = \"auto\"", "auto"),
            ("lang = \"AUTO\"", "auto"),
            ("lang = \"en\"", "en"),
            ("lang = \"zh-CN\"", "zh-CN"),
            ("lang = \"zh-cn\"", "zh-CN"),
        ] {
            assert_eq!(
                parse(text),
                (
                    Config {
                        lang: want,
                        ..Config::default()
                    },
                    vec![]
                ),
                "{text}"
            );
        }
        // 拼错或没有译文：回退 auto，警告列出该键与可选值
        for bad in ["\"xx\"", "\"ja\"", "\"zh\"", "1"] {
            let (c, w) = parse(&format!("lang = {bad}"));
            assert_eq!((c.lang, keys(&w)), ("auto", vec![Some("lang")]), "{bad}");
            assert!(w[0].message.contains("auto en zh-CN"), "{}", w[0].message);
        }
        let (_, w) = parse("lang = \"xx\"\nwidth = 200");
        assert!(summarize(&w).unwrap().contains("lang="), "汇总进配置警告");
    }

    #[test]
    fn cursor_accepts_three_shapes_and_falls_back_to_bar() {
        crate::i18n::init_for_test("en");
        assert_eq!(
            (Config::default().cursor, Config::default().cursor_blink),
            (CursorShape::Bar, false),
            "默认与原来一样：常亮竖线"
        );
        for (text, want) in [
            ("cursor = \"bar\"", CursorShape::Bar),
            ("cursor = \"block\"", CursorShape::Block),
            ("cursor = \"underline\"", CursorShape::Underline),
            ("cursor = \"Block\"", CursorShape::Block),
        ] {
            let (c, w) = parse(text);
            assert_eq!((c.cursor, w), (want, vec![]), "{text}");
        }
        // 非法值：回退 bar，警告列出该键与可选值，并汇总进配置警告
        for bad in ["\"beam\"", "\"\"", "1", "true"] {
            let (c, w) = parse(&format!("cursor = {bad}"));
            assert_eq!(
                (c.cursor, keys(&w)),
                (CursorShape::Bar, vec![Some("cursor")]),
                "{bad}"
            );
            assert!(
                w[0].message.contains("bar block underline"),
                "{}",
                w[0].message
            );
        }
        let (c, w) = parse("cursor_blink = \"yes\"");
        assert_eq!(
            (c.cursor_blink, keys(&w)),
            (false, vec![Some("cursor_blink")])
        );
        let (_, w) = parse("cursor = \"beam\"\nwidth = 200");
        assert!(summarize(&w).unwrap().contains("cursor="));
    }

    #[test]
    fn cursor_shape_maps_to_decscusr() {
        use SetCursorStyle::*;
        let cases = [
            (CursorShape::Bar, false, SteadyBar),
            (CursorShape::Bar, true, BlinkingBar),
            (CursorShape::Block, false, SteadyBlock),
            (CursorShape::Block, true, BlinkingBlock),
            (CursorShape::Underline, false, SteadyUnderScore),
            (CursorShape::Underline, true, BlinkingUnderScore),
        ];
        let ansi = |s: SetCursorStyle| {
            let mut out = String::new();
            crossterm::Command::write_ansi(&s, &mut out).unwrap();
            out
        };
        for (shape, blink, want) in cases {
            assert_eq!(
                ansi(shape.style(blink)),
                ansi(want),
                "{shape:?} blink={blink}"
            );
        }
        // DECSCUSR 的编号：闪烁为奇数、常亮为偶数
        assert_eq!(ansi(CursorShape::Bar.style(false)), "\x1b[6 q");
        assert_eq!(ansi(CursorShape::Underline.style(true)), "\x1b[3 q");
    }

    #[test]
    fn write_back_cursor() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".mustty.toml");
        std::fs::write(&path, "# mine\ncursor = \"bar\" # comment\n").unwrap();
        let old = loaded("cursor = \"bar\"\n");
        let new = Config {
            cursor: CursorShape::Block,
            cursor_blink: true,
            ..old.clone()
        };
        assert_eq!(changed_keys(&old, &new), vec!["cursor", "cursor_blink"]);
        write_back(&path, &old, &new).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            written,
            "# mine\ncursor = \"block\" # comment\ncursor_blink = true\n"
        );
        assert_eq!(loaded(&written), new);
    }

    #[test]
    fn write_back_lang() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".mustty.toml");
        std::fs::write(&path, "width = 80\n").unwrap();
        let old = loaded("width = 80\n");
        let new = Config {
            lang: "zh-CN",
            ..old.clone()
        };
        assert_eq!(changed_keys(&old, &new), vec!["lang"]);
        write_back(&path, &old, &new).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        assert_eq!(written, "width = 80\nlang = \"zh-CN\"\n");
        assert_eq!(loaded(&written), new);
    }

    #[test]
    fn numeric_bounds_are_clamped() {
        // (配置, 键, 生效值, 是否警告)
        let cases = [
            ("width = 39", "width", 40, true),
            ("width = 40", "width", 40, false),
            ("width = 120", "width", 120, false),
            ("width = 121", "width", 120, true),
            ("width = 99999999999", "width", 120, true),
            ("tab_width = 0", "tab_width", 1, true),
            ("tab_width = 1", "tab_width", 1, false),
            ("tab_width = 16", "tab_width", 16, false),
            ("tab_width = 17", "tab_width", 16, true),
            ("autosave_idle_ms = 499", "autosave_idle_ms", 500, true),
            ("autosave_idle_ms = 60001", "autosave_idle_ms", 60000, true),
            ("scrolloff = 10", "scrolloff", 10, false),
            ("scrolloff = 11", "scrolloff", 10, true),
            ("scrolloff = -1", "scrolloff", 0, true),
        ];
        for (text, key, want, warned) in cases {
            let (c, w) = parse(text);
            let got = match key {
                "width" => c.width as u64,
                "tab_width" => c.tab_width as u64,
                "scrolloff" => c.scrolloff as u64,
                _ => c.autosave_idle.as_millis() as u64,
            };
            assert_eq!(got, want, "{text}");
            assert_eq!(
                keys(&w),
                if warned { vec![Some(key)] } else { vec![] },
                "{text}"
            );
        }
    }

    #[test]
    fn code_color_names() {
        use CodeColor::*;
        let names = [
            ("black", Black, Color::Black),
            ("red", Red, Color::Red),
            ("green", Green, Color::Green),
            ("yellow", Yellow, Color::Yellow),
            ("blue", Blue, Color::Blue),
            ("magenta", Magenta, Color::Magenta),
            ("cyan", Cyan, Color::Cyan),
            ("white", White, Color::Gray), // SGR 37
            ("gray", White, Color::Gray),  // 需求中的写法，同为 SGR 37
            ("YELLOW", Yellow, Color::Yellow),
            ("Magenta", Magenta, Color::Magenta),
        ];
        for (name, choice, color) in names {
            assert_eq!(
                parse(&format!("code_color = {name:?}")),
                (
                    Config {
                        code_color: choice,
                        ..Config::default()
                    },
                    vec![]
                ),
                "{name}"
            );
            assert_eq!(choice.color(), color, "{name}");
        }
        for bad in [
            "\"orange\"",
            "\"brightred\"",
            "\"grey\"",
            "\"#ff0000\"",
            "3",
        ] {
            let (c, w) = parse(&format!("code_color = {bad}"));
            assert_eq!(
                (c.code_color, keys(&w)),
                (CodeColor::Cyan, vec![Some("code_color")]),
                "{bad}"
            );
        }
    }

    #[test]
    fn single_bad_key_only_falls_back_itself() {
        let (c, w) = parse("width = \"wide\"\nvim = true\nmouse = 1\ntab_width = 8");
        assert_eq!((c.width, c.vim, c.mouse, c.tab_width), (72, true, false, 8));
        assert_eq!(keys(&w), vec![Some("mouse"), Some("width")]);
    }

    #[test]
    fn syntax_error_reports_line() {
        crate::i18n::init_for_test("zh-CN");
        let (p, w) = PartialConfig::from_toml_str("width = 64\nvim = true\nmouse = = 1\n");
        assert_eq!(p, PartialConfig::default());
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].key, None);
        assert!(w[0].message.contains("第 3 行语法错误"), "{}", w[0].message);
        crate::i18n::init_for_test("en");
        let (_, w) = PartialConfig::from_toml_str("width = 64\nvim = true\nmouse = = 1\n");
        assert!(
            w[0].message.contains("line 3: syntax error"),
            "{}",
            w[0].message
        );
    }

    #[test]
    fn unknown_keys_are_silent() {
        let (c, w) = parse("colour = \"red\"\nwidth = 64\n[keys]\nsave = \"C-s\"");
        assert_eq!((c.width, w), (64, vec![]));
    }

    #[test]
    fn file_layer_overrides_defaults_key_by_key() {
        let (file, _) =
            PartialConfig::from_toml_str("width = 64\nvim = true\nmouse = true\nautosave = false");
        let c = Config::default().merge(&file);
        assert_eq!(
            (c.width, c.vim, c.mouse, c.autosave),
            (64, true, true, false)
        );
        // 文件里没写的键保持默认值；空的一层什么都不改
        assert_eq!((c.tab_width, c.code_color), (4, CodeColor::Cyan));
        assert_eq!(c.clone().merge(&PartialConfig::default()), c);
    }

    #[test]
    fn summary_lists_keys() {
        crate::i18n::init_for_test("zh-CN");
        let (_, w) =
            PartialConfig::from_toml_str("width = 200\ntab_width = 0\ncode_color = \"orange\"");
        let s = summarize(&w).unwrap();
        assert!(s.starts_with("配置已修正："), "{s}");
        for key in ["width", "tab_width", "code_color"] {
            assert!(s.contains(key), "{s}");
        }
        assert_eq!(summarize(&[]), None);
    }

    /// 按文件内容算出打开面板时的配置。
    fn loaded(text: &str) -> Config {
        Config::default().merge(&PartialConfig::from_toml_str(text).0)
    }

    #[test]
    fn write_back_keeps_comments_order_and_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".mustty.toml");
        let text = "# my config\nwidth = 72   # my width\nvim = true # keep\ncolour = \"red\"\n\n[keys]\nsave = \"C-s\"\n";
        std::fs::write(&path, text).unwrap();
        let old = loaded(text);
        let new = Config {
            width: 64,
            tab_width: 8,
            code_color: CodeColor::White,
            ..old.clone()
        };
        write_back(&path, &old, &new).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            written,
            "# my config\nwidth = 64   # my width\nvim = true # keep\ncolour = \"red\"\ntab_width = 8\ncode_color = \"white\"\n\n[keys]\nsave = \"C-s\"\n"
        );
        assert_eq!(loaded(&written), new);
    }

    #[test]
    fn write_back_only_writes_changed_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".mustty.toml");
        std::fs::write(&path, "width = 64\n").unwrap();
        // 用户只改了 vim：文件里已有的 width 原样不动
        let old = loaded("width = 64\n");
        assert_eq!(old.width, 64);
        let new = Config {
            vim: true,
            ..old.clone()
        };
        assert_eq!(changed_keys(&old, &new), vec!["vim"]);
        write_back(&path, &old, &new).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "width = 64\nvim = true\n",
            "只写变化的键"
        );
        // 没有变化：不写，也不创建不存在的文件
        let missing = dir.path().join("none.toml");
        write_back(&missing, &old, &old).unwrap();
        assert!(!missing.exists());
        // 文件不存在而有变化：新建，只含变化的键
        write_back(&missing, &old, &new).unwrap();
        assert_eq!(std::fs::read_to_string(&missing).unwrap(), "vim = true\n");
    }

    #[test]
    fn write_back_refuses_files_with_syntax_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".mustty.toml");
        let text = "width = = 64\n# 用户自己的笔记\n";
        std::fs::write(&path, text).unwrap();
        let old = Config::default();
        let new = Config {
            vim: true,
            ..old.clone()
        };
        assert!(matches!(
            write_back(&path, &old, &new),
            Err(WriteBackError::Syntax)
        ));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text, "原样不动");
    }

    #[cfg(unix)]
    #[test]
    fn write_back_follows_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let dotfiles = dir.path().join("dotfiles");
        std::fs::create_dir(&dotfiles).unwrap();
        let real = dotfiles.join("mustty.toml");
        std::fs::write(&real, "# tracked in git\nvim = false\n").unwrap();
        let link = dir.path().join(".mustty.toml");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let old = Config::default();
        let new = Config {
            vim: true,
            ..old.clone()
        };
        write_back(&link, &old, &new).unwrap();
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink(),
            "链接本身保留"
        );
        assert_eq!(
            std::fs::read_to_string(&real).unwrap(),
            "# tracked in git\nvim = true\n"
        );
    }

    proptest! {
        /// 任意文本（含随机 TOML 片段）都不会让解析 panic。
        #[test]
        fn never_panics(lines in prop::collection::vec(
            prop_oneof![
                "[a-z_]{1,16} = -?[0-9]{1,20}",
                "[a-z_]{1,16} = \"[^\"\\\\]{0,10}\"",
                "(width|vim|mouse|tab_width|code_color|scrolloff) = (true|false|[0-9]{1,6}|\"[a-z]{0,8}\"|\\[1, 2\\])",
                "\\[[a-z]{1,5}\\]",
                ".{0,20}",
            ],
            0..12,
        )) {
            let _ = PartialConfig::from_toml_str(&lines.join("\n"));
        }
    }
}
