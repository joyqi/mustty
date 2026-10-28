//! `~/.mustty.toml`（方案 §8.5）：优先级 命令行 > 配置文件 > 内置默认值，按键逐个合并。
//! 配置写错永远不阻止启动：语法错误整份作废，单个键出错只回退这个键，越界钳制；都只产生警告。

use std::path::{Path, PathBuf};
use std::time::Duration;

use ratatui::style::Color;
use serde::Deserialize;

use crate::i18n::{t, t_args};

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
    pub code_color: Color,
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
            code_color: Color::Cyan,
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

/// 一层配置（文件或命令行）：未给出的键为 None。
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

const KEYS: [&str; 10] = [
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
            let Some(&key) = KEYS.iter().find(|&&k| k == key) else {
                tracing::debug!(key, "unknown config key ignored");
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
                _ => unreachable!("KEYS 与 match 分支一一对应"),
            }
        }
        (p, warnings)
    }
}

impl Config {
    /// 用一层配置覆盖当前值；`width` 再钳制一次（命令行的 `--width` 也走这里）。
    pub fn merge(mut self, p: &PartialConfig) -> Self {
        if let Some(w) = p.width {
            self.width = w.clamp(WIDTH_RANGE.0, WIDTH_RANGE.1);
        }
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
        self.code_color = p.code_color.map_or(self.code_color, CodeColor::color);
        self
    }
}

pub fn path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".mustty.toml"))
}

/// 读 `~/.mustty.toml` 并按 默认 ← 文件 ← 命令行 合并。
pub fn load(cli: &PartialConfig) -> (Config, Vec<ConfigWarning>) {
    let (file, mut warnings) = match path() {
        Some(p) => read_file(&p),
        None => (PartialConfig::default(), vec![]),
    };
    if let Some(w) = cli
        .width
        .filter(|w| !(WIDTH_RANGE.0..=WIDTH_RANGE.1).contains(w))
    {
        let c = w.clamp(WIDTH_RANGE.0, WIDTH_RANGE.1);
        warnings.push(ConfigWarning {
            key: Some("width"),
            message: format!("--width {w} → {c}"),
        });
    }
    (Config::default().merge(&file).merge(cli), warnings)
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
            code_color: Color::Yellow,
        };
        assert_eq!(parse(text), (want, vec![]));
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
        let names = [
            ("black", Color::Black),
            ("red", Color::Red),
            ("green", Color::Green),
            ("yellow", Color::Yellow),
            ("blue", Color::Blue),
            ("magenta", Color::Magenta),
            ("cyan", Color::Cyan),
            ("white", Color::Gray), // SGR 37
            ("gray", Color::Gray),  // 需求中的写法，同为 SGR 37
            ("YELLOW", Color::Yellow),
            ("Magenta", Color::Magenta),
        ];
        for (name, color) in names {
            assert_eq!(
                parse(&format!("code_color = {name:?}")),
                (
                    Config {
                        code_color: color,
                        ..Config::default()
                    },
                    vec![]
                ),
                "{name}"
            );
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
                (Color::Cyan, vec![Some("code_color")]),
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
    fn layers_merge_in_priority_order() {
        let (file, _) =
            PartialConfig::from_toml_str("width = 64\nvim = true\nmouse = true\nautosave = true");
        let cli = PartialConfig {
            width: Some(90),
            vim: Some(false),
            mouse: Some(false),
            autosave: Some(false),
            ..PartialConfig::default()
        };
        let c = Config::default().merge(&file).merge(&cli);
        assert_eq!(
            (c.width, c.vim, c.mouse, c.autosave),
            (90, false, false, false)
        );
        // 命令行没给的键不覆盖文件值
        let c = Config::default()
            .merge(&file)
            .merge(&PartialConfig::default());
        assert_eq!(
            (c.width, c.vim, c.mouse, c.autosave),
            (64, true, true, true)
        );
        // 命令行的 --width 同样钳制
        let wide = PartialConfig {
            width: Some(500),
            ..PartialConfig::default()
        };
        assert_eq!(Config::default().merge(&wide).width, 120);
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
