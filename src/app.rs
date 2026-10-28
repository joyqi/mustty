//! App 状态与同步事件循环（方案 §11.3）。

use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::event::{self, Event, KeyEvent};
use ratatui::DefaultTerminal;
use ratatui::layout::{Rect, Size};

use crate::action::{Action, Motion};
use crate::editor::Editor;
use crate::editor::document::Document;
use crate::io::file;
use crate::keymap::{Keymap, Mode};
use crate::status::{Prompt, StatusModel};
use crate::text::wrap::WrapCache;
use crate::view;
use crate::view::layout::{Layout, MEASURE, compute_layout};
use crate::view::viewport::Viewport;

pub struct App {
    pub(crate) editor: Editor,
    keymap: Keymap,
    mode: Mode,
    pub(crate) status: StatusModel,
    pub(crate) wrap: WrapCache,
    pub(crate) viewport: Viewport,
    area: Rect,
    /// 有未保存修改时按过一次 Ctrl+Q
    quit_armed: bool,
    pub should_quit: bool,
    pub needs_redraw: bool,
}

impl App {
    pub fn new(doc: Document, size: Size) -> Self {
        let area = Rect::new(0, 0, size.width, size.height);
        Self {
            editor: Editor::new(doc),
            keymap: Keymap { vim: false },
            mode: Mode::Insert,
            status: StatusModel::default(),
            wrap: WrapCache::new(compute_layout(area, MEASURE).body.width),
            viewport: Viewport::default(),
            area,
            quit_armed: false,
            should_quit: false,
            needs_redraw: true,
        }
    }

    pub fn layout(&self) -> Layout {
        compute_layout(self.area, MEASURE)
    }

    /// Unbound 标题的占位：文件名去扩展名，或 `Untitled`。
    pub(crate) fn placeholder(&self) -> String {
        self.editor
            .doc
            .meta
            .path
            .as_deref()
            .and_then(Path::file_stem)
            .map_or_else(
                || "Untitled".to_owned(),
                |s| s.to_string_lossy().into_owned(),
            )
    }

    pub fn handle(&mut self, ev: Event) {
        if let Event::Resize(w, h) = ev {
            // 同一批里的多个 Resize 只记下尺寸，重排在下一次绘制时进行一次
            self.area = Rect::new(0, 0, w, h);
            self.needs_redraw = true;
            return;
        }
        #[cfg(debug_assertions)]
        if let Event::Key(key) = &ev
            && key.code == crossterm::event::KeyCode::F(12)
            && std::env::var_os("MUSTTY_DEBUG_PANIC").is_some_and(|v| v == "1")
        {
            panic!("MUSTTY_DEBUG_PANIC: F12");
        }
        let prompt_active = self.status.prompt.is_some();
        if let Some(action) = self.keymap.map_event(&ev, &mut self.mode, prompt_active) {
            self.dispatch(action);
        }
    }

    fn dispatch(&mut self, action: Action) {
        self.needs_redraw = true;
        self.status.message = None;
        let armed = std::mem::take(&mut self.quit_armed);
        match action {
            Action::Quit => {
                if armed || !self.editor.is_dirty() {
                    self.should_quit = true;
                } else {
                    self.quit_armed = true;
                    self.status.message = Some("未保存，再按 Ctrl+Q 放弃".into());
                }
            }
            Action::Cancel => self.status.prompt = None,
            Action::Save => match self.editor.doc.meta.path.clone() {
                Some(path) => self.save_to(path),
                None => self.status.prompt = Some(Prompt::default()),
            },
            Action::PromptInput(key) => self.prompt_input(key),
            action => self.edit(&action),
        }
    }

    fn edit(&mut self, action: &Action) {
        let layout = self.layout();
        if layout.too_small {
            return;
        }
        self.sync_body(layout.body);
        let page = (layout.body.height as isize - 2).max(1);
        self.editor.apply(action, &mut self.wrap, page);
        match action {
            Action::Move(Motion::PageUp, _) => {
                self.viewport
                    .scroll(-page, &mut self.wrap, &self.editor.doc.body)
            }
            Action::Move(Motion::PageDown, _) => {
                self.viewport
                    .scroll(page, &mut self.wrap, &self.editor.doc.body)
            }
            _ => {}
        }
        self.sync_body(layout.body);
    }

    fn prompt_input(&mut self, key: KeyEvent) {
        let Some(prompt) = &mut self.status.prompt else {
            return;
        };
        if !prompt.edit(key) || prompt.input.is_empty() {
            return;
        }
        // 相对 cwd，不做 ~ 展开
        let path = PathBuf::from(&prompt.input);
        if path.exists() && !prompt.confirm_overwrite {
            prompt.confirm_overwrite = true;
            return;
        }
        self.status.prompt = None;
        self.save_to(path);
    }

    fn save_to(&mut self, path: PathBuf) {
        match file::save(&self.editor.doc, &path) {
            Ok(()) => {
                self.editor.doc.meta.path = Some(path);
                self.editor.mark_saved();
            }
            Err(e) => self.status.message = Some(format!("保存失败：{e}")),
        }
    }

    /// 让换行宽度与视口适配当前正文区域，并保证光标可见。
    pub(crate) fn sync_body(&mut self, body: Rect) {
        let rope = &self.editor.doc.body;
        if self.wrap.width() != body.width {
            self.viewport.rewrap(body.width, &mut self.wrap, rope);
        }
        let pos = self.editor.cursor.visual(rope, &mut self.wrap);
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
        // 先排空同一批事件再重绘：IME 一次提交的多个字符只重绘一次
        while event::poll(Duration::ZERO)? {
            app.handle(event::read()?);
        }
        if app.should_quit {
            return Ok(());
        }
    }
}
