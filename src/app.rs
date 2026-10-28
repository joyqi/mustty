//! App 状态与同步事件循环（方案 §11.3）。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;
use ratatui::layout::{Rect, Size};

use crate::action::{Action, Motion};
use crate::editor::document::Document;
use crate::editor::{Editor, Effect};
use crate::io::file::{self, IoError};
use crate::keymap::{Keymap, Mode};
use crate::status::{Prompt, StatusModel};
use crate::view;
use crate::view::layout::{Layout, MEASURE, compute_layout};
use crate::view::viewport::Viewport;

/// 最后一次编辑后空闲多久自动保存。
pub const AUTOSAVE_IDLE: Duration = Duration::from_secs(2);
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
    autosave: bool,
    /// 大文件：字数只在保存时刷新
    large: bool,
    /// 最后一次改动内容的时刻；自动保存尝试后清空，避免失败时反复重试
    last_edit: Option<Instant>,
    /// 有未保存修改时按过一次 Ctrl+Q
    quit_armed: bool,
    /// 待写给终端的 OSC 52 序列（由事件循环写出）
    osc52: Vec<String>,
    /// 终端支持 OSC 52（macOS Terminal 不支持）
    osc52_supported: bool,
    osc52_warned: bool,
    pub should_quit: bool,
    pub needs_redraw: bool,
}

impl App {
    pub fn new(doc: Document, size: Size, autosave: bool) -> Self {
        let area = Rect::new(0, 0, size.width, size.height);
        let large = doc.body().len_bytes() > LARGE_FILE;
        let editor = Editor::new(doc, compute_layout(area, MEASURE).body.width);
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
            keymap: Keymap { vim: false },
            mode: Mode::Insert,
            status,
            viewport: Viewport::default(),
            area,
            autosave,
            large,
            last_edit: None,
            quit_armed: false,
            osc52: Vec::new(),
            osc52_supported: std::env::var_os("TERM_PROGRAM").is_none_or(|t| t != "Apple_Terminal"),
            osc52_warned: false,
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
        if let Some(action) = self.keymap.map_event(&ev, &mut self.mode, prompt_active) {
            self.dispatch(action, now);
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
            self.osc52.push(crate::terminal::osc52(text));
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
        std::mem::take(&mut self.osc52)
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
            .map(|t| t + AUTOSAVE_IDLE)
    }

    /// 下一次需要醒来的时刻：自动保存与消息过期中较早者。
    pub fn next_deadline(&self) -> Option<Instant> {
        [self.autosave_deadline(), self.status.expiry()]
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
    use crossterm::event::KeyEventState;

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
