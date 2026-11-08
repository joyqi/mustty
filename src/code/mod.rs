//! Line-by-line lexer for the code inside fenced blocks, with five tokens. Like the Markdown scanner,
//! each line depends only on the state it starts in, which is small and `Copy`, so a per-line cache
//! can stop rescanning as soon as a state matches.
//!
//! One scanner serves every language: a language is a [`Profile`] (comment markers, string
//! delimiters, raw strings, numbers — shared by a family of languages, with a field or two changed)
//! plus a word [`Table`] generated from highlight.js. See docs/syntax-highlighting.md §7.

pub mod aliases;
mod scan;
#[rustfmt::skip]
mod tables;

pub use scan::scan;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    Keyword,
    Type,
    Str,
    Comment,
    Number,
    /// A diff's added line (`+`)
    DiffAdd,
    /// A diff's removed line (`-`)
    DiffDel,
}

/// Char offsets `[start, end)` within the line. Unlike Markdown's, code ranges never nest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StyledRange {
    pub start: usize,
    pub end: usize,
    pub token: Token,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Lang {
    // Tier 1
    Rust,
    JavaScript,
    TypeScript,
    Python,
    Go,
    Bash,
    PowerShell,
    Json,
    Yaml,
    Toml,
    Sql,
    Html,
    Css,
    C,
    Cpp,
    Java,
    CSharp,
    // Tier 2
    Php,
    Kotlin,
    Lua,
    Ruby,
    Dart,
    Swift,
    Markdown,
    Diff,
    Xml,
    Scss,
    Dockerfile,
    Makefile,
    Protobuf,
}

/// A language as one byte, small enough to sit in the Markdown scanner's block state. Besides the
/// 30 languages there is [`LangId::PLAIN`]: a fence that names `text` or `console` explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LangId(u8);

impl LangId {
    pub const PLAIN: LangId = LangId(u8::MAX);

    /// `None` for [`LangId::PLAIN`], which is never lexed
    pub fn lang(self) -> Option<Lang> {
        Lang::ALL.get(self.0 as usize).copied()
    }
}

impl From<Lang> for LangId {
    fn from(lang: Lang) -> LangId {
        LangId(lang as u8)
    }
}

/// Which branch of the scanner a language goes through
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// `//` and `/* */`, `"` strings: C, C++, Java, C#, Go, Rust, JS, Kotlin, Swift, Dart, PHP, CSS…
    C,
    /// `#` comments after a space, `'` without escapes: Bash, PowerShell, Ruby, Dockerfile, Makefile
    Shell,
    /// `--` comments, `'` strings, `"` quoted identifiers: SQL, Lua
    Sql,
    /// `#` comments, prefixed and triple-quoted strings
    Python,
    /// Tags and `<!-- -->`: HTML, XML
    Markup,
    /// Keys and values: JSON, YAML, TOML
    Data,
    /// Decided by how each line starts: Diff, Markdown
    Prose,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quote {
    pub ch: char,
    /// The escape character inside, if any (`\`, or PowerShell's backtick)
    pub escape: Option<char>,
}

/// A language's syntax, apart from its words
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Profile {
    pub family: Family,
    pub line_comments: &'static [&'static str],
    pub block_comment: Option<(&'static str, &'static str)>,
    /// `/* a /* b */ c */` is one comment (Rust, Swift, Kotlin, Dart)
    pub nested_comments: bool,
    /// A line comment starts only at the line start or after whitespace (`$#` in a shell is not one)
    pub comment_after_space: bool,
    pub quotes: &'static [Quote],
    /// `'` starts a char literal (`'x'`, `'\n'`) or else a lifetime or label, never a string
    pub char_literals: bool,
    /// A `"` string still open at the end of the line carries on (Rust)
    pub multiline_strings: bool,
    /// `"""` (and `'''` where `'` quotes) opens a string that may span lines
    pub triple_quotes: bool,
    /// Raw strings: `r#"…"#`, `R"x(…)x"`, `@"…"`, `` `…` `` and `#"…"#` (C family)
    pub raw_strings: bool,
    /// Numbers: decimal, hex/octal/binary, `_` and `'` separators, exponents and suffixes
    pub numbers: bool,
    /// Characters an identifier may contain besides letters, digits and `_`
    pub ident_extra: &'static [char],
    /// `Foo` and `HashMap` are types, `MAX_LEN` is not
    pub capitalized_types: bool,
    /// `#include`, `#region`, `#if` at the start of a line are keywords
    pub preprocessor: bool,
    /// JS: regex literals and template strings
    pub js: bool,
    /// CSS: `@media`, `!important`, `#fff`
    pub css: bool,
    /// Lua: `--[[ … ]]` comments and `[[ … ]]` strings
    pub long_brackets: bool,
    /// TOML / INI: `[section]` at the start of a line is a type
    pub sections: bool,
}

const ESC: Option<char> = Some('\\');
const DQ: Quote = Quote {
    ch: '"',
    escape: ESC,
};
const SQ: Quote = Quote {
    ch: '\'',
    escape: ESC,
};
/// `'…'` with no escapes (shells, SQL, YAML, TOML)
const SQ_RAW: Quote = Quote {
    ch: '\'',
    escape: None,
};

// The family templates. A language takes one as is or changes a field or two.

const C: Profile = Profile {
    family: Family::C,
    line_comments: &["//"],
    block_comment: Some(("/*", "*/")),
    nested_comments: false,
    comment_after_space: false,
    quotes: &[DQ],
    char_literals: true,
    multiline_strings: false,
    triple_quotes: true,
    raw_strings: true,
    numbers: true,
    ident_extra: &[],
    capitalized_types: true,
    preprocessor: false,
    js: false,
    css: false,
    long_brackets: false,
    sections: false,
};

const SHELL: Profile = Profile {
    family: Family::Shell,
    line_comments: &["#"],
    block_comment: None,
    comment_after_space: true,
    quotes: &[DQ, SQ_RAW],
    char_literals: false,
    triple_quotes: false,
    raw_strings: false,
    ident_extra: &['-'],
    capitalized_types: false,
    ..C
};

const SQL: Profile = Profile {
    family: Family::Sql,
    line_comments: &["--"],
    quotes: &[SQ_RAW],
    char_literals: false,
    triple_quotes: false,
    raw_strings: false,
    capitalized_types: false,
    ..C
};

const PYTHON: Profile = Profile {
    family: Family::Python,
    line_comments: &["#"],
    block_comment: None,
    quotes: &[DQ, SQ],
    char_literals: false,
    raw_strings: false,
    ..C
};

const MARKUP: Profile = Profile {
    family: Family::Markup,
    line_comments: &[],
    block_comment: Some(("<!--", "-->")),
    quotes: &[
        Quote {
            ch: '"',
            escape: None,
        },
        SQ_RAW,
    ],
    char_literals: false,
    triple_quotes: false,
    raw_strings: false,
    numbers: false,
    capitalized_types: false,
    ..C
};

const DATA: Profile = Profile {
    family: Family::Data,
    quotes: &[DQ, SQ],
    char_literals: false,
    triple_quotes: false,
    raw_strings: false,
    capitalized_types: false,
    ..C
};

const PROSE: Profile = Profile {
    family: Family::Prose,
    line_comments: &[],
    block_comment: None,
    quotes: &[],
    numbers: false,
    ..MARKUP
};

impl Lang {
    pub const ALL: [Lang; 30] = [
        Lang::Rust,
        Lang::JavaScript,
        Lang::TypeScript,
        Lang::Python,
        Lang::Go,
        Lang::Bash,
        Lang::PowerShell,
        Lang::Json,
        Lang::Yaml,
        Lang::Toml,
        Lang::Sql,
        Lang::Html,
        Lang::Css,
        Lang::C,
        Lang::Cpp,
        Lang::Java,
        Lang::CSharp,
        Lang::Php,
        Lang::Kotlin,
        Lang::Lua,
        Lang::Ruby,
        Lang::Dart,
        Lang::Swift,
        Lang::Markdown,
        Lang::Diff,
        Lang::Xml,
        Lang::Scss,
        Lang::Dockerfile,
        Lang::Makefile,
        Lang::Protobuf,
    ];

    /// 1 or 2, as in docs/syntax-highlighting.md §7.1
    pub fn tier(self) -> u8 {
        if (self as u8) < Lang::Php as u8 { 1 } else { 2 }
    }

    pub fn profile(self) -> &'static Profile {
        const STRING_QUOTES: Profile = Profile {
            quotes: &[DQ, SQ],
            char_literals: false,
            ..C
        };
        const NESTED: Profile = Profile {
            nested_comments: true,
            ..C
        };
        match self {
            Lang::C | Lang::Cpp | Lang::CSharp => &Profile {
                preprocessor: true,
                ..C
            },
            Lang::Java | Lang::Go => &C,
            Lang::Rust => &Profile {
                multiline_strings: true,
                ..NESTED
            },
            Lang::Kotlin => &NESTED,
            Lang::Swift => &Profile {
                preprocessor: true,
                ..NESTED
            },
            Lang::Dart => &Profile {
                nested_comments: true,
                ident_extra: &['$'],
                ..STRING_QUOTES
            },
            Lang::Protobuf => &STRING_QUOTES,
            Lang::Php => &Profile {
                line_comments: &["//", "#"],
                ident_extra: &['$'],
                ..STRING_QUOTES
            },
            Lang::JavaScript | Lang::TypeScript => &Profile {
                ident_extra: &['$'],
                triple_quotes: false,
                raw_strings: false,
                js: true,
                ..STRING_QUOTES
            },
            Lang::Css => &Profile {
                line_comments: &[],
                ident_extra: &['-'],
                triple_quotes: false,
                raw_strings: false,
                capitalized_types: false,
                css: true,
                ..STRING_QUOTES
            },
            Lang::Scss => &Profile {
                ident_extra: &['-', '$'],
                triple_quotes: false,
                raw_strings: false,
                capitalized_types: false,
                css: true,
                ..STRING_QUOTES
            },
            Lang::Bash | Lang::Dockerfile | Lang::Makefile => &SHELL,
            Lang::PowerShell => &Profile {
                block_comment: Some(("<#", "#>")),
                quotes: &[
                    Quote {
                        ch: '"',
                        escape: Some('`'),
                    },
                    SQ_RAW,
                ],
                ..SHELL
            },
            Lang::Ruby => &Profile {
                comment_after_space: false,
                quotes: &[DQ, SQ],
                ident_extra: &[],
                capitalized_types: true,
                ..SHELL
            },
            Lang::Sql => &Profile {
                block_comment: Some(("/*", "*/")),
                ..SQL
            },
            Lang::Lua => &Profile {
                block_comment: None,
                quotes: &[DQ, SQ],
                long_brackets: true,
                ..SQL
            },
            Lang::Python => &PYTHON,
            Lang::Html | Lang::Xml => &MARKUP,
            Lang::Json => &DATA,
            Lang::Yaml => &Profile {
                line_comments: &["#"],
                block_comment: None,
                comment_after_space: true,
                quotes: &[DQ, SQ_RAW],
                ..DATA
            },
            Lang::Toml => &Profile {
                line_comments: &["#", ";"],
                block_comment: None,
                quotes: &[DQ, SQ_RAW],
                triple_quotes: true,
                sections: true,
                ..DATA
            },
            Lang::Markdown | Lang::Diff => &PROSE,
        }
    }

    pub fn table(self) -> &'static Table {
        use tables as t;
        match self {
            Lang::Rust => &t::RUST,
            Lang::JavaScript => &t::JAVASCRIPT,
            Lang::TypeScript => &t::TYPESCRIPT,
            Lang::Python => &t::PYTHON,
            Lang::Go => &t::GO,
            Lang::Bash => &t::BASH,
            Lang::PowerShell => &t::POWERSHELL,
            Lang::Json => &t::JSON,
            Lang::Yaml => &t::YAML,
            Lang::Toml => &t::TOML,
            Lang::Sql => &t::SQL,
            Lang::Html => &t::HTML,
            Lang::Css => &t::CSS,
            Lang::C => &t::C,
            Lang::Cpp => &t::CPP,
            Lang::Java => &t::JAVA,
            Lang::CSharp => &t::CSHARP,
            Lang::Php => &t::PHP,
            Lang::Kotlin => &t::KOTLIN,
            Lang::Lua => &t::LUA,
            Lang::Ruby => &t::RUBY,
            Lang::Dart => &t::DART,
            Lang::Swift => &t::SWIFT,
            Lang::Markdown => &t::MARKDOWN,
            Lang::Diff => &t::DIFF,
            Lang::Xml => &t::XML,
            Lang::Scss => &t::SCSS,
            Lang::Dockerfile => &t::DOCKERFILE,
            Lang::Makefile => &t::MAKEFILE,
            Lang::Protobuf => &t::PROTOBUF,
        }
    }
}

/// A language's words, generated into tables.rs. Both lists are sorted, so lookups binary-search;
/// in a case-insensitive language they are lowercase and so is the word looked up.
#[derive(Debug)]
pub struct Table {
    pub keywords: &'static [&'static str],
    pub types: &'static [&'static str],
    pub case_insensitive: bool,
}

/// The source of tables.rs, for the test that checks it against the hashes in its header
pub const TABLES_SOURCE: &str = include_str!("tables.rs");

/// Where a line starts. Only constructs that legally span lines carry over.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CodeState {
    #[default]
    Normal,
    /// Inside a block comment, at this nesting depth
    BlockComment(u8),
    /// Inside a `"…"` string that carries over a line end (Rust)
    Str(char),
    /// Inside a raw string closed by `quote` and this many `#`: `r#"…"#`, `#"…"#`, `@"…"`, `` `…` ``
    RawStr { quote: char, hashes: u8 },
    /// Inside a C++ `R"delim(…)delim"`; the delimiter is at most 8 bytes
    CppRaw { delim: [u8; 8], len: u8 },
    /// Inside `"""…"""` or `'''…'''`; `f` for a Python f-string
    Triple { quote: char, f: bool },
    /// Inside a JS template literal's text
    Template,
    /// Inside a JS template's `${ … }`, at this brace depth
    TemplateExpr(u8),
    /// Inside a Lua `[==[ … ]==]` with this many `=`, a comment if it opened with `--`
    LongBracket { level: u8, comment: bool },
    /// Inside an HTML/XML tag, among its attributes
    Tag,
    /// Inside `<![CDATA[ … ]]>`
    Cdata,
}
