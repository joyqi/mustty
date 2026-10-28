//! 界面文案（方案 §11.4）：`locales/*.toml` 编译期嵌入，`en` 是源语言也是回退语言。
//!
//! 只有两个入口：`t("id")` 与 `t_args("id", &[("name", value)])`，第一个参数必须是
//! 字符串字面量——测试会扫描源码，确认每个 id 都在 `en.toml` 里。
//!
//! 界面语言可在运行时切换（设置面板的「语言」项）：`use_config` 重新协商，`t()` 随即返回新语言。

use std::cell::Cell;
use std::collections::{BTreeSet, HashMap};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

/// (语言标签, 文件内容)。新增一门语言：在这里加一行。第一行必须是 `en`。
pub const LOCALES: &[(&str, &str)] = &[
    ("en", include_str!("../locales/en.toml")),
    ("zh-CN", include_str!("../locales/zh-CN.toml")),
];

/// 配置项 `lang` 为 `auto` 时依次读取的环境变量；都没有时用系统语言，再没有用 `en`。
pub const ENV_VARS: [&str; 3] = ["LC_ALL", "LC_MESSAGES", "LANG"];

/// 配置项 `lang` 的默认值：按环境变量与系统语言自动判断。
pub const AUTO: &str = "auto";

/// 不经 `t()` 读取、而是按语言直接取的元信息：每份译文用自己的语言写出语言名。
pub const META_KEYS: [&str; 1] = ["language-name"];

pub type Catalog = HashMap<String, String>;

/// 当前语言在 `LOCALES` 里的下标；运行时可切换。
static ACTIVE: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    /// 测试按线程固定语言：cargo test 每个测试一个线程，互不干扰。
    static TEST_LOCALE: Cell<Option<usize>> = const { Cell::new(None) };
}

/// 按环境变量与系统语言选定界面语言（还没读配置文件时的初值）。
/// `main()` 随后用配置项 `lang` 调 `use_config`，都在 clap 解析之前（`--help` 也要翻译）。
pub fn init() {
    use_config(AUTO);
}

/// 按配置项 `lang`（`auto` 或语言标签）重新协商并切换；返回生效的语言。
pub fn use_config(lang: &str) -> &'static str {
    let tag = negotiate_with(lang, |k| std::env::var(k).ok(), sys_locale::get_locale());
    set_language(tag);
    tag
}

/// 切换界面语言。测试线程固定过语言时只改这个线程，不影响并行的其他测试。
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

/// 所有可选的语言标签，`LOCALES` 的顺序。
pub fn tags() -> impl Iterator<Item = &'static str> {
    LOCALES.iter().map(|(tag, _)| *tag)
}

/// 配置文件里写的语言标签（大小写不敏感）→ `LOCALES` 里的写法。
pub fn known_tag(tag: &str) -> Option<&'static str> {
    tags().find(|t| t.eq_ignore_ascii_case(tag))
}

/// 语言自己的名字（English、简体中文），不随当前界面语言变化。
pub fn native_name(tag: &str) -> &'static str {
    let i = index(tag);
    resolve(catalogs(), i, META_KEYS[0]).map_or(LOCALES[i].0, |(s, _)| s)
}

/// 测试里固定当前线程的语言。
pub fn init_for_test(tag: &str) {
    TEST_LOCALE.with(|c| c.set(Some(index(tag))));
}

/// 当前语言的标签。
pub fn current() -> &'static str {
    LOCALES[active()].0
}

pub fn t(id: &'static str) -> &'static str {
    lookup(id)
}

/// 按存在数据表里的 id 取文案（键位帮助的说明）。源码里能直接写字面量的地方一律用 `t()`；
/// 这些 id 由 `help::ids()` 列出，测试同样确认它们都在 en.toml 里。
pub fn t_id(id: &'static str) -> &'static str {
    lookup(id)
}

/// `{name}` 简单替换；缺参数时保留原样。
pub fn t_args(id: &'static str, args: &[(&str, &str)]) -> String {
    let (s, missing) = substitute(lookup(id), args);
    debug_assert!(missing.is_empty(), "{id}: missing arguments {missing:?}");
    s
}

fn lookup(id: &'static str) -> &'static str {
    let locale = active();
    // 缺译文时回退 en：每份译文的 key 都由测试检查，这里不再另外报告
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

/// 扁平的 `id = "文案"` 表；值必须是字符串。
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

/// 查 `locale`，缺了查 `en`（第二项为 true 表示发生了回退）；都没有为 None。
fn resolve<'a>(cats: &'a [Catalog], locale: usize, id: &str) -> Option<(&'a str, bool)> {
    match cats[locale].get(id) {
        Some(s) => Some((s, false)),
        None => cats[0].get(id).map(|s| (s.as_str(), true)),
    }
}

/// 替换 `{name}`；返回结果与没有对应参数的占位符名。
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

/// 文案里出现的占位符名。
pub fn placeholders(s: &str) -> BTreeSet<&str> {
    let mut names = BTreeSet::new();
    let mut rest = s;
    while let Some((_, name, after)) = next_placeholder(rest) {
        names.insert(name);
        rest = after;
    }
    names
}

/// 依次取第一个非空的环境变量，否则用系统语言；再与 `LOCALES` 匹配。
pub fn negotiate(env: impl Fn(&str) -> Option<String>, system: Option<String>) -> &'static str {
    negotiate_with(AUTO, env, system)
}

/// 优先级：配置项 `lang`（不是 `auto` 时）> `LC_ALL` / `LC_MESSAGES` / `LANG` > 系统语言 > `en`。
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

/// POSIX（`zh_CN.UTF-8@x`）与 BCP 47（`zh-Hans-CN`）两种写法：先比语言 + 地区，再只比语言。
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
        // 繁体请求 zh-TW；没有这份译文时下面按语言匹配回退到 zh-CN
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
        assert_eq!(files, registered, "locales/ 与 LOCALES 表不一致");
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
                    .unwrap_or_else(|| panic!("{tag}: 孤儿 key {id}"));
                assert_eq!(
                    placeholders(text),
                    placeholders(en),
                    "{tag}: {id} 的占位符与 en 不一致"
                );
            }
        }
    }

    /// `src/**/*.rs`（各文件 `#[cfg(test)]` 之前的部分）里的 `t(` / `t_args(` 调用。
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
            // 注释行换成空行（保留行号）；cargo fmt 可能把字面量折到下一行，所以按整段扫
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
                        .unwrap_or_else(|| panic!("{at}: {name}… 的第一个参数必须是字符串字面量"));
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
        assert!(ids.len() >= 30, "扫描到的调用太少：{}", ids.len());
        for (id, at) in &ids {
            assert!(en.contains_key(id), "{at}: en.toml 缺少 {id}");
        }
        let mut used: BTreeSet<&str> = ids.iter().map(|(id, _)| id.as_str()).collect();
        used.extend(META_KEYS);
        for id in crate::help::ids() {
            assert!(en.contains_key(id), "help.rs: en.toml 缺少 {id}");
            used.insert(id);
        }
        let unused: Vec<&String> = en.keys().filter(|k| !used.contains(k.as_str())).collect();
        assert!(unused.is_empty(), "en.toml 里没被使用的 key：{unused:?}");
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
            "参数值不再替换；缺参数保留原样"
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
        assert_eq!(resolve(&cats, 1, "b"), Some(("B", true)), "缺 key 回退 en");
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
            // MUSTTY_LANG 已删除：设置了也没有任何效果
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
        // 固定过语言的测试线程只改自己：另一个线程不受影响
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
                assert!(cats[i].contains_key(key), "{tag} 缺少 {key}");
            }
        }
        assert_eq!(native_name("en"), "English");
        assert_eq!(native_name("zh-CN"), "简体中文");
        init_for_test("zh-CN");
        assert_eq!(native_name("en"), "English", "不随界面语言翻译");
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
            // MUSTTY_LANG 不再参与协商
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
