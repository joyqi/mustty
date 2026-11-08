//! Accuracy table for the experimental code lexer in examples/hl_probe (not wired into mustty).
//! Each case lists the spans it should produce, in order, as (text, token). The cases the lexer is
//! known to get wrong are marked `limit` with the reason; the test fails if a case that should pass
//! fails, or if a known limit starts passing (so the table stays honest). The pass rate is printed:
//! `cargo test --test hl_probe -- --nocapture`.

#[path = "../examples/hl_probe/lexer.rs"]
#[allow(dead_code)]
mod lexer;

use lexer::{Lang, Tok, scan_block};

const K: Tok = Tok::Keyword;
const T: Tok = Tok::Type;
const S: Tok = Tok::Str;
const C: Tok = Tok::Comment;
const N: Tok = Tok::Number;

struct Case {
    id: &'static str,
    lang: Lang,
    src: &'static str,
    want: &'static [(&'static str, Tok)],
    /// Why the lexer can't get this right, for cases it is known to fail
    limit: Option<&'static str>,
}

const fn case(
    id: &'static str,
    lang: Lang,
    src: &'static str,
    want: &'static [(&'static str, Tok)],
) -> Case {
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
    want: &'static [(&'static str, Tok)],
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

use Lang::{Js, Python as Py, Rust as Rs};

const CASES: &[Case] = &[
    // Required cases, numbered as in the brief
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

fn spans(lang: Lang, src: &str) -> Vec<(String, Tok)> {
    let lines: Vec<&str> = src.lines().collect();
    scan_block(lang, src)
        .into_iter()
        .enumerate()
        .flat_map(|(l, spans)| {
            let chars: Vec<char> = lines[l].chars().collect();
            spans
                .into_iter()
                .map(move |(start, len, tok)| {
                    (chars[start..start + len].iter().collect::<String>(), tok)
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn confusable_cases() {
    let (mut pass, mut wrong) = (0, Vec::new());
    for c in CASES {
        let got = spans(c.lang, c.src);
        let want: Vec<(String, Tok)> = c.want.iter().map(|(s, t)| (s.to_string(), *t)).collect();
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
    let required = CASES
        .iter()
        .filter(|c| c.id.starts_with(|ch: char| ch.is_ascii_digit()))
        .collect::<Vec<_>>();
    let required_pass = required
        .iter()
        .filter(|c| {
            spans(c.lang, c.src)
                == c.want
                    .iter()
                    .map(|(s, t)| (s.to_string(), *t))
                    .collect::<Vec<_>>()
        })
        .count();
    println!(
        "\nall cases: {pass}/{} pass ({:.0}%); required cases: {required_pass}/{}",
        CASES.len(),
        100.0 * pass as f64 / CASES.len() as f64,
        required.len()
    );
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn unknown_languages_stay_one_color() {
    for info in ["haskell", "text", "", "  ", "rusty", "sh"] {
        assert_eq!(Lang::from_info(info), None, "{info:?}");
    }
    for (info, lang) in [
        ("rust", Lang::Rust),
        ("RS ignore", Lang::Rust),
        ("javascript", Lang::Js),
        ("py", Lang::Python),
    ] {
        assert_eq!(Lang::from_info(info), Some(lang), "{info:?}");
    }
}
