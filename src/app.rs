//! App 状态与同步事件循环（方案 §11.3）。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::Command;
use crossterm::cursor::SetCursorStyle;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent};
use ratatui::DefaultTerminal;
use ratatui::layout::{Rect, Size};
use ratatui::style::Color;

use crate::action::{Action, Motion, Target};
use crate::config::Config;
use crate::editor::document::Document;
use crate::editor::{Editor, Effect};
use crate::io::file::{self, IoError};
use crate::keymap::{Keymap, Mode};
use crate::status::{Prompt, StatusModel};
use crate::view;
use crate::view::layout::{Layout, compute_layout};
use crate::view::mouse::{DRAG_SCROLL_INTERVAL, DragState, drag_target, map_mouse};
use crate::view::viewport::Viewport;

/// 默认：最后一次编辑后空闲多久自动保存（配置项 autosave_idle_ms）。
pub const AUTOSAVE_IDLE: Duration = Duration::from_secs(2);
/// 启动键位提示的显示时长。
pub const HINT_TTL: Duration = Duration::from_secs(5);
/// 「已保存」类提示的显示时长。
pub const SAVED_TTL: Duration = Duration::from_secs(2);
/// 其他临时提示的显示时长。
pub const INFO_TTL: Duration = Duration::from_secs(3);
/// 超过此大小的正文不实时刷新字数，改为保存时刷新。
pub const LARGE_FILE: usize = 5 * 1024 * 1024;
/// Windows 上同一批事件里连续插入这么多字符视为一次粘贴。
pub const PASTE_MIN_CHARS: usize = 16;

pub struct App {
    pub(crate) editor: Editor,
    keymap: Keymap,
    mode: Mode,
    pub(crate) status: StatusModel,
    pub(crate) viewport: Viewport,
    area: Rect,
    /// 正文宽度（配置项 width / --width）
    measure: u16,
    autosave: bool,
    autosave_idle: Duration,
    /// 处理鼠标事件（配置项 mouse / --mouse；关闭时终端不会送来鼠标事件）
    mouse: bool,
    /// 左键拖拽中（仅鼠标开启时）
    drag: Option<DragState>,
    pub(crate) focus_mode: bool,
    pub(crate) code_color: Color,
    /// 大文件：字数只在保存时刷新
    large: bool,
    /// 最后一次改动内容的时刻；自动保存尝试后清空，避免失败时反复重试
    last_edit: Option<Instant>,
    /// 有未保存修改时按过一次 Ctrl+Q
    quit_armed: bool,
    /// 待写给终端的控制序列（OSC 52、光标形状），由事件循环写出
    output: Vec<String>,
    /// 终端支持 OSC 52（macOS Terminal 不支持）
    osc52_supported: bool,
    osc52_warned: bool,
    pub should_quit: bool,
    pub needs_redraw: bool,
}

impl App {
    /// 默认配置，只指定是否自动保存。
    pub fn new(doc: Document, size: Size, autosave: bool) -> Self {
        let config = Config {
            autosave,
            ..Config::default()
        };
        Self::with_config(doc, size, &config)
    }

    pub fn with_config(doc: Document, size: Size, config: &Config) -> Self {
        let area = Rect::new(0, 0, size.width, size.height);
        let large = doc.body().len_bytes() > LARGE_FILE;
        let editor = Editor::new(doc, compute_layout(area, config.width).body.width);
        let mut status = StatusModel::default();
        if large {
            status.info(
                "文件超过 5 MB，字数改为保存时更新",
                Instant::now(),
                INFO_TTL,
            );
        }
        status.words = editor.word_count();
        Self {
            editor,
            keymap: Keymap { vim: config.vim },
            mode: Mode::Insert,
            status,
            viewport: Viewport {
                typewriter: config.typewriter,
                scrolloff: config.scrolloff,
                ..Viewport::default()
            },
            area,
            measure: config.width,
            autosave: config.autosave,
            autosave_idle: config.autosave_idle,
            mouse: config.mouse,
            drag: None,
            focus_mode: config.focus_mode,
            code_color: config.code_color,
            large,
            last_edit: None,
            quit_armed: false,
            output: Vec::new(),
            osc52_supported: std::env::var_os("TERM_PROGRAM").is_none_or(|t| t != "Apple_Terminal"),
            osc52_warned: false,
            should_quit: false,
            needs_redraw: true,
        }
    }

    pub fn layout(&self) -> Layout {
        compute_layout(self.area, self.measure)
    }

    pub fn measure(&self) -> u16 {
        self.measure
    }

    /// 启动后 5 s 在 status bar 右段显示键位提示。
    pub fn show_startup_hint(&mut self, now: Instant) {
        self.status.hint_until = Some(now + HINT_TTL);
    }

    /// 配置文件的警告：一条 status bar 错误消息，保留到第一次按键。
    pub fn show_startup_warning(&mut self, text: String) {
        self.status.error(text);
    }

    /// status bar 最前的模式标识，仅 vim 开启时显示。
    pub(crate) fn mode_label(&self) -> Option<&'static str> {
        self.keymap.vim.then_some(match self.mode {
            Mode::Insert => "INS",
            Mode::Normal(s) if s.visual => "VIS",
            Mode::Normal(_) => "NOR",
        })
    }

    /// Unbound 标题的占位：文件名去扩展名，或 `Untitled`。
    pub(crate) fn placeholder(&self) -> String {
        self.editor
            .doc()
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
        self.handle_at(ev, Instant::now());
    }

    pub fn handle_at(&mut self, ev: Event, now: Instant) {
        match ev {
            Event::Resize(w, h) => {
                // 同一批里的多个 Resize 只记下尺寸，重排在下一次绘制时进行一次
                self.area = Rect::new(0, 0, w, h);
                self.needs_redraw = true;
                return;
            }
            Event::FocusLost => {
                if self.editor.is_dirty() {
                    self.autosave_now(now);
                }
                return;
            }
            Event::Mouse(m) => {
                if self.mouse {
                    self.on_mouse(m, now);
                }
                return;
            }
            _ => {}
        }
        #[cfg(debug_assertions)]
        if let Event::Key(key) = &ev
            && key.code == KeyCode::F(12)
            && std::env::var_os("MUSTTY_DEBUG_PANIC").is_some_and(|v| v == "1")
        {
            panic!("MUSTTY_DEBUG_PANIC: F12");
        }
        let prompt_active = self.status.prompt.is_some();
        let before = matches!(self.mode, Mode::Normal(_));
        let was_visual = matches!(self.mode, Mode::Normal(s) if s.visual);
        let action = self.keymap.map_event(&ev, &mut self.mode, prompt_active);
        let after = matches!(self.mode, Mode::Normal(_));
        let left_visual = was_visual && !matches!(self.mode, Mode::Normal(s) if s.visual);
        if before != after || self.mode_label().is_some() {
            self.needs_redraw = true;
        }
        if before != after {
            // Normal 态块状光标，Insert 态竖线
            let style = if after {
                SetCursorStyle::SteadyBlock
            } else {
                SetCursorStyle::SteadyBar
            };
            let mut seq = String::new();
            if style.write_ansi(&mut seq).is_ok() {
                self.output.push(seq);
            }
        }
        if let Some(action) = action {
            self.dispatch(action, now);
        }
        // 离开 visual（y 复制之后等）：与 vim 一样不再保留选区
        if left_visual && self.editor.selection().is_some() {
            self.edit(&Action::Cancel, now);
        }
    }

    fn dispatch(&mut self, action: Action, now: Instant) {
        self.needs_redraw = true;
        self.status.on_input();
        let armed = std::mem::take(&mut self.quit_armed);
        let revision = self.editor.doc().revision();
        match action {
            Action::Quit => self.quit(armed, now),
            Action::Cancel if self.status.prompt.is_some() => self.status.prompt = None,
            Action::Save => match self.editor.doc().meta.path.clone() {
                Some(path) => {
                    self.save(path, false, now);
                }
                None => self.status.prompt = Some(Prompt::default()),
            },
            Action::PromptInput(key) => self.prompt_input(key, now),
            Action::ScrollView(delta) => self.scroll_view(delta, now),
            Action::ToggleTypewriter => {
                self.viewport.typewriter = !self.viewport.typewriter;
                let state = if self.viewport.typewriter {
                    "开"
                } else {
                    "关"
                };
                self.status
                    .info(format!("Typewriter 模式：{state}"), now, SAVED_TTL);
                let body = self.layout().body;
                self.sync_body(body);
            }
            Action::ToggleFocusMode => {
                self.focus_mode = !self.focus_mode;
                let state = if self.focus_mode { "开" } else { "关" };
                self.status
                    .info(format!("Focus 模式：{state}"), now, SAVED_TTL);
            }
            Action::Paste(s) => match &mut self.status.prompt {
                Some(prompt) => prompt.paste(&s),
                None => self.edit(&Action::Paste(s), now),
            },
            action => self.edit(&action, now),
        }
        if self.editor.doc().revision() != revision {
            self.last_edit = Some(now);
        }
    }

    fn quit(&mut self, armed: bool, now: Instant) {
        if armed || !self.editor.is_dirty() || (self.can_autosave() && self.autosave_now(now)) {
            self.should_quit = true;
        } else {
            self.quit_armed = true;
            self.status.error("未保存，再按 Ctrl+Q 放弃");
        }
    }

    fn edit(&mut self, action: &Action, now: Instant) {
        let layout = self.layout();
        if layout.too_small {
            return;
        }
        self.sync_body(layout.body);
        let page = (layout.body.height as isize - 2).max(1);
        let effect = self.editor.apply(action, page, now);
        let (doc, wrap, _) = self.editor.view();
        match action {
            Action::Move(Motion::PageUp, _) => self.viewport.scroll(-page, wrap, doc.body()),
            Action::Move(Motion::PageDown, _) => self.viewport.scroll(page, wrap, doc.body()),
            _ => {}
        }
        self.sync_body(layout.body);
        if let Effect::Copied(text) = effect {
            self.copy_to_clipboard(&text, now);
        }
    }

    /// 内部寄存器已由 Editor 写入；这里再通过 OSC 52 写系统剪贴板（可穿透 SSH / tmux）。
    fn copy_to_clipboard(&mut self, text: &str, now: Instant) {
        if self.osc52_supported {
            self.output.push(crate::terminal::osc52(text));
        } else if !std::mem::replace(&mut self.osc52_warned, true) {
            self.status.info(
                "此终端不支持 OSC 52：已复制到内部寄存器，Ctrl+V 可粘贴",
                now,
                INFO_TTL,
            );
        }
    }

    /// 取出待写给终端的控制序列。
    pub fn take_terminal_output(&mut self) -> Vec<String> {
        std::mem::take(&mut self.output)
    }

    fn prompt_input(&mut self, key: KeyEvent, now: Instant) {
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
        self.save(path, false, now);
    }

    fn can_autosave(&self) -> bool {
        self.autosave && self.editor.doc().meta.path.is_some() && !self.status.externally_modified
    }

    /// 立即自动保存（检查外部修改）；返回是否成功。
    fn autosave_now(&mut self, now: Instant) -> bool {
        self.last_edit = None;
        match self.editor.doc().meta.path.clone() {
            Some(path) if self.can_autosave() => self.save(path, true, now),
            _ => false,
        }
    }

    fn save(&mut self, path: PathBuf, auto: bool, now: Instant) -> bool {
        self.needs_redraw = true;
        match file::save(self.editor.doc(), &path, auto) {
            Ok(mtime) => {
                tracing::debug!(path = %path.display(), auto, "saved");
                self.editor.mark_saved(path, mtime);
                self.status.externally_modified = false;
                self.status
                    .info(if auto { "已自动保存" } else { "已保存" }, now, SAVED_TTL);
                if self.large {
                    self.status.words = self.editor.word_count();
                }
                true
            }
            Err(IoError::ExternallyModified) => {
                tracing::warn!(path = %path.display(), "externally modified, autosave skipped");
                self.status.externally_modified = true;
                false
            }
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "save failed");
                self.status.error(format!("保存失败：{e}"));
                false
            }
        }
    }

    fn autosave_deadline(&self) -> Option<Instant> {
        self.last_edit
            .filter(|_| self.can_autosave() && self.editor.is_dirty())
            .map(|t| t + self.autosave_idle)
    }

    /// 下一次需要醒来的时刻：自动保存与消息过期中较早者。
    pub fn next_deadline(&self) -> Option<Instant> {
        [
            self.autosave_deadline(),
            self.status.expiry(),
            self.status.hint_until,
            self.drag.and_then(|d| d.deadline),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// 处理到期的自动保存与消息。
    pub fn tick(&mut self, now: Instant) {
        if self.autosave_deadline().is_some_and(|d| now >= d) {
            self.autosave_now(now);
        }
        if self.status.expire(now) {
            self.needs_redraw = true;
        }
        self.drag_autoscroll(now);
        if self.status.hint_until.is_some_and(|t| now >= t) {
            self.status.hint_until = None;
            self.needs_redraw = true;
        }
    }

    /// 鼠标（方案 §10.5）：由 `view::mouse::map_mouse` 转成 Action，与键盘共用同一条编辑路径。
    fn on_mouse(&mut self, m: MouseEvent, now: Instant) {
        if self.status.prompt.is_some() {
            return;
        }
        let layout = self.layout();
        if layout.too_small {
            return;
        }
        self.sync_body(layout.body);
        let title = self.editor.doc().title_text().to_owned();
        let (doc, wrap, _) = self.editor.view();
        let action = map_mouse(
            &m,
            &layout,
            &self.viewport,
            wrap,
            doc.body(),
            &title,
            &mut self.drag,
            now,
        );
        if let Some(action) = action {
            self.mouse_action(action, now);
        }
    }

    fn mouse_action(&mut self, action: Action, now: Instant) {
        // vim Normal 态拖拽出选区：同时进入 visual，之后可以直接 d / y
        if let (Action::MoveTo(_, true), Mode::Normal(state)) = (&action, &mut self.mode) {
            state.visual = true;
        }
        self.dispatch(action, now);
    }

    /// 滚轮：只移动视口；光标若离开 scrolloff 区间，就拉回可见区域边缘（保持 preferred_col），
    /// 光标必须始终在屏幕上，IME 候选框才有处锚定。
    fn scroll_view(&mut self, delta: i32, now: Instant) {
        let layout = self.layout();
        if layout.too_small {
            return;
        }
        let body = layout.body;
        let h = body.height as isize;
        let so = self.viewport.margin(body.height) as isize;
        let cursor = self.editor.cursor;
        let (doc, wrap, _) = self.editor.view();
        self.viewport.scroll(delta as isize, wrap, doc.body());
        let top = (self.viewport.top.line, self.viewport.top.row);
        let pos = cursor.visual(doc.body(), wrap);
        let d = self.viewport.top_padding as isize
            + wrap.rows_between(doc.body(), top, (pos.line, pos.row));
        let pull = if d < so {
            Some((Motion::PageDown, so - d))
        } else if d > h - 1 - so {
            Some((Motion::PageUp, d - (h - 1 - so)))
        } else {
            None
        };
        if let Some((motion, rows)) = pull {
            self.editor.apply(&Action::Move(motion, false), rows, now);
        }
        self.sync_body(body);
        self.needs_redraw = true;
    }

    /// 拖到正文上下边缘之外并保持不动：每 50 ms 滚动一行，光标跟到新的边缘行。
    fn drag_autoscroll(&mut self, now: Instant) {
        let Some(state) = self.drag else { return };
        if state.deadline.is_none_or(|d| now < d) {
            return;
        }
        let body = self.layout().body;
        let delta = if state.last.y < body.y { -1 } else { 1 };
        let (doc, wrap, _) = self.editor.view();
        self.viewport.scroll(delta, wrap, doc.body());
        let at = drag_target(body, state.last, &self.viewport, wrap, doc.body());
        self.drag = Some(DragState {
            deadline: Some(now + DRAG_SCROLL_INTERVAL),
            ..state
        });
        self.mouse_action(Action::MoveTo(Target::Body(at), true), now);
    }

    /// 绘制前调用：让换行宽度与视口适配当前正文区域，保证光标可见；刷新字数。
    pub(crate) fn sync_body(&mut self, body: Rect) {
        let cursor = self.editor.cursor;
        let (doc, wrap, _) = self.editor.view();
        let rope = doc.body();
        if wrap.width() != body.width {
            self.viewport.rewrap(body.width, wrap, rope);
        }
        let pos = cursor.visual(rope, wrap);
        self.viewport.follow(pos, body.height, wrap, rope);
        if !self.large {
            self.status.words = self.editor.word_count();
        }
    }
}

/// Windows 不产生 `Event::Paste`：把同一批里连续 ≥ `PASTE_MIN_CHARS` 个字符输入合并为一次粘贴。
pub fn coalesce_paste(events: Vec<Event>) -> Vec<Event> {
    let mut out = Vec::with_capacity(events.len());
    let mut run = String::new();
    let mut run_events = Vec::new();
    let flush = |out: &mut Vec<Event>, run: &mut String, run_events: &mut Vec<Event>| {
        if run.chars().count() >= PASTE_MIN_CHARS {
            out.push(Event::Paste(std::mem::take(run)));
            run_events.clear();
        } else {
            run.clear();
            out.append(run_events);
        }
    };
    for ev in events {
        match &ev {
            Event::Key(k) if k.kind == KeyEventKind::Release => run_events.push(ev),
            Event::Key(k) if typed_char(k).is_some() => {
                run.push(typed_char(k).unwrap());
                run_events.push(ev);
            }
            _ => {
                flush(&mut out, &mut run, &mut run_events);
                out.push(ev);
            }
        }
    }
    flush(&mut out, &mut run, &mut run_events);
    out
}

/// 粘贴时终端逐键送来的字符（含回车）。
fn typed_char(k: &KeyEvent) -> Option<char> {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let altgr = ctrl && k.modifiers.contains(KeyModifiers::ALT);
    match k.code {
        KeyCode::Char(c) if !ctrl || altgr => Some(c),
        KeyCode::Enter => Some('\n'),
        _ => None,
    }
}

pub fn run(terminal: &mut DefaultTerminal, app: &mut App) -> anyhow::Result<()> {
    // 上一批事件的处理耗时，与随后的绘制耗时一起记为一帧
    let mut handle_us = 0;
    loop {
        let output = app.take_terminal_output();
        if !output.is_empty() {
            let backend = terminal.backend_mut();
            for seq in output {
                backend.write_all(seq.as_bytes())?;
            }
            backend.flush()?;
        }
        if app.needs_redraw {
            let start = Instant::now();
            terminal.draw(|frame| view::render(frame, app))?;
            app.needs_redraw = false;
            let draw_us = start.elapsed().as_micros() as u64;
            tracing::debug!(target: "mustty::frame", handle_us, draw_us, total_us = handle_us + draw_us, "frame");
        }
        let timeout = app
            .next_deadline()
            .map(|d| d.saturating_duration_since(Instant::now()));
        let mut batch = Vec::new();
        let ready = match timeout {
            Some(t) => event::poll(t)?,
            None => true, // 无事可等：阻塞在 read 上，空闲时 0 CPU
        };
        if ready {
            batch.push(event::read()?);
            // 先排空同一批事件再重绘：IME 一次提交的多个字符只重绘一次
            while event::poll(Duration::ZERO)? {
                batch.push(event::read()?);
            }
        }
        if cfg!(windows) {
            batch = coalesce_paste(batch);
        }
        let start = Instant::now();
        for ev in batch {
            app.handle(ev);
        }
        handle_us = start.elapsed().as_micros() as u64;
        app.tick(Instant::now());
        if app.should_quit {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventState, MouseButton, MouseEventKind};

    fn press(c: KeyCode) -> Event {
        Event::Key(KeyEvent::new(c, KeyModifiers::NONE))
    }

    fn release(c: KeyCode) -> Event {
        Event::Key(KeyEvent::new_with_kind_and_state(
            c,
            KeyModifiers::NONE,
            KeyEventKind::Release,
            KeyEventState::NONE,
        ))
    }

    #[test]
    fn autosave_after_idle_and_external_modification() {
        use std::fs;
        use std::time::SystemTime;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.md");
        fs::write(&path, "v1").unwrap();
        let doc = file::load(&path).unwrap();
        let mut app = App::new(doc, Size::new(80, 24), true);
        let t0 = Instant::now();
        app.handle_at(press(KeyCode::Char('x')), t0);
        assert_eq!(app.next_deadline(), Some(t0 + AUTOSAVE_IDLE));
        app.tick(t0 + Duration::from_millis(1999));
        assert_eq!(fs::read_to_string(&path).unwrap(), "v1");
        app.tick(t0 + AUTOSAVE_IDLE);
        assert_eq!(fs::read_to_string(&path).unwrap(), "xv1");
        assert!(!app.editor.is_dirty());
        // 「已自动保存」2 s 后消失
        assert_eq!(app.next_deadline(), Some(t0 + AUTOSAVE_IDLE + SAVED_TTL));

        // 外部修改后：放弃自动保存、常驻提示、不再安排自动保存
        fs::write(&path, "external").unwrap();
        let later = SystemTime::now() + Duration::from_secs(5);
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(later)
            .unwrap();
        let t1 = t0 + Duration::from_secs(10);
        app.handle_at(press(KeyCode::Char('y')), t1);
        app.tick(t1 + AUTOSAVE_IDLE);
        assert_eq!(fs::read_to_string(&path).unwrap(), "external");
        assert!(app.status.externally_modified);
        app.tick(t1 + Duration::from_secs(60));
        assert_eq!(app.next_deadline(), None);
        // 退出需要确认；Ctrl+S 强制覆盖
        let ctrl = |c| Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
        app.handle_at(ctrl('q'), t1);
        assert!(!app.should_quit);
        app.handle_at(ctrl('s'), t1);
        assert_eq!(fs::read_to_string(&path).unwrap(), "xyv1");
        assert!(!app.status.externally_modified);
    }

    #[test]
    fn untitled_and_no_autosave_never_autosave() {
        let t0 = Instant::now();
        let untitled = Document::parse("", crate::editor::document::FileMeta::new(None));
        let mut app = App::new(untitled, Size::new(80, 24), true);
        app.handle_at(press(KeyCode::Char('x')), t0);
        assert_eq!(app.next_deadline(), None);

        let dir = tempfile::tempdir().unwrap();
        let doc = file::load(&dir.path().join("n.md")).unwrap();
        let mut app = App::new(doc, Size::new(80, 24), false);
        app.handle_at(press(KeyCode::Char('x')), t0);
        app.handle_at(Event::FocusLost, t0);
        assert_eq!(app.next_deadline(), None);
        assert!(!dir.path().join("n.md").exists());
    }

    fn vim_app(text: &str) -> App {
        let doc = Document::parse(text, crate::editor::document::FileMeta::new(None));
        let config = Config {
            vim: true,
            autosave: false,
            ..Config::default()
        };
        App::with_config(doc, Size::new(80, 24), &config)
    }

    fn typed(app: &mut App, keys: &str) {
        for c in keys.chars() {
            let code = if c == '\x1b' {
                KeyCode::Esc
            } else {
                KeyCode::Char(c)
            };
            app.handle(press(code));
        }
    }

    fn body(app: &App) -> String {
        app.editor.doc().body().to_string()
    }

    #[test]
    fn vim_flow_moves_first_line_to_end() {
        // 方案 §12 M4 验收 5：i 输入 → Esc → gg → dd → G → p
        let mut app = vim_app("");
        typed(&mut app, "\x1bione");
        app.handle(press(KeyCode::Enter));
        typed(&mut app, "two");
        app.handle(press(KeyCode::Enter));
        typed(&mut app, "three\x1b");
        assert_eq!(app.mode_label(), Some("NOR"));
        typed(&mut app, "ggddGp");
        assert_eq!(body(&app), "two\nthree\none");
        // u 与 Ctrl+Z 效果相同
        typed(&mut app, "u");
        assert_eq!(body(&app), "two\nthree");
        app.handle(Event::Key(KeyEvent::new(
            KeyCode::Char('z'),
            KeyModifiers::CONTROL,
        )));
        assert_eq!(body(&app), "one\ntwo\nthree");
        app.handle(Event::Key(KeyEvent::new(
            KeyCode::Char('r'),
            KeyModifiers::CONTROL,
        )));
        assert_eq!(body(&app), "two\nthree");
    }

    #[test]
    fn vim_visual_word_delete_and_ignored_chars() {
        let mut app = vim_app("hello world foo");
        typed(&mut app, "\x1b");
        // Normal 态的可打印字符（含汉字）不插入
        typed(&mut app, "中q");
        assert_eq!(body(&app), "hello world foo");
        // v + w + d 删除一个词
        typed(&mut app, "vw");
        assert_eq!(app.mode_label(), Some("VIS"));
        typed(&mut app, "d");
        assert_eq!(
            (body(&app).as_str(), app.mode_label()),
            ("world foo", Some("NOR"))
        );
        // v + w + y 复制后选区消失，p 粘贴到光标处
        typed(&mut app, "vwy");
        assert_eq!(
            (
                app.editor.register().unwrap().text.as_str(),
                app.editor.selection()
            ),
            ("world ", None)
        );
        typed(&mut app, "0p");
        assert_eq!(body(&app), "world world foo");
        // 选区不含光标所在的字（竖线光标模型）：v + e 停在词的最后一个字之前
        typed(&mut app, "0vey");
        assert_eq!(app.editor.register().unwrap().text, "worl");
    }

    #[test]
    fn vim_cursor_shape_follows_mode() {
        let mut app = vim_app("x");
        app.take_terminal_output();
        typed(&mut app, "\x1b");
        assert_eq!(
            app.take_terminal_output(),
            vec!["\x1b[2 q".to_owned()],
            "Normal：SteadyBlock"
        );
        typed(&mut app, "v\x1b");
        assert!(
            app.take_terminal_output().is_empty(),
            "visual 仍是 Normal，不换形状"
        );
        typed(&mut app, "i");
        assert_eq!(
            app.take_terminal_output(),
            vec!["\x1b[6 q".to_owned()],
            "Insert：SteadyBar"
        );
        // 默认模式从不改变光标形状、不显示模式
        let mut plain = App::new(
            Document::parse("x", crate::editor::document::FileMeta::new(None)),
            Size::new(80, 24),
            false,
        );
        typed(&mut plain, "\x1bab");
        assert!(plain.take_terminal_output().is_empty());
        assert_eq!(plain.mode_label(), None);
    }

    fn mouse(kind: MouseEventKind, column: u16, row: u16) -> Event {
        Event::Mouse(MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        })
    }

    fn mouse_app(text: &str, enabled: bool) -> App {
        let doc = Document::parse(text, crate::editor::document::FileMeta::new(None));
        let config = Config {
            mouse: enabled,
            autosave: false,
            ..Config::default()
        };
        App::with_config(doc, Size::new(80, 24), &config)
    }

    #[test]
    fn mouse_click_drag_and_wheel_use_move_actions() {
        // 80×24：正文 x = 4、y = 5、高 14
        let text: String = (0..100).map(|i| format!("第{i:02}行 line\n")).collect();
        let mut app = mouse_app(&text, true);
        let left = MouseButton::Left;
        let t0 = Instant::now();
        // 每行 "第NN行 line\n" 共 10 个 char：「第」占第 0–1 列，「0」第 2 列，「2」第 3 列
        app.handle_at(mouse(MouseEventKind::Down(left), 4 + 3, 5 + 2), t0);
        assert_eq!(app.editor.cursor.head, 2 * 10 + 2);
        // 宽字符右半格吸附左侧
        app.handle_at(mouse(MouseEventKind::Down(left), 4 + 1, 5 + 2), t0);
        assert_eq!(app.editor.cursor.head, 2 * 10);
        app.handle_at(mouse(MouseEventKind::Down(left), 4 + 3, 5 + 2), t0);
        // 点在正文之外：忽略
        app.handle_at(mouse(MouseEventKind::Down(left), 1, 7), t0);
        app.handle_at(mouse(MouseEventKind::Down(left), 4, 22), t0);
        assert_eq!(app.editor.cursor.head, 2 * 10 + 2);
        app.handle_at(mouse(MouseEventKind::Down(left), 4 + 3, 5 + 2), t0);
        // 拖拽形成选区
        app.handle_at(mouse(MouseEventKind::Drag(left), 4, 5 + 4), t0);
        assert_eq!(app.editor.selection(), Some(2 * 10 + 2..4 * 10));
        // 拖到正文下方并保持不动：每 50 ms 滚动一行，选区跟着延伸；松开后停止
        let top = app.viewport.top.line;
        app.handle_at(mouse(MouseEventKind::Drag(left), 10, 23), t0);
        assert_eq!(app.next_deadline(), Some(t0 + DRAG_SCROLL_INTERVAL));
        for k in 1..=10 {
            app.tick(t0 + DRAG_SCROLL_INTERVAL * k);
        }
        assert_eq!(app.viewport.top.line, top + 10, "持续滚动");
        let sel = app.editor.selection().unwrap();
        assert_eq!(sel.start, 2 * 10 + 2);
        assert!(sel.end > 20 * 10, "{sel:?}");
        app.handle_at(mouse(MouseEventKind::Up(left), 10, 23), t0);
        app.tick(t0 + DRAG_SCROLL_INTERVAL * 20);
        assert_eq!(app.viewport.top.line, top + 10, "松开后停止");
        // 滚轮只移动视口；光标离开 scrolloff 区间时被拉回边缘，始终可见
        let line = |app: &App| app.editor.doc().body().char_to_line(app.editor.cursor.head);
        let (top, cursor_line) = (app.viewport.top.line, line(&app));
        app.handle_at(mouse(MouseEventKind::ScrollDown, 10, 10), t0);
        assert_eq!(
            (app.viewport.top.line, line(&app)),
            (top + 3, cursor_line),
            "光标仍在区间内：不动"
        );
        for _ in 0..10 {
            app.handle_at(mouse(MouseEventKind::ScrollDown, 10, 10), t0);
        }
        assert_eq!(app.viewport.top.line, top + 33);
        assert_eq!(line(&app), top + 33 + 3, "被拉到上边缘（scrolloff 3）");
        // 鼠标关闭：完全忽略鼠标事件
        let mut off = mouse_app(&text, false);
        off.handle(mouse(MouseEventKind::Down(left), 4 + 3, 5 + 2));
        assert_eq!(off.editor.cursor.head, 0);
    }

    #[test]
    fn vim_normal_drag_enters_visual() {
        let doc = Document::parse(
            "hello world\nsecond",
            crate::editor::document::FileMeta::new(None),
        );
        let config = Config {
            vim: true,
            mouse: true,
            autosave: false,
            ..Config::default()
        };
        let mut app = App::with_config(doc, Size::new(80, 24), &config);
        typed(&mut app, "\x1b");
        app.handle(mouse(MouseEventKind::Down(MouseButton::Left), 4, 5));
        app.handle(mouse(MouseEventKind::Drag(MouseButton::Left), 4 + 6, 5));
        assert_eq!(app.mode_label(), Some("VIS"));
        typed(&mut app, "d");
        assert_eq!(
            (body(&app).as_str(), app.mode_label()),
            ("world\nsecond", Some("NOR"))
        );
    }

    #[test]
    fn click_from_title_returns_to_body() {
        let mut app = mouse_app("# T\n\nabc", true);
        app.handle(press(KeyCode::Up));
        assert_eq!(app.editor.focus, crate::editor::Focus::Title);
        app.handle(mouse(MouseEventKind::Down(MouseButton::Left), 4 + 2, 5));
        assert_eq!(
            (app.editor.focus, app.editor.cursor.head),
            (crate::editor::Focus::Body, 2)
        );
    }

    #[test]
    fn typewriter_toggle_and_config() {
        let text: String = (0..100).map(|i| format!("{i}\n")).collect();
        let doc = Document::parse(&text, crate::editor::document::FileMeta::new(None));
        let config = Config {
            typewriter: true,
            autosave: false,
            ..Config::default()
        };
        let mut app = App::with_config(doc, Size::new(80, 24), &config);
        let body = app.layout().body; // 高 14 → 中线第 7 行
        app.sync_body(body);
        assert_eq!(app.viewport.top_padding, 7);
        for _ in 0..30 {
            app.handle(press(KeyCode::Down));
            let cur = app.editor.cursor;
            let (doc, wrap, _) = app.editor.view();
            let pos = cur.visual(doc.body(), wrap);
            let row = app.viewport.top_padding as isize
                + wrap.rows_between(
                    doc.body(),
                    (app.viewport.top.line, app.viewport.top.row),
                    (pos.line, pos.row),
                );
            assert_eq!(row, 7);
        }
        app.handle(Event::Key(KeyEvent::new(
            KeyCode::Char('t'),
            KeyModifiers::CONTROL,
        )));
        assert!(!app.viewport.typewriter);
        assert_eq!(app.viewport.top_padding, 0);
    }

    #[test]
    fn config_width_idle_and_startup_hint() {
        let doc = Document::parse("", crate::editor::document::FileMeta::new(None));
        let config = Config {
            width: 50,
            autosave_idle: Duration::from_millis(500),
            ..Config::default()
        };
        let mut app = App::with_config(doc, Size::new(120, 40), &config);
        assert_eq!(app.layout().body.width, 50);
        assert_eq!(app.autosave_idle, Duration::from_millis(500));
        let t0 = Instant::now();
        app.show_startup_hint(t0);
        assert_eq!(app.next_deadline(), Some(t0 + HINT_TTL));
        app.tick(t0 + HINT_TTL);
        assert_eq!(app.status.hint_until, None);
    }

    #[test]
    fn windows_batch_paste_detection() {
        let typed = |s: &str| {
            s.chars()
                .map(|c| press(KeyCode::Char(c)))
                .collect::<Vec<_>>()
        };
        // 15 个字符：仍按逐字输入
        let short = typed("123456789012345");
        assert_eq!(coalesce_paste(short.clone()), short);
        // 16 个字符（含回车，夹杂 Release）：合并为一次粘贴
        let mut long = typed("第一行abc");
        long.push(press(KeyCode::Enter));
        long.push(release(KeyCode::Enter));
        long.extend(typed("第二行defghi"));
        assert_eq!(
            coalesce_paste(long),
            vec![Event::Paste("第一行abc\n第二行defghi".into())]
        );
        // 中间被其他按键打断：前后分别判断
        let mut mixed = typed("abc");
        mixed.push(press(KeyCode::Left));
        mixed.extend(typed("0123456789abcdef"));
        let out = coalesce_paste(mixed);
        assert_eq!(out.len(), 5);
        assert_eq!(out[4], Event::Paste("0123456789abcdef".into()));
        // Ctrl 组合键不是输入
        let ctrl_s = Event::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert_eq!(coalesce_paste(vec![ctrl_s.clone()]), vec![ctrl_s]);
    }
}
