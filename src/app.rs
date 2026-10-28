//! App 状态与同步事件循环（方案 §11.3）。M0 只读：只有浏览与退出。

use std::path::Path;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;
use ratatui::layout::{Rect, Size};

use crate::editor::cursor::Cursor;
use crate::editor::document::Document;
use crate::status::StatusModel;
use crate::text::wrap::WrapCache;
use crate::view;
use crate::view::layout::{Layout, MEASURE, compute_layout};
use crate::view::viewport::Viewport;

pub struct App {
    pub(crate) doc: Document,
    pub(crate) placeholder: String,
    pub(crate) status: StatusModel,
    pub(crate) cursor: Cursor,
    pub(crate) wrap: WrapCache,
    pub(crate) viewport: Viewport,
    area: Rect,
    pub should_quit: bool,
    pub needs_redraw: bool,
}

impl App {
    pub fn new(doc: Document, path: &Path, size: Size) -> Self {
        let area = Rect::new(0, 0, size.width, size.height);
        let placeholder = path.file_stem().map_or_else(
            || "Untitled".to_owned(),
            |s| s.to_string_lossy().into_owned(),
        );
        Self {
            doc,
            placeholder,
            status: StatusModel::new(path),
            cursor: Cursor::default(),
            wrap: WrapCache::new(compute_layout(area, MEASURE).body.width),
            viewport: Viewport::default(),
            area,
            should_quit: false,
            needs_redraw: true,
        }
    }

    pub fn layout(&self) -> Layout {
        compute_layout(self.area, MEASURE)
    }

    pub fn handle(&mut self, ev: Event) {
        match ev {
            Event::Key(key) if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
                self.on_key(key)
            }
            Event::Resize(w, h) => {
                // 同一批里的多个 Resize 只记下尺寸，重排在下一次绘制时进行一次
                self.area = Rect::new(0, 0, w, h);
                self.needs_redraw = true;
            }
            _ => {}
        }
    }

    fn on_key(&mut self, key: KeyEvent) {
        #[cfg(debug_assertions)]
        if key.code == KeyCode::F(12)
            && std::env::var_os("MUSTTY_DEBUG_PANIC").is_some_and(|v| v == "1")
        {
            panic!("MUSTTY_DEBUG_PANIC: F12");
        }
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('q' | 'Q'))
        {
            self.should_quit = true;
            return;
        }
        let layout = self.layout();
        if layout.too_small {
            return;
        }
        self.sync_body(layout.body);
        let rope = &self.doc.body;
        let page = (layout.body.height as isize - 2).max(1);
        match key.code {
            KeyCode::Left => self.cursor.move_left(rope),
            KeyCode::Right => self.cursor.move_right(rope),
            KeyCode::Up => self.cursor.move_vertical(rope, &mut self.wrap, -1),
            KeyCode::Down => self.cursor.move_vertical(rope, &mut self.wrap, 1),
            KeyCode::PageUp | KeyCode::PageDown => {
                let delta = if key.code == KeyCode::PageUp {
                    -page
                } else {
                    page
                };
                self.cursor.move_vertical(rope, &mut self.wrap, delta);
                self.viewport.scroll(delta, &mut self.wrap, rope);
            }
            _ => return,
        }
        self.sync_body(layout.body);
        self.needs_redraw = true;
    }

    /// 让换行宽度与视口适配当前正文区域，并保证光标可见。
    pub(crate) fn sync_body(&mut self, body: Rect) {
        let rope = &self.doc.body;
        if self.wrap.width() != body.width {
            self.viewport.rewrap(body.width, &mut self.wrap, rope);
        }
        let pos = self.wrap.char_to_visual(rope, self.cursor.head);
        self.viewport.follow(pos, body.height, &mut self.wrap, rope);
    }
}

pub fn run(terminal: &mut DefaultTerminal, app: &mut App) -> anyhow::Result<()> {
    loop {
        if app.needs_redraw {
            terminal.draw(|frame| view::render(frame, app))?;
            app.needs_redraw = false;
        }
        app.handle(event::read()?);
        // 先排空同一批事件再重绘
        while event::poll(Duration::ZERO)? {
            app.handle(event::read()?);
        }
        if app.should_quit {
            return Ok(());
        }
    }
}
