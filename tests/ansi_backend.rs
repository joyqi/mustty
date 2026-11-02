//! `AnsiBackend` against crossterm's backend: the same frame diffs go through both, each byte stream
//! goes into its own vt100 screen, and the screens must match cell by cell (symbol, width, colors,
//! bold / dim / italic / underline / reverse) along with the cursor's position and visibility.
//! The bytes themselves may differ.
//!
//! Not on Windows: there crossterm can bypass the writer and drive the console API directly.

#![cfg(not(windows))]

use std::cell::RefCell;
use std::io;
use std::num::NonZeroU16;
use std::rc::Rc;

use mustty::ansi::AnsiBackend;
use mustty::app::App;
use mustty::editor::document::{Document, FileMeta};
use mustty::input::{Event, KeyCode, KeyEvent, KeyModifiers};
use mustty::view;
use proptest::prelude::*;
use ratatui::Terminal;
use ratatui::backend::{Backend, CrosstermBackend, TestBackend};
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect, Size};
use ratatui::style::{Color, Modifier, Style};

/// One frame as `Terminal` would hand it to a backend: the cells that changed, then the cursor.
fn emit<B: Backend>(backend: &mut B, prev: &Buffer, cur: &Buffer, cursor: Option<Position>) {
    backend.draw(prev.diff(cur).into_iter()).ok().unwrap();
    match cursor {
        Some(p) => {
            backend.show_cursor().ok().unwrap();
            backend.set_cursor_position(p).ok().unwrap();
        }
        None => backend.hide_cursor().ok().unwrap(),
    }
    backend.flush().ok().unwrap();
}

/// What crossterm's backend writes, readable while the backend still owns the writer
#[derive(Clone, Default)]
struct Shared(Rc<RefCell<Vec<u8>>>);

impl io::Write for Shared {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn screen(bytes: &[u8], size: Size) -> vt100::Parser {
    let nonzero = |n| NonZeroU16::new(n).unwrap();
    let mut parser = vt100::Parser::new(nonzero(size.height), nonzero(size.width), 0);
    parser.process(bytes);
    parser
}

/// Runs the frames through both backends and compares the resulting screens.
fn assert_same_screen(frames: &[(Buffer, Option<Position>)]) {
    let area = frames[0].0.area;
    let size = Size::new(area.width, area.height);
    let mut ours = AnsiBackend::new(size);
    let written = Shared::default();
    let mut theirs = CrosstermBackend::new(written.clone());
    let mut prev = Buffer::empty(area);
    // AnsiBackend hands its bytes out once; keep the whole stream to replay it
    let mut ours_stream = Vec::new();
    for (n, (cur, cursor)) in frames.iter().enumerate() {
        emit(&mut ours, &prev, cur, *cursor);
        emit(&mut theirs, &prev, cur, *cursor);
        ours_stream.extend(ours.take());
        let a = screen(&ours_stream, size);
        let b = screen(&written.0.borrow(), size);
        compare(a.screen(), b.screen(), n);
        prev = cur.clone();
    }
}

fn compare(a: &vt100::Screen, b: &vt100::Screen, frame: usize) {
    let (rows, cols) = a.size();
    for row in 0..rows.get() {
        for col in 0..cols.get() {
            let (x, y) = (a.cell(row, col).unwrap(), b.cell(row, col).unwrap());
            let cell = |c: &vt100::Cell| {
                (
                    c.contents().to_owned(),
                    c.is_wide(),
                    c.is_wide_continuation(),
                    c.fgcolor(),
                    c.bgcolor(),
                    (c.bold(), c.dim(), c.italic(), c.underline(), c.inverse()),
                )
            };
            assert_eq!(cell(x), cell(y), "frame {frame}, row {row}, col {col}");
        }
    }
    assert_eq!(
        a.cursor_position(),
        b.cursor_position(),
        "frame {frame}: cursor"
    );
    assert_eq!(
        a.hide_cursor(),
        b.hide_cursor(),
        "frame {frame}: cursor visibility"
    );
}

/// A frame after every event, rendered the way the app renders.
fn app_frames(raw: &str, w: u16, h: u16, events: &[Event]) -> Vec<(Buffer, Option<Position>)> {
    mustty::i18n::init_for_test("zh-CN");
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    // No path: Ctrl+S opens the save-as prompt instead of writing a file into the working directory
    let meta = FileMeta::new(None);
    let mut app = App::new(Document::parse(raw, meta), Size::new(w, h), false);
    let mut frames = Vec::new();
    let mut snap = |terminal: &mut Terminal<TestBackend>, app: &mut App| {
        terminal.draw(|f| view::render(f, app)).unwrap();
        let backend = terminal.backend_mut();
        let cursor = backend
            .cursor_visible()
            .then(|| backend.get_cursor_position().unwrap());
        frames.push((backend.buffer().clone(), cursor));
    };
    snap(&mut terminal, &mut app);
    for ev in events {
        app.handle(ev.clone());
        snap(&mut terminal, &mut app);
    }
    frames
}

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::from(code))
}

fn with(code: KeyCode, mods: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(code, mods))
}

fn chars(s: &str) -> Vec<Event> {
    s.chars().map(|c| key(KeyCode::Char(c))).collect()
}

const MARKDOWN: &str = "# Field Notes\n\nThe rain stopped. **Bold**, *italic*, `code` and [a link](https://example.com).\n\n## 列表\n\n- one\n- 二\n\n> a quote\n";
const CJK: &str = "# 春日\n\n春眠不觉晓，处处闻啼鸟。夜来风雨声，花落知多少。床前明月光，疑是地上霜。\n\nMixed 中英混排 text with「引号」和（括号）。\n";

#[test]
fn snapshot_scenes_look_the_same() {
    let ctrl = |c| with(KeyCode::Char(c), KeyModifiers::CONTROL);
    let shift = |code| with(code, KeyModifiers::SHIFT);
    let long: String = (1..=200)
        .map(|i| format!("Line {i} of a long document.\n"))
        .collect();
    let scenes: Vec<(&str, String, u16, u16, Vec<Event>)> = vec![
        ("empty", String::new(), 80, 24, vec![]),
        (
            "cjk",
            CJK.into(),
            40,
            10,
            vec![key(KeyCode::Down), key(KeyCode::Right)],
        ),
        ("scrolled", long, 80, 24, vec![key(KeyCode::PageDown); 5]),
        ("too small", "# 标题\n\n正文".into(), 8, 3, vec![]),
        ("markdown", MARKDOWN.into(), 60, 16, vec![]),
        (
            "typing and the dirty mark",
            MARKDOWN.into(),
            60,
            16,
            [vec![key(KeyCode::Up)], chars("春天 x")].concat(),
        ),
        (
            "selection",
            MARKDOWN.into(),
            60,
            16,
            vec![
                key(KeyCode::Down),
                shift(KeyCode::Right),
                shift(KeyCode::Right),
                shift(KeyCode::Down),
            ],
        ),
        (
            "typewriter and focus",
            MARKDOWN.into(),
            60,
            16,
            vec![ctrl('t'), key(KeyCode::Down), ctrl('f'), key(KeyCode::Down)],
        ),
        (
            "settings panel",
            MARKDOWN.into(),
            72,
            30,
            vec![
                ctrl('p'),
                key(KeyCode::Down),
                key(KeyCode::Right),
                key(KeyCode::Esc),
            ],
        ),
        (
            "help panel",
            CJK.into(),
            80,
            24,
            vec![ctrl('g'), key(KeyCode::Down), key(KeyCode::Esc)],
        ),
        (
            "vim modes",
            MARKDOWN.into(),
            60,
            16,
            [
                vec![
                    ctrl('p'),
                    key(KeyCode::Down),
                    key(KeyCode::Char(' ')),
                    key(KeyCode::Enter),
                ],
                vec![
                    key(KeyCode::Esc),
                    key(KeyCode::Char('v')),
                    key(KeyCode::Char('j')),
                    key(KeyCode::Esc),
                    key(KeyCode::Char('i')),
                ],
            ]
            .concat(),
        ),
        (
            "save as prompt",
            "draft".into(),
            60,
            10,
            vec![ctrl('s'), key(KeyCode::Backspace)],
        ),
    ];
    for (name, raw, w, h, events) in scenes {
        let frames = app_frames(&raw, w, h, &events);
        let result = std::panic::catch_unwind(|| assert_same_screen(&frames));
        assert!(result.is_ok(), "scene {name:?} differs");
    }
}

const SYMBOLS: &[&str] = &["a", "Z", " ", "#", "中", "文", "「", "，", "é", "~", "春"];

fn color() -> impl Strategy<Value = Color> {
    prop_oneof![
        Just(Color::Reset),
        (0u8..16).prop_map(|i| [
            Color::Black,
            Color::Red,
            Color::Green,
            Color::Yellow,
            Color::Blue,
            Color::Magenta,
            Color::Cyan,
            Color::Gray,
            Color::DarkGray,
            Color::LightRed,
            Color::LightGreen,
            Color::LightYellow,
            Color::LightBlue,
            Color::LightMagenta,
            Color::LightCyan,
            Color::White,
        ][i as usize]),
        any::<u8>().prop_map(Color::Indexed),
        (any::<u8>(), any::<u8>(), any::<u8>()).prop_map(|(r, g, b)| Color::Rgb(r, g, b)),
    ]
}

fn style() -> impl Strategy<Value = Style> {
    (color(), color(), any::<u16>()).prop_map(|(fg, bg, bits)| {
        Style::default()
            .fg(fg)
            .bg(bg)
            .add_modifier(Modifier::from_bits_truncate(bits))
    })
}

/// A frame of random runs of text, each in a random style, plus a random (or hidden) cursor.
fn frame(w: u16, h: u16) -> impl Strategy<Value = (Buffer, Option<Position>)> {
    let runs = proptest::collection::vec(
        (
            0..h,
            0..w,
            proptest::collection::vec(0..SYMBOLS.len(), 1..12),
            style(),
        ),
        0..24,
    );
    let cursor = proptest::option::of((0..w, 0..h).prop_map(|(x, y)| Position { x, y }));
    (runs, cursor).prop_map(move |(runs, cursor)| {
        let mut buf = Buffer::empty(Rect::new(0, 0, w, h));
        for (y, x, symbols, style) in runs {
            let text: String = symbols.into_iter().map(|i| SYMBOLS[i]).collect();
            buf.set_stringn(x, y, text, (w - x) as usize, style);
        }
        (buf, cursor)
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Random styles (every color kind, every modifier combination), wide characters and partial
    /// redraws over several frames.
    #[test]
    fn random_frames_look_the_same(frames in proptest::collection::vec(frame(20, 6), 1..5)) {
        assert_same_screen(&frames);
    }
}
