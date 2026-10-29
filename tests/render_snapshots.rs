use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use mustty::app::App;
use mustty::editor::document::{Document, FileMeta};
use mustty::view;
use ratatui::Terminal;
use ratatui::backend::{Backend, TestBackend};
use ratatui::layout::{Position, Size};

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::from(code))
}

fn draw(path: Option<&str>, raw: &str, w: u16, h: u16, events: &[Event]) -> TestBackend {
    // Snapshots are recorded with the Chinese UI
    mustty::i18n::init_for_test("zh-CN");
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    let meta = FileMeta::new(path.map(Into::into));
    let mut app = App::new(Document::parse(raw, meta), Size::new(w, h), false);
    for ev in events {
        app.handle(ev.clone());
    }
    terminal.draw(|f| view::render(f, &mut app)).unwrap();
    terminal.backend().clone()
}

fn render(raw: &str, w: u16, h: u16, keys: &[KeyCode]) -> String {
    let events: Vec<_> = keys.iter().map(|&k| key(k)).collect();
    snapshot(draw(Some("notes.md"), raw, w, h, &events))
}

fn snapshot(mut backend: TestBackend) -> String {
    let cursor = backend.get_cursor_position().unwrap();
    let visible = backend.cursor_visible();
    format!("{backend}\ncursor: {cursor:?} visible: {visible}")
}

#[test]
fn empty_document() {
    insta::assert_snapshot!(render("", 80, 24, &[]));
}

#[test]
fn cjk_content() {
    let raw = "# 春日\n\n春眠不觉晓，处处闻啼鸟。夜来风雨声，花落知多少。床前明月光，疑是地上霜。\n\nMixed 中英混排 text with「引号」和（括号）。\n";
    insta::assert_snapshot!(render(raw, 40, 10, &[KeyCode::Down, KeyCode::Right]));
}

#[test]
fn scrolled_to_middle() {
    let raw: String = (1..=200)
        .map(|i| format!("Line {i} of a long document.\n"))
        .collect();
    let keys = [KeyCode::PageDown; 5];
    insta::assert_snapshot!(render(&raw, 80, 24, &keys));
}

#[test]
fn too_small() {
    insta::assert_snapshot!(render("# 标题\n\n正文", 8, 3, &[]));
}

#[test]
fn end_of_full_row_does_not_overlap() {
    // 40×10: body is 36 wide at x = 2, y = 1. Exactly 18 CJK characters fill the first visual row
    let raw = "一二三四五六七八九十一二三四五六七八九十";
    let mut b = draw(Some("n.md"), raw, 40, 10, &[key(KeyCode::End)]);
    assert_eq!(b.get_cursor_position().unwrap(), Position::new(38, 1));
    // The same char index shows at the start of the next row by default
    let events = [key(KeyCode::Down), key(KeyCode::Home)];
    let mut b = draw(Some("n.md"), raw, 40, 10, &events);
    assert_eq!(b.get_cursor_position().unwrap(), Position::new(2, 2));
}

#[test]
fn editing_title_and_dirty_marker() {
    let mut events = vec![key(KeyCode::Up)];
    events.extend("新".chars().map(|c| key(KeyCode::Char(c))));
    insta::assert_snapshot!(snapshot(draw(
        Some("notes.md"),
        "# 旧标题\n\n正文\n",
        40,
        10,
        &events
    )));
}

#[test]
fn untitled_save_as_prompt() {
    mustty::i18n::init_for_test("zh-CN");
    // The prefilled name depends on what the directory holds, so use an empty temp dir
    let dir = tempfile::tempdir().unwrap();
    let mut app = App::new(
        Document::parse("", FileMeta::new(None)),
        Size::new(40, 10),
        false,
    );
    app.save_dir = dir.path().to_path_buf();
    let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
    let mut shot = |app: &mut App| {
        terminal.draw(|f| view::render(f, app)).unwrap();
        snapshot(terminal.backend().clone())
    };
    for c in "hi".chars() {
        app.handle(key(KeyCode::Char(c)));
    }
    app.handle(Event::Key(KeyEvent::new(
        KeyCode::Char('s'),
        KeyModifiers::CONTROL,
    )));
    // Prefilled with untitled.md, cursor at the end
    insta::assert_snapshot!("untitled_save_as_prompt_prefilled", shot(&mut app));
    // Delete the prefill and type a.md instead
    for _ in 0.."untitled.md".len() {
        app.handle(key(KeyCode::Backspace));
    }
    for c in "a.md".chars() {
        app.handle(key(KeyCode::Char(c)));
    }
    insta::assert_snapshot!(shot(&mut app));
}

/// Splits each line into styled runs (`"text" style`), skipping unstyled whitespace, so
/// highlighting and selection are easy to review in snapshots.
fn styled(backend: &TestBackend) -> String {
    let buf = backend.buffer();
    let mut out = String::new();
    for y in 0..buf.area.height {
        let mut runs: Vec<(String, String)> = Vec::new();
        for x in 0..buf.area.width {
            let cell = &buf[(x, y)];
            let mut style = format!("{:?}", cell.modifier);
            if cell.fg != ratatui::style::Color::Reset {
                style += &format!(" fg={:?}", cell.fg);
            }
            if cell.bg != ratatui::style::Color::Reset {
                style += &format!(" bg={:?}", cell.bg);
            }
            match runs.last_mut() {
                Some((text, s)) if *s == style => text.push_str(cell.symbol()),
                _ => runs.push((cell.symbol().to_owned(), style)),
            }
        }
        let shown: Vec<String> = runs
            .into_iter()
            .filter(|(t, s)| !(t.trim().is_empty() && s == "NONE"))
            .map(|(t, s)| format!("{:?} {s}", t.trim_end()))
            .collect();
        if !shown.is_empty() {
            out += &format!("{y:2}: {}\n", shown.join(" | "));
        }
    }
    out
}

const MARKDOWN: &str = "# 高亮演示\n\n## 二级 **标题**\n普通 *强调* 与 **加粗** 与 `code`\n**未闭合的加粗 后文不应变粗\n> 引用\n- [ ] 列表 [链接](https://x.io)\n---\n```rust\nlet a = 1; // **不是加粗**\n```\n栅栏之后恢复正常\n";

#[test]
fn markdown_highlight_styles() {
    insta::assert_snapshot!(styled(&draw(Some("hl.md"), MARKDOWN, 60, 24, &[])));
}

#[test]
fn selection_is_reversed() {
    let shift = |code| Event::Key(KeyEvent::new(code, KeyModifiers::SHIFT));
    let events = [
        key(KeyCode::Down),
        Event::Key(KeyEvent::new(
            KeyCode::Right,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        )),
        shift(KeyCode::Down),
    ];
    let b = draw(Some("hl.md"), MARKDOWN, 60, 24, &events);
    insta::assert_snapshot!(format!("{}\n{}", styled(&b), snapshot(b.clone())));
}

fn draw_with(
    config: &mustty::config::Config,
    raw: &str,
    w: u16,
    h: u16,
    events: &[Event],
    hint: bool,
) -> TestBackend {
    mustty::i18n::init_for_test("zh-CN");
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    let meta = FileMeta::new(Some("notes.md".into()));
    let mut app = App::with_config(Document::parse(raw, meta), Size::new(w, h), config);
    if hint {
        app.show_startup_hint(std::time::Instant::now());
    }
    for ev in events {
        app.handle(ev.clone());
    }
    terminal.draw(|f| view::render(f, &mut app)).unwrap();
    terminal.backend().clone()
}

#[test]
fn typewriter_at_document_start_and_startup_hint() {
    let config = mustty::config::Config {
        typewriter: true,
        ..Default::default()
    };
    // 40×12: the body is 6 rows tall and the cursor line stays on row 3; blank space above the
    // start of the document
    let b = draw_with(
        &config,
        "第一行\n第二行\n第三行\n",
        40,
        12,
        &[key(KeyCode::Down)],
        true,
    );
    insta::assert_snapshot!(snapshot(b));
}

#[test]
fn focus_mode_dims_other_paragraphs_and_vim_label() {
    let config = mustty::config::Config {
        focus_mode: true,
        vim: true,
        ..Default::default()
    };
    let raw = "第一段第一行\n第一段第二行\n\n第二段（光标在这里）\n第二段续\n\n第三段\n";
    let events = [
        key(KeyCode::Esc),
        key(KeyCode::Char('j')),
        key(KeyCode::Char('j')),
        key(KeyCode::Char('j')),
    ];
    let b = draw_with(&config, raw, 40, 12, &events, false);
    insta::assert_snapshot!(format!("{}\n{}", styled(&b), snapshot(b.clone())));
}

fn draw_meta(meta: FileMeta, raw: &str, w: u16, h: u16, events: &[Event]) -> TestBackend {
    mustty::i18n::init_for_test("zh-CN");
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    let mut app = App::new(Document::parse(raw, meta), Size::new(w, h), false);
    for ev in events {
        app.handle(ev.clone());
    }
    terminal.draw(|f| view::render(f, &mut app)).unwrap();
    terminal.backend().clone()
}

/// A file that exists on disk (mtime / size recorded when opened).
fn on_disk(path: &str) -> FileMeta {
    FileMeta {
        stamp: Some(mustty::editor::document::DiskStamp {
            mtime: std::time::SystemTime::UNIX_EPOCH,
            len: 0,
        }),
        ..FileMeta::new(Some(path.into()))
    }
}

/// Styled runs of the status bar row only.
fn status_line(b: &TestBackend, y: u16) -> String {
    styled(b)
        .lines()
        .find(|l| l.starts_with(&format!("{y:2}:")))
        .unwrap_or("")
        .to_owned()
}

#[test]
fn save_mark_states() {
    let typed = [key(KeyCode::Char('x'))];
    let cases = [
        (
            "已保存：打开已有文件未修改",
            draw_meta(on_disk("notes.md"), "正文\n", 40, 10, &[]),
        ),
        (
            "有未保存修改",
            draw_meta(on_disk("notes.md"), "正文\n", 40, 10, &typed),
        ),
        (
            "空 Untitled",
            draw_meta(FileMeta::new(None), "", 40, 10, &[]),
        ),
        (
            "Untitled 有修改",
            draw_meta(FileMeta::new(None), "", 40, 10, &typed),
        ),
        (
            "尚未创建的新文件",
            draw_meta(FileMeta::new(Some("new.md".into())), "", 40, 10, &[]),
        ),
        // 24 columns: body is 20 wide, the file name is truncated from the left, and the mark
        // stays to the right of the truncated name
        (
            "窄窗口截断文件名",
            draw_meta(on_disk("a-rather-long-file-name.md"), "", 24, 10, &[]),
        ),
    ];
    let rows: Vec<String> = cases
        .iter()
        .map(|(name, b)| format!("{name}\n{}", status_line(b, 9)))
        .collect();
    insta::assert_snapshot!(rows.join("\n"));
    // The mark is never DIM: saved is a green ●, unsaved is * in the default foreground
    let saved = &cases[0].1;
    let cell = (0..40)
        .map(|x| &saved.buffer()[(x, 9)])
        .find(|c| c.symbol() == "●")
        .unwrap();
    assert_eq!(
        (cell.fg, cell.modifier),
        (
            ratatui::style::Color::Green,
            ratatui::style::Modifier::empty()
        )
    );
    let dirty = &cases[1].1;
    let cell = (0..40)
        .map(|x| &dirty.buffer()[(x, 9)])
        .find(|c| c.symbol() == "*")
        .unwrap();
    assert_eq!(
        (cell.fg, cell.modifier),
        (
            ratatui::style::Color::Reset,
            ratatui::style::Modifier::empty()
        )
    );
}

#[test]
fn prompt_hides_save_mark() {
    let ctrl_s = Event::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    let b = draw_meta(
        FileMeta::new(None),
        "",
        40,
        10,
        &[key(KeyCode::Char('x')), ctrl_s.clone()],
    );
    assert!(!status_line(&b, 9).contains('*'), "{}", status_line(&b, 9));
    // Restored once Esc closes the prompt
    let b = draw_meta(
        FileMeta::new(None),
        "",
        40,
        10,
        &[key(KeyCode::Char('x')), ctrl_s, key(KeyCode::Esc)],
    );
    assert!(status_line(&b, 9).contains('*'), "{}", status_line(&b, 9));
}

#[test]
fn english_status_bar() {
    mustty::i18n::init_for_test("en");
    let text = |events: &[Event]| {
        let mut terminal = Terminal::new(TestBackend::new(60, 10)).unwrap();
        let mut app = App::new(
            Document::parse("", FileMeta::new(None)),
            Size::new(60, 10),
            false,
        );
        for ev in events {
            app.handle(ev.clone());
        }
        terminal.draw(|f| view::render(f, &mut app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        buf.content.iter().map(|c| c.symbol()).collect::<String>()
    };
    let empty = text(&[]);
    assert!(
        empty.contains("Untitled") && empty.contains("0 words"),
        "{empty}"
    );
    let one = text(&[key(KeyCode::Down), key(KeyCode::Char('a'))]);
    assert!(one.contains("1 word ") && !one.contains("1 words"), "{one}");
    let title = text(&[key(KeyCode::Up)]);
    assert!(title.contains("Title"), "{title}");
}

fn ctrl(c: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
}

/// Opens the settings panel with Ctrl+P, then presses ↓ `downs` times.
fn settings_panel(width: u16, w: u16, h: u16, downs: usize) -> String {
    let config = mustty::config::Config {
        width,
        autosave: false,
        ..mustty::config::Config::default()
    };
    let mut events = vec![ctrl('p')];
    events.extend(std::iter::repeat_n(key(KeyCode::Down), downs));
    let b = draw_with(&config, "# 标题\n\n正文\n", w, h, &events, false);
    format!("{}\n{}", styled(&b), snapshot(b.clone()))
}

#[test]
fn settings_panel_width_40() {
    insta::assert_snapshot!(settings_panel(40, 50, 20, 1));
}

#[test]
fn settings_panel_width_72() {
    // Code color selected: the help line carries a colored sample
    insta::assert_snapshot!(settings_panel(72, 80, 24, 9));
}

#[test]
fn settings_panel_scrolls_when_short() {
    insta::assert_snapshot!(settings_panel(72, 80, 12, 7));
}

/// Settings panel with the `Cursor` item selected; `vim` decides the labels and help line of the
/// two cursor items.
fn settings_cursor_rows(vim: bool, events: &[Event]) -> TestBackend {
    let config = mustty::config::Config {
        vim,
        autosave: false,
        ..mustty::config::Config::default()
    };
    draw_with(&config, "# 标题\n\n正文\n", 80, 24, events, false)
}

fn cursor_row_events() -> Vec<Event> {
    let mut events = vec![ctrl('p')];
    events.extend(std::iter::repeat_n(key(KeyCode::Down), 10));
    events
}

#[test]
fn settings_cursor_labels_without_vim() {
    let b = settings_cursor_rows(false, &cursor_row_events());
    let text = format!("{b}");
    assert!(text.contains("光标闪烁") && !text.contains("插入模式"));
    let help = text
        .lines()
        .find(|l| l.contains("写作时的光标形状"))
        .unwrap();
    assert!(
        !help.contains("vim") && !help.contains("方块"),
        "help line must not mention vim: {help:?}"
    );
    insta::assert_snapshot!(snapshot(b));
}

#[test]
fn settings_cursor_labels_with_vim() {
    let b = settings_cursor_rows(true, &cursor_row_events());
    let text = format!("{b}");
    assert!(text.contains("插入模式光标 ") && text.contains("插入模式光标闪烁"));
    assert!(text.contains("仅在插入模式生效；普通和可视模式固定为方块"));
    insta::assert_snapshot!(snapshot(b));
}

#[test]
fn settings_cursor_labels_follow_vim_toggle_live() {
    // With the panel open, toggle the Vim row: the two cursor labels change on the spot, no need to
    // close and reopen the panel
    let mut events = vec![ctrl('p'), key(KeyCode::Down), key(KeyCode::Right)];
    let on = format!("{}", settings_cursor_rows(false, &events));
    assert!(on.contains("插入模式光标闪烁"), "{on}");
    events.push(key(KeyCode::Right));
    let off = format!("{}", settings_cursor_rows(false, &events));
    assert!(
        off.contains("光标闪烁") && !off.contains("插入模式"),
        "{off}"
    );
}

/// vim on: Esc enters Normal mode, then v enters Visual mode.
fn vim_frame(events: &[Event]) -> TestBackend {
    let config = mustty::config::Config {
        vim: true,
        autosave: false,
        ..mustty::config::Config::default()
    };
    draw_with(&config, "hello world\n", 40, 10, events, false)
}

/// Position of the first occurrence of some text in the buffer.
fn find_text(b: &TestBackend, text: &str) -> (u16, u16) {
    let buf = b.buffer();
    (0..buf.area.height)
        .find_map(|y| {
            let row: String = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
            row.find(text).map(|x| (x as u16, y))
        })
        .unwrap_or_else(|| panic!("{text:?} not on screen"))
}

#[test]
fn vim_mode_indicator_styles() {
    use ratatui::style::{Color, Modifier};
    let cases = [
        ("INS", vec![], Color::Reset, Modifier::empty()),
        ("NOR", vec![key(KeyCode::Esc)], Color::Reset, Modifier::DIM),
        (
            "VIS",
            vec![key(KeyCode::Esc), key(KeyCode::Char('v'))],
            Color::Yellow,
            Modifier::empty(),
        ),
    ];
    let mut lines = Vec::new();
    for (label, events, fg, modifier) in cases {
        let b = vim_frame(&events);
        let (x, y) = find_text(&b, &format!("{label} notes.md"));
        for i in 0..3 {
            let cell = &b.buffer()[(x + i, y)];
            assert_eq!((cell.fg, cell.modifier), (fg, modifier), "{label}");
        }
        // The space and file name stay DIM, setting them apart from the mode label
        for i in 3..12 {
            assert_eq!(b.buffer()[(x + i, y)].modifier, Modifier::DIM, "{label}");
        }
        lines.push(status_line(&b, y));
    }
    insta::assert_snapshot!(lines.join("\n"));
}

/// Collects the bytes CrosstermBackend writes.
#[derive(Clone, Default)]
struct Sink(std::rc::Rc<std::cell::RefCell<Vec<u8>>>);

impl std::io::Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Bytes from the real terminal backend: no DIM before INS, DIM (SGR 2) on NOR, and a yellow
/// foreground (SGR 33 or 38;5;3) on VIS.
#[test]
fn vim_mode_indicator_sgr() {
    use ratatui::backend::CrosstermBackend;
    use ratatui::{TerminalOptions, Viewport};
    let sgr_before = |events: &[Event], label: &str| -> (bool, Option<u8>) {
        mustty::i18n::init_for_test("zh-CN");
        let config = mustty::config::Config {
            vim: true,
            autosave: false,
            ..mustty::config::Config::default()
        };
        let (w, h) = (40, 10);
        let mut app = App::with_config(
            Document::parse("hello world\n", FileMeta::new(Some("notes.md".into()))),
            Size::new(w, h),
            &config,
        );
        for ev in events {
            app.handle(ev.clone());
        }
        let options = TerminalOptions {
            viewport: Viewport::Fixed(ratatui::layout::Rect::new(0, 0, w, h)),
        };
        let sink = Sink::default();
        let mut terminal =
            Terminal::with_options(CrosstermBackend::new(sink.clone()), options).unwrap();
        terminal.draw(|f| view::render(f, &mut app)).unwrap();
        let out = String::from_utf8_lossy(&sink.0.borrow()).into_owned();
        let at = out
            .find(label)
            .unwrap_or_else(|| panic!("{label} in {out:?}"));
        sgr_state(&out[..at])
    };
    assert_eq!(
        sgr_before(&[], "INS"),
        (false, None),
        "INS: no DIM, default foreground"
    );
    assert_eq!(
        sgr_before(&[key(KeyCode::Esc)], "NOR"),
        (true, None),
        "NOR: DIM (SGR 2)"
    );
    assert_eq!(
        sgr_before(&[key(KeyCode::Esc), key(KeyCode::Char('v'))], "VIS"),
        (false, Some(3)),
        "VIS: yellow foreground (SGR 33 / 38;5;3), no DIM"
    );
}

/// Replays the SGR sequences in a byte stream and returns the final (DIM?, ANSI foreground index).
fn sgr_state(bytes: &str) -> (bool, Option<u8>) {
    let (mut dim, mut fg) = (false, None);
    for seq in bytes.split("\x1b[").skip(1) {
        let Some(params) = seq.split_once('m').map(|(p, _)| p) else {
            continue;
        };
        if !params.chars().all(|c| c.is_ascii_digit() || c == ';') {
            continue;
        }
        let mut it = params.split(';').map(|p| p.parse::<u16>().unwrap_or(0));
        while let Some(p) = it.next() {
            match p {
                0 => (dim, fg) = (false, None),
                2 => dim = true,
                22 => dim = false,
                30..=37 => fg = Some((p - 30) as u8),
                38 if it.next() == Some(5) => fg = it.next().map(|n| n as u8),
                39 => fg = None,
                _ => {}
            }
        }
    }
    (dim, fg)
}

#[test]
fn without_vim_status_starts_with_file_name() {
    let b = draw(Some("notes.md"), "hello\n", 40, 10, &[]);
    let (x, y) = find_text(&b, "notes.md");
    let row: String = (0..40).map(|x| b.buffer()[(x, y)].symbol()).collect();
    // The left segment starts at the body's left edge: only centering padding before it, no space
    // left over from a mode label
    assert!(row.trim_start().starts_with("notes.md"), "{row:?}");
    let body_x = find_text(&b, "hello").0;
    assert_eq!(x, body_x, "aligned with the body's left edge");
}

/// Switching the language from Auto to English in the panel: with the same App, by the next frame
/// the panel's own labels, the help line and the status bar hints are all in English.
#[test]
fn settings_language_switches_live() {
    let config = mustty::config::Config {
        autosave: false,
        ..mustty::config::Config::default()
    };
    mustty::i18n::init_for_test("zh-CN");
    let (w, h) = (80, 24);
    let doc = Document::parse("# 标题\n\n正文\n", FileMeta::new(Some("notes.md".into())));
    let mut app = App::with_config(doc, Size::new(w, h), &config);
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    app.handle(ctrl('p'));
    for _ in 0..12 {
        app.handle(key(KeyCode::Down));
    }
    terminal.draw(|f| view::render(f, &mut app)).unwrap();
    let zh = terminal.backend().clone();
    // Auto → English
    app.handle(key(KeyCode::Right));
    terminal.draw(|f| view::render(f, &mut app)).unwrap();
    let en = terminal.backend().clone();
    insta::assert_snapshot!("settings_language_zh", snapshot(zh));
    insta::assert_snapshot!("settings_language_en", snapshot(en.clone()));
    let text: String = en.buffer().content.iter().map(|c| c.symbol()).collect();
    for s in [
        "Settings",
        "Body width",
        "Interface language",
        "Enter save",
        "‹ English ›",
    ] {
        assert!(text.contains(s), "{s:?} was not switched");
    }
    // Back to Auto: renegotiate from the real environment variables and system locale, and show the
    // result after Auto
    app.handle(key(KeyCode::Left));
    let negotiated = mustty::i18n::negotiate(|k| std::env::var(k).ok(), sys_locale_or_none());
    assert_eq!(mustty::i18n::current(), negotiated);
    let shown = app_lang_shown(&mut terminal, &mut app);
    assert!(shown.ends_with(&format!("›  ({negotiated})")), "{shown:?}");
}

/// Value of the language row in the panel.
fn app_lang_shown(terminal: &mut Terminal<TestBackend>, app: &mut App) -> String {
    terminal.draw(|f| view::render(f, app)).unwrap();
    let buf = terminal.backend().buffer();
    let row = (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .find(|r| r.contains('‹') && (r.contains("Language") || r.contains("语言")))
        .unwrap();
    row[row.find('‹').unwrap()..].trim_end().to_owned()
}

/// Same system locale source as `i18n::use_config`.
fn sys_locale_or_none() -> Option<String> {
    sys_locale::get_locale()
}

/// Opens the help panel with Ctrl+G, presses `keys` in order, and returns the rendered frame and
/// the App.
fn help_panel(vim: bool, w: u16, h: u16, keys: &[Event]) -> (TestBackend, App) {
    mustty::i18n::init_for_test("en");
    let config = mustty::config::Config {
        vim,
        autosave: false,
        ..mustty::config::Config::default()
    };
    let doc = Document::parse("hello\n", FileMeta::new(Some("notes.md".into())));
    let mut app = App::with_config(doc, Size::new(w, h), &config);
    app.handle(ctrl('g'));
    for k in keys {
        app.handle(k.clone());
    }
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| view::render(f, &mut app)).unwrap();
    (terminal.backend().clone(), app)
}

#[test]
fn help_panel_default() {
    let (b, _) = help_panel(false, 80, 50, &[]);
    insta::assert_snapshot!(format!("{}\n{}", styled(&b), snapshot(b.clone())));
}

#[test]
fn help_panel_vim_shows_three_groups() {
    let (b, _) = help_panel(true, 80, 90, &[]);
    let text = snapshot(b.clone());
    for group in ["Insert mode", "Normal mode", "Visual (selection)"] {
        assert!(text.contains(group), "{group}");
    }
    insta::assert_snapshot!(text);
}

#[test]
fn help_panel_scrolled_to_the_end() {
    let downs = vec![key(KeyCode::PageDown); 20];
    let (b, _) = help_panel(true, 80, 24, &downs);
    let text = snapshot(b.clone());
    assert!(text.contains("End the selection"), "last entries visible");
    insta::assert_snapshot!(text);
}

/// When the window is too small it shows only the notice, like every other screen; the panel is
/// still there once the size recovers.
#[test]
fn help_panel_survives_too_small() {
    let (_, mut app) = help_panel(false, 80, 24, &[]);
    // Under 10 columns wide counts as "window too small"
    let mut terminal = Terminal::new(TestBackend::new(9, 3)).unwrap();
    app.handle(Event::Resize(9, 3));
    terminal.draw(|f| view::render(f, &mut app)).unwrap();
    let tiny: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(tiny.starts_with("Window"), "{tiny:?}");
    app.handle(Event::Resize(80, 24));
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|f| view::render(f, &mut app)).unwrap();
    let back: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(
        back.contains("Keys") && back.contains("Ctrl+S"),
        "still the help panel after recovery"
    );
}
