//! The code lexer (src/code): the confusable cases from the examples/hl_probe experiment, basic cases
//! for every language, the alias table and the generated word tables.
//!
//! Each case lists the ranges it should produce, in order, as (text, token). Cases the lexer is
//! known to get wrong are marked `limit` with the reason; the test fails if a case that should pass
//! fails, or if a known limit starts passing (so the table stays honest). The pass rate is printed:
//! `cargo test --test code_scan confusable -- --nocapture`.

use mustty::code::aliases::{ALIASES, from_info, lookup};
use mustty::code::{CodeState, Family, Lang, LangId, TABLES_SOURCE, Token, scan};
use proptest::prelude::*;

const K: Token = Token::Keyword;
const T: Token = Token::Type;
const S: Token = Token::Str;
const C: Token = Token::Comment;
const N: Token = Token::Number;
const ADD: Token = Token::DiffAdd;
const DEL: Token = Token::DiffDel;

type Want = &'static [(&'static str, Token)];

/// Every line of `src`, each starting in the previous line's state, as (text, token)
fn ranges(lang: Lang, src: &str) -> Vec<(String, Token)> {
    let mut state = CodeState::Normal;
    let mut out = Vec::new();
    for line in src.split('\n') {
        let (ranges, next) = scan(lang.into(), line, state);
        let chars: Vec<char> = line.chars().collect();
        out.extend(
            ranges
                .iter()
                .map(|r| (chars[r.start..r.end].iter().collect(), r.token)),
        );
        state = next;
    }
    out
}

fn owned(want: Want) -> Vec<(String, Token)> {
    want.iter().map(|(s, t)| (s.to_string(), *t)).collect()
}

/// Check a language's cases, reporting every failure at once
fn check(lang: Lang, cases: &[(&str, Want)]) {
    let wrong: Vec<String> = cases
        .iter()
        .filter_map(|(src, want)| {
            let got = ranges(lang, src);
            (got != owned(want)).then(|| format!("{src:?}\n  want {want:?}\n  got  {got:?}"))
        })
        .collect();
    assert!(wrong.is_empty(), "{lang:?}:\n{}", wrong.join("\n"));
}

struct Case {
    id: &'static str,
    lang: Lang,
    src: &'static str,
    want: Want,
    /// Why the lexer can't get this right, for cases it is known to fail
    limit: Option<&'static str>,
}

const fn case(id: &'static str, lang: Lang, src: &'static str, want: Want) -> Case {
    Case {
        id,
        lang,
        src,
        want,
        limit: None,
    }
}

const fn limit(
    id: &'static str,
    lang: Lang,
    src: &'static str,
    want: Want,
    why: &'static str,
) -> Case {
    Case {
        id,
        lang,
        src,
        want,
        limit: Some(why),
    }
}

use Lang::{JavaScript as Js, Python as Py, Rust as Rs};

/// The experiment's accuracy table, unchanged: numbered cases were the brief's required ones
const CASES: &[Case] = &[
    case(
        "1 escaped quote, quote in comment",
        Rs,
        r#"let s = "a \" b"; // tail"#,
        &[("let", K), (r#""a \" b""#, S), ("// tail", C)],
    ),
    case(
        "2 char literal vs lifetime",
        Rs,
        "let c = 'x'; fn f<'a>(x: &'a str)",
        &[("let", K), ("'x'", S), ("fn", K), ("str", T)],
    ),
    case(
        "3 raw string",
        Rs,
        r###"let r = r#"raw "quoted" text"#;"###,
        &[("let", K), (r###"r#"raw "quoted" text"#"###, S)],
    ),
    case(
        "4 nested block comment",
        Rs,
        "/* outer /* inner */ still outer */ x",
        &[("/* outer /* inner */ still outer */", C)],
    ),
    case(
        "5 apostrophe in comment",
        Rs,
        r#"// it's a comment with "quotes""#,
        &[(r#"// it's a comment with "quotes""#, C)],
    ),
    case(
        "6 number shapes",
        Rs,
        "let n = 0xFF_FFu32; let e = 1e-9; let p = 3.14;",
        &[
            ("let", K),
            ("0xFF_FFu32", N),
            ("let", K),
            ("1e-9", N),
            ("let", K),
            ("3.14", N),
        ],
    ),
    case(
        "7 regex vs division",
        Js,
        "const re = /ab+c/gi; const x = a / b / c;",
        &[("const", K), ("/ab+c/gi", S), ("const", K)],
    ),
    case(
        "8 template interpolation",
        Js,
        "const t = `hi ${name} and ${obj.x}`;",
        &[("const", K), ("`hi ", S), (" and ", S), ("`", S)],
    ),
    case(
        "9 escaped single quote",
        Js,
        r"const s = 'it\'s';",
        &[("const", K), (r"'it\'s'", S)],
    ),
    case(
        "10 apostrophe in JS comment",
        Js,
        "// don't split this",
        &[("// don't split this", C)],
    ),
    case(
        "11 triple quotes",
        Py,
        r#"s = """triple "quoted" here""""#,
        &[(r#""""triple "quoted" here""""#, S)],
    ),
    case(
        "12 f-string",
        Py,
        r#"f"{x} and {y}""#,
        &[(r#"f""#, S), (" and ", S), (r#"""#, S)],
    ),
    case(
        "13 quote in Python comment",
        Py,
        "# comment with 'quote'",
        &[("# comment with 'quote'", C)],
    ),
    case(
        "14 escaped quote in Python",
        Py,
        r"s = 'it\'s'",
        &[(r"'it\'s'", S)],
    ),
    case(
        "15a triple quote spans lines",
        Py,
        "s = \"\"\"one\ntwo\n\"\"\" + x",
        &[("\"\"\"one", S), ("two", S), ("\"\"\"", S)],
    ),
    case(
        "15b plain quote stops at line end",
        Py,
        "s = 'open\nx = 1",
        &[("'open", S), ("1", N)],
    ),
    case(
        "15c JS quote stops at line end",
        Js,
        "s = 'open\nlet x",
        &[("'open", S), ("let", K)],
    ),
    case(
        "15d Rust string spans lines",
        Rs,
        "let s = \"one\ntwo\"; let",
        &[("let", K), ("\"one", S), ("two\"", S), ("let", K)],
    ),
    case(
        "15e JS template spans lines",
        Js,
        "t = `one\ntwo ${x}`; let",
        &[("`one", S), ("two ", S), ("`", S), ("let", K)],
    ),
    // Variants of the same ambiguities
    case(
        "R char escape",
        Rs,
        r"let q = '\''; let n = '\n';",
        &[("let", K), (r"'\''", S), ("let", K), (r"'\n'", S)],
    ),
    case(
        "R byte and unicode char",
        Rs,
        r"b'a' '\u{1F600}' '中'",
        &[("b'a'", S), (r"'\u{1F600}'", S), ("'中'", S)],
    ),
    case(
        "R lifetimes in generics",
        Rs,
        "impl<'a> Foo<'a> for &'static str",
        &[("impl", K), ("Foo", T), ("for", K), ("str", T)],
    ),
    case(
        "R loop label",
        Rs,
        "'outer: loop { break 'outer; }",
        &[("loop", K), ("break", K)],
    ),
    case(
        "R raw string with #s",
        Rs,
        r####"r##"a "# b"##"####,
        &[(r####"r##"a "# b"##"####, S)],
    ),
    case(
        "R raw string spans lines",
        Rs,
        "r#\"one\ntwo\"# x",
        &[("r#\"one", S), ("two\"#", S)],
    ),
    case(
        "R byte string",
        Rs,
        r#"b"bytes" c"cstr""#,
        &[(r#"b"bytes""#, S), (r#"c"cstr""#, S)],
    ),
    case(
        "R range, tuple field, method",
        Rs,
        "0..10 t.0 1.max(2) 2.5e10f64",
        &[
            ("0", N),
            ("10", N),
            ("0", N),
            ("1", N),
            ("2", N),
            ("2.5e10f64", N),
        ],
    ),
    case(
        "R block comment spans lines",
        Rs,
        "/* a\n/* b */\nc */ fn",
        &[("/* a", C), ("/* b */", C), ("c */", C), ("fn", K)],
    ),
    case(
        "R doc comment",
        Rs,
        r#"/// a "doc" it's"#,
        &[(r#"/// a "doc" it's"#, C)],
    ),
    case(
        "R type vs constant",
        Rs,
        "const MAX: usize = Vec::<T>::new()",
        &[("const", K), ("usize", T), ("Vec", T), ("T", T)],
    ),
    limit(
        "R attribute",
        Rs,
        "#[derive(Debug)]",
        &[("#[derive(Debug)]", K)],
        "attributes need their own token; Debug shows as a type",
    ),
    limit(
        "R macro call",
        Rs,
        "println!(\"x\")",
        &[("println!", K), ("\"x\"", S)],
        "macros are plain identifiers; a sixth token or `!` lookahead",
    ),
    case(
        "J regex after return",
        Js,
        r"return /\d+/.test(s)",
        &[("return", K), (r"/\d+/", S)],
    ),
    case("J regex in call", Js, r"s.split(/,\s*/)", &[(r"/,\s*/", S)]),
    case(
        "J regex with / in class",
        Js,
        "x = /[/]+/g",
        &[("/[/]+/g", S)],
    ),
    case(
        "J division chain",
        Js,
        "x = (a + 1) / 2 / b",
        &[("1", N), ("2", N)],
    ),
    limit(
        "J regex after )",
        Js,
        "if (ok) /x/.test(s)",
        &[("if", K), ("/x/", S)],
        "after `)` only a parser knows whether it closed a condition or an expression",
    ),
    limit(
        "J regex after }",
        Js,
        "} /x/.test(s)",
        &[("/x/", S)],
        "after `}` only a parser knows whether it closed a block or an object literal",
    ),
    case("J division at line start", Js, "x = a\n  / b / c", &[]),
    case(
        "J BigInt, hex, leading dot",
        Js,
        "[1_000n, 0x1F, .5]",
        &[("1_000n", N), ("0x1F", N), (".5", N)],
    ),
    case(
        "J block comment, code after",
        Js,
        "/* a */ b /* c */",
        &[("/* a */", C), ("/* c */", C)],
    ),
    case(
        "J URL in comment",
        Js,
        "// http://x.com 'q",
        &[("// http://x.com 'q", C)],
    ),
    case(
        "J string inside interpolation",
        Js,
        "`a ${f('}')} b`",
        &[("`a ", S), ("'}'", S), (" b`", S)],
    ),
    limit(
        "J nested template",
        Js,
        "`a ${`b ${c}`} d`",
        &[("`a ", S), ("`b ", S), ("`", S), (" d`", S)],
        "a template inside `${}` needs a stack of states",
    ),
    case(
        "J keyword as property",
        Js,
        "obj.class = new Foo()",
        &[("new", K), ("Foo", T)],
    ),
    case(
        "R .await stays a keyword",
        Rs,
        "x.await.match_y",
        &[("await", K)],
    ),
    case(
        "P string prefixes",
        Py,
        r#"r'\d+' b"x" rb'y' u'z'"#,
        &[(r"r'\d+'", S), ("b\"x\"", S), ("rb'y'", S), ("u'z'", S)],
    ),
    case(
        "P f-string escaped braces",
        Py,
        r#"f"{{literal}} {x}""#,
        &[(r#"f"{{literal}} "#, S), ("\"", S)],
    ),
    case(
        "P hash inside string",
        Py,
        r##"print("#not") # yes"##,
        &[(r##""#not""##, S), ("# yes", C)],
    ),
    case(
        "P numbers",
        Py,
        "[1_000.5e-3, 1j, 0o17, .5]",
        &[("1_000.5e-3", N), ("1j", N), ("0o17", N), (".5", N)],
    ),
    case(
        "P def, class, keywords",
        Py,
        "class Foo(Bar):\n    def f(self): return None",
        &[
            ("class", K),
            ("Foo", T),
            ("Bar", T),
            ("def", K),
            ("return", K),
            ("None", K),
        ],
    ),
    case(
        "P builtin types",
        Py,
        "x: int = str(y)",
        &[("int", T), ("str", T)],
    ),
    case(
        "P f triple spans lines",
        Py,
        "f\"\"\"a {x}\nb {y}\"\"\"",
        &[("f\"\"\"a ", S), ("b ", S), ("\"\"\"", S)],
    ),
    limit(
        "P decorator",
        Py,
        "@property",
        &[("@property", K)],
        "decorators need their own token",
    ),
];

#[test]
fn confusable_cases() {
    let (mut pass, mut wrong) = (0, Vec::new());
    for c in CASES {
        let got = ranges(c.lang, c.src);
        let want = owned(c.want);
        let ok = got == want;
        pass += ok as usize;
        println!("{} {:?} {}", if ok { "PASS" } else { "FAIL" }, c.lang, c.id);
        if !ok {
            println!("     want {want:?}\n     got  {got:?}");
        }
        match (ok, c.limit) {
            (false, None) => wrong.push(format!("{} should pass", c.id)),
            (true, Some(_)) => wrong.push(format!("{} passes now: drop its `limit`", c.id)),
            (false, Some(why)) => println!("     known limit: {why}"),
            _ => {}
        }
    }
    println!(
        "\nall cases: {pass}/{} pass ({:.0}%)",
        CASES.len(),
        100.0 * pass as f64 / CASES.len() as f64,
    );
    assert_eq!(CASES.len(), 54);
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

// Basic cases, one test per language: keywords, types, strings, comments, numbers, and whatever
// that language's profile adds

#[test]
fn rust() {
    check(
        Lang::Rust,
        &[
            (
                "fn main() { let x: u32 = 1; } // done",
                &[("fn", K), ("let", K), ("u32", T), ("1", N), ("// done", C)],
            ),
            ("let gen = Some(x);", &[("let", K), ("gen", K), ("Some", T)]),
            ("let raw = union;", &[("let", K)]),
        ],
    );
}

#[test]
fn javascript() {
    check(
        Lang::JavaScript,
        &[
            (
                "const x = 'a' + 1; // c",
                &[("const", K), ("'a'", S), ("1", N), ("// c", C)],
            ),
            ("console.log(this, $el)", &[("this", K)]),
        ],
    );
}

#[test]
fn typescript() {
    check(
        Lang::TypeScript,
        &[
            (
                "let n: number = 0x1F as const;",
                &[
                    ("let", K),
                    ("number", T),
                    ("0x1F", N),
                    ("as", K),
                    ("const", K),
                ],
            ),
            (
                "type Id = string | undefined",
                &[("type", K), ("Id", T), ("string", T), ("undefined", K)],
            ),
            ("const re = /x/g", &[("const", K), ("/x/g", S)]),
        ],
    );
}

#[test]
fn python() {
    check(
        Lang::Python,
        &[
            (
                "def f(x: int) -> None: return \"s\"  # c",
                &[
                    ("def", K),
                    ("int", T),
                    ("None", K),
                    ("return", K),
                    ("\"s\"", S),
                    ("# c", C),
                ],
            ),
            ("lazy import json", &[("lazy", K), ("import", K)]),
        ],
    );
}

#[test]
fn go() {
    check(
        Lang::Go,
        &[
            (
                "var x int64 = 0o17 // c",
                &[("var", K), ("int64", T), ("0o17", N), ("// c", C)],
            ),
            (
                r"func main() { s := `raw\` }",
                &[("func", K), (r"`raw\`", S)],
            ),
            ("x := `a\nb` + nil", &[("`a", S), ("b`", S), ("nil", K)]),
            ("r := 'x'", &[("'x'", S)]),
        ],
    );
}

#[test]
fn bash() {
    check(
        Lang::Bash,
        &[
            (
                "for f in *.md; do echo \"$f\"; done # loop",
                &[
                    ("for", K),
                    ("in", K),
                    ("do", K),
                    ("\"$f\"", S),
                    ("done", K),
                    ("# loop", C),
                ],
            ),
            ("echo ${#arr[@]} $# x#y", &[]),
            ("git log --until=now --for", &[]),
            ("echo 'a\\' x=1 # c", &[("'a\\'", S), ("1", N), ("# c", C)]),
        ],
    );
}

#[test]
fn powershell() {
    check(
        Lang::PowerShell,
        &[
            (
                "<# block #> IF ($x -eq 1) { Write-Host \"a`\"b\" } # c",
                &[
                    ("<# block #>", C),
                    ("IF", K),
                    ("1", N),
                    ("\"a`\"b\"", S),
                    ("# c", C),
                ],
            ),
            (
                "<# one\ntwo #> foreach",
                &[("<# one", C), ("two #>", C), ("foreach", K)],
            ),
            ("'it''s'", &[("'it'", S), ("'s'", S)]),
        ],
    );
}

#[test]
fn json() {
    check(
        Lang::Json,
        &[
            (
                r#"{"a": [1, -2.5e3, true, null]} // c"#,
                &[
                    ("\"a\"", S),
                    ("1", N),
                    ("2.5e3", N),
                    ("true", K),
                    ("null", K),
                    ("// c", C),
                ],
            ),
            ("/* jsonc */ {}", &[("/* jsonc */", C)]),
        ],
    );
}

#[test]
fn yaml() {
    check(
        Lang::Yaml,
        &[
            ("title: Don't panic # note", &[("# note", C)]),
            ("enabled: True", &[("True", K)]),
            ("port: 8080", &[("8080", N)]),
            ("url: http://x/#frag\nv: \"1.0\"", &[("\"1.0\"", S)]),
            ("- 'single' # c", &[("'single'", S), ("# c", C)]),
        ],
    );
}

#[test]
fn toml() {
    check(
        Lang::Toml,
        &[
            ("[package]", &[("[package]", T)]),
            ("[[bin]] # c", &[("[[bin]]", T), ("# c", C)]),
            ("name = \"mustty\" # c", &[("\"mustty\"", S), ("# c", C)]),
            ("debug = true", &[("true", K)]),
            ("s = '''a\nb'''", &[("'''a", S), ("b'''", S)]),
            ("; ini comment", &[("; ini comment", C)]),
        ],
    );
}

#[test]
fn sql() {
    check(
        Lang::Sql,
        &[
            (
                "SELECT name FROM users WHERE id = 1 -- c",
                &[
                    ("SELECT", K),
                    ("FROM", K),
                    ("WHERE", K),
                    ("1", N),
                    ("-- c", C),
                ],
            ),
            ("select \"select\" from t", &[("select", K), ("from", K)]),
            ("'it''s'", &[("'it'", S), ("'s'", S)]),
            (
                "name VARCHAR(10) /* c */",
                &[("VARCHAR", T), ("10", N), ("/* c */", C)],
            ),
        ],
    );
}

#[test]
fn html() {
    check(
        Lang::Html,
        &[
            (
                "<div class=\"a\">it's &amp; <!-- c --></div>",
                &[
                    ("<div", K),
                    ("\"a\"", S),
                    (">", K),
                    ("&amp;", N),
                    ("<!-- c -->", C),
                    ("</div", K),
                    (">", K),
                ],
            ),
            ("<a\n  href='x'>a < b", &[("<a", K), ("'x'", S), (">", K)]),
            ("<!DOCTYPE html>", &[("<!DOCTYPE", K), (">", K)]),
        ],
    );
}

#[test]
fn css() {
    check(
        Lang::Css,
        &[
            (
                "@media (max-width: 600px) { .a { color: #fff !important; } } /* c */",
                &[
                    ("@media", K),
                    ("600px", N),
                    ("#fff", N),
                    ("!important", K),
                    ("/* c */", C),
                ],
            ),
            (
                "a { background: url(//cdn/x.png); width: 50%; content: \"x\" }",
                &[("50%", N), ("\"x\"", S)],
            ),
            ("#header { margin: -1px }", &[("1px", N)]),
        ],
    );
}

#[test]
fn c() {
    check(
        Lang::C,
        &[
            ("#include <stdio.h>", &[("#include", K)]),
            ("  # define NAME \"y\"", &[("# define", K), ("\"y\"", S)]),
            (
                "int main(void) { char c = 'x'; return 0; } // c",
                &[
                    ("int", T),
                    ("void", T),
                    ("char", T),
                    ("'x'", S),
                    ("return", K),
                    ("0", N),
                    ("// c", C),
                ],
            ),
            (
                r#"static const char *s = L"a\"b";"#,
                &[("static", K), ("const", K), ("char", T), (r#"L"a\"b""#, S)],
            ),
            ("/* a /* b */ x */", &[("/* a /* b */", C)]),
        ],
    );
}

#[test]
fn cpp() {
    check(
        Lang::Cpp,
        &[
            (
                r#"auto s = R"(raw "str")";"#,
                &[("auto", K), (r#"R"(raw "str")""#, S)],
            ),
            (
                "R\"x(one\n)\" two)x\" nullptr",
                &[("R\"x(one", S), (")\" two)x\"", S), ("nullptr", K)],
            ),
            (
                "template <typename T> class Foo {};",
                &[
                    ("template", K),
                    ("typename", K),
                    ("T", T),
                    ("class", K),
                    ("Foo", T),
                ],
            ),
        ],
    );
}

#[test]
fn java() {
    check(
        Lang::Java,
        &[
            (
                "public final class Foo extends Bar { int x = 0x1F; }",
                &[
                    ("public", K),
                    ("final", K),
                    ("class", K),
                    ("Foo", T),
                    ("extends", K),
                    ("Bar", T),
                    ("int", T),
                    ("0x1F", N),
                ],
            ),
            (
                "String s = \"\"\"\n  hi\n  \"\"\";",
                &[("String", T), ("\"\"\"", S), ("  hi", S), ("  \"\"\"", S)],
            ),
            ("this.x = null;", &[("this", K), ("null", K)]),
        ],
    );
}

#[test]
fn csharp() {
    check(
        Lang::CSharp,
        &[
            (
                r#"var s = @"C:\path\""x""";"#,
                &[("var", K), (r#"@"C:\path\""x""""#, S)],
            ),
            ("#region Foo", &[("#region", K), ("Foo", T)]),
            (
                "string? name = null; // c",
                &[("string", T), ("null", K), ("// c", C)],
            ),
        ],
    );
}

#[test]
fn php() {
    check(
        Lang::Php,
        &[
            ("$x = 'a'; # c", &[("'a'", S), ("# c", C)]),
            (
                "function foo(): ?int { return null; }",
                &[("function", K), ("int", K), ("return", K), ("null", K)],
            ),
        ],
    );
}

#[test]
fn kotlin() {
    check(
        Lang::Kotlin,
        &[
            (
                "fun main() { val s = \"\"\"raw $x\"\"\" } // c",
                &[
                    ("fun", K),
                    ("val", K),
                    ("\"\"\"raw $x\"\"\"", S),
                    ("// c", C),
                ],
            ),
            (
                "/* a /* b */ c */ Int",
                &[("/* a /* b */ c */", C), ("Int", T)],
            ),
        ],
    );
}

#[test]
fn lua() {
    check(
        Lang::Lua,
        &[
            (
                "local t = { [[long\nstring]] } -- c",
                &[("local", K), ("[[long", S), ("string]]", S), ("-- c", C)],
            ),
            (
                "--[==[ block\n]] still ]==] x",
                &[("--[==[ block", C), ("]] still ]==]", C)],
            ),
            (
                "if a ~= nil then print(\"a\") end",
                &[("if", K), ("nil", K), ("then", K), ("\"a\"", S), ("end", K)],
            ),
            ("t[a[1]] = 'x'", &[("1", N), ("'x'", S)]),
        ],
    );
}

#[test]
fn ruby() {
    check(
        Lang::Ruby,
        &[
            (
                "def foo(x) = puts \"#{x}\" # c",
                &[("def", K), ("\"#{x}\"", S), ("# c", C)],
            ),
            (
                "class Foo < Bar; end",
                &[("class", K), ("Foo", T), ("Bar", T), ("end", K)],
            ),
            (r"'it\'s' nil", &[(r"'it\'s'", S), ("nil", K)]),
        ],
    );
}

#[test]
fn dart() {
    check(
        Lang::Dart,
        &[
            (
                r"final s = r'raw\'; int x = 1;",
                &[("final", K), (r"r'raw\'", S), ("int", T), ("1", N)],
            ),
            (
                "String name = '$x'; // c",
                &[("String", T), ("'$x'", S), ("// c", C)],
            ),
        ],
    );
}

#[test]
fn swift() {
    check(
        Lang::Swift,
        &[
            (
                r##"let s = #"raw "q""#; var n: Int = 1"##,
                &[
                    ("let", K),
                    (r##"#"raw "q""#"##, S),
                    ("var", K),
                    ("Int", T),
                    ("1", N),
                ],
            ),
            ("#if DEBUG", &[("#if", K)]),
            ("/* a /* b */ c */", &[("/* a /* b */ c */", C)]),
        ],
    );
}

#[test]
fn markdown() {
    check(
        Lang::Markdown,
        &[
            ("# Title", &[("# Title", K)]),
            ("#hashtag", &[]),
            ("> quote", &[("> quote", C)]),
            (
                "Use `code` and ``a`b`` here, not `this",
                &[("`code`", S), ("``a`b``", S)],
            ),
        ],
    );
}

#[test]
fn diff() {
    check(
        Lang::Diff,
        &[(
            "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new\n ctx",
            &[
                ("--- a/x", DEL),
                ("+++ b/x", ADD),
                ("-old", DEL),
                ("+new", ADD),
            ],
        )],
    );
}

#[test]
fn xml() {
    check(
        Lang::Xml,
        &[
            (
                "<?xml version=\"1.0\"?>",
                &[("<?xml", K), ("\"1.0\"", S), ("?>", K)],
            ),
            (
                "<![CDATA[ x < y\n]]> z",
                &[("<![CDATA[ x < y", S), ("]]>", S)],
            ),
            ("<a:b x='1'/>", &[("<a:b", K), ("'1'", S), ("/>", K)]),
        ],
    );
}

#[test]
fn scss() {
    check(
        Lang::Scss,
        &[
            ("$color: #333; // c", &[("#333", N), ("// c", C)]),
            ("@include mixin; @unknown x", &[("@include", K)]),
        ],
    );
}

#[test]
fn dockerfile() {
    check(
        Lang::Dockerfile,
        &[
            ("FROM rust:1.88 AS build", &[("FROM", K), ("1.88", N)]),
            (
                "RUN echo \"hi\" # c",
                &[("RUN", K), ("\"hi\"", S), ("# c", C)],
            ),
        ],
    );
}

#[test]
fn makefile() {
    check(
        Lang::Makefile,
        &[
            ("ifeq ($(OS),Windows_NT) # c", &[("ifeq", K), ("# c", C)]),
            ("-include deps.mk", &[("-include", K)]),
        ],
    );
}

#[test]
fn protobuf() {
    check(
        Lang::Protobuf,
        &[(
            "message Foo { repeated string name = 1; } // c",
            &[
                ("message", K),
                ("Foo", T),
                ("repeated", K),
                ("string", T),
                ("1", N),
                ("// c", C),
            ],
        )],
    );
}

#[test]
fn every_language_has_cases_or_is_listed_here() {
    // Keeps this file in step with Lang::ALL: a new language needs its own test above
    assert_eq!(Lang::ALL.len(), 30);
    assert_eq!(Lang::ALL.iter().filter(|l| l.tier() == 1).count(), 17);
}

#[test]
fn plain_and_unknown_stay_one_color() {
    for info in ["text", "TXT", "plaintext", "console", "shellsession"] {
        let id = from_info(info).unwrap();
        assert_eq!(id, LangId::PLAIN, "{info}");
        assert_eq!(id.lang(), None);
        let (ranges, state) = scan(id, "let x = \"s\"; // c", CodeState::Normal);
        assert!(ranges.is_empty(), "{info}");
        assert_eq!(state, CodeState::Normal);
    }
    for info in ["", "   ", "haskell", "rusty", "{r}", "zig", "{}"] {
        assert_eq!(from_info(info), None, "{info:?}");
    }
}

#[test]
fn info_string_forms() {
    for (info, lang) in [
        ("RUST", Lang::Rust),
        ("Rust", Lang::Rust),
        ("rust,no_run", Lang::Rust),
        ("js title=\"x.js\"", Lang::JavaScript),
        ("{.python}", Lang::Python),
        ("{python}", Lang::Python),
        ("  ts {1,3}", Lang::TypeScript),
        ("C#", Lang::CSharp),
        ("c++", Lang::Cpp),
    ] {
        assert_eq!(from_info(info), Some(lang.into()), "{info:?}");
    }
}

/// The alias table, entry by entry, against highlight.js's lists as docs/syntax-highlighting.md
/// §7.2 records them
#[test]
fn every_alias() {
    use Lang::*;
    let expected: &[(&str, Option<Lang>)] = &[
        ("rust", Some(Rust)),
        ("rs", Some(Rust)),
        ("javascript", Some(JavaScript)),
        ("js", Some(JavaScript)),
        ("jsx", Some(JavaScript)),
        ("mjs", Some(JavaScript)),
        ("cjs", Some(JavaScript)),
        ("typescript", Some(TypeScript)),
        ("ts", Some(TypeScript)),
        ("tsx", Some(TypeScript)),
        ("mts", Some(TypeScript)),
        ("cts", Some(TypeScript)),
        ("python", Some(Python)),
        ("py", Some(Python)),
        ("gyp", Some(Python)),
        ("ipython", Some(Python)),
        ("python3", Some(Python)),
        ("go", Some(Go)),
        ("golang", Some(Go)),
        ("bash", Some(Bash)),
        ("sh", Some(Bash)),
        ("zsh", Some(Bash)),
        ("shell", Some(Bash)),
        ("powershell", Some(PowerShell)),
        ("pwsh", Some(PowerShell)),
        ("ps", Some(PowerShell)),
        ("ps1", Some(PowerShell)),
        ("json", Some(Json)),
        ("jsonc", Some(Json)),
        ("json5", Some(Json)),
        ("yaml", Some(Yaml)),
        ("yml", Some(Yaml)),
        ("toml", Some(Toml)),
        ("ini", Some(Toml)),
        ("sql", Some(Sql)),
        ("css", Some(Css)),
        ("html", Some(Html)),
        ("xhtml", Some(Html)),
        ("c", Some(C)),
        ("h", Some(C)),
        ("cpp", Some(Cpp)),
        ("cc", Some(Cpp)),
        ("c++", Some(Cpp)),
        ("h++", Some(Cpp)),
        ("hpp", Some(Cpp)),
        ("hh", Some(Cpp)),
        ("hxx", Some(Cpp)),
        ("cxx", Some(Cpp)),
        ("java", Some(Java)),
        ("jsp", Some(Java)),
        ("csharp", Some(CSharp)),
        ("cs", Some(CSharp)),
        ("c#", Some(CSharp)),
        ("php", Some(Php)),
        ("swift", Some(Swift)),
        ("dart", Some(Dart)),
        ("scss", Some(Scss)),
        ("kotlin", Some(Kotlin)),
        ("kt", Some(Kotlin)),
        ("kts", Some(Kotlin)),
        ("ktm", Some(Kotlin)),
        ("ktx", Some(Kotlin)),
        ("lua", Some(Lua)),
        ("pluto", Some(Lua)),
        ("ruby", Some(Ruby)),
        ("rb", Some(Ruby)),
        ("gemspec", Some(Ruby)),
        ("podspec", Some(Ruby)),
        ("thor", Some(Ruby)),
        ("irb", Some(Ruby)),
        ("xml", Some(Xml)),
        ("rss", Some(Xml)),
        ("atom", Some(Xml)),
        ("xjb", Some(Xml)),
        ("xsd", Some(Xml)),
        ("xsl", Some(Xml)),
        ("plist", Some(Xml)),
        ("wsf", Some(Xml)),
        ("svg", Some(Xml)),
        ("markdown", Some(Markdown)),
        ("md", Some(Markdown)),
        ("mkdown", Some(Markdown)),
        ("mkd", Some(Markdown)),
        ("diff", Some(Diff)),
        ("patch", Some(Diff)),
        ("protobuf", Some(Protobuf)),
        ("proto", Some(Protobuf)),
        ("dockerfile", Some(Dockerfile)),
        ("docker", Some(Dockerfile)),
        ("makefile", Some(Makefile)),
        ("mk", Some(Makefile)),
        ("mak", Some(Makefile)),
        ("make", Some(Makefile)),
        ("text", None),
        ("txt", None),
        ("plaintext", None),
        ("console", None),
        ("shellsession", None),
    ];
    assert_eq!(expected.len(), 98);
    assert_eq!(ALIASES.len(), 98);
    for &(alias, lang) in expected {
        let want = lang.map_or(LangId::PLAIN, LangId::from);
        assert_eq!(lookup(alias), Some(want), "{alias}");
        assert_eq!(from_info(&alias.to_uppercase()), Some(want), "{alias}");
    }
    // Sorted and unique, which the binary search relies on
    assert!(ALIASES.windows(2).all(|w| w[0].0 < w[1].0));
    // Every language is reachable by its own name or an alias
    for lang in Lang::ALL {
        assert!(ALIASES.iter().any(|&(_, id)| id == lang.into()), "{lang:?}");
    }
}

#[test]
fn word_tables_are_sorted_and_filled() {
    for lang in Lang::ALL {
        let t = lang.table();
        for list in [t.keywords, t.types] {
            assert!(list.windows(2).all(|w| w[0] < w[1]), "{lang:?} not sorted");
            if t.case_insensitive {
                assert!(list.iter().all(|w| *w == w.to_lowercase()), "{lang:?}");
            }
        }
        // Markup and prose are colored by structure, not by words
        let wordless = matches!(lang.profile().family, Family::Markup | Family::Prose);
        assert_eq!(t.keywords.is_empty(), wordless, "{lang:?}");
    }
}

/// Only the languages' own words: library names that highlight.js lists as builtins stay out
#[test]
fn word_tables_hold_no_library_names() {
    for (lang, names) in [
        (Lang::C, &["printf", "malloc", "complex"][..]),
        (Lang::Rust, &["String", "Vec", "Option", "println"]),
        (Lang::Python, &["print", "len", "Ellipsis", "List"]),
        (Lang::JavaScript, &["console", "document", "window", "Math"]),
        (Lang::Sql, &["current_date", "count"]),
        (Lang::Dart, &["print", "window", "DateTime"]),
        (Lang::Ruby, &["require", "raise"]),
        (Lang::Kotlin, &["Int", "Unit"]),
    ] {
        let t = lang.table();
        for name in names {
            let found = t.keywords.contains(name) || t.types.contains(name);
            assert!(!found, "{lang:?} lists {name}");
        }
    }
}

#[test]
fn tables_match_their_hashes() {
    let source = TABLES_SOURCE.replace("\r\n", "\n");
    let (header, body) = source.split_once("\n\n").unwrap();
    let field = |name: &str| {
        header
            .lines()
            .find_map(|l| l.split_once(name).map(|(_, v)| v.trim().to_string()))
            .unwrap_or_else(|| panic!("no {name} in the tables.rs header"))
    };
    assert!(header.starts_with("// @generated"));
    assert!(header.contains("highlight.js 11.12.0"));
    assert_eq!(
        sha256_hex(body.as_bytes()),
        field("(everything after the blank line below):"),
        "src/code/tables.rs was edited by hand: change overrides.toml and run gen.mjs"
    );
    let overrides = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tools/lexer-tables/overrides.toml"
    ))
    .unwrap()
    .replace("\r\n", "\n");
    assert_eq!(
        sha256_hex(overrides.as_bytes()),
        field("overrides.toml sha256:"),
        "overrides.toml changed: run tools/lexer-tables/gen.mjs"
    );
}

#[test]
fn code_state_is_small_copy_and_comparable() {
    fn copy_eq<T: Copy + Eq + Default>() {}
    copy_eq::<CodeState>();
    assert!(std::mem::size_of::<CodeState>() <= 12);
}

proptest! {
    /// Any line in any state: no panic, and ranges are in order, non-empty, non-overlapping and
    /// inside the line
    #[test]
    fn ranges_are_well_formed(
        lang in 0..Lang::ALL.len(),
        lines in prop::collection::vec("[ -~é中\t]{0,40}", 1..6),
    ) {
        let lang = Lang::ALL[lang];
        let mut state = CodeState::Normal;
        for line in &lines {
            let (ranges, next) = scan(lang.into(), line, state);
            let n = line.chars().count();
            let mut end = 0;
            for r in &ranges {
                prop_assert!(r.start >= end && r.start < r.end && r.end <= n, "{lang:?} {line:?} {ranges:?}");
                end = r.end;
            }
            // The same input gives the same output: scan is pure
            prop_assert_eq!(scan(lang.into(), line, state), (ranges, next));
            state = next;
        }
    }
}

/// SHA-256 (FIPS 180-4), enough to check the hashes gen.mjs writes without a dependency
fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in msg.chunks(64) {
        let mut w = [0u32; 64];
        for (i, word) in block.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes(word.try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7]
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [
                t1.wrapping_add(t2),
                v[0],
                v[1],
                v[2],
                v[3].wrapping_add(t1),
                v[4],
                v[5],
                v[6],
            ];
        }
        for (a, b) in h.iter_mut().zip(v) {
            *a = a.wrapping_add(b);
        }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}
