//! README screenshots: render one real frame into a `TestBackend` and convert the buffer to SVG.
//!
//! `cargo run --example screenshot` rewrites `assets/screenshot-{dark,light}.svg`.
//! Every glyph gets its own x coordinate, so columns line up whatever font the viewer has.

use std::fmt::Write as _;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use mustty::app::App;
use mustty::editor::document::{DiskStamp, Document, FileMeta};
use mustty::view;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::Size;
use ratatui::style::{Color, Modifier};

const COLS: u16 = 100;
const ROWS: u16 = 26;
const CHAR_W: f64 = 8.4;
const LINE_H: f64 = 18.0;
const FONT_SIZE: f64 = 14.0;
/// Baseline offset inside an 18 px row for a 14 px font.
const BASELINE: f64 = 13.0;

const DOC: &str = "\
# Field Notes

The rain stopped a little after four. I left the window open and wrote down \
whatever came to mind: the smell of wet stone, the neighbour's radio, a sentence \
I had been chasing all week.

## What **mustty** does

It keeps one column of text in the middle of the screen and gets out of the \
way. Markdown stays exactly as you typed it, *emphasis* and `inline code` \
included; the markers are only dimmed. Links such as \
[the repository](https://github.com/joyqi/mustty) read as prose too.

- Autosave after two quiet seconds, with an atomic write every time.
";

struct Theme {
    name: &'static str,
    bg: &'static str,
    fg: &'static str,
    green: &'static str,
    cyan: &'static str,
}

const THEMES: [Theme; 2] = [
    Theme {
        name: "dark",
        bg: "#0D1117",
        fg: "#E6EDF3",
        green: "#3FB950",
        cyan: "#39C5CF",
    },
    Theme {
        name: "light",
        bg: "#FFFFFF",
        fg: "#1F2328",
        green: "#2DA44E",
        cyan: "#39C5CF",
    },
];

/// The frame shown in the README: English UI, a file that matches the disk, cursor at the end.
fn render_frame() -> Buffer {
    mustty::i18n::init_for_test("en");
    let meta = FileMeta {
        stamp: Some(DiskStamp {
            mtime: std::time::SystemTime::UNIX_EPOCH,
            len: DOC.len() as u64,
        }),
        ..FileMeta::new(Some("notes.md".into()))
    };
    let size = Size::new(COLS, ROWS);
    let mut app = App::new(Document::parse(DOC, meta), size, false);
    // Cursor at the end of the last line
    let ctrl_end = KeyEvent::new(KeyCode::End, KeyModifiers::CONTROL);
    app.handle(Event::Key(ctrl_end));
    app.handle(Event::Key(KeyEvent::from(KeyCode::Left)));
    let mut terminal = Terminal::new(TestBackend::new(COLS, ROWS)).unwrap();
    terminal.draw(|f| view::render(f, &mut app)).unwrap();
    terminal.backend().buffer().clone()
}

fn color(c: Color, theme: &Theme, default: &'static str) -> &'static str {
    match c {
        Color::Green => theme.green,
        Color::Cyan => theme.cyan,
        _ => default,
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Cells with the same style and no space between them become one `<text>`.
fn same_style(a: &Cell, b: &Cell) -> bool {
    (a.fg, a.bg, a.modifier) == (b.fg, b.bg, b.modifier)
}

fn to_svg(buf: &Buffer, theme: &Theme) -> String {
    let (w, h) = (COLS as f64 * CHAR_W, ROWS as f64 * LINE_H);
    let mut out = String::new();
    writeln!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" width="{w}" height="{h}" font-family="ui-monospace, SFMono-Regular, Menlo, Consolas, monospace" font-size="{FONT_SIZE}">"#
    )
    .unwrap();
    writeln!(
        out,
        r#"<rect width="{w}" height="{h}" fill="{}"/>"#,
        theme.bg
    )
    .unwrap();
    for y in 0..ROWS {
        let cells: Vec<&Cell> = (0..COLS).map(|x| &buf[(x, y)]).collect();
        let mut x = 0;
        while x < cells.len() {
            let first = cells[x];
            let blank = first.symbol() == " " && !first.modifier.contains(Modifier::REVERSED);
            if blank {
                x += 1;
                continue;
            }
            let mut end = x + 1;
            while end < cells.len()
                && same_style(first, cells[end])
                && (cells[end].symbol() != " " || first.modifier.contains(Modifier::REVERSED))
            {
                end += 1;
            }
            write_run(&mut out, theme, &cells[x..end], x, y);
            x = end;
        }
    }
    out.push_str("</svg>\n");
    out
}

fn write_run(out: &mut String, theme: &Theme, run: &[&Cell], x0: usize, y: u16) {
    let cell = run[0];
    let m = cell.modifier;
    let mut fg = color(cell.fg, theme, theme.fg);
    let ty = y as f64 * LINE_H;
    if m.contains(Modifier::REVERSED) {
        // Swap: the foreground becomes a background block, the text takes the page color
        let bx = x0 as f64 * CHAR_W;
        let bw = run.len() as f64 * CHAR_W;
        writeln!(
            out,
            r#"<rect x="{bx:.1}" y="{ty}" width="{bw:.1}" height="{LINE_H}" fill="{fg}"/>"#
        )
        .unwrap();
        fg = color(cell.bg, theme, theme.bg);
    }
    let mut xs = Vec::new();
    let mut text = String::new();
    for (i, c) in run.iter().enumerate() {
        let s = c.symbol();
        if s.is_empty() || s == " " {
            continue;
        }
        xs.push(format!("{:.1}", (x0 + i) as f64 * CHAR_W));
        text.push_str(s);
    }
    if text.is_empty() {
        return;
    }
    let mut attrs = format!(r#"fill="{fg}""#);
    if m.contains(Modifier::BOLD) {
        attrs.push_str(r#" font-weight="600""#);
    }
    if m.contains(Modifier::ITALIC) {
        attrs.push_str(r#" font-style="italic""#);
    }
    if m.contains(Modifier::UNDERLINED) {
        attrs.push_str(r#" text-decoration="underline""#);
    }
    if m.contains(Modifier::DIM) {
        attrs.push_str(r#" fill-opacity="0.55""#);
    }
    writeln!(
        out,
        r#"<text x="{}" y="{}" {attrs}>{}</text>"#,
        xs.join(" "),
        ty + BASELINE,
        escape(&text)
    )
    .unwrap();
}

/// Text content of the SVG (glyphs only), for the sanity checks.
fn glyphs(svg: &str) -> String {
    svg.split("<text ")
        .skip(1)
        .filter_map(|t| t.split_once('>').and_then(|(_, r)| r.split_once("</text>")))
        .map(|(s, _)| s)
        .collect()
}

fn check(svg: &str) {
    let text = glyphs(svg);
    for needle in [
        "Field", "Notes", "notes.md", "●", "words", "mustty", "`inline", "[the",
    ] {
        assert!(text.contains(needle), "screenshot is missing {needle:?}");
    }
    let n = text.chars().count();
    assert!(n > 400, "screenshot has only {n} glyphs");
}

fn main() {
    let buf = render_frame();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
    for theme in &THEMES {
        let svg = to_svg(&buf, theme);
        check(&svg);
        let path = dir.join(format!("screenshot-{}.svg", theme.name));
        std::fs::write(&path, &svg).unwrap();
        println!("{} ({} bytes)", path.display(), svg.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshot_is_not_blank() {
        let buf = render_frame();
        for theme in &THEMES {
            check(&to_svg(&buf, theme));
        }
    }
}
