//! Probe for the code lexer (src/code), which began here as an experiment.
//!
//! cargo run --example hl_probe -- side <lang> <file> [first-line] [last-line]
//!     each line on the left, its tokens on the right; <lang> is any fence alias (rust, js, py, …)
//! cargo run --example hl_probe -- sample <lang> <file> <count> <seed>
//!     random non-blank positions with the token they got, for checking by hand
//! cargo run --example hl_probe -- render <out-dir>
//!     a page with three code blocks drawn into a TestBackend in two color schemes: printed as ANSI
//!     (cat it in a terminal) and written as <out-dir>/hl-{dark,light}.svg

use std::fmt::Write as _;

use mustty::code::aliases::from_info;
use mustty::code::{CodeState, LangId, Token as Tok, scan};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::{Buffer, Cell};
use ratatui::style::{Color, Modifier, Style};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| args.get(i).map(String::as_str).unwrap_or_default();
    let lang = || from_info(arg(1)).expect("a known language alias");
    let read = || std::fs::read_to_string(arg(2)).expect("readable file");
    match arg(0) {
        "side" => {
            let first = arg(3).parse().unwrap_or(1);
            let last = arg(4).parse().unwrap_or(usize::MAX);
            side_by_side(lang(), &read(), first, last);
        }
        "sample" => sample(
            lang(),
            &read(),
            arg(3).parse().unwrap_or(7),
            arg(4).parse().unwrap_or(1),
        ),
        "render" => render(std::path::Path::new(arg(1))),
        _ => eprintln!("usage: side | sample | render (see the top of examples/hl_probe/main.rs)"),
    }
}

/// Every line of a block, each starting in the previous line's state, as (start, len, token)
fn scan_block(lang: LangId, text: &str) -> Vec<Vec<(usize, usize, Tok)>> {
    let mut state = CodeState::Normal;
    text.lines()
        .map(|line| {
            let (ranges, next) = scan(lang, line, state);
            state = next;
            ranges
                .iter()
                .map(|r| (r.start, r.end - r.start, r.token))
                .collect()
        })
        .collect()
}

fn short(tok: Tok) -> &'static str {
    match tok {
        Tok::Keyword => "K",
        Tok::Type => "T",
        Tok::Str => "S",
        Tok::Comment => "C",
        Tok::Number => "N",
    }
}

fn side_by_side(lang: LangId, text: &str, first: usize, last: usize) {
    let lines: Vec<&str> = text.lines().collect();
    for (n, spans) in scan_block(lang, text).into_iter().enumerate() {
        if n + 1 < first || n + 1 > last {
            continue;
        }
        let chars: Vec<char> = lines[n].chars().collect();
        let left: String = chars.iter().take(56).collect();
        let marks: Vec<String> = spans
            .iter()
            .map(|&(s, l, t)| {
                let text: String = chars[s..s + l].iter().take(18).collect();
                format!("{}:{text}", short(t))
            })
            .collect();
        println!("{:>4} {left:<56} │ {}", n + 1, marks.join("  "));
    }
}

/// Deterministic positions (xorshift), so a report can be reproduced
fn sample(lang: LangId, text: &str, count: usize, seed: u64) {
    let lines: Vec<&str> = text.lines().collect();
    let spans = scan_block(lang, text);
    let mut spots = Vec::new();
    for (n, line) in lines.iter().enumerate() {
        for (i, c) in line.chars().enumerate() {
            if !c.is_whitespace() {
                spots.push((n, i));
            }
        }
    }
    let mut x = seed.max(1);
    for k in 0..count {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let (n, i) = spots[(x % spots.len() as u64) as usize];
        let tok = spans[n]
            .iter()
            .find(|&&(s, l, _)| (s..s + l).contains(&i))
            .map(|&(_, _, t)| t);
        let chars: Vec<char> = lines[n].chars().collect();
        let from = i.saturating_sub(30);
        let ctx: String = chars[from..chars.len().min(i + 30)].iter().collect();
        println!(
            "#{:<2} line {:>4} col {:>3} '{}' -> {:<7} │ {}",
            k + 1,
            n + 1,
            i + 1,
            chars[i],
            tok.map_or("plain", short),
            ctx.trim_end()
        );
        println!("{:>48}{}^", "", " ".repeat(i - from));
    }
}

const PAGE: &str = "\
## Parsing the fence

The scanner hands each code line to the lexer with the state the
previous line ended in, so a cache can stop early.

```rust
/// Scans one line; 'a is a lifetime, 'x' a char.
fn scan<'a>(line: &'a str, state: State) -> Vec<Span> {
    let raw = r#\"say \"hi\"\"#; // a raw string
    let n = 0xFF_u32 + 3.14 as u32;
    if line.is_empty() { return Vec::new(); }
}
```

```js
const re = /ab+c/gi, half = total / 2;
const msg = `hi ${user.name}, ${n} new`; // template
export default function load(path) { return fetch(path); }
```

```python
def greet(name: str) -> None:
    \"\"\"Say hello.\"\"\"  # docstring
    print(f\"hi {name}\", 42, None)
```";

const COLS: u16 = 72;

/// How the five tokens look. `plain` is code that is none of them.
struct Scheme {
    title: &'static str,
    plain: Style,
    tok: fn(Tok) -> Style,
}

const SCHEMES: [Scheme; 2] = [
    Scheme {
        title: "A  five colors",
        plain: Style::new(),
        tok: |t| match t {
            Tok::Keyword => Style::new().fg(Color::Magenta),
            Tok::Type => Style::new().fg(Color::Yellow),
            Tok::Str => Style::new().fg(Color::Green),
            Tok::Comment => Style::new().add_modifier(Modifier::DIM),
            Tok::Number => Style::new().fg(Color::Blue),
        },
    },
    Scheme {
        title: "B  quiet: code_color + bold keywords, green strings, dim comments",
        plain: Style::new().fg(Color::Cyan),
        tok: |t| match t {
            Tok::Keyword => Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            Tok::Str => Style::new().fg(Color::Green),
            Tok::Comment => Style::new().add_modifier(Modifier::DIM),
            Tok::Type | Tok::Number => Style::new().fg(Color::Cyan),
        },
    },
];

/// The page as mustty would lay it out (Markdown text as typed, fences dimmed), one scheme per half
fn draw_page(buf: &mut Buffer, x0: u16, scheme: &Scheme) {
    let dim = Style::new().add_modifier(Modifier::DIM);
    buf.set_string(
        x0,
        0,
        scheme.title,
        Style::new().add_modifier(Modifier::REVERSED),
    );
    let mut lang = None;
    let mut code = Vec::new();
    let mut y = 2;
    let flush = |buf: &mut Buffer, lang: LangId, code: &mut Vec<&str>, y: &mut u16| {
        let text = code.join("\n");
        for (line, spans) in code.iter().zip(scan_block(lang, &text)) {
            let chars: Vec<char> = line.chars().collect();
            buf.set_string(x0, *y, line, scheme.plain);
            for (s, l, t) in spans {
                let piece: String = chars[s..s + l].iter().collect();
                buf.set_string(x0 + s as u16, *y, piece, (scheme.tok)(t));
            }
            *y += 1;
        }
        code.clear();
    };
    for line in PAGE.lines() {
        if let Some(info) = line.strip_prefix("```") {
            if let Some(l) = lang.take() {
                flush(buf, l, &mut code, &mut y);
            } else {
                lang = from_info(info);
            }
            buf.set_string(x0, y, line, dim);
        } else if lang.is_some() {
            code.push(line);
            continue;
        } else if let Some(h) = line.strip_prefix("## ") {
            buf.set_string(x0, y, "## ", dim);
            buf.set_string(x0 + 3, y, h, Style::new().add_modifier(Modifier::BOLD));
        } else {
            buf.set_string(x0, y, line, Style::new());
        }
        y += 1;
    }
}

fn render(out_dir: &std::path::Path) {
    let rows = PAGE.lines().count() as u16 + 2;
    let width = COLS * 2 + 4;
    let mut terminal = Terminal::new(TestBackend::new(width, rows)).unwrap();
    terminal
        .draw(|f| {
            draw_page(f.buffer_mut(), 0, &SCHEMES[0]);
            draw_page(f.buffer_mut(), COLS + 4, &SCHEMES[1]);
        })
        .unwrap();
    let buf = terminal.backend().buffer();
    print!("{}", to_ansi(buf));
    std::fs::create_dir_all(out_dir).unwrap();
    for theme in &THEMES {
        let path = out_dir.join(format!("hl-{}.svg", theme.name));
        std::fs::write(&path, to_svg(buf, theme)).unwrap();
        eprintln!("{}", path.display());
    }
}

/// Rows of the buffer as text with SGR codes, for `cat` in a real terminal
fn to_ansi(buf: &Buffer) -> String {
    let mut out = String::new();
    for y in 0..buf.area.height {
        let mut last = None;
        for x in 0..buf.area.width {
            let cell = &buf[(x, y)];
            let style = (cell.fg, cell.modifier);
            if last != Some(style) {
                out.push_str("\x1b[0");
                if let Some(code) = ansi_fg(cell.fg) {
                    write!(out, ";{code}").unwrap();
                }
                for (m, code) in [
                    (Modifier::BOLD, 1),
                    (Modifier::DIM, 2),
                    (Modifier::REVERSED, 7),
                ] {
                    if cell.modifier.contains(m) {
                        write!(out, ";{code}").unwrap();
                    }
                }
                out.push('m');
                last = Some(style);
            }
            out.push_str(cell.symbol());
        }
        out.push_str("\x1b[0m\n");
    }
    out
}

fn ansi_fg(c: Color) -> Option<u8> {
    Some(match c {
        Color::Green => 32,
        Color::Yellow => 33,
        Color::Blue => 34,
        Color::Magenta => 35,
        Color::Cyan => 36,
        _ => return None,
    })
}

struct Theme {
    name: &'static str,
    bg: &'static str,
    fg: &'static str,
    /// green, yellow, blue, magenta, cyan: xterm.js's GitHub palettes from web/src/theme.js
    colors: [&'static str; 5],
}

const THEMES: [Theme; 2] = [
    Theme {
        name: "dark",
        bg: "#0d1117",
        fg: "#e6edf3",
        colors: ["#3fb950", "#d29922", "#58a6ff", "#bc8cff", "#39c5cf"],
    },
    Theme {
        name: "light",
        bg: "#ffffff",
        fg: "#1f2328",
        colors: ["#1a7f37", "#9a6700", "#0969da", "#8250df", "#1b7c83"],
    },
];

fn svg_color(c: Color, theme: &Theme) -> &'static str {
    match c {
        Color::Green => theme.colors[0],
        Color::Yellow => theme.colors[1],
        Color::Blue => theme.colors[2],
        Color::Magenta => theme.colors[3],
        Color::Cyan => theme.colors[4],
        _ => theme.fg,
    }
}

/// One `<text>` per cell run of the same style, every glyph at its own column like examples/screenshot.rs
fn to_svg(buf: &Buffer, theme: &Theme) -> String {
    const CW: f64 = 8.4;
    const LH: f64 = 18.0;
    let (w, h) = (buf.area.width as f64 * CW, buf.area.height as f64 * LH);
    let mut out = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" width="{w}" height="{h}" font-family="Menlo, monospace" font-size="14"><rect width="{w}" height="{h}" fill="{}"/>"#,
        theme.bg
    );
    for y in 0..buf.area.height {
        let mut x = 0;
        while x < buf.area.width {
            let first: &Cell = &buf[(x, y)];
            let mut end = x + 1;
            while end < buf.area.width
                && (buf[(end, y)].fg, buf[(end, y)].modifier) == (first.fg, first.modifier)
            {
                end += 1;
            }
            let (mut fg, m) = (svg_color(first.fg, theme), first.modifier);
            let top = y as f64 * LH;
            if m.contains(Modifier::REVERSED) {
                write!(
                    out,
                    r#"<rect x="{}" y="{top}" width="{}" height="{LH}" fill="{fg}"/>"#,
                    x as f64 * CW,
                    (end - x) as f64 * CW
                )
                .unwrap();
                fg = theme.bg;
            }
            let mut attrs = format!(r#"fill="{fg}""#);
            if m.contains(Modifier::BOLD) {
                attrs.push_str(r#" font-weight="700""#);
            }
            if m.contains(Modifier::DIM) {
                attrs.push_str(r#" fill-opacity="0.55""#);
            }
            for cx in x..end {
                let s = buf[(cx, y)].symbol();
                if s != " " && !s.is_empty() {
                    let s = s
                        .replace('&', "&amp;")
                        .replace('<', "&lt;")
                        .replace('>', "&gt;");
                    write!(
                        out,
                        r#"<text x="{}" y="{}" {attrs}>{s}</text>"#,
                        cx as f64 * CW,
                        top + 13.0
                    )
                    .unwrap();
                }
            }
            x = end;
        }
    }
    out.push_str("</svg>\n");
    out
}
