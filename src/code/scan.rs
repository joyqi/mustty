//! The scanner behind [`scan`]: one pass over a line's chars, branching on the language's
//! [`Profile`]. Grown from the experiment in examples/hl_probe, whose accuracy cases now live in
//! tests/code_scan.rs.

use super::{CodeState, Family, Lang, LangId, Profile, StyledRange, Table, Token};

/// Lex one line of a code block that starts in `state`. Returns the line's ranges, which don't
/// overlap and come in order, and the state the next line starts in. Text outside every range is
/// plain code. A [`LangId::PLAIN`] line has no ranges at all.
pub fn scan(lang: LangId, line: &str, state: CodeState) -> (Vec<StyledRange>, CodeState) {
    let Some(lang) = lang.lang() else {
        return (Vec::new(), CodeState::Normal);
    };
    let chars: Vec<char> = line.chars().collect();
    let mut s = Scan {
        lang,
        p: lang.profile(),
        t: lang.table(),
        c: &chars,
        i: 0,
        out: Vec::new(),
        expr_start: true,
    };
    if s.p.family == Family::Prose {
        s.prose();
        return (s.out, CodeState::Normal);
    }
    let mut state = state;
    if state == CodeState::Normal {
        s.line_start();
    }
    while s.i < chars.len() {
        let at = s.i;
        state = match state {
            CodeState::Normal => s.normal(),
            CodeState::BlockComment(depth) => s.block(at, depth),
            CodeState::Str(quote) => s.quoted(at, quote, true),
            CodeState::RawStr { quote, hashes } => s.raw(at, quote, hashes),
            CodeState::CppRaw { delim, len } => s.cpp_raw(at, delim, len),
            CodeState::Triple { quote, f } => s.triple(at, quote, f),
            CodeState::Template => s.template(at),
            CodeState::TemplateExpr(depth) => s.template_expr(depth),
            CodeState::LongBracket { level, comment } => s.long_bracket(at, level, comment),
            CodeState::Tag => s.tag(),
            CodeState::Cdata => s.cdata(at),
        };
    }
    (s.out, state)
}

/// Keywords after which a JS `/` starts a regex rather than dividing
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
    p: &'static Profile,
    t: &'static Table,
    c: &'a [char],
    i: usize,
    out: Vec<StyledRange>,
    /// JS only: can a `/` here start a regex (nothing yet, an operator or a keyword before it)?
    expr_start: bool,
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

    fn prev(&self) -> Option<char> {
        self.i.checked_sub(1).map(|k| self.c[k])
    }

    fn push(&mut self, start: usize, token: Token) {
        self.i = self.i.min(self.c.len());
        if self.i > start {
            self.out.push(StyledRange {
                start,
                end: self.i,
                token,
            });
        }
    }

    fn skip_spaces(&mut self) {
        while self.peek(0).is_some_and(char::is_whitespace) {
            self.i += 1;
        }
    }

    fn is_ident(&self, ch: char) -> bool {
        ch.is_alphanumeric() || ch == '_' || self.p.ident_extra.contains(&ch)
    }

    fn ident_start(&self, ch: char) -> bool {
        ch.is_alphabetic()
            || ch == '_'
            || (ch == '$' && self.p.ident_extra.contains(&'$'))
            // A shell word may start with `-`: `--until` is a flag, not the keyword
            || (ch == '-' && self.p.family == Family::Shell)
    }

    /// Constructs that only count at the start of a line: `#include`, TOML's `[section]`
    fn line_start(&mut self) {
        self.skip_spaces();
        let start = self.i;
        if self.p.preprocessor && self.peek(0) == Some('#') {
            self.i += 1;
            self.skip_spaces();
            let name = self.i;
            while self
                .peek(0)
                .is_some_and(|c| c.is_alphanumeric() || c == '_')
            {
                self.i += 1;
            }
            if self.i > name {
                self.push(start, Token::Keyword);
            } else {
                self.i = start;
            }
        } else if self.p.sections
            && self.peek(0) == Some('[')
            && let Some(close) = self.c[self.i..].iter().position(|&c| c == ']')
        {
            // `[[array]]` closes with as many brackets as it opened with
            self.i += close;
            while self.peek(0) == Some(']') {
                self.i += 1;
            }
            self.push(start, Token::Type);
        }
    }

    fn normal(&mut self) -> CodeState {
        if self.p.family == Family::Markup {
            return self.markup();
        }
        let start = self.i;
        let ch = self.c[self.i];
        if self.p.long_brackets {
            if self.at("--")
                && let Some(level) = self.long_open(self.i + 2)
            {
                self.i += 4 + level as usize;
                return self.long_bracket(start, level, true);
            }
            if let Some(level) = self.long_open(self.i) {
                self.i += 2 + level as usize;
                return self.long_bracket(start, level, false);
            }
        }
        if let Some((open, _)) = self.p.block_comment
            && self.at(open)
        {
            self.i += open.chars().count();
            return self.block(start, 1);
        }
        if self.line_comment() {
            self.i = self.c.len();
            self.push(start, Token::Comment);
            return CodeState::Normal;
        }
        if self.p.js {
            if ch == '`' {
                self.i += 1;
                self.expr_start = false;
                return self.template(start);
            }
            if ch == '/' && self.expr_start && self.regex() {
                self.push(start, Token::Str);
                self.expr_start = false;
                return CodeState::Normal;
            }
        }
        if self.p.css && self.css(start) {
            return CodeState::Normal;
        }
        if self.p.raw_strings
            && let Some(state) = self.raw_open(start)
        {
            return state;
        }
        if self.p.family == Family::Sql
            && matches!(ch, '"' | '`')
            && !self.p.quotes.iter().any(|q| q.ch == ch)
        {
            // A quoted identifier: skip it so `"select"` isn't a keyword, but leave it plain
            self.i += 1;
            while self.peek(0).is_some_and(|c| c != ch) {
                self.i += 1;
            }
            self.i += 1;
            return CodeState::Normal;
        }
        if ch == '\'' && self.p.char_literals {
            self.char_or_lifetime(start);
            return CodeState::Normal;
        }
        if self.p.quotes.iter().any(|q| q.ch == ch) {
            // `it's` in a YAML value is not a quote: one only opens a scalar
            let opens =
                self.p.family != Family::Data || !self.prev().is_some_and(char::is_alphanumeric);
            if opens {
                return self.string_start(start, "");
            }
        }
        if self.p.numbers
            && (ch.is_ascii_digit()
                || (ch == '.'
                    && self.lang != Lang::Rust
                    && self.peek(1).is_some_and(|d| d.is_ascii_digit())
                    && !self.after_operand()))
        {
            self.number();
            self.push(start, Token::Number);
            self.expr_start = false;
            return CodeState::Normal;
        }
        if self.ident_start(ch) {
            self.i += 1;
            while self.peek(0).is_some_and(|c| self.is_ident(c)) {
                self.i += 1;
            }
            let word: String = self.c[start..self.i].iter().collect();
            if let Some(state) = self.prefixed_string(start, &word) {
                return state;
            }
            self.word(start, &word);
            return CodeState::Normal;
        }
        self.i += 1;
        if !ch.is_whitespace() {
            // `)` `]` `}` end an operand: a `/` after them divides
            self.expr_start = !matches!(ch, ')' | ']' | '}');
        }
        CodeState::Normal
    }

    fn line_comment(&self) -> bool {
        let spaced = !self.p.comment_after_space || self.prev().is_none_or(char::is_whitespace);
        spaced && self.p.line_comments.iter().any(|m| self.at(m))
    }

    /// Color an identifier that ends at `self.i`
    fn word(&mut self, start: usize, word: &str) {
        let lower;
        let key = if self.t.case_insensitive {
            lower = word.to_ascii_lowercase();
            lower.as_str()
        } else {
            word
        };
        // `obj.class` is a property, but Rust's `.await` is a keyword
        let member = start > 0 && self.c[start - 1] == '.' && word != "await";
        // CSS words are keywords only as at-rules (`@media`), which css() colors
        let keyword = !member && !self.p.css && self.t.keywords.binary_search(&key).is_ok();
        let token = if keyword {
            Some(Token::Keyword)
        } else if self.t.types.binary_search(&key).is_ok()
            || (self.p.capitalized_types && is_type_name(word))
        {
            Some(Token::Type)
        } else {
            None
        };
        if let Some(token) = token {
            self.push(start, token);
        }
        self.expr_start = token == Some(Token::Keyword) && JS_BEFORE_EXPR.contains(&word);
    }

    /// The previous non-space character ends an operand (`x.5` is not a number, `x .5` is)
    fn after_operand(&self) -> bool {
        self.c[..self.i]
            .iter()
            .rev()
            .find(|c| !c.is_whitespace())
            .is_some_and(|c| c.is_alphanumeric() || matches!(c, ')' | ']' | '_'))
    }

    fn block(&mut self, start: usize, depth: u8) -> CodeState {
        let (open, close) = self
            .p
            .block_comment
            .expect("only reached through block_comment");
        let mut depth = depth;
        while self.i < self.c.len() {
            if self.at(close) {
                self.i += close.chars().count();
                depth -= 1;
                if depth == 0 {
                    self.push(start, Token::Comment);
                    return CodeState::Normal;
                }
            } else if self.p.nested_comments && self.at(open) {
                self.i += open.chars().count();
                depth = depth.saturating_add(1);
            } else {
                self.i += 1;
            }
        }
        self.push(start, Token::Comment);
        CodeState::BlockComment(depth)
    }

    /// A string opened by `prefix` + quote, the prefix starting at `start`
    fn string_start(&mut self, start: usize, prefix: &str) -> CodeState {
        let quote = self.c[self.i];
        self.expr_start = false;
        if self.p.triple_quotes && self.peek(1) == Some(quote) && self.peek(2) == Some(quote) {
            self.i += 3;
            let f = self.p.family == Family::Python && prefix.contains(['f', 'F']);
            return self.triple(start, quote, f);
        }
        self.i += 1;
        if self.p.family == Family::Python && prefix.contains(['f', 'F']) {
            return self.fstring(start, quote);
        }
        self.quoted(start, quote, self.p.multiline_strings)
    }

    /// Rest of a quoted string up to `quote`, escapes skipped. At the end of the line a string
    /// carries on only where the language allows it; elsewhere an unclosed quote is a syntax
    /// error, and stopping there keeps it from swallowing later lines.
    fn quoted(&mut self, start: usize, quote: char, spans_lines: bool) -> CodeState {
        let escape = self
            .p
            .quotes
            .iter()
            .find(|q| q.ch == quote)
            .and_then(|q| q.escape);
        while self.i < self.c.len() {
            match self.c[self.i] {
                c if Some(c) == escape => self.i += 2,
                c if c == quote => {
                    self.i += 1;
                    self.push(start, Token::Str);
                    return CodeState::Normal;
                }
                _ => self.i += 1,
            }
        }
        self.push(start, Token::Str);
        if spans_lines {
            CodeState::Str(quote)
        } else {
            CodeState::Normal
        }
    }

    /// Python f-string body on one line: `{…}` is code (left plain), `{{` is a literal brace
    fn fstring(&mut self, start: usize, quote: char) -> CodeState {
        let mut start = start;
        while self.i < self.c.len() {
            match self.c[self.i] {
                '\\' => self.i += 2,
                '{' if self.peek(1) == Some('{') => self.i += 2,
                '{' => {
                    self.push(start, Token::Str);
                    self.skip_braces();
                    start = self.i;
                }
                c if c == quote => {
                    self.i += 1;
                    self.push(start, Token::Str);
                    return CodeState::Normal;
                }
                _ => self.i += 1,
            }
        }
        self.push(start, Token::Str);
        CodeState::Normal
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

    fn triple(&mut self, start: usize, quote: char, f: bool) -> CodeState {
        let mut start = start;
        while self.i < self.c.len() {
            if self.c[self.i] == '\\' {
                self.i += 2;
            } else if f && self.c[self.i] == '{' && self.peek(1) != Some('{') {
                self.push(start, Token::Str);
                self.skip_braces();
                start = self.i;
            } else if self.c[self.i] == quote
                && self.peek(1) == Some(quote)
                && self.peek(2) == Some(quote)
            {
                self.i += 3;
                self.push(start, Token::Str);
                return CodeState::Normal;
            } else {
                self.i += 1;
            }
        }
        self.push(start, Token::Str);
        CodeState::Triple { quote, f }
    }

    /// A string whose prefix is the identifier just read: Rust `b"…"`, `r#"…"#`, `b'x'`; C `L"…"`,
    /// C++ `R"x(…)x"`; Dart `r'…'`; Python `r'…'`, `f"…"`, `rb"""…"""`
    fn prefixed_string(&mut self, start: usize, word: &str) -> Option<CodeState> {
        let next = self.peek(0)?;
        match self.p.family {
            Family::C if self.p.raw_strings => {
                if matches!(word, "r" | "br" | "cr") && (next == '"' || next == '#') {
                    let mut hashes = 0;
                    while self.peek(hashes) == Some('#') {
                        hashes += 1;
                    }
                    if self.peek(hashes) != Some('"') || hashes > u8::MAX as usize {
                        return None;
                    }
                    self.i += hashes + 1;
                    return Some(self.raw(start, '"', hashes as u8));
                }
                if word == "r" && next == '\'' && !self.p.char_literals {
                    self.i += 1;
                    return Some(self.raw(start, '\'', 0));
                }
                if matches!(word, "R" | "LR" | "uR" | "UR" | "u8R") && next == '"' {
                    return self.cpp_raw_open(start);
                }
                let prefix = matches!(word, "b" | "c" | "L" | "u" | "U" | "u8");
                if prefix && next == '"' {
                    return Some(self.string_start(start, word));
                }
                if prefix && next == '\'' && self.p.char_literals {
                    self.char_or_lifetime(start);
                    return Some(CodeState::Normal);
                }
                None
            }
            Family::Python => {
                let lower = word.to_ascii_lowercase();
                let prefix = matches!(
                    lower.as_str(),
                    "r" | "b" | "u" | "f" | "rb" | "br" | "fr" | "rf"
                );
                (prefix && (next == '"' || next == '\'')).then(|| self.string_start(start, word))
            }
            _ => None,
        }
    }

    /// Raw strings that open with punctuation: C# `@"…"`, Swift `#"…"#`, Go `` `…` ``
    fn raw_open(&mut self, start: usize) -> Option<CodeState> {
        match self.c[self.i] {
            '@' if self.peek(1) == Some('"') => {
                self.i += 2;
                Some(self.raw(start, '"', 0))
            }
            '#' => {
                let mut hashes = 0;
                while self.peek(hashes) == Some('#') {
                    hashes += 1;
                }
                if self.peek(hashes) != Some('"') || hashes > u8::MAX as usize {
                    return None;
                }
                self.i += hashes + 1;
                Some(self.raw(start, '"', hashes as u8))
            }
            '`' => {
                self.i += 1;
                Some(self.raw(start, '`', 0))
            }
            _ => None,
        }
    }

    /// No escapes; closed by `quote` and `hashes` `#`s. Without hashes a doubled `""` is a quote
    /// inside (C# `@"say ""hi"""`).
    fn raw(&mut self, start: usize, quote: char, hashes: u8) -> CodeState {
        while self.i < self.c.len() {
            if self.c[self.i] == quote {
                if hashes == 0 && quote == '"' && self.peek(1) == Some('"') {
                    self.i += 2;
                    continue;
                }
                let closes = (1..=hashes as usize).all(|k| self.peek(k) == Some('#'));
                if closes {
                    self.i += 1 + hashes as usize;
                    self.push(start, Token::Str);
                    return CodeState::Normal;
                }
            }
            self.i += 1;
        }
        self.push(start, Token::Str);
        CodeState::RawStr { quote, hashes }
    }

    /// At the `"` of C++ `R"delim(`: read the delimiter, then the body
    fn cpp_raw_open(&mut self, start: usize) -> Option<CodeState> {
        let open = self.c[self.i + 1..].iter().position(|&c| c == '(')?;
        let delim: String = self.c[self.i + 1..self.i + 1 + open].iter().collect();
        if delim.len() > 8 || !delim.chars().all(|c| c.is_ascii_graphic() && c != '\\') {
            return None;
        }
        let mut bytes = [0; 8];
        bytes[..delim.len()].copy_from_slice(delim.as_bytes());
        self.i += open + 2;
        Some(self.cpp_raw(start, bytes, delim.len() as u8))
    }

    fn cpp_raw(&mut self, start: usize, delim: [u8; 8], len: u8) -> CodeState {
        let close: String = std::iter::once(')')
            .chain(delim[..len as usize].iter().map(|&b| b as char))
            .chain(std::iter::once('"'))
            .collect();
        while self.i < self.c.len() {
            if self.at(&close) {
                self.i += close.chars().count();
                self.push(start, Token::Str);
                return CodeState::Normal;
            }
            self.i += 1;
        }
        self.push(start, Token::Str);
        CodeState::CppRaw { delim, len }
    }

    /// `'` where it isn't a string: a char literal (`'x'`, `'\n'`, `'\u{1F600}'`, `b'x'`) or a
    /// lifetime / label (`'a`), whose name stays plain
    fn char_or_lifetime(&mut self, start: usize) {
        let q = self.c[start..].iter().position(|&c| c == '\'').unwrap() + start;
        self.i = q + 1;
        if self.peek(0) == Some('\\') {
            self.i += 2;
            while self.i < self.c.len() && self.c[self.i] != '\'' {
                self.i += 1;
            }
            self.i = (self.i + 1).min(self.c.len());
            self.push(start, Token::Str);
        } else if self.peek(1) == Some('\'') {
            self.i += 2;
            self.push(start, Token::Str);
        } else {
            while self
                .peek(0)
                .is_some_and(|c| c.is_alphanumeric() || c == '_')
            {
                self.i += 1;
            }
        }
        self.expr_start = false;
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
        // Suffix: `u32`, `f64`, `n` (BigInt), `j` (imaginary), `px` (CSS)
        while self
            .peek(0)
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || (c == '%' && self.p.css))
        {
            self.i += 1;
        }
    }

    fn template(&mut self, start: usize) -> CodeState {
        while self.i < self.c.len() {
            match self.c[self.i] {
                '\\' => self.i += 2,
                '`' => {
                    self.i += 1;
                    self.push(start, Token::Str);
                    return CodeState::Normal;
                }
                '$' if self.peek(1) == Some('{') => {
                    self.push(start, Token::Str);
                    self.i += 2;
                    return CodeState::TemplateExpr(1);
                }
                _ => self.i += 1,
            }
        }
        self.push(start, Token::Str);
        CodeState::Template
    }

    /// Code inside `${ … }`, lexed like any other code until the brace that closes it
    fn template_expr(&mut self, depth: u8) -> CodeState {
        let mut depth = depth;
        while self.i < self.c.len() {
            match self.c[self.i] {
                '{' => depth = depth.saturating_add(1),
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        self.i += 1;
                        return CodeState::Template;
                    }
                }
                '`' | '"' | '\'' | '/' => {
                    // A nested string or comment: lex it as normal code, then carry on here
                    let inner = self.normal();
                    if inner != CodeState::Normal {
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
        CodeState::TemplateExpr(depth)
    }

    /// CSS: `@media` (a keyword when the table knows it), `!important`, `#fff`
    fn css(&mut self, start: usize) -> bool {
        let word_end = |s: &Self, from: usize| {
            (from..s.c.len())
                .find(|&k| !s.is_ident(s.c[k]))
                .unwrap_or(s.c.len())
        };
        match self.c[self.i] {
            '@' => {
                let end = word_end(self, self.i + 1);
                let word: String = self.c[self.i + 1..end].iter().collect();
                if word.is_empty() {
                    return false;
                }
                self.i = end;
                if self.t.keywords.binary_search(&word.as_str()).is_ok() {
                    self.push(start, Token::Keyword);
                }
                true
            }
            '!' if self.c[self.i + 1..]
                .starts_with(&['i', 'm', 'p', 'o', 'r', 't', 'a', 'n', 't']) =>
            {
                self.i += 10;
                self.push(start, Token::Keyword);
                true
            }
            '#' => {
                let end = word_end(self, self.i + 1);
                let hex = self.c[self.i + 1..end].iter().all(char::is_ascii_hexdigit);
                if !hex || ![3, 4, 6, 8].contains(&(end - self.i - 1)) {
                    return false;
                }
                self.i = end;
                self.push(start, Token::Number);
                true
            }
            _ => false,
        }
    }

    /// At `[` with `level` `=` signs and another `[` from `at`, the level
    fn long_open(&self, at: usize) -> Option<u8> {
        if self.c.get(at) != Some(&'[') {
            return None;
        }
        let level = self.c[at + 1..].iter().take_while(|&&c| c == '=').count();
        (self.c.get(at + 1 + level) == Some(&'[') && level <= u8::MAX as usize)
            .then_some(level as u8)
    }

    fn long_bracket(&mut self, start: usize, level: u8, comment: bool) -> CodeState {
        let close: String = std::iter::once(']')
            .chain((0..level).map(|_| '='))
            .chain(std::iter::once(']'))
            .collect();
        let token = if comment { Token::Comment } else { Token::Str };
        while self.i < self.c.len() {
            if self.at(&close) {
                self.i += close.len();
                self.push(start, token);
                return CodeState::Normal;
            }
            self.i += 1;
        }
        self.push(start, token);
        CodeState::LongBracket { level, comment }
    }

    /// Markup text: comments, CDATA, tags and entities; everything else is plain
    fn markup(&mut self) -> CodeState {
        let start = self.i;
        if self.at("<!--") {
            self.i += 4;
            return self.block(start, 1);
        }
        if self.at("<![CDATA[") {
            self.i += 9;
            return self.cdata(start);
        }
        let ch = self.c[self.i];
        if ch == '<' {
            let mut j = self.i + 1;
            if matches!(self.c.get(j), Some('/' | '!' | '?')) {
                j += 1;
            }
            if self.c.get(j).is_some_and(|c| c.is_alphabetic()) {
                while self
                    .c
                    .get(j)
                    .is_some_and(|&c| c.is_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'))
                {
                    j += 1;
                }
                self.i = j;
                self.push(start, Token::Keyword);
                return self.tag();
            }
        }
        if ch == '&' {
            let name = self.c[self.i + 1..]
                .iter()
                .take_while(|c| c.is_alphanumeric() || **c == '#')
                .count();
            if name > 0 && self.c.get(self.i + 1 + name) == Some(&';') {
                self.i += name + 2;
                self.push(start, Token::Number);
                return CodeState::Normal;
            }
        }
        self.i += 1;
        CodeState::Normal
    }

    /// Inside a tag: quoted attribute values, up to the `>` that closes it
    fn tag(&mut self) -> CodeState {
        while self.i < self.c.len() {
            let at = self.i;
            match self.c[self.i] {
                q @ ('"' | '\'') => {
                    self.i += 1;
                    while self.peek(0).is_some_and(|c| c != q) {
                        self.i += 1;
                    }
                    self.i += 1;
                    self.push(at, Token::Str);
                }
                '>' => {
                    self.i += 1;
                    self.push(at, Token::Keyword);
                    return CodeState::Normal;
                }
                '/' | '?' if self.peek(1) == Some('>') => {
                    self.i += 2;
                    self.push(at, Token::Keyword);
                    return CodeState::Normal;
                }
                _ => self.i += 1,
            }
        }
        CodeState::Tag
    }

    fn cdata(&mut self, start: usize) -> CodeState {
        while self.i < self.c.len() {
            if self.at("]]>") {
                self.i += 3;
                self.push(start, Token::Str);
                return CodeState::Normal;
            }
            self.i += 1;
        }
        self.push(start, Token::Str);
        CodeState::Cdata
    }

    /// Diff and Markdown: the whole line by how it starts, plus Markdown's code spans
    fn prose(&mut self) {
        let line: String = self.c.iter().collect();
        let whole = match self.lang {
            Lang::Diff => {
                let header = ["+++ ", "--- ", "diff ", "index "]
                    .iter()
                    .any(|p| line.starts_with(p));
                if header {
                    Some(Token::Comment)
                } else if line.starts_with("@@") {
                    Some(Token::Type)
                } else if line.starts_with('+') {
                    Some(Token::Str)
                } else if line.starts_with('-') {
                    // Until there's a token for deleted lines (docs/syntax-highlighting.md §7.3)
                    Some(Token::Keyword)
                } else {
                    None
                }
            }
            _ => {
                let indent = line.len() - line.trim_start_matches(' ').len();
                let rest = &line[indent..];
                let hashes = rest.len() - rest.trim_start_matches('#').len();
                let heading = (1..=6).contains(&hashes)
                    && rest[hashes..].chars().next().is_none_or(|c| c == ' ');
                if indent > 3 {
                    None
                } else if heading {
                    Some(Token::Keyword)
                } else if rest.starts_with('>') {
                    Some(Token::Comment)
                } else {
                    None
                }
            }
        };
        if let Some(token) = whole {
            self.i = self.c.len();
            self.push(0, token);
        } else if self.lang == Lang::Markdown {
            self.code_spans();
        }
    }

    /// Markdown `` `code` ``: a run of backticks closed by a run of the same length on this line
    fn code_spans(&mut self) {
        while self.i < self.c.len() {
            if self.c[self.i] != '`' {
                self.i += 1;
                continue;
            }
            let start = self.i;
            let run = self.c[start..].iter().take_while(|&&c| c == '`').count();
            let mut j = start + run;
            let close = loop {
                let Some(k) = self.c[j..].iter().position(|&c| c == '`') else {
                    break None;
                };
                let at = j + k;
                let len = self.c[at..].iter().take_while(|&&c| c == '`').count();
                if len == run {
                    break Some(at + len);
                }
                j = at + len;
            };
            match close {
                Some(end) => {
                    self.i = end;
                    self.push(start, Token::Str);
                }
                None => self.i = start + run,
            }
        }
    }
}

/// `Foo`, `HashMap` and `T`, but not `MAX_LEN` (a constant)
fn is_type_name(word: &str) -> bool {
    let first_upper = word.chars().next().is_some_and(char::is_uppercase);
    let shouting = word.len() > 1
        && word
            .chars()
            .all(|c| c.is_uppercase() || c.is_ascii_digit() || c == '_');
    first_upper && !shouting
}
