//! App state and the synchronous event loop.

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

/// Default idle time after the last edit before autosave (config key `autosave_idle_ms`).
pub const AUTOSAVE_IDLE: Duration = Duration::from_secs(2);
/// How long the startup key hint stays visible.
pub const HINT_TTL: Duration = Duration::from_secs(5);
/// How long the typewriter / focus mode toggle message stays visible.
pub const TOGGLE_TTL: Duration = Duration::from_secs(2);
/// How long other transient messages stay visible.
pub const INFO_TTL: Duration = Duration::from_secs(3);
/// Bodies larger than this refresh the word count on save instead of live.
pub const LARGE_FILE: usize = 5 * 1024 * 1024;
/// On Windows, this many consecutive typed characters in one event batch count as a paste.
pub const PASTE_MIN_CHARS: usize = 16;

pub struct App {
    pub(crate) editor: Editor,
    keymap: Keymap,
    mode: Mode,
    pub(crate) status: StatusModel,
    pub(crate) viewport: Viewport,
    area: Rect,
    /// The active config and the single source of truth; only `apply_config` modifies it.
    /// Parts of it are copied into sub-objects: Keymap (vim), Viewport (typewriter, scrolloff),
    /// and the Editor's WrapCache (tab_width, which also sets how many spaces Tab inserts). `apply_config` keeps them in sync
    pub(crate) config: Config,
    /// Config file the settings panel writes back to. With None, changes last only for this session (the test default, so tests never touch the real ~/.mustty.toml)
    config_path: Option<PathBuf>,
    /// Settings panel (Ctrl+P); swallows every key while open
    pub(crate) settings: Option<Settings>,
    /// Help panel (Ctrl+G, or `?` in vim Normal mode); also swallows every key
    pub(crate) help: Option<Help>,
    /// Pending mouse capture toggle; the event loop applies it with `execute!` (WinAPI on Windows)
    mouse_capture: Option<bool>,
    /// Left-button drag in progress (only when the mouse is enabled)
    drag: Option<DragState>,
    /// Screen-to-document coordinate map from the last rendered frame; mouse handling reads only this
    pub(crate) screen: Option<ScreenMap>,
    /// Large file: the word count refreshes only on save
    large: bool,
    /// When content last changed. Cleared after each autosave attempt so a failure doesn't retry forever
    last_edit: Option<Instant>,
    /// Ctrl+Q was pressed once with unsaved changes
    quit_armed: bool,
    /// Control sequences waiting to be written to the terminal (OSC 52, cursor shape); the event loop flushes them
    output: Vec<String>,
    /// Whether the terminal supports OSC 52 (macOS Terminal doesn't)
    osc52_supported: bool,
    osc52_warned: bool,
    /// Last cursor shape sequence written to the terminal, for dedup. TerminalGuard sets a bar at startup
    cursor_sent: String,
    /// Window title already written to the terminal, for dedup
    window_title: crate::terminal::WindowTitle,
    /// Base for relative "Save as" paths; the prefilled untitled name also avoids collisions here. Defaults to the working directory at startup
    pub save_dir: PathBuf,
    pub should_quit: bool,
    pub needs_redraw: bool,
}

impl App {
    /// Default config; only autosave is specified.
    pub fn new(doc: Document, size: Size, autosave: bool) -> Self {
        let config = Config {
            autosave,
            ..Config::default()
        };
        Self::with_config(doc, size, &config)
    }

    /// Builds with the default config, then applies `config` through `apply_config`, so startup and live changes share one code path.
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
        // TerminalGuard already set the startup terminal state (mouse capture, bar cursor), so don't emit it again.
        // Push the cursor shape once more only if the configured one isn't a bar
        app.output.clear();
        app.mouse_capture = None;
        app.cursor_sent = ansi(SetCursorStyle::SteadyBar);
        app.sync_cursor_style();
        app
    }

    /// Applies a config: compares each field with the current value and routes the differences to where each one lives.
    /// Startup (`with_config`) and every settings panel change go through here and nowhere else.
    pub fn apply_config(&mut self, new: &Config) {
        self.apply_config_at(new, Instant::now());
    }

    fn apply_config_at(&mut self, new: &Config, now: Instant) {
        let old = std::mem::replace(&mut self.config, new.clone());
        self.needs_redraw = true;
        // width: sync_body rewraps when it sees the body width change, so nothing to do here
        self.viewport.typewriter = new.typewriter;
        self.viewport.scrolloff = new.scrolloff;
        if new.tab_width != old.tab_width {
            // The other copy of tab_width: wrapping, rendering and the spaces Tab inserts all share the one in WrapCache
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
            // From the next frame on, all text (including the panel itself) uses the new language; Auto renegotiates from the environment
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
        // Turned from off to on: with unsaved changes, start the idle timer now so autosave fires as usual
        if new.autosave && !old.autosave && self.editor.is_dirty() {
            self.last_edit = Some(now);
        }
        // cursor / cursor_blink changed (live preview in the panel), or vim was turned off and we're back in Insert
        self.sync_cursor_style();
    }

    /// vim was turned off: force Normal mode back to Insert (the cursor returns to the configured shape) and clear pending / visual state and the selection.
    fn leave_vim(&mut self, now: Instant) {
        if matches!(self.mode, Mode::Insert) {
            return;
        }
        self.mode = Mode::Insert;
        if self.editor.selection().is_some() {
            self.editor.apply(&Action::Cancel, 1, now);
        }
    }

    /// The cursor shape to show right now. vim Normal mode (including Visual) is always a block: it signals command mode and isn't a preference.
    /// While the settings panel is open, always use the configured shape so the panel can preview it; otherwise use the configured shape too.
    pub(crate) fn cursor_style(&self) -> SetCursorStyle {
        if self.settings.is_none() && matches!(self.mode, Mode::Normal(_)) {
            SetCursorStyle::SteadyBlock
        } else {
            self.config.cursor.style(self.config.cursor_blink)
        }
    }

    /// The only place that pushes the cursor shape, and only when it differs from the last one written. Called after every event and every config change.
    fn sync_cursor_style(&mut self) {
        let seq = ansi(self.cursor_style());
        if seq != self.cursor_sent {
            self.output.push(seq.clone());
            self.cursor_sent = seq;
        }
    }

    /// Config file the settings panel writes back to on Enter.
    pub fn set_config_path(&mut self, path: Option<PathBuf>) {
        self.config_path = path;
    }

    pub fn layout(&self) -> Layout {
        compute_layout(self.area, self.config.width)
    }

    pub fn measure(&self) -> u16 {
        self.config.width
    }

    /// Shows the key hint in the right part of the status bar for 5 s after startup.
    pub fn show_startup_hint(&mut self, now: Instant) {
        self.status.hint_until = Some(now + HINT_TTL);
    }

    /// A vim Visual selection is inclusive: both the anchor and the character under the cursor are selected. The editing core only knows half-open ranges,
    /// so the App layer converts here and the core stays unaware of Mode.
    fn visual_range(&self) -> Option<Range<usize>> {
        if self.editor.focus != crate::editor::Focus::Body {
            return None;
        }
        let c = self.editor.cursor;
        let anchor = c.anchor?;
        let (lo, hi) = (anchor.min(c.head), anchor.max(c.head));
        Some(lo..Cursor::next(self.editor.doc().body(), hi).max(lo))
    }

    /// Selection to render: the inclusive range in vim Visual mode, the editing core's selection otherwise.
    pub(crate) fn visible_selection(&self) -> Option<Range<usize>> {
        match self.mode {
            Mode::Normal(s) if s.visual => self.visual_range(),
            _ => self.editor.selection(),
        }
    }

    /// Config file warning: a status bar error message that stays until the first key press.
    pub fn show_startup_warning(&mut self, text: String) {
        self.status.error(text);
    }

    /// Mode indicator and its style (`theme::mode`).
    pub(crate) fn mode_indicator(&self) -> Option<(&'static str, ratatui::style::Style)> {
        self.mode_label()
            .map(|label| (label, view::theme::mode(self.mode)))
    }

    /// Mode indicator at the start of the status bar, shown only when vim is on.
    pub(crate) fn mode_label(&self) -> Option<&'static str> {
        self.keymap.vim.then_some(match self.mode {
            Mode::Insert => "INS",
            Mode::Normal(s) if s.visual => "VIS",
            Mode::Normal(_) => "NOR",
        })
    }

    /// Placeholder for an Unbound title: the file name without extension, or `Untitled`.
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
        // Mode switches, panels opening or closing and config changes can all change the cursor shape
        self.sync_cursor_style();
    }

    fn handle_event(&mut self, ev: Event, now: Instant) {
        match ev {
            Event::Resize(w, h) => {
                // Multiple Resize events in one batch only record the size; relayout happens once at the next draw
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
                // Panels don't hit-test clicks
                if self.config.mouse && self.settings.is_none() && self.help.is_none() {
                    self.on_mouse(m, now);
                }
                return;
            }
            _ => {}
        }
        // At most one input-grabbing overlay is open at a time: while the prompt is open, Ctrl+P / Ctrl+G go to PromptInput,
        // and while a panel is open, the other panel's shortcut and Ctrl+S are swallowed
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
            // The panel swallows every key without going through Keymap, and mode stays as is; pastes are dropped
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
        // Entering Visual mode: the selection starts at the character under the cursor
        if is_visual && !was_visual {
            let head = self.editor.cursor.head;
            self.edit(&Action::MoveTo(Target::Body(head), true), now);
        }
        // d / x / y / p in Visual mode: first turn the selection into the inclusive range (including the character under the cursor), then hand it to the editing core
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
        // Leaving Visual mode (e.g. after a y copy): drop the selection, as vim does
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

    /// Lines of the help panel (they depend on whether vim is on).
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

    /// Esc: restore the snapshot taken when the panel opened and close it, without writing the file.
    fn close_settings(&mut self, now: Instant) {
        if let Some(settings) = self.settings.take() {
            self.apply_config_at(&settings.snapshot, now);
        }
    }

    /// Enter: write only the keys that differ from the snapshot. On failure the changes still apply for this session.
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

    /// Takes the pending mouse capture toggle.
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

    /// The Editor has already filled the internal register; this also writes the system clipboard via OSC 52, which works through SSH / tmux.
    fn copy_to_clipboard(&mut self, text: &str, now: Instant) {
        if self.osc52_supported {
            self.output.push(crate::terminal::osc52(text));
        } else if !std::mem::replace(&mut self.osc52_warned, true) {
            self.status.info(t("msg-osc52-unsupported"), now, INFO_TTL);
        }
    }

    /// Returns the OSC 2 sequence to write when the window title (including the unsaved mark) changed; checked once per event loop iteration, None when unchanged.
    pub fn take_window_title(&mut self) -> Option<String> {
        let doc = self.editor.doc();
        // The unsaved mark shares its source with the status bar's SaveMark: both read Editor::is_dirty() directly.
        // Dedup compares the final string, so a change in save state alone is pushed even when the title text is the same
        let title =
            StatusModel::window_title(&doc.title, doc.meta.path.as_deref(), self.editor.is_dirty());
        self.window_title.update(&title)
    }

    /// Takes the control sequences waiting to be written to the terminal.
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
        // Relative to save_dir (the cwd at startup), with no ~ expansion; absolute paths are used as is
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

    /// Autosaves immediately (checking for external modification); returns whether it succeeded.
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
                // The mark after the file name shows the save state, so no "Saved" message is shown
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

    /// When to wake up next: the earliest of the autosave, message expiry, hint and drag deadlines.
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

    /// Handles autosaves and messages that are due.
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

    /// Mouse: `view::mouse::map_mouse` turns events into Actions, which share the keyboard's editing path.
    fn on_mouse(&mut self, m: MouseEvent, now: Instant) {
        if self.status.prompt.is_some() {
            return;
        }
        // Use only the map from the last rendered frame; ignore the event if nothing has rendered yet (or the window is too small)
        let Some(map) = self.screen else { return };
        let title = self.editor.doc().title_text().to_owned();
        let (doc, wrap, _) = self.editor.view();
        let action = map_mouse(&m, &map, wrap, doc.body(), &title, &mut self.drag, now);
        if let Some(action) = action {
            self.mouse_action(action, now);
        }
    }

    fn mouse_action(&mut self, action: Action, now: Instant) {
        // Dragging a selection in vim Normal mode also enters Visual mode, so d / y work right after
        if let (Action::MoveTo(_, true), Mode::Normal(state)) = (&action, &mut self.mode) {
            state.visual = true;
        }
        self.dispatch(action, now);
        // Moving the cursor may scroll the viewport; later mouse events in the same batch must map against the new viewport
        if let Some(map) = &mut self.screen {
            map.follow(&self.viewport);
        }
    }

    /// Wheel: moves only the viewport. If the cursor leaves the scrolloff band, pull it back to the edge of the visible area (keeping preferred_col).
    /// The cursor must stay on screen so the IME candidate window has something to anchor to.
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

    /// Dragging past the top or bottom of the body and holding still scrolls one row every 50 ms; the cursor follows to the new edge row.
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

    /// Called before drawing: fits the wrap width and viewport to the current body area, keeps the cursor visible and refreshes the word count.
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

/// Windows never produces `Event::Paste`, so merge a run of at least `PASTE_MIN_CHARS` typed characters in one batch into a single paste.
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

/// Characters the terminal sends key by key during a paste (including Enter).
/// Escape sequence for a cursor shape (DECSCUSR).
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
        // The window title (OSC 2) also goes straight to stdout: it doesn't move the cursor and bypasses ratatui's Buffer
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
            None => true, // Nothing to wait for: block on read, so idle uses zero CPU
        };
        if ready {
            batch.push(event::read()?);
            // Drain the whole batch before redrawing, so several characters committed by the IME at once cost one redraw
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
        // A successful save shows no message, so there is no timer to wake up for
        assert_eq!(
            (app.status.message.clone(), app.next_deadline()),
            (None, None)
        );

        // After an external modification: skip the autosave, keep the warning visible and schedule no further autosave
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
        // Quitting needs confirmation; Ctrl+S forces an overwrite
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
        assert_eq!(body(&app), "  x", "tab_width = 2 inserts two spaces");
        app.apply_config(&Config {
            tab_width: 8,
            ..app.config.clone()
        });
        app.handle(press(KeyCode::Tab));
        assert_eq!(body(&app), format!("{}x", " ".repeat(10)));
        // ↑ moves into the title, Tab returns to the start of the body and leaves the title alone; Shift+Tab does nothing
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
        // M4 acceptance: i to type → Esc → gg → dd → G → p
        let mut app = vim_app("");
        typed(&mut app, "\x1bione");
        app.handle(press(KeyCode::Enter));
        typed(&mut app, "two");
        app.handle(press(KeyCode::Enter));
        typed(&mut app, "three\x1b");
        assert_eq!(app.mode_label(), Some("NOR"));
        typed(&mut app, "ggddGp");
        assert_eq!(body(&app), "two\nthree\none");
        // u does the same as Ctrl+Z
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
        // Printable characters in Normal mode (including CJK) are not inserted
        typed(&mut app, "中q");
        assert_eq!(body(&app), "hello world foo");
        // Visual selection is inclusive: right after v, the character under the cursor is selected
        typed(&mut app, "v");
        assert_eq!(
            (app.mode_label(), app.visible_selection()),
            (Some("VIS"), Some(0..1))
        );
        // v e d deletes the whole word, including its last letter
        typed(&mut app, "ed");
        assert_eq!(
            (body(&app).as_str(), app.mode_label()),
            (" world foo", Some("NOR"))
        );
        assert_eq!(app.editor.cursor.head, 0);
        // v w d deletes up to and including the first character of the next word
        let mut app = vim_app("hello world foo");
        typed(&mut app, "\x1bvw");
        assert_eq!(
            app.visible_selection(),
            Some(0..7),
            "the rendered selection includes the character under the cursor"
        );
        typed(&mut app, "d");
        assert_eq!(body(&app), "orld foo");
        // v e y copies the whole word; the selection goes away and the cursor returns to its start. p pastes at the cursor
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
        // A reversed selection (cursor before the anchor) also includes both ends
        let mut app = vim_app("abcdef");
        typed(&mut app, "\x1blllvhhd"); // anchor on 'd', cursor moves back to 'b'
        assert_eq!(body(&app), "aef");
        // v alone then x deletes the one character under the cursor
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
            "Normal: SteadyBlock"
        );
        typed(&mut app, "v\x1b");
        assert!(
            app.take_terminal_output().is_empty(),
            "Visual is still Normal, so the shape doesn't change"
        );
        typed(&mut app, "i");
        assert_eq!(
            app.take_terminal_output(),
            vec!["\x1b[6 q".to_owned()],
            "Insert: SteadyBar"
        );
        // The default (non-vim) mode never changes the cursor shape or shows a mode
        let mut plain = App::new(
            Document::parse("x", crate::editor::document::FileMeta::new(None)),
            Size::new(80, 24),
            false,
        );
        typed(&mut plain, "\x1bab");
        assert!(plain.take_terminal_output().is_empty());
        assert_eq!(plain.mode_label(), None);
    }

    /// Selects a field in the settings panel.
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
        // Startup: push once when the configured shape isn't a bar (TerminalGuard already set the bar, so no repeat)
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
        // Normal (including Visual) is always SteadyBlock, regardless of config
        typed(&mut app, "\x1b");
        assert_eq!(app.take_terminal_output(), vec!["\x1b[2 q".to_owned()]);
        typed(&mut app, "v");
        assert!(
            app.take_terminal_output().is_empty(),
            "Visual behaves like Normal"
        );
        typed(&mut app, "\x1bi");
        assert_eq!(
            app.take_terminal_output(),
            vec!["\x1b[4 q".to_owned()],
            "back in Insert: the configured underline"
        );
        // Blinking
        app.apply_config(&Config {
            cursor_blink: true,
            ..app.config.clone()
        });
        assert_eq!(app.take_terminal_output(), vec!["\x1b[3 q".to_owned()]);
        // Turning vim off while in Normal returns to the configured shape (not a hardcoded bar)
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
        // Opening the panel from Normal previews the configured shape (bar), not Normal's block
        app.handle(ctrl('p'));
        assert_eq!(app.take_terminal_output(), vec!["\x1b[6 q".to_owned()]);
        select_field(&mut app, Field::Cursor);
        assert!(
            app.take_terminal_output().is_empty(),
            "moving the selection pushes nothing"
        );
        // Every change pushes the new shape right away, without waiting for the panel to close
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
        // Turning vim off in the panel (Normal is forced back to Insert): the mode switch doesn't override the previewed shape
        select_field(&mut app, Field::Vim);
        app.handle(press(KeyCode::Char(' ')));
        assert!(!app.config.vim);
        assert_eq!(app.mode, Mode::Insert);
        assert!(
            app.take_terminal_output().is_empty(),
            "still the previewed blinking block"
        );
        // Esc rolls back: the shape returns to the snapshot's steady bar (we're in Insert now)
        app.handle(press(KeyCode::Esc));
        assert!(app.settings.is_none());
        assert!(app.config.vim);
        assert_eq!(app.config.cursor, CursorShape::Bar);
        assert_eq!(app.take_terminal_output(), vec!["\x1b[6 q".to_owned()]);
        // Reopen the panel from Normal, switch to underline and keep it with Enter: back in Normal it's still a block, and only Insert shows the underline
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

    /// Like the event loop: draw a frame first (producing the ScreenMap), then handle the next event.
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
        // 80×24: body at x = 4, y = 5, height 14
        let text: String = (0..100).map(|i| format!("第{i:02}行 line\n")).collect();
        let mut app = mouse_app(&text, true);
        let left = MouseButton::Left;
        let t0 = Instant::now();
        // Each line "第NN行 line\n" is 10 chars: '第' takes columns 0–1, '0' column 2, '2' column 3
        rendered(
            &mut app,
            mouse(MouseEventKind::Down(left), 4 + 3, 5 + 2),
            t0,
        );
        assert_eq!(app.editor.cursor.head, 2 * 10 + 2);
        // Clicking the right half of a wide character snaps to its left edge
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
        // Clicks outside the body are ignored
        rendered(&mut app, mouse(MouseEventKind::Down(left), 1, 7), t0);
        rendered(&mut app, mouse(MouseEventKind::Down(left), 4, 22), t0);
        assert_eq!(app.editor.cursor.head, 2 * 10 + 2);
        rendered(
            &mut app,
            mouse(MouseEventKind::Down(left), 4 + 3, 5 + 2),
            t0,
        );
        // Dragging creates a selection
        rendered(&mut app, mouse(MouseEventKind::Drag(left), 4, 5 + 4), t0);
        assert_eq!(app.editor.selection(), Some(2 * 10 + 2..4 * 10));
        // Drag below the body and hold still: scrolls one row every 50 ms and the selection extends with it; stops on release
        let top = app.viewport.top.line;
        rendered(&mut app, mouse(MouseEventKind::Drag(left), 10, 23), t0);
        assert_eq!(app.next_deadline(), Some(t0 + DRAG_SCROLL_INTERVAL));
        for k in 1..=10 {
            app.tick(t0 + DRAG_SCROLL_INTERVAL * k);
        }
        assert_eq!(app.viewport.top.line, top + 10, "keeps scrolling");
        let sel = app.editor.selection().unwrap();
        assert_eq!(sel.start, 2 * 10 + 2);
        assert!(sel.end > 20 * 10, "{sel:?}");
        rendered(&mut app, mouse(MouseEventKind::Up(left), 10, 23), t0);
        app.tick(t0 + DRAG_SCROLL_INTERVAL * 20);
        assert_eq!(app.viewport.top.line, top + 10, "stops after release");
        // The wheel moves only the viewport; a cursor leaving the scrolloff band is pulled back to the edge and stays visible
        let line = |app: &App| app.editor.doc().body().char_to_line(app.editor.cursor.head);
        let (top, cursor_line) = (app.viewport.top.line, line(&app));
        rendered(&mut app, mouse(MouseEventKind::ScrollDown, 10, 10), t0);
        assert_eq!(
            (app.viewport.top.line, line(&app)),
            (top + 3, cursor_line),
            "cursor still inside the band: it doesn't move"
        );
        for _ in 0..10 {
            rendered(&mut app, mouse(MouseEventKind::ScrollDown, 10, 10), t0);
        }
        assert_eq!(app.viewport.top.line, top + 33);
        assert_eq!(
            line(&app),
            top + 33 + 3,
            "pulled to the top edge (scrolloff 3)"
        );
        // Mouse disabled: mouse events are ignored entirely
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
            // Inclusive: dragging to column 6 deletes the 'w' too
            ("orld\nsecond", Some("NOR"))
        );
    }

    #[test]
    fn click_on_horizontally_scrolled_title() {
        // 80×24: title row at y = 3, x = 4, width 72. A 100-letter title, focused with the cursor at the end, is scrolled by 100 + 1 - 72 = 29 columns
        let long: String = ('a'..='z').cycle().take(100).collect();
        let mut app = mouse_app(&format!("# {long}\n\nbody"), true);
        app.handle(press(KeyCode::Up));
        assert_eq!(
            (app.editor.focus, app.editor.title_cursor),
            (crate::editor::Focus::Title, 100)
        );
        // Click the first cell of the title on screen, which shows letter 29
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
        let body = app.layout().body; // height 14 → middle row 7
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
        // 15 characters: still typed one by one
        let short = typed("123456789012345");
        assert_eq!(coalesce_paste(short.clone()), short);
        // 16 characters (including Enter, mixed with Release events): merged into one paste
        let mut long = typed("第一行abc");
        long.push(press(KeyCode::Enter));
        long.push(release(KeyCode::Enter));
        long.extend(typed("第二行defghi"));
        assert_eq!(
            coalesce_paste(long),
            vec![Event::Paste("第一行abc\n第二行defghi".into())]
        );
        // Interrupted by another key: each side is judged separately
        let mut mixed = typed("abc");
        mixed.push(press(KeyCode::Left));
        mixed.extend(typed("0123456789abcdef"));
        let out = coalesce_paste(mixed);
        assert_eq!(out.len(), 5);
        assert_eq!(out[4], Event::Paste("0123456789abcdef".into()));
        // Ctrl combinations are not typed input
        let ctrl_s = Event::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert_eq!(coalesce_paste(vec![ctrl_s.clone()]), vec![ctrl_s]);
    }

    fn ctrl(c: char) -> Event {
        Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
    }

    /// Window title: sent once at startup, then only when the final string actually changes (edits, undo, save, the unsaved mark).
    #[test]
    fn window_title_follows_the_document() {
        crate::i18n::init_for_test("en");
        let osc = |s: &str| Some(format!("\x1b]2;{s}\x07"));
        let mut app = App::new(lf_doc("# Hello\n\nbody"), Size::new(80, 24), false);
        assert_eq!(app.take_window_title(), osc("Hello"));
        assert_eq!(app.take_window_title(), None, "nothing sent when unchanged");
        // Typing in the body: the title text is unchanged but there are unsaved changes → `* `; moving afterwards sends nothing
        typed(&mut app, "x");
        assert_eq!(app.take_window_title(), osc("* Hello"));
        typed(&mut app, "y");
        app.handle(press(KeyCode::Down));
        assert_eq!(app.take_window_title(), None);
        // Move into the title and type at the end: every character is a change
        app.handle(ctrl_key(KeyCode::Home));
        app.handle(press(KeyCode::Up));
        app.handle(press(KeyCode::End));
        assert_eq!(app.take_window_title(), None, "only moved into the title");
        typed(&mut app, "!");
        assert_eq!(app.take_window_title(), osc("* Hello!"));
        typed(&mut app, "?");
        assert_eq!(app.take_window_title(), osc("* Hello!?"));
        // Undo affects the title; the xy in the body isn't undone yet, so it's still dirty
        app.handle(ctrl('z'));
        assert_eq!(
            app.take_window_title(),
            osc("* Hello"),
            "consecutive typing merges into one undo step"
        );
        // Undoing again reaches the save point: the asterisk goes away
        app.handle(ctrl('z'));
        assert!(!app.editor.is_dirty());
        assert_eq!(app.take_window_title(), osc("Hello"));
        // Control characters never make it into the sequence
        app.handle(Event::Paste("\x1b]0;x\x07".into()));
        let pasted = app.take_window_title().unwrap_or_default();
        assert_eq!(pasted.matches('\x1b').count(), 1, "{pasted:?}");
        assert_eq!(pasted.matches('\x07').count(), 1, "{pasted:?}");
        assert!(pasted.starts_with("\x1b]2;* "), "{pasted:?}");

        // No H1: Untitled → `* Untitled` after typing → the file name without an asterisk after Save as
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
        // Saving with Ctrl+S: the asterisk appears and then goes away
        typed(&mut app, "!");
        assert_eq!(app.take_window_title(), osc("* untitled.md"));
        app.handle(ctrl('s'));
        assert_eq!(app.take_window_title(), osc("untitled.md"));
    }

    /// The title's `* ` and the status bar's save mark both come from `Editor::is_dirty()`, so they agree in every state.
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
                "{label}: mark {mark:?}, title {title:?}"
            );
        }
        let dir = tempfile::tempdir().unwrap();
        // Empty Untitled: no status bar mark and no asterisk in the title
        let untitled = Document::parse("", crate::editor::document::FileMeta::new(None));
        let mut app = App::new(untitled, Size::new(80, 24), false);
        app.save_dir = dir.path().to_path_buf();
        check(&mut app, "empty Untitled");
        assert_eq!(app.take_window_title(), Some("\x1b]2;Untitled\x07".into()));
        typed(&mut app, "a");
        check(&mut app, "Untitled with changes");
        app.handle(ctrl('z'));
        check(&mut app, "Untitled undone back to empty");
        typed(&mut app, "b");
        app.handle(ctrl('s'));
        app.handle(press(KeyCode::Enter));
        check(&mut app, "after Save as");
        typed(&mut app, "c");
        check(&mut app, "saved file with changes");
        app.handle(ctrl('z'));
        check(&mut app, "undone back to the save point");
        assert!(!app.editor.is_dirty());
    }

    /// The asterisk goes away once autosave finishes.
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
        assert!(!app.editor.is_dirty(), "autosaved");
        assert_eq!(app.take_window_title(), osc("Draft"));
    }

    fn ctrl_key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::CONTROL))
    }

    /// Deletes the file name prefilled in the prompt.
    fn clear_prompt(app: &mut App) {
        let n = app.status.prompt.as_ref().unwrap().input.chars().count();
        for _ in 0..n {
            app.handle(press(KeyCode::Backspace));
        }
        assert_eq!(app.status.prompt.as_ref().unwrap().input, "");
    }

    /// Ctrl+S on a document without a path prefills untitled.md with the cursor at the end; Enter saves, and the next document avoids that name.
    #[test]
    fn untitled_save_prefills_a_free_name() {
        crate::i18n::init_for_test("en");
        let dir = tempfile::tempdir().unwrap();
        let untitled = || Document::parse("", crate::editor::document::FileMeta::new(None));
        let mut app = App::new(untitled(), Size::new(80, 24), false);
        app.save_dir = dir.path().to_path_buf();
        typed(&mut app, "hello");
        app.handle(ctrl('s'));
        let prompt = app.status.prompt.as_ref().expect("prompt is open");
        assert_eq!(prompt.input, "untitled.md");
        assert_eq!(
            prompt.cursor,
            "untitled.md".len(),
            "cursor at the end of the file name"
        );
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
        // Once there is a path, Ctrl+S saves directly without the prompt
        typed(&mut app, "!");
        app.handle(ctrl('s'));
        assert!(app.status.prompt.is_none());
        assert_eq!(std::fs::read_to_string(&saved).unwrap(), "hello!");

        // A second document without a path: untitled.md is already taken
        let mut app = App::new(untitled(), Size::new(80, 24), false);
        app.save_dir = dir.path().to_path_buf();
        typed(&mut app, "two");
        app.handle(ctrl('s'));
        assert_eq!(app.status.prompt.as_ref().unwrap().input, "untitled-2.md");
        // Changing to an existing name still asks to confirm the overwrite
        clear_prompt(&mut app);
        app.handle(press(KeyCode::Enter));
        assert!(
            app.status.prompt.is_some(),
            "Enter on empty input doesn't save"
        );
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

    /// Makes WrapCache recompute every line, then checks that every copy of tab_width agrees and the cache matches a fresh wrap.
    fn assert_tab_width_synced(app: &mut App, tab: u16) {
        assert_eq!(app.config.tab_width, tab, "App config");
        assert_eq!(
            app.editor.tab_spaces(),
            " ".repeat(tab as usize),
            "spaces inserted by Tab"
        );
        let (doc, wrap, _) = app.editor.view();
        let rope = doc.body();
        assert_eq!(wrap.tab_width(), tab, "WrapCache: wrapping and rendering");
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
        // Tab inserts spaces at the new width
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
        /// Random text (with tabs) plus random tab_width changes: the cache always matches a fresh wrap, and the cursor's screen position round-trips.
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
        assert_eq!(app.editor.selection(), None, "selection cleared");
        assert_eq!(
            app.take_terminal_output(),
            vec!["\x1b[6 q".to_owned()],
            "back to the bar cursor"
        );
        // Already in Insert when turned off: no cursor sequence is pushed
        let mut app = vim_app("hello");
        app.apply_config(&Config {
            vim: false,
            ..app.config.clone()
        });
        assert!(app.take_terminal_output().is_empty());
        // Turning it back on stays in Insert
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
            "TerminalGuard handles it at startup"
        );
        let on = Config {
            mouse: true,
            ..app.config.clone()
        };
        app.apply_config(&on);
        assert_eq!(app.take_mouse_capture(), Some(true));
        app.apply_config(&on);
        assert_eq!(
            app.take_mouse_capture(),
            None,
            "not sent again when nothing changed"
        );
        app.drag = Some(DragState {
            last: ratatui::layout::Position::new(0, 0),
            deadline: None,
        });
        app.apply_config(&Config {
            mouse: false,
            ..app.config.clone()
        });
        assert_eq!(app.take_mouse_capture(), Some(false));
        assert!(app.drag.is_none(), "drag state cleared");
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
        // width: → adds 2 per press and applies immediately
        app.handle(press(KeyCode::Right));
        assert_eq!(app.config.width, 74);
        assert_eq!(app.layout().body.width, 74);
        // The panel swallows every key: x deletes nothing and mode doesn't change; pastes are dropped; the mouse is ignored
        typed(&mut app, "xi");
        app.handle(Event::Paste("pasted".into()));
        app.handle(ctrl('s'));
        assert_eq!(body(&app), "hello");
        assert_eq!(app.mode_label(), Some("NOR"));
        assert!(app.status.prompt.is_none());
        // Move down to tab_width and set it to 5
        for _ in 0..8 {
            app.handle(press(KeyCode::Down));
        }
        app.handle(press(KeyCode::Right));
        assert_eq!(app.editor.tab_spaces(), "     ");
        app.handle(press(KeyCode::Esc));
        assert!(app.settings.is_none());
        assert_eq!(
            app.config,
            vim_app("").config,
            "rolled back to the snapshot"
        );
        assert_eq!(app.editor.tab_spaces(), "    ");
        assert_eq!(
            app.mode_label(),
            Some("NOR"),
            "Esc only closes the panel and doesn't switch modes"
        );
        // Pressing Ctrl+P again also cancels
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
        assert!(app.config.vim, "Enter keeps the changes");
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
        assert!(app.settings.is_none(), "panel closes as usual");
        assert_eq!(
            app.config.width, 74,
            "the change still applies for this session"
        );
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
        assert_eq!(app.config.width, 72, "rolled back first");
        assert!(app.should_quit);
        // With unsaved changes: after the rollback it still asks for a second press
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
        assert!(
            app.settings.is_none(),
            "with the prompt open, Ctrl+P goes into the input"
        );
        app.handle(press(KeyCode::Esc));
        app.handle(ctrl('p'));
        app.handle(ctrl('s'));
        assert!(
            app.status.prompt.is_none(),
            "with the panel open, Ctrl+S is swallowed"
        );
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
        // Auto → English → Simplified Chinese: applies as soon as it's selected
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
        // Rolling back the language with Esc also applies immediately
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

    /// Failures that used to go only to the log now show up in the status bar.
    #[test]
    fn failures_reach_the_status_bar() {
        crate::i18n::init_for_test("zh-CN");
        let dir = tempfile::tempdir().unwrap();
        // Save failure: the target directory doesn't exist
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
        // Config write-back failure (not a syntax error): the config path is a directory
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
        assert_eq!(
            app.config.width, 74,
            "the change still applies for this session"
        );
    }

    #[test]
    fn help_panel_opens_swallows_keys_and_closes() {
        let mut app = App::new(lf_doc("hello"), Size::new(80, 24), false);
        app.handle(ctrl('g'));
        assert!(app.help.is_some());
        // Swallows every key: no edits, no settings panel, no save prompt; pastes are dropped
        typed(&mut app, "x?");
        app.handle(press(KeyCode::Enter));
        app.handle(ctrl('p'));
        app.handle(ctrl('s'));
        app.handle(Event::Paste("pasted".into()));
        assert_eq!(body(&app), "hello");
        assert!(app.settings.is_none() && app.status.prompt.is_none());
        assert!(app.help.is_some(), "? doesn't close it in the default mode");
        app.handle(press(KeyCode::Esc));
        assert!(app.help.is_none());
        app.handle(ctrl('g'));
        app.handle(ctrl('g'));
        assert!(app.help.is_none(), "pressing Ctrl+G again closes it");
    }

    #[test]
    fn help_panel_in_vim_normal_mode() {
        let mut app = vim_app("hello");
        typed(&mut app, "\x1b?");
        assert!(app.help.is_some(), "? opens it in Normal mode");
        assert_eq!(app.mode_label(), Some("NOR"));
        typed(&mut app, "dd");
        assert_eq!(
            body(&app),
            "hello",
            "dd has no effect while the panel is open"
        );
        typed(&mut app, "?");
        assert!(app.help.is_none(), "pressing ? again closes it");
        assert_eq!(app.mode_label(), Some("NOR"), "mode stays the same");
        // In Insert mode ? is an ordinary character
        typed(&mut app, "i?");
        assert!(app.help.is_none());
        assert_eq!(body(&app), "?hello");
    }

    #[test]
    fn help_scrolls_within_bounds() {
        let mut app = vim_app("");
        let total = app.help_lines().len();
        let visible = app.layout().body.height as usize;
        assert!(total > visible, "the three vim groups don't fit in 24 rows");
        app.handle(ctrl('g'));
        app.handle(press(KeyCode::Up));
        assert_eq!(app.help.unwrap().scroll, 0);
        for _ in 0..20 {
            app.handle(press(KeyCode::PageDown));
        }
        assert_eq!(
            app.help.unwrap().scroll,
            total - visible,
            "the last line stops at the bottom"
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
            "the settings panel swallows Ctrl+G"
        );
        app.handle(press(KeyCode::Esc));
        app.handle(ctrl('g'));
        app.handle(ctrl('p'));
        assert!(
            app.help.is_some() && app.settings.is_none(),
            "the help panel swallows Ctrl+P"
        );
        app.handle(press(KeyCode::Esc));
        // With the save prompt open, Ctrl+G goes into the input
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
        // With unsaved changes: close the panel first, then ask for a second press as usual
        let mut app = App::new(lf_doc("x"), Size::new(80, 24), false);
        typed(&mut app, "y");
        app.handle(ctrl('g'));
        app.handle(ctrl('q'));
        assert!(app.help.is_none() && !app.should_quit);
        app.handle(ctrl('q'));
        assert!(app.should_quit);
    }
}
