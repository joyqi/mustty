use std::path::Path;

use crossterm::event::{Event, KeyCode, KeyEvent};
use mustty::app::App;
use mustty::editor::document::Document;
use mustty::view;
use ratatui::Terminal;
use ratatui::backend::{Backend, TestBackend};
use ratatui::layout::Size;

fn render(raw: &str, w: u16, h: u16, keys: &[KeyCode]) -> String {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    let mut app = App::new(Document::parse(raw), Path::new("notes.md"), Size::new(w, h));
    for &code in keys {
        app.handle(Event::Key(KeyEvent::from(code)));
    }
    terminal.draw(|f| view::render(f, &mut app)).unwrap();
    let backend = terminal.backend_mut();
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
