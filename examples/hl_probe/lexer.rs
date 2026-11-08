//! Experimental line-by-line code lexer with five tokens, for measuring how far a hand-written lexer
//! gets before it is wired into `src/markdown.rs`. Like the Markdown scanner, each line depends only
//! on the state it starts in, which is small and `Copy`, so a per-line cache can stop rescanning as
//! soon as a state matches.
//!
//! Word tables: copied by hand from highlight.js 11.12.0 (BSD-3-Clause, (c) 2006 Ivan Sagalaev),
//! `lib/languages/{rust,javascript,python}.js`, with the corrections the research doc proposes:
//! prelude names (`Some`, `Ok`) and host globals (`console`, `window`) are not keywords, and neither
//! are Rust's weak keywords `raw` and `union`, which are ordinary names almost everywhere.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tok {
    Keyword,
    Type,
    Str,
    Comment,
    Number,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Rust,
    Js,
    Python,
}

impl Lang {
    /// From a fence's info string: its first word, case-insensitive. Anything else stays one color.
    pub fn from_info(info: &str) -> Option<Lang> {
        let word = info.split_whitespace().next()?.to_ascii_lowercase();
        match word.as_str() {
            "rust" | "rs" => Some(Lang::Rust),
            "js" | "javascript" | "jsx" | "mjs" | "cjs" => Some(Lang::Js),
            "py" | "python" | "python3" => Some(Lang::Python),
            _ => None,
        }
    }

    fn keywords(self) -> &'static [&'static str] {
        match self {
            Lang::Rust => &[
                "Self", "abstract", "as", "async", "await", "become", "box", "break", "const",
                "continue", "crate", "do", "dyn", "else", "enum", "extern", "false", "final", "fn",
                "for", "gen", "if", "impl", "in", "let", "loop", "macro", "match", "mod", "move",
                "mut", "override", "priv", "pub", "ref", "return", "self", "static", "struct",
                "super", "trait", "true", "try", "type", "typeof", "unsafe", "unsized", "use",
                "virtual", "where", "while", "yield",
            ],
            Lang::Js => &[
                "as",
                "async",
                "await",
                "break",
                "case",
                "catch",
                "class",
                "const",
                "continue",
                "debugger",
                "default",
                "delete",
                "do",
                "else",
                "export",
                "extends",
                "finally",
                "for",
                "from",
                "function",
                "if",
                "import",
                "in",
                "instanceof",
                "let",
                "new",
                "of",
                "return",
                "static",
                "super",
                "switch",
                "this",
                "throw",
                "try",
                "typeof",
                "using",
                "var",
                "void",
                "while",
                "with",
                "yield",
                "Infinity",
                "NaN",
                "false",
                "null",
                "true",
                "undefined",
            ],
            Lang::Python => &[
                "and", "as", "assert", "async", "await", "break", "case", "class", "continue",
                "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if",
                "import", "in", "is", "lambda", "lazy", "match", "nonlocal", "not", "or", "pass",
                "raise", "return", "try", "while", "with", "yield", "False", "None", "True",
            ],
        }
    }

    fn types(self) -> &'static [&'static str] {
        match self {
            Lang::Rust => &[
                "bool", "char", "f16", "f32", "f64", "f128", "i8", "i16", "i32", "i64", "i128",
                "isize", "str", "u8", "u16", "u32", "u64", "u128", "usize",
            ],
            Lang::Js => &[],
            Lang::Python => &[
                "bool",
                "bytearray",
                "bytes",
                "complex",
                "dict",
                "float",
                "frozenset",
                "int",
                "list",
                "object",
                "set",
                "str",
                "tuple",
            ],
        }
    }
}

/// Where a line starts. Only constructs that legally span lines carry over.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum State {
    #[default]
    Normal,
    /// Inside a block comment, nesting depth (Rust nests, JS doesn't)
    Block(u8),
    /// Inside a Rust "…" string, which may span lines
    RustStr,
    /// Inside a Rust raw string closed by `"` and this many `#`
    Raw(u8),
    /// Inside a Python triple-quoted string; `f` for f-strings
    Triple { quote: char, f: bool },
    /// Inside a JS template literal's text
    Template,
    /// Inside a JS template's `${ … }`, at this brace depth
    TemplateExpr(u8),
}

pub type Span = (usize, usize, Tok);

/// Keywords after which a JS `/` starts a regex rather than dividing.
const JS_BEFORE_EXPR: &[&str] = &[
    "return",
    "typeof",
    "instanceof",
    "in",
    "of",
    "new",
    "delete",
    "void",
    "throw",
    "case",
    "do",
    "else",
    "yield",
    "await",
];

struct Scan<'a> {
    lang: Lang,
    c: &'a [char],
    i: usize,
    out: Vec<Span>,
    /// JS only: can a `/` here start a regex (nothing yet, an operator or a keyword before it)?
    expr_start: bool,
}

pub fn scan_line(lang: Lang, line: &str, state: State) -> (Vec<Span>, State) {
    let c: Vec<char> = line.chars().collect();
    let mut s = Scan {
        lang,
        c: &c,
        i: 0,
        out: Vec::new(),
        expr_start: true,
    };
    let mut state = state;
    while s.i < c.len() {
        let at = s.i;
        state = match state {
            State::Normal => s.normal(),
            State::Block(depth) => s.block(at, depth),
            State::RustStr => s.quoted(at, '"', true),
            State::Raw(hashes) => s.raw(at, hashes),
            State::Triple { quote, f } => s.triple(at, quote, f),
            State::Template => s.template(at),
            State::TemplateExpr(depth) => s.template_expr(depth),
        };
    }
    (s.out, state)
}

impl Scan<'_> {
    fn at(&self, s: &str) -> bool {
        s.chars()
            .enumerate()
            .all(|(k, ch)| self.c.get(self.i + k) == Some(&ch))
    }

    fn peek(&self, k: usize) -> Option<char> {
        self.c.get(self.i + k).copied()
    }

    fn push(&mut self, start: usize, tok: Tok) {
        self.i = self.i.min(self.c.len());
        if self.i > start {
            self.out.push((start, self.i - start, tok));
        }
    }

    fn normal(&mut self) -> State {
        let start = self.i;
        let ch = self.c[self.i];
        let line_comment = match self.lang {
            Lang::Python => self.at("#"),
            _ => self.at("//"),
        };
        if line_comment {
            self.i = self.c.len();
            self.push(start, Tok::Comment);
            return State::Normal;
        }
        if self.lang != Lang::Python && self.at("/*") {
            self.i += 2;
            return self.block(start, 1);
        }
        if ch == '"' || (ch == '\'' && self.lang != Lang::Rust) {
            return self.string_start(start, "");
        }
        if ch == '\'' && self.lang == Lang::Rust {
            self.rust_quote(start);
            return State::Normal;
        }
        if ch == '`' && self.lang == Lang::Js {
            self.i += 1;
            self.expr_start = false;
            return self.template(start);
        }
        if ch == '/' && self.lang == Lang::Js && self.expr_start && self.regex() {
            self.push(start, Tok::Str);
            self.expr_start = false;
            return State::Normal;
        }
        if ch.is_ascii_digit()
            || (ch == '.'
                && self.lang != Lang::Rust
                && self.peek(1).is_some_and(|d| d.is_ascii_digit())
                && !self.after_operand())
        {
            self.number();
            self.push(start, Tok::Number);
            self.expr_start = false;
            return State::Normal;
        }
        if ch.is_alphabetic() || ch == '_' || (ch == '$' && self.lang == Lang::Js) {
            while self.i < self.c.len()
                && (self.c[self.i].is_alphanumeric()
                    || self.c[self.i] == '_'
                    || (self.c[self.i] == '$' && self.lang == Lang::Js))
            {
                self.i += 1;
            }
            let word: String = self.c[start..self.i].iter().collect();
            if let Some(state) = self.prefixed_string(start, &word) {
                return state;
            }
            // `obj.class` is a property, but Rust's `.await` is a keyword
            let member = self.c[..start].last() == Some(&'.') && word != "await";
            let tok = if !member && self.lang.keywords().contains(&word.as_str()) {
                Some(Tok::Keyword)
            } else if self.lang.types().contains(&word.as_str()) || is_type_name(&word) {
                Some(Tok::Type)
            } else {
                None
            };
            if let Some(tok) = tok {
                self.push(start, tok);
            }
            self.expr_start = tok == Some(Tok::Keyword) && JS_BEFORE_EXPR.contains(&word.as_str());
            return State::Normal;
        }
        self.i += 1;
        if !ch.is_whitespace() {
            // `)` `]` `}` end an operand: a `/` after them divides
            self.expr_start = !matches!(ch, ')' | ']' | '}');
        }
        State::Normal
    }

    /// The previous non-space character ends an operand (`x.5` is not a number, `x .5` is)
    fn after_operand(&self) -> bool {
        self.c[..self.i]
            .iter()
            .rev()
            .find(|c| !c.is_whitespace())
            .is_some_and(|c| c.is_alphanumeric() || matches!(c, ')' | ']' | '_'))
    }

    fn block(&mut self, start: usize, depth: u8) -> State {
        let mut depth = depth;
        let nests = self.lang == Lang::Rust;
        while self.i < self.c.len() {
            if self.at("*/") {
                self.i += 2;
                depth -= 1;
                if depth == 0 {
                    self.push(start, Tok::Comment);
                    return State::Normal;
                }
            } else if nests && self.at("/*") {
                self.i += 2;
                depth += 1;
            } else {
                self.i += 1;
            }
        }
        self.push(start, Tok::Comment);
        State::Block(depth)
    }

    /// A string opened by `prefix` + quote, the prefix starting at `start`
    fn string_start(&mut self, start: usize, prefix: &str) -> State {
        let quote = self.c[self.i];
        self.expr_start = false;
        if self.lang == Lang::Python && self.peek(1) == Some(quote) && self.peek(2) == Some(quote) {
            self.i += 3;
            let f = prefix.contains(['f', 'F']);
            return self.triple(start, quote, f);
        }
        self.i += 1;
        if self.lang == Lang::Python && prefix.contains(['f', 'F']) {
            return self.fstring(start, quote);
        }
        self.quoted(start, quote, self.lang == Lang::Rust)
    }

    /// Rest of a quoted string up to `quote` (escapes skipped). At the end of the line a Rust string
    /// carries on; a JS or Python one stops there (it's a syntax error, so don't swallow later lines).
    fn quoted(&mut self, start: usize, quote: char, spans_lines: bool) -> State {
        while self.i < self.c.len() {
            match self.c[self.i] {
                '\\' => self.i += 2,
                c if c == quote => {
                    self.i += 1;
                    self.push(start, Tok::Str);
                    return State::Normal;
                }
                _ => self.i += 1,
            }
        }
        self.i = self.c.len();
        self.push(start, Tok::Str);
        if spans_lines {
            State::RustStr
        } else {
            State::Normal
        }
    }

    /// Python f-string body on one line: `{…}` is code (left plain), `{{` is a literal brace
    fn fstring(&mut self, start: usize, quote: char) -> State {
        let mut start = start;
        while self.i < self.c.len() {
            match self.c[self.i] {
                '\\' => self.i += 2,
                '{' if self.peek(1) == Some('{') => self.i += 2,
                '{' => {
                    self.push(start, Tok::Str);
                    self.skip_braces();
                    start = self.i;
                }
                c if c == quote => {
                    self.i += 1;
                    self.push(start, Tok::Str);
                    return State::Normal;
                }
                _ => self.i += 1,
            }
        }
        self.i = self.i.min(self.c.len());
        self.push(start, Tok::Str);
        State::Normal
    }

    /// Past a `{ … }` with nested braces, on this line
    fn skip_braces(&mut self) {
        let mut depth = 0;
        while self.i < self.c.len() {
            match self.c[self.i] {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        self.i += 1;
                        return;
                    }
                }
                _ => {}
            }
            self.i += 1;
        }
    }

    fn triple(&mut self, start: usize, quote: char, f: bool) -> State {
        let mut start = start;
        while self.i < self.c.len() {
            if self.c[self.i] == '\\' {
                self.i += 2;
            } else if f && self.c[self.i] == '{' && self.peek(1) != Some('{') {
                self.push(start, Tok::Str);
                self.skip_braces();
                start = self.i;
            } else if self.c[self.i] == quote
                && self.peek(1) == Some(quote)
                && self.peek(2) == Some(quote)
            {
                self.i += 3;
                self.push(start, Tok::Str);
                return State::Normal;
            } else {
                self.i += 1;
            }
        }
        self.i = self.c.len();
        self.push(start, Tok::Str);
        State::Triple { quote, f }
    }

    /// Rust `r"…"`, `br#"…"#`, `b"…"`, `c"…"`; Python `r'…'`, `f"…"`, `rb"""…"""`
    fn prefixed_string(&mut self, start: usize, word: &str) -> Option<State> {
        let next = self.peek(0)?;
        match self.lang {
            Lang::Rust => {
                let raw = matches!(word, "r" | "br" | "cr");
                if raw && (next == '"' || next == '#') {
                    let mut hashes = 0;
                    while self.peek(hashes) == Some('#') {
                        hashes += 1;
                    }
                    if self.peek(hashes) != Some('"') {
                        return None;
                    }
                    self.i += hashes + 1;
                    return Some(self.raw(start, hashes as u8));
                }
                if matches!(word, "b" | "c") && next == '"' {
                    return Some(self.string_start(start, word));
                }
                if word == "b" && next == '\'' {
                    self.rust_quote(start);
                    return Some(State::Normal);
                }
                None
            }
            Lang::Python => {
                let lower = word.to_ascii_lowercase();
                let prefix = matches!(
                    lower.as_str(),
                    "r" | "b" | "u" | "f" | "rb" | "br" | "fr" | "rf"
                );
                (prefix && (next == '"' || next == '\'')).then(|| self.string_start(start, word))
            }
            Lang::Js => None,
        }
    }

    fn raw(&mut self, start: usize, hashes: u8) -> State {
        let close: String = std::iter::once('"')
            .chain((0..hashes).map(|_| '#'))
            .collect();
        while self.i < self.c.len() {
            if self.at(&close) {
                self.i += close.len();
                self.push(start, Tok::Str);
                return State::Normal;
            }
            self.i += 1;
        }
        self.push(start, Tok::Str);
        State::Raw(hashes)
    }

    /// Rust `'`: a char literal (`'x'`, `'\n'`, `'\u{1F600}'`, `b'x'`) or a lifetime / label (`'a`)
    fn rust_quote(&mut self, start: usize) {
        let q = self.c[start..].iter().position(|&c| c == '\'').unwrap() + start;
        self.i = q + 1;
        if self.peek(0) == Some('\\') {
            self.i += 2;
            while self.i < self.c.len() && self.c[self.i] != '\'' {
                self.i += 1;
            }
            self.i = (self.i + 1).min(self.c.len());
            self.push(start, Tok::Str);
        } else if self.peek(1) == Some('\'') {
            self.i += 2;
            self.push(start, Tok::Str);
        } else {
            // Lifetime: the name stays plain
            while self.i < self.c.len()
                && (self.c[self.i].is_alphanumeric() || self.c[self.i] == '_')
            {
                self.i += 1;
            }
        }
    }

    /// A JS regex literal from this `/` to its closing `/` and flags, if it closes on this line
    fn regex(&mut self) -> bool {
        let mut j = self.i + 1;
        let mut class = false;
        if self.c.get(j).is_none_or(|c| c.is_whitespace()) {
            return false;
        }
        while j < self.c.len() {
            match self.c[j] {
                '\\' => j += 1,
                '[' => class = true,
                ']' => class = false,
                '/' if !class => {
                    j += 1;
                    while j < self.c.len() && self.c[j].is_ascii_alphabetic() {
                        j += 1;
                    }
                    self.i = j;
                    return true;
                }
                _ => {}
            }
            j += 1;
        }
        false
    }

    fn number(&mut self) {
        let radix = self.c[self.i] == '0'
            && matches!(self.peek(1), Some('x' | 'X' | 'o' | 'O' | 'b' | 'B'));
        if radix {
            self.i += 2;
            while self
                .peek(0)
                .is_some_and(|c| c.is_ascii_hexdigit() || c == '_')
            {
                self.i += 1;
            }
        } else {
            let digits = |s: &mut Self| {
                while s.peek(0).is_some_and(|c| c.is_ascii_digit() || c == '_') {
                    s.i += 1;
                }
            };
            digits(self);
            // `1.5` but not `1..2` (range) or `1.max(2)` (method)
            if self.peek(0) == Some('.') && self.peek(1).is_some_and(|c| c.is_ascii_digit()) {
                self.i += 1;
                digits(self);
            }
            if matches!(self.peek(0), Some('e' | 'E')) {
                let sign = matches!(self.peek(1), Some('+' | '-')) as usize;
                if self.peek(1 + sign).is_some_and(|c| c.is_ascii_digit()) {
                    self.i += 1 + sign;
                    digits(self);
                }
            }
        }
        // Suffix: `u32`, `f64`, `n` (BigInt), `j` (imaginary)
        while self
            .peek(0)
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            self.i += 1;
        }
    }

    fn template(&mut self, start: usize) -> State {
        while self.i < self.c.len() {
            match self.c[self.i] {
                '\\' => self.i += 2,
                '`' => {
                    self.i += 1;
                    self.push(start, Tok::Str);
                    return State::Normal;
                }
                '$' if self.peek(1) == Some('{') => {
                    self.push(start, Tok::Str);
                    self.i += 2;
                    return State::TemplateExpr(1);
                }
                _ => self.i += 1,
            }
        }
        self.i = self.c.len();
        self.push(start, Tok::Str);
        State::Template
    }

    /// Code inside `${ … }`, lexed like any other code until the brace that closes it
    fn template_expr(&mut self, depth: u8) -> State {
        let mut depth = depth;
        while self.i < self.c.len() {
            match self.c[self.i] {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        self.i += 1;
                        return State::Template;
                    }
                }
                '`' | '"' | '\'' | '/' => {
                    // A nested string or comment: lex it as normal code, then carry on here
                    let inner = self.normal();
                    if inner != State::Normal {
                        return inner;
                    }
                    continue;
                }
                c if c.is_alphanumeric() || c == '_' => {
                    self.normal();
                    continue;
                }
                _ => {}
            }
            self.i += 1;
        }
        State::TemplateExpr(depth)
    }
}

/// `Foo`, `HashMap` and `T`, but not `MAX_LEN` (a constant)
fn is_type_name(word: &str) -> bool {
    let mut chars = word.chars();
    let first_upper = chars.next().is_some_and(|c| c.is_uppercase());
    let shouting = word.len() > 1
        && word
            .chars()
            .all(|c| c.is_uppercase() || c.is_ascii_digit() || c == '_');
    first_upper && !shouting
}

/// Every line of a block, each starting in the previous line's state
pub fn scan_block(lang: Lang, text: &str) -> Vec<Vec<Span>> {
    let mut state = State::Normal;
    text.lines()
        .map(|line| {
            let (spans, next) = scan_line(lang, line, state);
            state = next;
            spans
        })
        .collect()
}
