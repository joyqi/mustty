//! Fence info strings to languages. The aliases are highlight.js 11.12.0's own (the `aliases` of each
//! language module), plus `python3` and `shell` (which highlight.js reserves for shell sessions, but
//! fences use for scripts) and the names that mean "plain text". See docs/syntax-highlighting.md §7.2.

use super::{Lang, LangId};

const fn l(lang: Lang) -> LangId {
    LangId(lang as u8)
}

const PLAIN: LangId = LangId::PLAIN;

/// Sorted by name, for binary search
pub const ALIASES: &[(&str, LangId)] = &[
    ("atom", l(Lang::Xml)),
    ("bash", l(Lang::Bash)),
    ("c", l(Lang::C)),
    ("c#", l(Lang::CSharp)),
    ("c++", l(Lang::Cpp)),
    ("cc", l(Lang::Cpp)),
    ("cjs", l(Lang::JavaScript)),
    ("console", PLAIN),
    ("cpp", l(Lang::Cpp)),
    ("cs", l(Lang::CSharp)),
    ("csharp", l(Lang::CSharp)),
    ("css", l(Lang::Css)),
    ("cts", l(Lang::TypeScript)),
    ("cxx", l(Lang::Cpp)),
    ("dart", l(Lang::Dart)),
    ("diff", l(Lang::Diff)),
    ("docker", l(Lang::Dockerfile)),
    ("dockerfile", l(Lang::Dockerfile)),
    ("gemspec", l(Lang::Ruby)),
    ("go", l(Lang::Go)),
    ("golang", l(Lang::Go)),
    ("gyp", l(Lang::Python)),
    ("h", l(Lang::C)),
    ("h++", l(Lang::Cpp)),
    ("hh", l(Lang::Cpp)),
    ("hpp", l(Lang::Cpp)),
    ("html", l(Lang::Html)),
    ("hxx", l(Lang::Cpp)),
    ("ini", l(Lang::Toml)),
    ("ipython", l(Lang::Python)),
    ("irb", l(Lang::Ruby)),
    ("java", l(Lang::Java)),
    ("javascript", l(Lang::JavaScript)),
    ("js", l(Lang::JavaScript)),
    ("json", l(Lang::Json)),
    ("json5", l(Lang::Json)),
    ("jsonc", l(Lang::Json)),
    ("jsp", l(Lang::Java)),
    ("jsx", l(Lang::JavaScript)),
    ("kotlin", l(Lang::Kotlin)),
    ("kt", l(Lang::Kotlin)),
    ("ktm", l(Lang::Kotlin)),
    ("kts", l(Lang::Kotlin)),
    ("ktx", l(Lang::Kotlin)),
    ("lua", l(Lang::Lua)),
    ("mak", l(Lang::Makefile)),
    ("make", l(Lang::Makefile)),
    ("makefile", l(Lang::Makefile)),
    ("markdown", l(Lang::Markdown)),
    ("md", l(Lang::Markdown)),
    ("mjs", l(Lang::JavaScript)),
    ("mk", l(Lang::Makefile)),
    ("mkd", l(Lang::Markdown)),
    ("mkdown", l(Lang::Markdown)),
    ("mts", l(Lang::TypeScript)),
    ("patch", l(Lang::Diff)),
    ("php", l(Lang::Php)),
    ("plaintext", PLAIN),
    ("plist", l(Lang::Xml)),
    ("pluto", l(Lang::Lua)),
    ("podspec", l(Lang::Ruby)),
    ("powershell", l(Lang::PowerShell)),
    ("proto", l(Lang::Protobuf)),
    ("protobuf", l(Lang::Protobuf)),
    ("ps", l(Lang::PowerShell)),
    ("ps1", l(Lang::PowerShell)),
    ("pwsh", l(Lang::PowerShell)),
    ("py", l(Lang::Python)),
    ("python", l(Lang::Python)),
    ("python3", l(Lang::Python)),
    ("rb", l(Lang::Ruby)),
    ("rs", l(Lang::Rust)),
    ("rss", l(Lang::Xml)),
    ("ruby", l(Lang::Ruby)),
    ("rust", l(Lang::Rust)),
    ("scss", l(Lang::Scss)),
    ("sh", l(Lang::Bash)),
    ("shell", l(Lang::Bash)),
    ("shellsession", PLAIN),
    ("sql", l(Lang::Sql)),
    ("svg", l(Lang::Xml)),
    ("swift", l(Lang::Swift)),
    ("text", PLAIN),
    ("thor", l(Lang::Ruby)),
    ("toml", l(Lang::Toml)),
    ("ts", l(Lang::TypeScript)),
    ("tsx", l(Lang::TypeScript)),
    ("txt", PLAIN),
    ("typescript", l(Lang::TypeScript)),
    ("wsf", l(Lang::Xml)),
    ("xhtml", l(Lang::Html)),
    ("xjb", l(Lang::Xml)),
    ("xml", l(Lang::Xml)),
    ("xsd", l(Lang::Xml)),
    ("xsl", l(Lang::Xml)),
    ("yaml", l(Lang::Yaml)),
    ("yml", l(Lang::Yaml)),
    ("zsh", l(Lang::Bash)),
];

/// The language of a fence from its info string: the first word, with a Pandoc-style `{.lang}` or
/// `{lang}` unwrapped and anything after a space, `,` or `{` dropped (`rust,no_run`,
/// `js title="x.js"`), compared case-insensitively. `None` when the info string is empty or names
/// a language we don't know; the block then stays one color, as does [`LangId::PLAIN`].
pub fn from_info(info: &str) -> Option<LangId> {
    let word = info.trim_start();
    let word = word.strip_prefix('{').unwrap_or(word);
    let word = word.strip_prefix('.').unwrap_or(word);
    let end = word
        .find(|c: char| c.is_whitespace() || matches!(c, ',' | '{' | '}'))
        .unwrap_or(word.len());
    lookup(&word[..end].to_ascii_lowercase())
}

/// An alias exactly as listed (lowercase)
pub fn lookup(name: &str) -> Option<LangId> {
    ALIASES
        .binary_search_by(|(alias, _)| (*alias).cmp(name))
        .ok()
        .map(|k| ALIASES[k].1)
}
