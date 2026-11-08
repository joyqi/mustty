//! Typing latency on a large document: each keystroke's `App::handle` plus a full frame drawn into
//! a `TestBackend`, the work the event loop does per key. The document has 10 000 lines with 20
//! fenced code blocks in five languages; typing happens in prose, in a code block, and in a code
//! block where the first key opens a string that runs on to the block's end.
//!
//! cargo run --release --example typing_latency

use std::time::{Duration, Instant};

use mustty::app::App;
use mustty::config::Config;
use mustty::editor::document::{Document, FileMeta, LineEnding};
use mustty::input::{Event, KeyCode, KeyEvent};
use mustty::view;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Size;

const BLOCKS: [(&str, &str); 5] = [
    (
        "rust",
        "fn parse<'a>(s: &'a str) -> Option<u32> {\n    let n = 0x1F + 3; // a comment\n    Some(r#\"raw\"#.len() as u32)\n}",
    ),
    (
        "js",
        "const re = /ab+c/gi, half = total / 2;\nconst msg = `hi ${user.name}`; // template\nexport function load(p) { return null; }",
    ),
    (
        "python",
        "def greet(name: str) -> None:\n    \"\"\"Say hello.\"\"\"  # doc\n    print(f\"hi {name}\", 42)",
    ),
    (
        "go",
        "func main() {\n    s := `raw`\n    var n int64 = 0o17 // c\n}",
    ),
    ("diff", "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new"),
];

/// 10 000 body lines: 20 sections of prose, each followed by a ~30-line code block
fn document() -> (String, usize, usize) {
    let mut lines = Vec::new();
    let mut code_line = 0;
    for section in 0..20 {
        while lines.len() < (section + 1) * 500 - 32 {
            let n = lines.len();
            lines.push(format!(
                "Line {n} of prose with *emphasis*, `code` and a [link](https://x.io)."
            ));
        }
        let (lang, code) = BLOCKS[section % BLOCKS.len()];
        lines.push(format!("```{lang}"));
        for _ in 0..7 {
            lines.extend(code.lines().map(String::from));
        }
        if section == 10 {
            code_line = lines.len() - 13;
        }
        lines.push("```".into());
    }
    lines.truncate(10_000);
    (lines.join("\n"), 5_100, code_line)
}

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::from(code))
}

/// Per-key time to handle `keys` and draw, with the cursor first moved to `line`
fn measure(text: &str, line: usize, keys: &[char]) -> Vec<Duration> {
    let meta = FileMeta {
        path: None,
        line_ending: LineEnding::Lf,
        has_bom: false,
        stamp: None,
    };
    let doc = Document::parse(&format!("# Bench\n{text}"), meta);
    let (w, h) = (100, 40);
    let config = Config {
        autosave: false,
        ..Config::default()
    };
    let mut app = App::with_config(doc, Size::new(w, h), &config);
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    // From the title, the first Down reaches body line 0
    for _ in 0..=line {
        app.handle(key(KeyCode::Down));
    }
    app.handle(key(KeyCode::End));
    terminal.draw(|f| view::render(f, &mut app)).unwrap();
    let times = keys
        .iter()
        .map(|&c| {
            let start = Instant::now();
            app.handle(key(KeyCode::Char(c)));
            terminal.draw(|f| view::render(f, &mut app)).unwrap();
            start.elapsed()
        })
        .collect();
    // Show where the typing landed: the first screen row holding typed text, and the row above
    let buf = terminal.backend().buffer();
    let rows: Vec<String> = (0..h)
        .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect())
        .collect();
    if let Some(y) = rows.iter().position(|r| r.contains("the quick")) {
        for row in &rows[y.saturating_sub(1)..=y] {
            eprintln!("    {}", row.trim());
        }
    }
    times
}

fn report(name: &str, mut times: Vec<Duration>) {
    times.sort();
    let at = |q: f64| times[((times.len() - 1) as f64 * q) as usize].as_secs_f64() * 1e3;
    println!(
        "{name:<28} p50 {:.3} ms  p95 {:.3} ms  max {:.3} ms  ({} keys)",
        at(0.5),
        at(0.95),
        at(1.0),
        times.len()
    );
}

fn main() {
    let (text, prose, code) = document();
    let typed: Vec<char> = "the quick brown fox jumps over the lazy dog "
        .chars()
        .cycle()
        .take(400)
        .collect();
    report("prose", measure(&text, prose, &typed));
    report("inside a code block", measure(&text, code, &typed));
    let mut quote = vec!['"'];
    quote.extend(&typed);
    report("code block, open string", measure(&text, code, &quote));
}
