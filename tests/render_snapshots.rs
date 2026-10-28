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
    // 快照按中文界面录制
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

/// 磁盘上存在的文件（打开时记录过 mtime / 大小）。
fn on_disk(path: &str) -> FileMeta {
    FileMeta {
        stamp: Some(mustty::editor::document::DiskStamp {
            mtime: std::time::SystemTime::UNIX_EPOCH,
            len: 0,
        }),
        ..FileMeta::new(Some(path.into()))
    }
}

/// 只取 status bar 那一行的样式分段。
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
        // 24 列：正文宽 20，文件名从左截断，标记仍在截断后的文件名右侧
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
    // 标记不 DIM：已保存为绿色 ●，未保存为默认前景色 *
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
    // Esc 关闭提示行后恢复
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
