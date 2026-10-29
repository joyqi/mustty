//! App 状态与同步事件循环。

use std::io::Write;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::Command;
use crossterm::cursor::SetCursorStyle;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent};
use ratatui::DefaultTerminal;
use ratatui::layout::{Rect, Size};

use crate::action::{Action, Motion, Target};
use crate::config::{self, Config};
use crate::editor::cursor::Cursor;
use crate::editor::document::Document;
use crate::editor::{Editor, Effect};
use crate::help::{self, Help};
use crate::i18n::{t, t_args};
use crate::io::file::{self, IoError};
use crate::keymap::{Keymap, Mode};
use crate::settings::{Outcome, Settings};
use crate::status::{Prompt, StatusModel};
use crate::view;
use crate::view::layout::{Layout, compute_layout};
use crate::view::mouse::{DRAG_SCROLL_INTERVAL, DragState, map_mouse};
use crate::view::screen::ScreenMap;
use crate::view::viewport::Viewport;

/// 默认：最后一次编辑后空闲多久自动保存（配置项 autosave_idle_ms）。
pub const AUTOSAVE_IDLE: Duration = Duration::from_secs(2);
/// 启动键位提示的显示时长。
pub const HINT_TTL: Duration = Duration::from_secs(5);
/// Typewriter / Focus 模式切换提示的显示时长。
pub const TOGGLE_TTL: Duration = Duration::from_secs(2);
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
    /// 当前生效的配置，唯一的真值来源；只经 `apply_config` 修改。
    /// 它的一部分还复制在子对象里：Keymap（vim）、Viewport（typewriter、scrolloff）、
    /// Editor 的 WrapCache（tab_width，Tab 键插入的空格数也从这里取），由 `apply_config` 同步
    pub(crate) config: Config,
    /// 设置面板写回的配置文件；None 时只在本次会话生效（测试默认如此，不会碰到真实的 ~/.mustty.toml）
    config_path: Option<PathBuf>,
    /// 设置面板（Ctrl+P）；打开时吞掉所有按键
    pub(crate) settings: Option<Settings>,
    /// 键位帮助（Ctrl+G / vim Normal 态 `?`）；同样吞掉所有按键
    pub(crate) help: Option<Help>,
    /// 待执行的鼠标捕获开关，由事件循环用 `execute!` 执行（Windows 上走 WinAPI）
    mouse_capture: Option<bool>,
    /// 左键拖拽中（仅鼠标开启时）
    drag: Option<DragState>,
    /// 上一帧渲染输出的屏幕 ↔ 文档坐标换算，鼠标只查它
    pub(crate) screen: Option<ScreenMap>,
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
    /// 最后一次写给终端的光标形状序列（去重）；启动时 TerminalGuard 已设成竖线
    cursor_sent: String,
    /// 已写给终端的窗口标题（去重）
    window_title: crate::terminal::WindowTitle,
    /// 「保存为」的相对路径以它为基准，预填的 untitled 名也在这里避让重名；默认是启动时的工作目录
    pub save_dir: PathBuf,
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

    /// 先按默认配置搭好，再经 `apply_config` 应用 `config`：初始化与运行中修改走同一条路径。
    pub fn with_config(doc: Document, size: Size, config: &Config) -> Self {
        let area = Rect::new(0, 0, size.width, size.height);
        let large = doc.body().len_bytes() > LARGE_FILE;
        let base = Config::default();
        let editor = Editor::new(
            doc,
            compute_layout(area, base.width).body.width,
            base.tab_width,
        );
        let mut status = StatusModel::default();
        if large {
            status.info(t("msg-large-file"), Instant::now(), INFO_TTL);
        }
        status.words = editor.word_count();
        let mut app = Self {
            editor,
            keymap: Keymap { vim: base.vim },
            mode: Mode::Insert,
            status,
            viewport: Viewport {
                typewriter: base.typewriter,
                scrolloff: base.scrolloff,
                ..Viewport::default()
            },
            area,
            config: base,
            config_path: None,
            settings: None,
            help: None,
            mouse_capture: None,
            drag: None,
            screen: None,
            large,
            last_edit: None,
            quit_armed: false,
            output: Vec::new(),
            osc52_supported: std::env::var_os("TERM_PROGRAM").is_none_or(|t| t != "Apple_Terminal"),
            osc52_warned: false,
            cursor_sent: String::new(),
            window_title: Default::default(),
            save_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            should_quit: false,
            needs_redraw: true,
        };
        app.apply_config(config);
        // 启动时的终端状态（鼠标捕获、竖线光标）已由 TerminalGuard 设好，不再重复输出；
        // 配置的光标形状不是竖线时，再推一次
        app.output.clear();
        app.mouse_capture = None;
        app.cursor_sent = ansi(SetCursorStyle::SteadyBar);
        app.sync_cursor_style();
        app
    }

    /// 应用一份配置：逐项与当前值比较，把差异分发到各自的存放位置。
    /// 启动（`with_config`）和设置面板的每次调整都只走这里。
    pub fn apply_config(&mut self, new: &Config) {
        self.apply_config_at(new, Instant::now());
    }

    fn apply_config_at(&mut self, new: &Config, now: Instant) {
        let old = std::mem::replace(&mut self.config, new.clone());
        self.needs_redraw = true;
        // width：sync_body 发现正文宽度变了会 rewrap，这里不用做什么
        self.viewport.typewriter = new.typewriter;
        self.viewport.scrolloff = new.scrolloff;
        if new.tab_width != old.tab_width {
            // tab_width 的另一份副本：换行、渲染与 Tab 键插入的空格数共用 WrapCache 里这一份
            let (doc, wrap, _) = self.editor.view();
            self.viewport.retab(new.tab_width, wrap, doc.body());
        }
        if new.typewriter != old.typewriter || new.width != old.width {
            let layout = self.layout();
            if !layout.too_small {
                self.sync_body(layout.body);
            }
        }
        if new.lang != old.lang {
            // 下一帧起所有文案（含面板自己）都用新语言；Auto 时按环境变量重新协商
            crate::i18n::use_config(new.lang);
        }
        self.keymap.vim = new.vim;
        if old.vim && !new.vim {
            self.leave_vim(now);
        }
        if new.mouse != old.mouse {
            self.drag = None;
            self.mouse_capture = Some(new.mouse);
        }
        // 从关切到开：有未保存修改时从现在起计时，空闲后照常自动保存
        if new.autosave && !old.autosave && self.editor.is_dirty() {
            self.last_edit = Some(now);
        }
        // cursor / cursor_blink 变了（面板里即时预览），或 vim 关掉后回到 Insert
        self.sync_cursor_style();
    }

    /// vim 被关掉：Normal 态强制回 Insert（光标换回配置的形状），清掉 pending / visual 与选区。
    fn leave_vim(&mut self, now: Instant) {
        if matches!(self.mode, Mode::Insert) {
            return;
        }
        self.mode = Mode::Insert;
        if self.editor.selection().is_some() {
            self.editor.apply(&Action::Cancel, 1, now);
        }
    }

    /// 当前应有的光标形状。vim Normal 态（含 visual）始终是方块：那是「命令模式」的信号，不是偏好；
    /// 设置面板打开时一律用配置的形状，好在面板里预览；其余用配置的形状。
    pub(crate) fn cursor_style(&self) -> SetCursorStyle {
        if self.settings.is_none() && matches!(self.mode, Mode::Normal(_)) {
            SetCursorStyle::SteadyBlock
        } else {
            self.config.cursor.style(self.config.cursor_blink)
        }
    }

    /// 光标形状的唯一推送点：与上次写出的不同时才推序列。每个事件处理完、每次应用配置后调用。
    fn sync_cursor_style(&mut self) {
        let seq = ansi(self.cursor_style());
        if seq != self.cursor_sent {
            self.output.push(seq.clone());
            self.cursor_sent = seq;
        }
    }

    /// 设置面板按 Enter 时写回的配置文件。
    pub fn set_config_path(&mut self, path: Option<PathBuf>) {
        self.config_path = path;
    }

    pub fn layout(&self) -> Layout {
        compute_layout(self.area, self.config.width)
    }

    pub fn measure(&self) -> u16 {
        self.config.width
    }

    /// 启动后 5 s 在 status bar 右段显示键位提示。
    pub fn show_startup_hint(&mut self, now: Instant) {
        self.status.hint_until = Some(now + HINT_TTL);
    }

    /// vim visual 的选区是闭区间：锚点与光标所在的字都在选区内。编辑内核只认半开区间，
    /// 这里在 App 层换算，内核不感知 Mode。
    fn visual_range(&self) -> Option<Range<usize>> {
        if self.editor.focus != crate::editor::Focus::Body {
            return None;
        }
        let c = self.editor.cursor;
        let anchor = c.anchor?;
        let (lo, hi) = (anchor.min(c.head), anchor.max(c.head));
        Some(lo..Cursor::next(self.editor.doc().body(), hi).max(lo))
    }

    /// 渲染用的选区：vim visual 下为闭区间，其余为编辑内核的选区。
    pub(crate) fn visible_selection(&self) -> Option<Range<usize>> {
        match self.mode {
            Mode::Normal(s) if s.visual => self.visual_range(),
            _ => self.editor.selection(),
        }
    }

    /// 配置文件的警告：一条 status bar 错误消息，保留到第一次按键。
    pub fn show_startup_warning(&mut self, text: String) {
        self.status.error(text);
    }

    /// 模式标识及其样式（`theme::mode`）。
    pub(crate) fn mode_indicator(&self) -> Option<(&'static str, ratatui::style::Style)> {
        self.mode_label()
            .map(|label| (label, view::theme::mode(self.mode)))
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
                || t("status-untitled").to_owned(),
                |s| s.to_string_lossy().into_owned(),
            )
    }

    pub fn handle(&mut self, ev: Event) {
        self.handle_at(ev, Instant::now());
    }

    pub fn handle_at(&mut self, ev: Event, now: Instant) {
        self.handle_event(ev, now);
        // 模式切换、面板开关、配置变化都可能改变光标形状
        self.sync_cursor_style();
    }

    fn handle_event(&mut self, ev: Event, now: Instant) {
        match ev {
            Event::Resize(w, h) => {
                // 同一批里的多个 Resize 只记下尺寸，重排在下一次绘制时进行一次
                self.area = Rect::new(0, 0, w, h);
                self.needs_redraw = true;
                return;
            }
            Event::FocusLost => {
                if self.editor.is_dirty() {
                    self.autosave_now();
                }
                return;
            }
            Event::Mouse(m) => {
                // 面板不做点击 hit-test
                if self.config.mouse && self.settings.is_none() && self.help.is_none() {
                    self.on_mouse(m, now);
                }
                return;
            }
            _ => {}
        }
        // 抢占输入的浮层同一时刻最多一个：提示行打开时 Ctrl+P / Ctrl+G 进了 PromptInput，
        // 面板打开时其他面板的快捷键与 Ctrl+S 都被吞掉
        debug_assert!(
            [
                self.settings.is_some(),
                self.help.is_some(),
                self.status.prompt.is_some()
            ]
            .iter()
            .filter(|&&open| open)
            .count()
                <= 1
        );
        if self.help.is_some() {
            if let Event::Key(key) = ev {
                self.help_key(key, now);
            }
            return;
        }
        if self.settings.is_some() {
            // 面板吞掉所有按键，不经过 Keymap，mode 保持原样；粘贴丢弃
            if let Event::Key(key) = ev {
                self.settings_key(key, now);
            }
            return;
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
        let is_visual = matches!(self.mode, Mode::Normal(s) if s.visual);
        let left_visual = was_visual && !is_visual;
        if before != after || self.mode_label().is_some() {
            self.needs_redraw = true;
        }
        // 进入 visual：选区从光标所在的字开始
        if is_visual && !was_visual {
            let head = self.editor.cursor.head;
            self.edit(&Action::MoveTo(Target::Body(head), true), now);
        }
        // visual 下的 d / x / y / p：先把选区换成闭区间（含光标所在的字），再交给编辑内核
        if was_visual
            && matches!(
                action,
                Some(Action::Cut | Action::Copy | Action::PasteRegister)
            )
            && let Some(range) = self.visual_range()
        {
            self.edit(&Action::MoveTo(Target::Body(range.end), false), now);
            self.edit(&Action::MoveTo(Target::Body(range.start), true), now);
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
            Action::Quit => self.quit(armed),
            Action::Cancel if self.status.prompt.is_some() => self.status.prompt = None,
            Action::Save => match self.editor.doc().meta.path.clone() {
                Some(path) => {
                    self.save(path, false);
                }
                None => {
                    let name = file::untitled_name(&self.save_dir);
                    self.status.prompt = Some(Prompt::with_input(name));
                }
            },
            Action::PromptInput(key) => self.prompt_input(key),
            Action::ScrollView(delta) => self.scroll_view(delta, now),
            Action::ToggleTypewriter => {
                let on = !self.config.typewriter;
                self.apply_config_at(
                    &Config {
                        typewriter: on,
                        ..self.config.clone()
                    },
                    now,
                );
                let msg = t_args("msg-typewriter", &[("state", on_off(on))]);
                self.status.info(msg, now, TOGGLE_TTL);
            }
            Action::ToggleFocusMode => {
                let on = !self.config.focus_mode;
                self.apply_config_at(
                    &Config {
                        focus_mode: on,
                        ..self.config.clone()
                    },
                    now,
                );
                let msg = t_args("msg-focus", &[("state", on_off(on))]);
                self.status.info(msg, now, TOGGLE_TTL);
            }
            Action::OpenSettings => self.settings = Some(Settings::open(&self.config)),
            Action::OpenHelp => self.help = Some(Help::default()),
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

    /// 帮助面板的内容行（随 vim 开关变化）。
    pub(crate) fn help_lines(&self) -> Vec<help::Line> {
        help::lines(self.config.vim)
    }

    fn help_key(&mut self, key: KeyEvent, now: Instant) {
        let total = self.help_lines().len();
        let visible = self.layout().body.height.max(1) as usize;
        let Some(panel) = &mut self.help else {
            return;
        };
        match panel.handle_key(&key, self.config.vim, total, visible) {
            help::Outcome::Ignored => {}
            help::Outcome::Scrolled => self.needs_redraw = true,
            help::Outcome::Close => {
                self.help = None;
                self.needs_redraw = true;
            }
            help::Outcome::Quit => {
                self.help = None;
                self.dispatch(Action::Quit, now);
            }
        }
    }

    fn settings_key(&mut self, key: KeyEvent, now: Instant) {
        let Some(settings) = &mut self.settings else {
            return;
        };
        let outcome = settings.handle_key(&key, &self.config);
        if outcome != Outcome::Ignored {
            self.needs_redraw = true;
            self.status.on_input();
        }
        match outcome {
            Outcome::Ignored | Outcome::Moved => {}
            Outcome::Changed(config) => self.apply_config_at(&config, now),
            Outcome::Save => {
                let snapshot = self.settings.take().map(|s| s.snapshot);
                if let (Some(snapshot), Some(path)) = (snapshot, self.config_path.clone()) {
                    self.write_settings(&path, &snapshot);
                }
            }
            Outcome::Cancel => self.close_settings(now),
            Outcome::Quit => {
                self.close_settings(now);
                self.dispatch(Action::Quit, now);
            }
        }
    }

    /// Esc：恢复打开面板时的快照并关闭，不写文件。
    fn close_settings(&mut self, now: Instant) {
        if let Some(settings) = self.settings.take() {
            self.apply_config_at(&settings.snapshot, now);
        }
    }

    /// Enter：只写与快照不同的键。失败时修改仍在本次会话生效。
    fn write_settings(&mut self, path: &Path, snapshot: &Config) {
        match config::write_back(path, snapshot, &self.config) {
            Ok(()) => {}
            Err(config::WriteBackError::Syntax) => {
                self.status.error(t("settings-syntax-error"));
            }
            Err(config::WriteBackError::Io(e)) => {
                self.status.error(t_args(
                    "settings-write-failed",
                    &[("error", &e.to_string())],
                ));
            }
        }
    }

    /// 取出待执行的鼠标捕获开关。
    pub fn take_mouse_capture(&mut self) -> Option<bool> {
        self.mouse_capture.take()
    }

    fn quit(&mut self, armed: bool) {
        if armed || !self.editor.is_dirty() || (self.can_autosave() && self.autosave_now()) {
            self.should_quit = true;
        } else {
            self.quit_armed = true;
            self.status.error(t("msg-quit-unsaved"));
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
            self.status.info(t("msg-osc52-unsupported"), now, INFO_TTL);
        }
    }

    /// 窗口标题（含未保存标记）变了时返回要写的 OSC 2 序列（每轮事件循环查一次；没变就是 None）。
    pub fn take_window_title(&mut self) -> Option<String> {
        let doc = self.editor.doc();
        // 未保存标记与 status bar 的 SaveMark 同源：都直接取 Editor::is_dirty()。
        // 去重比较的是最终字符串：标题文字没变、只有保存状态变了，也会推送
        let title =
            StatusModel::window_title(&doc.title, doc.meta.path.as_deref(), self.editor.is_dirty());
        self.window_title.update(&title)
    }

    /// 取出待写给终端的控制序列。
    pub fn take_terminal_output(&mut self) -> Vec<String> {
        std::mem::take(&mut self.output)
    }

    fn prompt_input(&mut self, key: KeyEvent) {
        let Some(prompt) = &mut self.status.prompt else {
            return;
        };
        if !prompt.edit(key) || prompt.input.is_empty() {
            return;
        }
        // 相对 save_dir（启动时的 cwd），不做 ~ 展开；绝对路径原样
        let path = self.save_dir.join(&prompt.input);
        if path.exists() && !prompt.confirm_overwrite {
            prompt.confirm_overwrite = true;
            return;
        }
        self.status.prompt = None;
        self.save(path, false);
    }

    fn can_autosave(&self) -> bool {
        self.config.autosave
            && self.editor.doc().meta.path.is_some()
            && !self.status.externally_modified
    }

    /// 立即自动保存（检查外部修改）；返回是否成功。
    fn autosave_now(&mut self) -> bool {
        self.last_edit = None;
        match self.editor.doc().meta.path.clone() {
            Some(path) if self.can_autosave() => self.save(path, true),
            _ => false,
        }
    }

    fn save(&mut self, path: PathBuf, auto: bool) -> bool {
        self.needs_redraw = true;
        match file::save(self.editor.doc(), &path, auto) {
            Ok(mtime) => {
                self.editor.mark_saved(path, mtime);
                // 保存状态由文件名后的标记表示，不再显示「已保存」文案
                self.status.externally_modified = false;
                if self.large {
                    self.status.words = self.editor.word_count();
                }
                true
            }
            Err(IoError::ExternallyModified) => {
                self.status.externally_modified = true;
                false
            }
            Err(e) => {
                self.status
                    .error(t_args("msg-save-failed", &[("error", &e.to_string())]));
                false
            }
        }
    }

    fn autosave_deadline(&self) -> Option<Instant> {
        self.last_edit
            .filter(|_| self.can_autosave() && self.editor.is_dirty())
            .map(|t| t + self.config.autosave_idle)
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
            self.autosave_now();
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

    /// 鼠标：由 `view::mouse::map_mouse` 转成 Action，与键盘共用同一条编辑路径。
    fn on_mouse(&mut self, m: MouseEvent, now: Instant) {
        if self.status.prompt.is_some() {
            return;
        }
        // 只查上一帧渲染给出的换算；还没渲染过（或窗口太小）时忽略
        let Some(map) = self.screen else { return };
        let title = self.editor.doc().title_text().to_owned();
        let (doc, wrap, _) = self.editor.view();
        let action = map_mouse(&m, &map, wrap, doc.body(), &title, &mut self.drag, now);
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
        // 定位可能让视口跟随滚动：同一批里后续的鼠标事件按新的视口查表
        if let Some(map) = &mut self.screen {
            map.follow(&self.viewport);
        }
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
        let (Some(state), Some(mut map)) = (self.drag, self.screen) else {
            return;
        };
        if state.deadline.is_none_or(|d| now < d) {
            return;
        }
        let Some(delta) = map.drag_overflow(state.last) else {
            return;
        };
        let (doc, wrap, _) = self.editor.view();
        self.viewport.scroll(delta, wrap, doc.body());
        map.follow(&self.viewport);
        let at = map.drag_char(state.last, wrap, doc.body());
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

fn on_off(on: bool) -> &'static str {
    if on { t("msg-on") } else { t("msg-off") }
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
/// 光标形状的转义序列（DECSCUSR）。
fn ansi(style: SetCursorStyle) -> String {
    let mut seq = String::new();
    let _ = style.write_ansi(&mut seq);
    seq
}

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
    loop {
        if let Some(on) = app.take_mouse_capture() {
            crate::terminal::set_mouse_capture(terminal.backend_mut(), on)?;
        }
        let mut output = app.take_terminal_output();
        // 窗口标题（OSC 2）同样直接写 stdout：不移动光标，不经过 ratatui 的 Buffer
        output.extend(app.take_window_title());
        if !output.is_empty() {
            let backend = terminal.backend_mut();
            for seq in output {
                backend.write_all(seq.as_bytes())?;
            }
            backend.flush()?;
        }
        if app.needs_redraw {
            terminal.draw(|frame| view::render(frame, app))?;
            app.needs_redraw = false;
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
        for ev in batch {
            app.handle(ev);
        }
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
        // 保存成功不再显示文案，也就没有需要醒来的定时器
        assert_eq!(
            (app.status.message.clone(), app.next_deadline()),
            (None, None)
        );

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
    fn tab_key_follows_configured_tab_width() {
        let doc = Document::parse("# T\n\nx", crate::editor::document::FileMeta::new(None));
        let config = Config {
            tab_width: 2,
            autosave: false,
            ..Config::default()
        };
        let mut app = App::with_config(doc, Size::new(80, 24), &config);
        app.handle(press(KeyCode::Tab));
        assert_eq!(body(&app), "  x", "tab_width = 2 插两个空格");
        app.apply_config(&Config {
            tab_width: 8,
            ..app.config.clone()
        });
        app.handle(press(KeyCode::Tab));
        assert_eq!(body(&app), format!("{}x", " ".repeat(10)));
        // ↑ 进标题区，Tab 回到正文开头，标题不变；Shift+Tab 什么也不做
        app.handle(press(KeyCode::Up));
        app.handle(press(KeyCode::Up));
        app.handle(Event::Key(KeyEvent::new(
            KeyCode::BackTab,
            KeyModifiers::SHIFT,
        )));
        app.handle(press(KeyCode::Tab));
        assert_eq!(app.editor.doc().title_text(), "T");
        assert_eq!(app.editor.focus, crate::editor::Focus::Body);
        assert_eq!(app.editor.cursor.head, 0);
        assert_eq!(body(&app), format!("{}x", " ".repeat(10)));
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
        // M4 验收：i 输入 → Esc → gg → dd → G → p
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
        // visual 是闭区间：v 之后光标所在的字立即被选中
        typed(&mut app, "v");
        assert_eq!(
            (app.mode_label(), app.visible_selection()),
            (Some("VIS"), Some(0..1))
        );
        // v e d：删掉整个词，包括最后一个字母
        typed(&mut app, "ed");
        assert_eq!(
            (body(&app).as_str(), app.mode_label()),
            (" world foo", Some("NOR"))
        );
        assert_eq!(app.editor.cursor.head, 0);
        // v w d：删到下一个词首（含词首这个字）
        let mut app = vim_app("hello world foo");
        typed(&mut app, "\x1bvw");
        assert_eq!(
            app.visible_selection(),
            Some(0..7),
            "渲染出的选区含光标所在的字"
        );
        typed(&mut app, "d");
        assert_eq!(body(&app), "orld foo");
        // v e y 复制整个词，选区消失、光标回到选区开头；p 粘贴到光标处
        typed(&mut app, "vey");
        assert_eq!(
            (
                app.editor.register().unwrap().text.as_str(),
                app.editor.selection()
            ),
            ("orld", None)
        );
        assert_eq!(app.editor.cursor.head, 0);
        typed(&mut app, "p");
        assert_eq!(body(&app), "orldorld foo");
        // 反向选择（光标在锚点之前）同样包含两端
        let mut app = vim_app("abcdef");
        typed(&mut app, "\x1blllvhhd"); // 锚点在「d」，光标退到「b」
        assert_eq!(body(&app), "aef");
        // 单独 v 再 x：删掉光标所在的一个字
        let mut app = vim_app("中文");
        typed(&mut app, "\x1bvx");
        assert_eq!(body(&app), "文");
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

    /// 选到设置面板里的某一项。
    fn select_field(app: &mut App, field: crate::settings::Field) {
        let all = crate::settings::Field::ALL;
        let want = all.iter().position(|&f| f == field).unwrap();
        loop {
            let at = app.settings.as_ref().unwrap().selected;
            match at.cmp(&want) {
                std::cmp::Ordering::Less => app.handle(press(KeyCode::Down)),
                std::cmp::Ordering::Greater => app.handle(press(KeyCode::Up)),
                std::cmp::Ordering::Equal => return,
            }
        }
    }

    #[test]
    fn configured_cursor_shape_and_vim_normal_block() {
        use crate::config::CursorShape;
        let doc = || Document::parse("x", crate::editor::document::FileMeta::new(None));
        // 启动：配置不是竖线时推一次（竖线由 TerminalGuard 设好，不重复）
        let plain = App::new(doc(), Size::new(80, 24), false);
        assert!(plain.output.is_empty());
        let config = Config {
            vim: true,
            autosave: false,
            cursor: CursorShape::Underline,
            ..Config::default()
        };
        let mut app = App::with_config(doc(), Size::new(80, 24), &config);
        assert_eq!(app.take_terminal_output(), vec!["\x1b[4 q".to_owned()]);
        // Normal（含 visual）始终 SteadyBlock，不受配置影响
        typed(&mut app, "\x1b");
        assert_eq!(app.take_terminal_output(), vec!["\x1b[2 q".to_owned()]);
        typed(&mut app, "v");
        assert!(app.take_terminal_output().is_empty(), "visual 同 Normal");
        typed(&mut app, "\x1bi");
        assert_eq!(
            app.take_terminal_output(),
            vec!["\x1b[4 q".to_owned()],
            "回 Insert：配置的下划线"
        );
        // 闪烁
        app.apply_config(&Config {
            cursor_blink: true,
            ..app.config.clone()
        });
        assert_eq!(app.take_terminal_output(), vec!["\x1b[3 q".to_owned()]);
        // 关掉 vim 时正在 Normal：回到配置的形状（不是写死的竖线）
        typed(&mut app, "\x1b");
        app.take_terminal_output();
        app.apply_config(&Config {
            vim: false,
            ..app.config.clone()
        });
        assert_eq!(app.take_terminal_output(), vec!["\x1b[3 q".to_owned()]);
    }

    #[test]
    fn settings_panel_previews_cursor_shape() {
        use crate::config::CursorShape;
        use crate::settings::Field;
        let mut app = vim_app("hello");
        typed(&mut app, "\x1b");
        assert_eq!(app.take_terminal_output(), vec!["\x1b[2 q".to_owned()]);
        // 在 Normal 里打开面板：预览配置的形状（竖线），而不是 Normal 的方块
        app.handle(ctrl('p'));
        assert_eq!(app.take_terminal_output(), vec!["\x1b[6 q".to_owned()]);
        select_field(&mut app, Field::Cursor);
        assert!(app.take_terminal_output().is_empty(), "移动选中项不推");
        // 每改一次立刻推新形状，不等关面板
        let mut seen = Vec::new();
        for _ in 0..3 {
            app.handle(press(KeyCode::Right));
            seen.push((app.config.cursor, app.take_terminal_output()));
        }
        assert_eq!(
            seen,
            vec![
                (CursorShape::Block, vec!["\x1b[2 q".to_owned()]),
                (CursorShape::Underline, vec!["\x1b[4 q".to_owned()]),
                (CursorShape::Bar, vec!["\x1b[6 q".to_owned()]),
            ]
        );
        app.handle(press(KeyCode::Right));
        app.take_terminal_output();
        select_field(&mut app, Field::CursorBlink);
        app.handle(press(KeyCode::Char(' ')));
        assert!(app.config.cursor_blink);
        assert_eq!(app.take_terminal_output(), vec!["\x1b[1 q".to_owned()]);
        // 面板里关掉 vim（Normal 被强制回 Insert）：模式切换不覆盖预览形状
        select_field(&mut app, Field::Vim);
        app.handle(press(KeyCode::Char(' ')));
        assert!(!app.config.vim);
        assert_eq!(app.mode, Mode::Insert);
        assert!(
            app.take_terminal_output().is_empty(),
            "仍是预览中的闪烁方块"
        );
        // Esc 回滚：形状回到快照里的常亮竖线（此时在 Insert）
        app.handle(press(KeyCode::Esc));
        assert!(app.settings.is_none());
        assert!(app.config.vim);
        assert_eq!(app.config.cursor, CursorShape::Bar);
        assert_eq!(app.take_terminal_output(), vec!["\x1b[6 q".to_owned()]);
        // Normal 里再开面板改成 underline、Enter 保留：回到 Normal 后仍是方块，进 Insert 才是下划线
        typed(&mut app, "\x1b");
        assert_eq!(app.take_terminal_output(), vec!["\x1b[2 q".to_owned()]);
        app.handle(ctrl('p'));
        assert_eq!(app.take_terminal_output(), vec!["\x1b[6 q".to_owned()]);
        select_field(&mut app, Field::Cursor);
        app.handle(press(KeyCode::Left));
        assert_eq!(app.config.cursor, CursorShape::Underline);
        assert_eq!(app.take_terminal_output(), vec!["\x1b[4 q".to_owned()]);
        app.handle(press(KeyCode::Enter));
        assert_eq!(app.take_terminal_output(), vec!["\x1b[2 q".to_owned()]);
        typed(&mut app, "i");
        assert_eq!(app.take_terminal_output(), vec!["\x1b[4 q".to_owned()]);
    }

    fn mouse(kind: MouseEventKind, column: u16, row: u16) -> Event {
        Event::Mouse(MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        })
    }

    /// 与事件循环一样：先绘制一帧（产出 ScreenMap），再处理下一个事件。
    fn rendered(app: &mut App, ev: Event, now: Instant) {
        let size = app.area;
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(size.width, size.height))
                .unwrap();
        terminal.draw(|f| view::render(f, app)).unwrap();
        app.handle_at(ev, now);
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
        rendered(
            &mut app,
            mouse(MouseEventKind::Down(left), 4 + 3, 5 + 2),
            t0,
        );
        assert_eq!(app.editor.cursor.head, 2 * 10 + 2);
        // 宽字符右半格吸附左侧
        rendered(
            &mut app,
            mouse(MouseEventKind::Down(left), 4 + 1, 5 + 2),
            t0,
        );
        assert_eq!(app.editor.cursor.head, 2 * 10);
        rendered(
            &mut app,
            mouse(MouseEventKind::Down(left), 4 + 3, 5 + 2),
            t0,
        );
        // 点在正文之外：忽略
        rendered(&mut app, mouse(MouseEventKind::Down(left), 1, 7), t0);
        rendered(&mut app, mouse(MouseEventKind::Down(left), 4, 22), t0);
        assert_eq!(app.editor.cursor.head, 2 * 10 + 2);
        rendered(
            &mut app,
            mouse(MouseEventKind::Down(left), 4 + 3, 5 + 2),
            t0,
        );
        // 拖拽形成选区
        rendered(&mut app, mouse(MouseEventKind::Drag(left), 4, 5 + 4), t0);
        assert_eq!(app.editor.selection(), Some(2 * 10 + 2..4 * 10));
        // 拖到正文下方并保持不动：每 50 ms 滚动一行，选区跟着延伸；松开后停止
        let top = app.viewport.top.line;
        rendered(&mut app, mouse(MouseEventKind::Drag(left), 10, 23), t0);
        assert_eq!(app.next_deadline(), Some(t0 + DRAG_SCROLL_INTERVAL));
        for k in 1..=10 {
            app.tick(t0 + DRAG_SCROLL_INTERVAL * k);
        }
        assert_eq!(app.viewport.top.line, top + 10, "持续滚动");
        let sel = app.editor.selection().unwrap();
        assert_eq!(sel.start, 2 * 10 + 2);
        assert!(sel.end > 20 * 10, "{sel:?}");
        rendered(&mut app, mouse(MouseEventKind::Up(left), 10, 23), t0);
        app.tick(t0 + DRAG_SCROLL_INTERVAL * 20);
        assert_eq!(app.viewport.top.line, top + 10, "松开后停止");
        // 滚轮只移动视口；光标离开 scrolloff 区间时被拉回边缘，始终可见
        let line = |app: &App| app.editor.doc().body().char_to_line(app.editor.cursor.head);
        let (top, cursor_line) = (app.viewport.top.line, line(&app));
        rendered(&mut app, mouse(MouseEventKind::ScrollDown, 10, 10), t0);
        assert_eq!(
            (app.viewport.top.line, line(&app)),
            (top + 3, cursor_line),
            "光标仍在区间内：不动"
        );
        for _ in 0..10 {
            rendered(&mut app, mouse(MouseEventKind::ScrollDown, 10, 10), t0);
        }
        assert_eq!(app.viewport.top.line, top + 33);
        assert_eq!(line(&app), top + 33 + 3, "被拉到上边缘（scrolloff 3）");
        // 鼠标关闭：完全忽略鼠标事件
        let mut off = mouse_app(&text, false);
        rendered(
            &mut off,
            mouse(MouseEventKind::Down(left), 4 + 3, 5 + 2),
            Instant::now(),
        );
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
        rendered(
            &mut app,
            mouse(MouseEventKind::Down(MouseButton::Left), 4, 5),
            Instant::now(),
        );
        rendered(
            &mut app,
            mouse(MouseEventKind::Drag(MouseButton::Left), 4 + 6, 5),
            Instant::now(),
        );
        assert_eq!(app.mode_label(), Some("VIS"));
        typed(&mut app, "d");
        assert_eq!(
            (body(&app).as_str(), app.mode_label()),
            // 闭区间：拖到第 6 列，「w」也被删掉
            ("orld\nsecond", Some("NOR"))
        );
    }

    #[test]
    fn click_on_horizontally_scrolled_title() {
        // 80×24：标题行 y = 3、x = 4、宽 72。100 个字母的标题在焦点下、光标在末尾时滚动了 100 + 1 - 72 = 29 列
        let long: String = ('a'..='z').cycle().take(100).collect();
        let mut app = mouse_app(&format!("# {long}\n\nbody"), true);
        app.handle(press(KeyCode::Up));
        assert_eq!(
            (app.editor.focus, app.editor.title_cursor),
            (crate::editor::Focus::Title, 100)
        );
        // 点屏幕上标题的第一格：那里显示的是第 29 个字母
        rendered(
            &mut app,
            mouse(MouseEventKind::Down(MouseButton::Left), 4, 3),
            Instant::now(),
        );
        assert_eq!(app.editor.title_cursor, 29);
        assert_eq!(app.screen.unwrap().title.map(|t| t.scroll), Some(29));
    }

    #[test]
    fn click_from_title_returns_to_body() {
        let mut app = mouse_app("# T\n\nabc", true);
        app.handle(press(KeyCode::Up));
        assert_eq!(app.editor.focus, crate::editor::Focus::Title);
        rendered(
            &mut app,
            mouse(MouseEventKind::Down(MouseButton::Left), 4 + 2, 5),
            Instant::now(),
        );
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
        assert_eq!(app.config.autosave_idle, Duration::from_millis(500));
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

    fn ctrl(c: char) -> Event {
        Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
    }

    /// 窗口标题：启动后一次；只在最终字符串真的变化时再发（编辑、撤销、保存、未保存标记）。
    #[test]
    fn window_title_follows_the_document() {
        crate::i18n::init_for_test("en");
        let osc = |s: &str| Some(format!("\x1b]2;{s}\x07"));
        let mut app = App::new(lf_doc("# Hello\n\nbody"), Size::new(80, 24), false);
        assert_eq!(app.take_window_title(), osc("Hello"));
        assert_eq!(app.take_window_title(), None, "没变就不发");
        // 正文里打字：标题文字没变，但有了未保存修改 → `* `；之后移动不再发
        typed(&mut app, "x");
        assert_eq!(app.take_window_title(), osc("* Hello"));
        typed(&mut app, "y");
        app.handle(press(KeyCode::Down));
        assert_eq!(app.take_window_title(), None);
        // 进入标题、在末尾打字：每个字都是一次变化
        app.handle(ctrl_key(KeyCode::Home));
        app.handle(press(KeyCode::Up));
        app.handle(press(KeyCode::End));
        assert_eq!(app.take_window_title(), None, "只是移到标题里");
        typed(&mut app, "!");
        assert_eq!(app.take_window_title(), osc("* Hello!"));
        typed(&mut app, "?");
        assert_eq!(app.take_window_title(), osc("* Hello!?"));
        // 撤销影响到标题；正文的 xy 还没撤，仍有修改
        app.handle(ctrl('z'));
        assert_eq!(
            app.take_window_title(),
            osc("* Hello"),
            "连续输入合并为一步撤销"
        );
        // 再撤销回到保存点：星号消失
        app.handle(ctrl('z'));
        assert!(!app.editor.is_dirty());
        assert_eq!(app.take_window_title(), osc("Hello"));
        // 控制字符不会进序列
        app.handle(Event::Paste("\x1b]0;x\x07".into()));
        let pasted = app.take_window_title().unwrap_or_default();
        assert_eq!(pasted.matches('\x1b').count(), 1, "{pasted:?}");
        assert_eq!(pasted.matches('\x07').count(), 1, "{pasted:?}");
        assert!(pasted.starts_with("\x1b]2;* "), "{pasted:?}");

        // 没有 H1：Untitled → 打字后 `* Untitled` → 保存为之后是文件名、没有星号
        let dir = tempfile::tempdir().unwrap();
        let untitled = Document::parse("", crate::editor::document::FileMeta::new(None));
        let mut app = App::new(untitled, Size::new(80, 24), false);
        app.save_dir = dir.path().to_path_buf();
        assert_eq!(app.take_window_title(), osc("Untitled"));
        typed(&mut app, "words");
        assert_eq!(app.take_window_title(), osc("* Untitled"));
        app.handle(ctrl('s'));
        app.handle(press(KeyCode::Enter));
        assert_eq!(app.take_window_title(), osc("untitled.md"));
        // Ctrl+S 保存：星号出现又消失
        typed(&mut app, "!");
        assert_eq!(app.take_window_title(), osc("* untitled.md"));
        app.handle(ctrl('s'));
        assert_eq!(app.take_window_title(), osc("untitled.md"));
    }

    /// 标题的 `* ` 与 status bar 的保存标记出自同一个 `Editor::is_dirty()`：任何状态下两边一致。
    #[test]
    fn window_title_star_agrees_with_the_status_bar_mark() {
        use crate::status::SaveMark;
        crate::i18n::init_for_test("en");
        fn check(app: &mut App, label: &str) {
            let ed = &app.editor;
            let mark = SaveMark::of(ed.is_dirty(), ed.doc().meta.stamp.is_some());
            let doc = ed.doc();
            let title =
                StatusModel::window_title(&doc.title, doc.meta.path.as_deref(), ed.is_dirty());
            assert_eq!(
                mark == Some(SaveMark::Dirty),
                title.starts_with("* "),
                "{label}：标记 {mark:?}，标题 {title:?}"
            );
        }
        let dir = tempfile::tempdir().unwrap();
        // 空 Untitled：status bar 无标记，标题也没有星号
        let untitled = Document::parse("", crate::editor::document::FileMeta::new(None));
        let mut app = App::new(untitled, Size::new(80, 24), false);
        app.save_dir = dir.path().to_path_buf();
        check(&mut app, "空 Untitled");
        assert_eq!(app.take_window_title(), Some("\x1b]2;Untitled\x07".into()));
        typed(&mut app, "a");
        check(&mut app, "Untitled 有修改");
        app.handle(ctrl('z'));
        check(&mut app, "Untitled 撤销回空");
        typed(&mut app, "b");
        app.handle(ctrl('s'));
        app.handle(press(KeyCode::Enter));
        check(&mut app, "保存为之后");
        typed(&mut app, "c");
        check(&mut app, "已保存的文件有修改");
        app.handle(ctrl('z'));
        check(&mut app, "撤销回保存点");
        assert!(!app.editor.is_dirty());
    }

    /// 自动保存完成后星号消失。
    #[test]
    fn window_title_star_clears_after_autosave() {
        crate::i18n::init_for_test("en");
        let osc = |s: &str| Some(format!("\x1b]2;{s}\x07"));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.md");
        std::fs::write(&path, "# Draft\n\nbody\n").unwrap();
        let mut app = App::new(file::load(&path).unwrap(), Size::new(80, 24), true);
        assert_eq!(app.take_window_title(), osc("Draft"));
        let t0 = Instant::now();
        app.handle_at(press(KeyCode::Char('x')), t0);
        assert_eq!(app.take_window_title(), osc("* Draft"));
        app.tick(t0 + AUTOSAVE_IDLE);
        assert!(!app.editor.is_dirty(), "自动保存了");
        assert_eq!(app.take_window_title(), osc("Draft"));
    }

    fn ctrl_key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::CONTROL))
    }

    /// 删掉提示行里预填的文件名。
    fn clear_prompt(app: &mut App) {
        let n = app.status.prompt.as_ref().unwrap().input.chars().count();
        for _ in 0..n {
            app.handle(press(KeyCode::Backspace));
        }
        assert_eq!(app.status.prompt.as_ref().unwrap().input, "");
    }

    /// 无路径文档 Ctrl+S：预填 untitled.md、光标在末尾；回车即保存，下一次避开它。
    #[test]
    fn untitled_save_prefills_a_free_name() {
        crate::i18n::init_for_test("en");
        let dir = tempfile::tempdir().unwrap();
        let untitled = || Document::parse("", crate::editor::document::FileMeta::new(None));
        let mut app = App::new(untitled(), Size::new(80, 24), false);
        app.save_dir = dir.path().to_path_buf();
        typed(&mut app, "hello");
        app.handle(ctrl('s'));
        let prompt = app.status.prompt.as_ref().expect("提示行打开");
        assert_eq!(prompt.input, "untitled.md");
        assert_eq!(prompt.cursor, "untitled.md".len(), "光标在文件名末尾");
        assert!(!prompt.confirm_overwrite);
        app.handle(press(KeyCode::Enter));
        assert!(app.status.prompt.is_none());
        let saved = dir.path().join("untitled.md");
        assert_eq!(std::fs::read_to_string(&saved).unwrap(), "hello");
        assert_eq!(app.editor.doc().meta.path.as_deref(), Some(saved.as_path()));
        assert_eq!(
            StatusModel::file_name(app.editor.doc().meta.path.as_deref()),
            "untitled.md"
        );
        // 有路径之后 Ctrl+S 直接保存，不再弹提示行
        typed(&mut app, "!");
        app.handle(ctrl('s'));
        assert!(app.status.prompt.is_none());
        assert_eq!(std::fs::read_to_string(&saved).unwrap(), "hello!");

        // 第二个无路径文档：untitled.md 已被占用
        let mut app = App::new(untitled(), Size::new(80, 24), false);
        app.save_dir = dir.path().to_path_buf();
        typed(&mut app, "two");
        app.handle(ctrl('s'));
        assert_eq!(app.status.prompt.as_ref().unwrap().input, "untitled-2.md");
        // 改成已存在的名字仍要二次确认覆盖
        clear_prompt(&mut app);
        app.handle(press(KeyCode::Enter));
        assert!(app.status.prompt.is_some(), "空输入回车不保存");
        app.handle(Event::Paste("untitled.md".into()));
        app.handle(press(KeyCode::Enter));
        assert!(app.status.prompt.as_ref().unwrap().confirm_overwrite);
        assert_eq!(std::fs::read_to_string(&saved).unwrap(), "hello!");
        app.handle(press(KeyCode::Enter));
        assert!(app.status.prompt.is_none());
        assert_eq!(std::fs::read_to_string(&saved).unwrap(), "two");
        assert!(!dir.path().join("untitled-2.md").exists());
    }

    fn lf_doc(text: &str) -> Document {
        Document::parse(text, crate::editor::document::FileMeta::lf())
    }

    /// 让 WrapCache 把每一行都重新算一遍，并确认 tab_width 的各处取值一致、缓存与重新换行的结果一致。
    fn assert_tab_width_synced(app: &mut App, tab: u16) {
        assert_eq!(app.config.tab_width, tab, "App 的配置");
        assert_eq!(
            app.editor.tab_spaces(),
            " ".repeat(tab as usize),
            "Tab 键插入的空格数"
        );
        let (doc, wrap, _) = app.editor.view();
        let rope = doc.body();
        assert_eq!(wrap.tab_width(), tab, "WrapCache：换行与渲染");
        for l in 0..rope.len_lines() {
            let rows = wrap.rows(rope, l).to_vec();
            let fresh = crate::text::wrap::wrap_line(
                &crate::text::wrap::line_text(rope, l),
                wrap.width(),
                tab,
            );
            assert_eq!(rows, fresh, "line {l}");
        }
        assert!(wrap.is_consistent(rope, 0..rope.len_lines()));
    }

    #[test]
    fn apply_config_syncs_all_tab_width_copies() {
        let text = "a\tb\tc\td\te\tf\tg\th\ti\tj\tk\tl\tm\tn\to\tp\tq\tr\ts\tt\n\tindented\n";
        let mut app = App::new(lf_doc(text), Size::new(80, 24), false);
        let body = app.layout().body;
        app.sync_body(body);
        assert_tab_width_synced(&mut app, 4);
        app.apply_config(&Config {
            tab_width: 8,
            ..app.config.clone()
        });
        assert_tab_width_synced(&mut app, 8);
        // Tab 键按新的宽度插入空格
        app.handle(press(KeyCode::Tab));
        assert!(body_text(&app).starts_with("        a\t"));
        app.apply_config(&Config {
            tab_width: 1,
            ..app.config.clone()
        });
        assert_tab_width_synced(&mut app, 1);
    }

    fn body_text(app: &App) -> String {
        app.editor.doc().body().to_string()
    }

    proptest::proptest! {
        /// 随机文本（含 Tab）+ 随机改 tab_width：缓存始终与重新换行一致，光标的屏幕位置往返不变。
        #[test]
        fn retab_keeps_cache_consistent(
            text in "[a-z \t中]{0,60}(\n[a-z \t中]{0,60}){0,4}",
            tabs in proptest::collection::vec(1u16..=16, 1..5),
            at in 0usize..200,
        ) {
            let mut app = App::new(lf_doc(&text), Size::new(60, 20), false);
            let len = app.editor.doc().body().len_chars();
            app.editor.apply(&Action::MoveTo(Target::Body(at.min(len)), false), 1, Instant::now());
            for tab in tabs {
                app.apply_config(&Config { tab_width: tab, ..app.config.clone() });
                assert_tab_width_synced(&mut app, tab);
                let head = app.editor.cursor.head;
                let cursor = app.editor.cursor;
                let (doc, wrap, _) = app.editor.view();
                let pos = cursor.visual(doc.body(), wrap);
                let back = wrap.visual_to_char(doc.body(), pos);
                proptest::prop_assert_eq!(back, head);
            }
        }
    }

    #[test]
    fn turning_vim_off_resets_normal_mode() {
        let mut app = vim_app("hello");
        typed(&mut app, "\x1bvl");
        assert_eq!(app.mode_label(), Some("VIS"));
        assert!(app.visible_selection().is_some());
        app.take_terminal_output();
        app.apply_config(&Config {
            vim: false,
            ..app.config.clone()
        });
        assert_eq!(app.mode, Mode::Insert);
        assert_eq!(app.mode_label(), None);
        assert_eq!(app.editor.selection(), None, "选区清掉");
        assert_eq!(
            app.take_terminal_output(),
            vec!["\x1b[6 q".to_owned()],
            "换回竖线光标"
        );
        // 关闭时本来就在 Insert：不推光标序列
        let mut app = vim_app("hello");
        app.apply_config(&Config {
            vim: false,
            ..app.config.clone()
        });
        assert!(app.take_terminal_output().is_empty());
        // 再打开：保持 Insert
        app.apply_config(&Config {
            vim: true,
            ..app.config.clone()
        });
        assert_eq!(app.mode_label(), Some("INS"));
    }

    #[test]
    fn mouse_toggle_requests_capture_change() {
        let mut app = mouse_app("x", false);
        assert_eq!(
            app.take_mouse_capture(),
            None,
            "启动时由 TerminalGuard 处理"
        );
        let on = Config {
            mouse: true,
            ..app.config.clone()
        };
        app.apply_config(&on);
        assert_eq!(app.take_mouse_capture(), Some(true));
        app.apply_config(&on);
        assert_eq!(app.take_mouse_capture(), None, "没有变化不重复发送");
        app.drag = Some(DragState {
            last: ratatui::layout::Position::new(0, 0),
            deadline: None,
        });
        app.apply_config(&Config {
            mouse: false,
            ..app.config.clone()
        });
        assert_eq!(app.take_mouse_capture(), Some(false));
        assert!(app.drag.is_none(), "拖拽状态清掉");
        let mouse = mouse_app("x", true);
        assert!(mouse.config.mouse);
    }

    #[cfg(not(windows))]
    #[test]
    fn mouse_capture_sequences() {
        let mut out = Vec::new();
        crate::terminal::set_mouse_capture(&mut out, true).unwrap();
        let on = String::from_utf8(out).unwrap();
        assert!(
            on.contains("\x1b[?1000h") && on.contains("\x1b[?1006h"),
            "{on:?}"
        );
        let mut out = Vec::new();
        crate::terminal::set_mouse_capture(&mut out, false).unwrap();
        let off = String::from_utf8(out).unwrap();
        assert!(
            off.contains("\x1b[?1000l") && off.contains("\x1b[?1006l"),
            "{off:?}"
        );
    }

    #[test]
    fn enabling_autosave_with_unsaved_changes_starts_the_timer() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.md");
        std::fs::write(&path, "v1").unwrap();
        let t0 = Instant::now();
        let mut app = App::new(file::load(&path).unwrap(), Size::new(80, 24), false);
        app.handle_at(press(KeyCode::Char('x')), t0);
        assert_eq!(app.next_deadline(), None);
        let t1 = t0 + Duration::from_secs(10);
        app.apply_config_at(
            &Config {
                autosave: true,
                ..app.config.clone()
            },
            t1,
        );
        assert_eq!(app.next_deadline(), Some(t1 + AUTOSAVE_IDLE));
        app.tick(t1 + AUTOSAVE_IDLE);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "xv1");
    }

    #[test]
    fn settings_panel_applies_live_and_esc_restores() {
        let mut app = vim_app("hello");
        typed(&mut app, "\x1b");
        assert_eq!(app.mode_label(), Some("NOR"));
        app.handle(ctrl('p'));
        assert!(app.settings.is_some());
        // width：→ 一次加 2，立即生效
        app.handle(press(KeyCode::Right));
        assert_eq!(app.config.width, 74);
        assert_eq!(app.layout().body.width, 74);
        // 面板吞掉所有按键：x 不删字、mode 不变；粘贴丢弃；鼠标忽略
        typed(&mut app, "xi");
        app.handle(Event::Paste("pasted".into()));
        app.handle(ctrl('s'));
        assert_eq!(body(&app), "hello");
        assert_eq!(app.mode_label(), Some("NOR"));
        assert!(app.status.prompt.is_none());
        // 下到 tab_width，改成 5
        for _ in 0..8 {
            app.handle(press(KeyCode::Down));
        }
        app.handle(press(KeyCode::Right));
        assert_eq!(app.editor.tab_spaces(), "     ");
        app.handle(press(KeyCode::Esc));
        assert!(app.settings.is_none());
        assert_eq!(app.config, vim_app("").config, "回滚到快照");
        assert_eq!(app.editor.tab_spaces(), "    ");
        assert_eq!(app.mode_label(), Some("NOR"), "Esc 只关面板，不切换模式");
        // Ctrl+P 再按一次同样是取消
        app.handle(ctrl('p'));
        app.handle(press(KeyCode::Left));
        assert_eq!(app.config.width, 70);
        app.handle(ctrl('p'));
        assert!(app.settings.is_none());
        assert_eq!(app.config.width, 72);
    }

    #[test]
    fn settings_enter_writes_only_the_diff() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".mustty.toml");
        std::fs::write(&path, "# mine\nwidth = 64 # narrow\n").unwrap();
        let config = Config::default().merge(&config::PartialConfig::from_toml_str("width = 64").0);
        let mut app = App::with_config(lf_doc(""), Size::new(120, 30), &config);
        app.set_config_path(Some(path.clone()));
        app.handle(ctrl('p'));
        app.handle(press(KeyCode::Down));
        app.handle(press(KeyCode::Char(' ')));
        assert!(app.config.vim);
        app.handle(press(KeyCode::Enter));
        assert!(app.settings.is_none());
        assert!(app.config.vim, "Enter 保留修改");
        assert_eq!(app.config.width, 64);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# mine\nwidth = 64 # narrow\nvim = true\n"
        );
        assert!(app.status.message.is_none());
    }

    #[test]
    fn settings_write_errors_keep_the_session_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".mustty.toml");
        std::fs::write(&path, "width = = 1\n").unwrap();
        let mut app = App::new(lf_doc(""), Size::new(80, 24), false);
        app.set_config_path(Some(path.clone()));
        crate::i18n::init_for_test("zh-CN");
        app.handle(ctrl('p'));
        app.handle(press(KeyCode::Right));
        app.handle(press(KeyCode::Enter));
        assert!(app.settings.is_none(), "面板照常关闭");
        assert_eq!(app.config.width, 74, "修改仍在本次会话生效");
        assert_eq!(
            app.status.message.as_ref().map(|m| m.text.as_str()),
            Some("配置文件有语法错误，修改仅本次有效")
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "width = = 1\n");
    }

    #[test]
    fn ctrl_q_in_settings_rolls_back_then_quits() {
        let mut app = App::new(lf_doc("x"), Size::new(80, 24), false);
        app.handle(ctrl('p'));
        app.handle(press(KeyCode::Right));
        assert_eq!(app.config.width, 74);
        app.handle(ctrl('q'));
        assert!(app.settings.is_none());
        assert_eq!(app.config.width, 72, "先回滚");
        assert!(app.should_quit);
        // 有未保存修改：回滚后照常要求再按一次
        let mut app = App::new(lf_doc("x"), Size::new(80, 24), false);
        typed(&mut app, "y");
        app.handle(ctrl('p'));
        app.handle(press(KeyCode::Right));
        app.handle(ctrl('q'));
        assert!(app.settings.is_none() && !app.should_quit);
        assert_eq!(app.config.width, 72);
        app.handle(ctrl('q'));
        assert!(app.should_quit);
    }

    #[test]
    fn settings_and_save_prompt_are_exclusive() {
        let mut app = App::new(lf_doc("x"), Size::new(80, 24), false);
        app.handle(ctrl('s'));
        assert!(app.status.prompt.is_some());
        app.handle(ctrl('p'));
        assert!(app.settings.is_none(), "提示行打开时 Ctrl+P 进了输入框");
        app.handle(press(KeyCode::Esc));
        app.handle(ctrl('p'));
        app.handle(ctrl('s'));
        assert!(app.status.prompt.is_none(), "面板打开时 Ctrl+S 被吞掉");
    }

    #[test]
    fn settings_language_applies_now_and_writes_only_lang() {
        crate::i18n::init_for_test("en");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".mustty.toml");
        std::fs::write(&path, "# mine\nwidth = 80\n").unwrap();
        let config = Config::default().merge(&config::PartialConfig::from_toml_str("width = 80").0);
        let mut app = App::with_config(lf_doc(""), Size::new(100, 30), &config);
        app.set_config_path(Some(path.clone()));
        assert_eq!(app.placeholder(), "Untitled");
        app.handle(ctrl('p'));
        for _ in 0..12 {
            app.handle(press(KeyCode::Down));
        }
        assert_eq!(
            app.settings.as_ref().unwrap().field(),
            crate::settings::Field::Lang
        );
        // Auto → English → 简体中文：选中即生效
        app.handle(press(KeyCode::Right));
        app.handle(press(KeyCode::Right));
        assert_eq!(app.config.lang, "zh-CN");
        assert_eq!(crate::i18n::current(), "zh-CN");
        assert_eq!(t("settings-title"), "设置");
        assert_eq!(t("status-title"), "标题");
        app.handle(press(KeyCode::Enter));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# mine\nwidth = 80\nlang = \"zh-CN\"\n"
        );
        // Esc 回滚语言也立即生效
        app.handle(ctrl('p'));
        for _ in 0..12 {
            app.handle(press(KeyCode::Down));
        }
        app.handle(press(KeyCode::Left));
        assert_eq!((app.config.lang, crate::i18n::current()), ("en", "en"));
        app.handle(press(KeyCode::Esc));
        assert_eq!(
            (app.config.lang, crate::i18n::current()),
            ("zh-CN", "zh-CN")
        );
    }

    /// 原来只写进日志的失败，用户在 status bar 上都看得到。
    #[test]
    fn failures_reach_the_status_bar() {
        crate::i18n::init_for_test("zh-CN");
        let dir = tempfile::tempdir().unwrap();
        // 保存失败：目标目录不存在
        let mut app = App::new(lf_doc("x"), Size::new(80, 24), false);
        app.save_dir = dir.path().to_path_buf();
        app.handle(ctrl('s'));
        clear_prompt(&mut app);
        let target = dir.path().join("no/such/dir/a.md");
        app.handle(Event::Paste(target.display().to_string()));
        app.handle(press(KeyCode::Enter));
        let msg = app
            .status
            .message
            .as_ref()
            .map(|m| m.text.clone())
            .unwrap_or_default();
        assert!(msg.starts_with("保存失败："), "{msg:?}");
        // 写回配置失败（不是语法错误）：配置路径是个目录
        let mut app = App::new(lf_doc(""), Size::new(80, 24), false);
        app.set_config_path(Some(dir.path().to_path_buf()));
        app.handle(ctrl('p'));
        app.handle(press(KeyCode::Right));
        app.handle(press(KeyCode::Enter));
        let msg = app
            .status
            .message
            .as_ref()
            .map(|m| m.text.clone())
            .unwrap_or_default();
        assert!(
            msg.starts_with("无法写入配置文件") && msg.ends_with("修改仅本次有效"),
            "{msg:?}"
        );
        assert_eq!(app.config.width, 74, "修改仍在本次会话生效");
    }

    #[test]
    fn help_panel_opens_swallows_keys_and_closes() {
        let mut app = App::new(lf_doc("hello"), Size::new(80, 24), false);
        app.handle(ctrl('g'));
        assert!(app.help.is_some());
        // 吞掉所有按键：不编辑、不打开设置、不弹保存提示；粘贴丢弃
        typed(&mut app, "x?");
        app.handle(press(KeyCode::Enter));
        app.handle(ctrl('p'));
        app.handle(ctrl('s'));
        app.handle(Event::Paste("pasted".into()));
        assert_eq!(body(&app), "hello");
        assert!(app.settings.is_none() && app.status.prompt.is_none());
        assert!(app.help.is_some(), "默认模式里 ? 不关闭");
        app.handle(press(KeyCode::Esc));
        assert!(app.help.is_none());
        app.handle(ctrl('g'));
        app.handle(ctrl('g'));
        assert!(app.help.is_none(), "Ctrl+G 再按一次关闭");
    }

    #[test]
    fn help_panel_in_vim_normal_mode() {
        let mut app = vim_app("hello");
        typed(&mut app, "\x1b?");
        assert!(app.help.is_some(), "Normal 态 ? 打开");
        assert_eq!(app.mode_label(), Some("NOR"));
        typed(&mut app, "dd");
        assert_eq!(body(&app), "hello", "面板打开时 dd 不生效");
        typed(&mut app, "?");
        assert!(app.help.is_none(), "再按 ? 关闭");
        assert_eq!(app.mode_label(), Some("NOR"), "模式保持原样");
        // Insert 态里 ? 是普通字符
        typed(&mut app, "i?");
        assert!(app.help.is_none());
        assert_eq!(body(&app), "?hello");
    }

    #[test]
    fn help_scrolls_within_bounds() {
        let mut app = vim_app("");
        let total = app.help_lines().len();
        let visible = app.layout().body.height as usize;
        assert!(total > visible, "vim 的三组放不进 24 行");
        app.handle(ctrl('g'));
        app.handle(press(KeyCode::Up));
        assert_eq!(app.help.unwrap().scroll, 0);
        for _ in 0..20 {
            app.handle(press(KeyCode::PageDown));
        }
        assert_eq!(
            app.help.unwrap().scroll,
            total - visible,
            "最后一行停在最下面"
        );
        app.handle(press(KeyCode::Down));
        assert_eq!(app.help.unwrap().scroll, total - visible);
    }

    #[test]
    fn help_and_settings_are_exclusive() {
        let mut app = App::new(lf_doc("x"), Size::new(80, 24), false);
        app.handle(ctrl('p'));
        app.handle(ctrl('g'));
        assert!(
            app.settings.is_some() && app.help.is_none(),
            "设置面板吞掉 Ctrl+G"
        );
        app.handle(press(KeyCode::Esc));
        app.handle(ctrl('g'));
        app.handle(ctrl('p'));
        assert!(
            app.help.is_some() && app.settings.is_none(),
            "帮助面板吞掉 Ctrl+P"
        );
        app.handle(press(KeyCode::Esc));
        // 保存提示打开时 Ctrl+G 进了输入框
        app.handle(ctrl('s'));
        app.handle(ctrl('g'));
        assert!(app.help.is_none() && app.status.prompt.is_some());
    }

    #[test]
    fn ctrl_q_in_help_closes_then_quits() {
        let mut app = App::new(lf_doc("x"), Size::new(80, 24), false);
        app.handle(ctrl('g'));
        app.handle(ctrl('q'));
        assert!(app.help.is_none() && app.should_quit);
        // 有未保存修改：先关面板，再照常要求按第二次
        let mut app = App::new(lf_doc("x"), Size::new(80, 24), false);
        typed(&mut app, "y");
        app.handle(ctrl('g'));
        app.handle(ctrl('q'));
        assert!(app.help.is_none() && !app.should_quit);
        app.handle(ctrl('q'));
        assert!(app.should_quit);
    }
}
