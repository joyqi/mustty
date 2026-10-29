//! `~/.mustty.toml`: the config file takes precedence over built-in defaults, merged key by key.
//! A bad config never blocks startup: a syntax error discards the whole file, a bad key falls back on its own,
//! out-of-range values are clamped. Each case only produces a warning.

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
    /// Stores the choice itself rather than a `Color`: `white` and `gray` both map to `Color::Gray`, so it can't be reversed
    pub code_color: CodeColor,
    /// Cursor shape while editing (vim Normal mode ignores it and always uses a block)
    pub cursor: CursorShape,
    pub cursor_blink: bool,
    /// UI language: `i18n::AUTO` or a tag from `i18n::LOCALES`
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

/// Cursor shape: `cursor = "bar" | "block" | "underline"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CursorShape {
    Bar,
    Block,
    Underline,
}

impl CursorShape {
    /// Cycle order in the settings panel.
    pub const ALL: [CursorShape; 3] = [Self::Bar, Self::Block, Self::Underline];

    /// Spelling in the config file.
    pub fn name(self) -> &'static str {
        match self {
            Self::Bar => "bar",
            Self::Block => "block",
            Self::Underline => "underline",
        }
    }

    /// The matching terminal cursor style (DECSCUSR).
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

/// The 8 standard ANSI foreground colors (SGR 30–37). ratatui calls SGR 37 `Color::Gray`,
/// so `white` (and `gray`, as the spec spells it) both map to `Color::Gray`.
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
    /// Cycle order in the settings panel, which is SGR 30–37 order.
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

    /// Spelling in the config file.
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

/// The config file layer: missing (or invalid) keys are None and keep their defaults on merge.
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
    /// The offending key; None for whole-file problems (unreadable, syntax error)
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
    /// Never fails: a syntax error yields an empty config plus a warning with the line number; unknown keys are dropped.
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
            // Ignore unknown keys (including other tables), so new keys can be added later and the file can be shared with other tools
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
                    // Case-insensitive
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
                    // Case-insensitive
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
                    // auto or a locale tag that has a catalog, case-insensitive
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
                _ => unreachable!("KEYS and the match arms correspond one to one"),
            }
        }
        (p, warnings)
    }
}

impl Config {
    /// Overrides current values with the config file layer (numbers were already clamped in `from_toml_str`).
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

/// Reads `~/.mustty.toml` and merges it as defaults ← file.
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

/// A key's value as written in the config file.
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
            _ => unreachable!("key {key} is not in KEYS"),
        }
    }
}

/// Keys whose values differ between two configs, in `KEYS` order.
pub fn changed_keys(old: &Config, new: &Config) -> Vec<&'static str> {
    KEYS.into_iter()
        .filter(|k| old.scalar(k) != new.scalar(k))
        .collect()
}

#[derive(Debug)]
pub enum WriteBackError {
    /// The existing file has a syntax error: refuse to write rather than try to repair the user's file
    Syntax,
    Io(std::io::Error),
}

/// Writes the keys that changed from `old` to `new` back to the config file.
///
/// The file is re-read at write time and only these keys change on top of its latest content: existing keys
/// are updated in place (keeping comments and key order), new keys are appended to the end of the top level,
/// and unknown keys and other tables are left alone. Symlinks are resolved to the real target before the
/// atomic replace, so the link itself survives.
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
                // Swap the value, not the decor: whitespace around `=` and trailing comments stay as they were
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

/// Joins all warnings into one line for the status bar and stderr.
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
        // Misspelled or no catalog: falls back to auto, and the warning names the key and the choices
        for bad in ["\"xx\"", "\"ja\"", "\"zh\"", "1"] {
            let (c, w) = parse(&format!("lang = {bad}"));
            assert_eq!((c.lang, keys(&w)), ("auto", vec![Some("lang")]), "{bad}");
            assert!(w[0].message.contains("auto en zh-CN"), "{}", w[0].message);
        }
        let (_, w) = parse("lang = \"xx\"\nwidth = 200");
        assert!(
            summarize(&w).unwrap().contains("lang="),
            "included in the config warning summary"
        );
    }

    #[test]
    fn cursor_accepts_three_shapes_and_falls_back_to_bar() {
        crate::i18n::init_for_test("en");
        assert_eq!(
            (Config::default().cursor, Config::default().cursor_blink),
            (CursorShape::Bar, false),
            "default is unchanged: a steady bar"
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
        // Invalid value: falls back to bar, the warning names the key and the choices, and it shows up in the config warning summary
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
        // DECSCUSR numbering: blinking is odd, steady is even
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
        // (config, key, effective value, warned?)
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
            ("gray", White, Color::Gray),  // Spelling from the spec, also SGR 37
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
        // Keys absent from the file keep their defaults; an empty layer changes nothing
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

    /// The config the panel would open with, given this file content.
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
        // The user only changed vim: the width already in the file stays untouched
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
            "writes only changed keys"
        );
        // No changes: write nothing, and don't create a missing file
        let missing = dir.path().join("none.toml");
        write_back(&missing, &old, &old).unwrap();
        assert!(!missing.exists());
        // Missing file with changes: create it with only the changed keys
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
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            text,
            "left untouched"
        );
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
            "the link itself is kept"
        );
        assert_eq!(
            std::fs::read_to_string(&real).unwrap(),
            "# tracked in git\nvim = true\n"
        );
    }

    proptest! {
        /// No text (including random TOML fragments) makes parsing panic.
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
