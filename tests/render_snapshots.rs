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
    // 40×10：正文宽 36、x = 2、y = 1。首个视觉行恰好 18 个汉字写满
    let raw = "一二三四五六七八九十一二三四五六七八九十";
    let mut b = draw(Some("n.md"), raw, 40, 10, &[key(KeyCode::End)]);
    assert_eq!(b.get_cursor_position().unwrap(), Position::new(38, 1));
    // 同一 char 索引默认显示在下一行行首
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
    let mut events: Vec<_> = "hi a.md".chars().map(|c| key(KeyCode::Char(c))).collect();
    events.insert(
        2,
        Event::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
    );
    events.remove(3); // 去掉空格：输入的是 "a.md"
    insta::assert_snapshot!(snapshot(draw(None, "", 40, 10, &events)));
}

/// 每行按样式分段：`"文本" 样式`，跳过无样式的空白，便于快照审阅高亮与选区。
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
    // 40×12：正文高 6，光标行恒在第 3 行；文档开头上方留白
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
