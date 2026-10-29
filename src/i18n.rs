//! UI strings: `locales/*.toml` are embedded at compile time; `en` is both the source and the fallback locale.
//!
//! There are only two entry points: `t("id")` and `t_args("id", &[("name", value)])`. The first
//! argument must be a string literal, because a test scans the source to check every id is in `en.toml`.
//!
//! The UI language can change at runtime (the Language item in the settings panel): `use_config`
//! renegotiates, and `t()` returns the new language right away.

use std::cell::Cell;
use std::collections::{BTreeSet, HashMap};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

/// (locale tag, file contents). To add a language, add a row here. The first row must be `en`.
pub const LOCALES: &[(&str, &str)] = &[
    ("en", include_str!("../locales/en.toml")),
    ("zh-CN", include_str!("../locales/zh-CN.toml")),
];

/// Env vars read in order when the `lang` option is `auto`. If none is set, use the system locale, then `en`.
pub const ENV_VARS: [&str; 3] = ["LC_ALL", "LC_MESSAGES", "LANG"];

/// Default for the `lang` option: detect from env vars and the system locale.
pub const AUTO: &str = "auto";

/// Metadata read per locale rather than through `t()`: each catalog names its language in that language.
pub const META_KEYS: [&str; 1] = ["language-name"];

pub type Catalog = HashMap<String, String>;

/// Index of the active locale in `LOCALES`; can change at runtime.
static ACTIVE: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    /// Tests pin the locale per thread: cargo test runs each test on its own thread, so they don't interfere.
    static TEST_LOCALE: Cell<Option<usize>> = const { Cell::new(None) };
}

/// Picks the UI language from env vars and the system locale (the initial value before the config file is read).
/// `main()` then calls `use_config` with the `lang` option, all before clap parses (`--help` needs translating too).
pub fn init() {
    use_config(AUTO);
}

/// Renegotiates and switches based on the `lang` option (`auto` or a locale tag); returns the locale in effect.
pub fn use_config(lang: &str) -> &'static str {
    let tag = negotiate_with(lang, |k| std::env::var(k).ok(), sys_locale::get_locale());
    set_language(tag);
    tag
}

/// Switches the UI language. On a test thread with a pinned locale, only that thread changes, so parallel tests are unaffected.
pub fn set_language(tag: &str) {
    let i = index(tag);
    let pinned = TEST_LOCALE.with(|c| {
        let pinned = c.get().is_some();
        if pinned {
            c.set(Some(i));
        }
        pinned
    });
    if !pinned {
        ACTIVE.store(i, Ordering::Relaxed);
    }
}

/// All available locale tags, in `LOCALES` order.
pub fn tags() -> impl Iterator<Item = &'static str> {
    LOCALES.iter().map(|(tag, _)| *tag)
}

/// Maps a locale tag from the config file (case-insensitive) to its spelling in `LOCALES`.
pub fn known_tag(tag: &str) -> Option<&'static str> {
    tags().find(|t| t.eq_ignore_ascii_case(tag))
}

/// The language's own name (English, 简体中文); doesn't change with the current UI language.
pub fn native_name(tag: &str) -> &'static str {
    let i = index(tag);
    resolve(catalogs(), i, META_KEYS[0]).map_or(LOCALES[i].0, |(s, _)| s)
}

/// Pins the current thread's locale in tests.
pub fn init_for_test(tag: &str) {
    TEST_LOCALE.with(|c| c.set(Some(index(tag))));
}

/// Tag of the active locale.
pub fn current() -> &'static str {
    LOCALES[active()].0
}

pub fn t(id: &'static str) -> &'static str {
    lookup(id)
}

/// Looks up an id stored in a data table (key binding descriptions in help). Wherever the source can use a literal, use `t()`.
/// These ids are listed by `help::ids()`, and a test likewise checks they are all in en.toml.
pub fn t_id(id: &'static str) -> &'static str {
    lookup(id)
}

/// Plain `{name}` substitution; placeholders without an argument are left as-is.
pub fn t_args(id: &'static str, args: &[(&str, &str)]) -> String {
    let (s, missing) = substitute(lookup(id), args);
    debug_assert!(missing.is_empty(), "{id}: missing arguments {missing:?}");
    s
}

fn lookup(id: &'static str) -> &'static str {
    let locale = active();
    // Fall back to en for missing translations. Tests check every catalog's keys, so don't report it here
    match resolve(catalogs(), locale, id) {
        Some((s, _)) => s,
        None => {
            debug_assert!(false, "unknown message id {id}");
            id
        }
    }
}

fn active() -> usize {
    TEST_LOCALE
        .with(Cell::get)
        .unwrap_or_else(|| ACTIVE.load(Ordering::Relaxed))
}

fn index(tag: &str) -> usize {
    LOCALES
        .iter()
        .position(|(t, _)| *t == tag)
        .unwrap_or_else(|| panic!("unknown locale {tag}"))
}

fn catalogs() -> &'static [Catalog] {
    static CATALOGS: OnceLock<Vec<Catalog>> = OnceLock::new();
    CATALOGS.get_or_init(|| {
        LOCALES
            .iter()
            .map(|(tag, src)| parse(src).unwrap_or_else(|e| panic!("locales/{tag}.toml: {e}")))
            .collect()
    })
}

/// A flat `id = "text"` table; values must be strings.
pub fn parse(src: &str) -> Result<Catalog, String> {
    let table: toml::Table = src.parse().map_err(|e: toml::de::Error| e.to_string())?;
    table
        .into_iter()
        .map(|(k, v)| match v {
            toml::Value::String(s) => Ok((k, s)),
            other => Err(format!("{k}: expected a string, got {}", other.type_str())),
        })
        .collect()
}

/// Looks in `locale`, then in `en` (the second item is true on fallback); None if neither has it.
fn resolve<'a>(cats: &'a [Catalog], locale: usize, id: &str) -> Option<(&'a str, bool)> {
    match cats[locale].get(id) {
        Some(s) => Some((s, false)),
        None => cats[0].get(id).map(|s| (s.as_str(), true)),
    }
}

/// Substitutes `{name}`; returns the result and the names of placeholders that had no argument.
fn substitute<'a>(template: &'a str, args: &[(&str, &str)]) -> (String, Vec<&'a str>) {
    let mut out = String::with_capacity(template.len());
    let mut missing = Vec::new();
    let mut rest = template;
    while let Some((before, name, after)) = next_placeholder(rest) {
        out.push_str(before);
        match args.iter().find(|(k, _)| *k == name) {
            Some((_, v)) => out.push_str(v),
            None => {
                missing.push(name);
                out.push('{');
                out.push_str(name);
                out.push('}');
            }
        }
        rest = after;
    }
    out.push_str(rest);
    (out, missing)
}

fn next_placeholder(s: &str) -> Option<(&str, &str, &str)> {
    let open = s.find('{')?;
    let close = open + s[open..].find('}')?;
    Some((&s[..open], &s[open + 1..close], &s[close + 1..]))
}

/// Placeholder names that appear in a string.
pub fn placeholders(s: &str) -> BTreeSet<&str> {
    let mut names = BTreeSet::new();
    let mut rest = s;
    while let Some((_, name, after)) = next_placeholder(rest) {
        names.insert(name);
        rest = after;
    }
    names
}

/// Takes the first non-empty env var, or else the system locale, and matches it against `LOCALES`.
pub fn negotiate(env: impl Fn(&str) -> Option<String>, system: Option<String>) -> &'static str {
    negotiate_with(AUTO, env, system)
}

/// Precedence: the `lang` option (unless `auto`) > `LC_ALL` / `LC_MESSAGES` / `LANG` > system locale > `en`.
pub fn negotiate_with(
    config: &str,
    env: impl Fn(&str) -> Option<String>,
    system: Option<String>,
) -> &'static str {
    if let Some(tag) = known_tag(config) {
        return tag;
    }
    ENV_VARS
        .iter()
        .find_map(|k| env(k).filter(|v| !v.is_empty()))
        .or(system)
        .map_or("en", |tag| match_tag(&tag))
}

/// Accepts both POSIX (`zh_CN.UTF-8@x`) and BCP 47 (`zh-Hans-CN`) forms: match language + region first, then language alone.
fn match_tag(raw: &str) -> &'static str {
    let tag = raw.split(['.', '@']).next().unwrap_or_default();
    if matches!(tag, "" | "C" | "POSIX") {
        return "en";
    }
    let mut parts = tag.split(['-', '_']);
    let lang = parts.next().unwrap_or_default().to_ascii_lowercase();
    let (mut script, mut region) = (None, None);
    for p in parts {
        let alpha = p.chars().all(|c| c.is_ascii_alphabetic());
        match p.len() {
            4 if alpha => script = Some(p.to_ascii_lowercase()),
            2 if alpha => region = Some(p.to_ascii_uppercase()),
            3 if p.chars().all(|c| c.is_ascii_digit()) => region = Some(p.to_owned()),
            _ => {}
        }
    }
    if lang == "zh" {
        // Traditional Chinese asks for zh-TW; without that catalog, the language-only match below falls back to zh-CN
        let traditional = script.as_deref() == Some("hant")
            || matches!(region.as_deref(), Some("TW" | "HK" | "MO"));
        region = Some(if traditional { "TW" } else { "CN" }.to_owned());
    }
    let split = |t: &'static str| t.split_once('-').map_or((t, None), |(l, r)| (l, Some(r)));
    let tags = || LOCALES.iter().map(|(t, _)| *t);
    tags()
        .find(|t| split(t) == (lang.as_str(), region.as_deref()))
        .or_else(|| tags().find(|t| split(t).0 == lang))
        .unwrap_or("en")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn root() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR"))
    }

    #[test]
    fn every_locale_file_parses_and_is_registered() {
        assert_eq!(LOCALES[0].0, "en");
        let mut files: Vec<String> = std::fs::read_dir(root().join("locales"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        files.sort();
        let mut registered: Vec<String> =
            LOCALES.iter().map(|(t, _)| format!("{t}.toml")).collect();
        registered.sort();
        assert_eq!(
            files, registered,
            "locales/ doesn't match the LOCALES table"
        );
        for (tag, src) in LOCALES {
            let cat = parse(src).unwrap_or_else(|e| panic!("{tag}: {e}"));
            assert!(!cat.is_empty(), "{tag}");
        }
    }

    #[test]
    fn translations_are_subsets_of_en_with_same_placeholders() {
        let cats = catalogs();
        for (i, (tag, _)) in LOCALES.iter().enumerate().skip(1) {
            for (id, text) in &cats[i] {
                let en = cats[0]
                    .get(id)
                    .unwrap_or_else(|| panic!("{tag}: orphan key {id}"));
                assert_eq!(
                    placeholders(text),
                    placeholders(en),
                    "{tag}: placeholders of {id} differ from en"
                );
            }
        }
    }

    /// `t(` / `t_args(` calls in `src/**/*.rs` (the part of each file before `#[cfg(test)]`).
    fn source_ids() -> Vec<(String, String)> {
        fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
            for e in std::fs::read_dir(dir).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().is_some_and(|x| x == "rs") {
                    out.push(p);
                }
            }
        }
        let mut files = Vec::new();
        walk(&root().join("src"), &mut files);
        let mut ids = Vec::new();
        for file in files {
            let text = std::fs::read_to_string(&file).unwrap();
            // Blank out comment lines (keeping line numbers). cargo fmt may wrap the literal onto the next line, so scan the whole text
            let code: Vec<&str> = text
                .split("#[cfg(test)]")
                .next()
                .unwrap()
                .lines()
                .map(|l| {
                    if l.trim_start().starts_with("//") {
                        ""
                    } else {
                        l
                    }
                })
                .collect();
            let code = code.join("\n");
            for name in ["t(", "t_args("] {
                for (i, _) in code.match_indices(name) {
                    let prev = code[..i].chars().next_back();
                    let ident = prev.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.');
                    if ident || code[..i].ends_with("fn ") {
                        continue;
                    }
                    let line = code[..i].matches('\n').count() + 1;
                    let at = format!("{}:{line}", file.display());
                    let rest = code[i + name.len()..].trim_start();
                    let lit = rest
                        .strip_prefix('"')
                        .and_then(|r| r.split_once('"'))
                        .unwrap_or_else(|| panic!("{at}: {name}… needs a string literal first"));
                    ids.push((lit.0.to_owned(), at));
                }
            }
        }
        ids
    }

    #[test]
    fn en_covers_every_id_in_source_and_nothing_else() {
        let en = &catalogs()[0];
        let ids = source_ids();
        assert!(ids.len() >= 30, "too few calls found: {}", ids.len());
        for (id, at) in &ids {
            assert!(en.contains_key(id), "{at}: en.toml is missing {id}");
        }
        let mut used: BTreeSet<&str> = ids.iter().map(|(id, _)| id.as_str()).collect();
        used.extend(META_KEYS);
        for id in crate::help::ids() {
            assert!(en.contains_key(id), "help.rs: en.toml is missing {id}");
            used.insert(id);
        }
        let unused: Vec<&String> = en.keys().filter(|k| !used.contains(k.as_str())).collect();
        assert!(unused.is_empty(), "unused keys in en.toml: {unused:?}");
    }

    #[test]
    fn substitution_and_fallback() {
        assert_eq!(
            substitute("{n} words in {file}", &[("n", "3"), ("file", "a.md")]),
            ("3 words in a.md".to_owned(), vec![])
        );
        assert_eq!(
            substitute("{n} 字 {m}", &[("n", "{m}")]),
            ("{m} 字 {m}".to_owned(), vec!["m"]),
            "argument values aren't substituted again; missing arguments stay as-is"
        );
        assert_eq!(substitute("a {b", &[]), ("a {b".to_owned(), vec![]));
        assert_eq!(placeholders("{a} {b} {a}"), BTreeSet::from(["a", "b"]));

        let cat = |pairs: &[(&str, &str)]| -> Catalog {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        let cats = [cat(&[("a", "A"), ("b", "B")]), cat(&[("a", "甲")])];
        assert_eq!(resolve(&cats, 1, "a"), Some(("甲", false)));
        assert_eq!(
            resolve(&cats, 1, "b"),
            Some(("B", true)),
            "missing key falls back to en"
        );
        assert_eq!(resolve(&cats, 1, "c"), None);
    }

    #[test]
    fn test_locale_is_per_thread() {
        init_for_test("zh-CN");
        assert_eq!((current(), t("status-title")), ("zh-CN", "标题"));
        assert_eq!(t_args("status-words-other", &[("n", "1,234")]), "1,234 字");
        std::thread::spawn(|| {
            init_for_test("en");
            assert_eq!(t_args("status-words-one", &[("n", "1")]), "1 word");
        })
        .join()
        .unwrap();
        assert_eq!(current(), "zh-CN");
    }

    #[test]
    fn config_lang_comes_before_locale_vars() {
        type Env = &'static [(&'static str, &'static str)];
        let cases: &[(&str, Env, Option<&str>, &str)] = &[
            ("auto", &[("LANG", "zh_CN.UTF-8")], None, "zh-CN"),
            ("en", &[("LANG", "zh_CN.UTF-8")], Some("zh-Hans-CN"), "en"),
            ("zh-CN", &[("LC_ALL", "en_US.UTF-8")], None, "zh-CN"),
            ("zh-cn", &[], None, "zh-CN"),
            // MUSTTY_LANG was removed: setting it has no effect
            ("en", &[("MUSTTY_LANG", "zh-CN")], None, "en"),
            (
                "zh-CN",
                &[("MUSTTY_LANG", "en"), ("LANG", "zh_CN")],
                None,
                "zh-CN",
            ),
            ("auto", &[("MUSTTY_LANG", "zh-CN")], None, "en"),
            ("xx", &[("LANG", "zh_CN")], None, "zh-CN"),
            ("auto", &[], Some("zh-Hans-CN"), "zh-CN"),
            ("auto", &[], None, "en"),
        ];
        for (config, env, system, want) in cases {
            let got = negotiate_with(
                config,
                |k| {
                    env.iter()
                        .find(|(e, _)| *e == k)
                        .map(|(_, v)| v.to_string())
                },
                system.map(str::to_owned),
            );
            assert_eq!(got, *want, "lang={config} env {env:?} system {system:?}");
        }
    }

    #[test]
    fn set_language_switches_immediately() {
        init_for_test("en");
        assert_eq!(t("status-untitled"), "Untitled");
        assert_eq!(t("settings-title"), "Settings");
        set_language("zh-CN");
        assert_eq!((current(), t("settings-title")), ("zh-CN", "设置"));
        assert_eq!(t_args("status-words-other", &[("n", "3")]), "3 字");
        set_language("en");
        assert_eq!(t("settings-title"), "Settings");
        // A test thread with a pinned locale only changes itself: other threads are unaffected
        std::thread::spawn(|| {
            init_for_test("zh-CN");
            set_language("zh-CN");
        })
        .join()
        .unwrap();
        assert_eq!(current(), "en");
    }

    #[test]
    fn every_locale_names_itself() {
        let cats = catalogs();
        for (i, (tag, _)) in LOCALES.iter().enumerate() {
            for key in META_KEYS {
                assert!(cats[i].contains_key(key), "{tag} is missing {key}");
            }
        }
        assert_eq!(native_name("en"), "English");
        assert_eq!(native_name("zh-CN"), "简体中文");
        init_for_test("zh-CN");
        assert_eq!(
            native_name("en"),
            "English",
            "not translated with the UI language"
        );
        assert_eq!(known_tag("ZH-cn"), Some("zh-CN"));
        assert_eq!(known_tag("auto"), None);
        assert_eq!(tags().collect::<Vec<_>>(), vec!["en", "zh-CN"]);
    }

    #[test]
    fn negotiation() {
        type Env = &'static [(&'static str, &'static str)];
        let cases: &[(Env, Option<&str>, &str)] = &[
            (&[], None, "en"),
            (&[], Some("zh-Hans-CN"), "zh-CN"),
            (&[], Some("en-US"), "en"),
            (&[("LANG", "zh_CN.UTF-8")], Some("en-US"), "zh-CN"),
            (&[("LANG", "en_US.UTF-8")], Some("zh-Hans-CN"), "en"),
            // MUSTTY_LANG no longer takes part in negotiation
            (
                &[("MUSTTY_LANG", "en"), ("LC_ALL", "zh_CN.UTF-8")],
                None,
                "zh-CN",
            ),
            (
                &[("MUSTTY_LANG", "zh-CN"), ("LANG", "en_US.UTF-8")],
                None,
                "en",
            ),
            (
                &[("LC_ALL", "zh_CN.UTF-8"), ("LANG", "en_US.UTF-8")],
                None,
                "zh-CN",
            ),
            (
                &[("LC_MESSAGES", "zh_SG"), ("LANG", "en_US")],
                None,
                "zh-CN",
            ),
            (&[("LC_ALL", ""), ("LANG", "zh_CN")], None, "zh-CN"),
            (&[("LANG", "C")], Some("zh-Hans-CN"), "en"),
            (&[("LANG", "C.UTF-8")], None, "en"),
            (&[("LC_ALL", "POSIX")], None, "en"),
            (&[("LANG", "zh_TW.UTF-8")], None, "zh-CN"),
            (&[("LANG", "zh-Hant")], None, "zh-CN"),
            (&[("LANG", "zh_HK")], None, "zh-CN"),
            (&[("LANG", "zh")], None, "zh-CN"),
            (&[("LANG", "fr_FR.UTF-8")], Some("zh-Hans-CN"), "en"),
            (&[("LANG", "de_DE@euro")], None, "en"),
        ];
        for (env, system, want) in cases {
            let got = negotiate(
                |k| {
                    env.iter()
                        .find(|(e, _)| *e == k)
                        .map(|(_, v)| v.to_string())
                },
                system.map(str::to_owned),
            );
            assert_eq!(got, *want, "env {env:?}, system {system:?}");
        }
    }
}
