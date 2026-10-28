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
