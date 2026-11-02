//! mustty in the browser. xterm.js is the terminal: it hands over what the user types (`feed`) and
//! draws the ANSI bytes this writes back. Everything in between is the same App as in a real terminal.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use js_sys::{Function, Uint8Array};
use mustty::ansi::AnsiBackend;
use mustty::app::App;
use mustty::config::Config;
use mustty::editor::document::{DiskStamp, Document, FileMeta};
use mustty::host::{DocStore, HostCaps, HostOp};
use mustty::input::{Event, InputParser};
use mustty::io::file::{self, IoError};
use ratatui::Terminal;
use ratatui::layout::Size;
use wasm_bindgen::prelude::*;
use web_time::Instant;

/// The document the page opens with.
const SAMPLE: &str = include_str!("sample.md");

/// What the native host's TerminalGuard sets up: the alternate screen (where xterm.js turns the
/// wheel into arrow keys), bracketed paste, focus reports and a steady bar cursor.
const SESSION_START: &str = "\x1b[?1049h\x1b[?2004h\x1b[?1004h\x1b[6 q";

#[wasm_bindgen]
pub struct Session {
    app: App,
    terminal: Terminal<AnsiBackend>,
    parser: InputParser,
    /// Receives the bytes for xterm.js's `write`, as a `Uint8Array`
    write: Function,
}

#[wasm_bindgen]
impl Session {
    /// Opens the sample document in a `cols` × `rows` terminal and writes the first frame right away.
    #[wasm_bindgen(constructor)]
    pub fn new(cols: u16, rows: u16, write: Function) -> Session {
        std::panic::set_hook(Box::new(|info| {
            web_sys::console::error_1(&info.to_string().into());
        }));
        let size = Size::new(cols, rows);
        let doc = Document::parse(SAMPLE, FileMeta::new(Some("demo.md".into())));
        let caps = HostCaps {
            // Copying fills the internal register only for now; reaching the system clipboard comes later
            clipboard: true,
            can_quit: false,
        };
        // With mouse capture on, mustty takes clicks, drags and the wheel, so xterm.js's own drag
        // selection is gone: hold Shift to select text, as in a native terminal with mouse = true
        let config = Config {
            mouse: true,
            ..Config::default()
        };
        let mut app = App::with_host(
            doc,
            size,
            &config,
            Box::new(MemoryDocStore::default()),
            None,
            caps,
        );
        app.show_startup_hint(Instant::now());
        let mut backend = AnsiBackend::new(size);
        backend.push(SESSION_START);
        // The App leaves the startup mouse mode to the host (natively TerminalGuard sets it)
        backend.push(&HostOp::MouseCapture(config.mouse).to_ansi());
        let terminal = Terminal::new(backend).unwrap_or_else(|e| match e {});
        let mut session = Session {
            app,
            terminal,
            parser: InputParser::default(),
            write,
        };
        session.frame();
        session
    }

    /// One `onData` delivery from xterm.js. Called synchronously; drawing waits for `frame`.
    pub fn feed(&mut self, data: &str) {
        let mut events = Vec::new();
        self.parser.feed(data.as_bytes(), &mut events);
        self.turn(events);
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.terminal.backend_mut().resize(Size::new(cols, rows));
        self.turn(vec![Event::Resize(cols, rows)]);
    }

    /// Replaces the document with the file `bytes`, as if the page had opened it: fresh cursor, no undo
    /// history, no selection. `name` is the file name shown and offered for download (no path). Decoded
    /// like a file on disk: the BOM and CRLF line endings are noted and given back by `save_text`, and
    /// anything but UTF-8 is refused.
    pub fn open(&mut self, bytes: &[u8], name: &str) -> Result<(), JsError> {
        // The same message (in the UI language) the terminal shows for a file it can't open
        let doc = file::decode(bytes, Some(name.into()))
            .map_err(|e| JsError::new(&format!("{name}: {e}")))?;
        self.app.open_document(doc);
        self.frame();
        Ok(())
    }

    /// The document as a file: the original BOM, line endings (CRLF stays CRLF) and final newline
    pub fn save_text(&self) -> String {
        self.app.document().serialize()
    }

    /// The name to download the document under: the opened file's, or untitled.md
    pub fn file_name(&self) -> String {
        self.app
            .document()
            .meta
            .path
            .as_deref()
            .and_then(Path::file_name)
            .map_or_else(
                || "untitled.md".into(),
                |n| n.to_string_lossy().into_owned(),
            )
    }

    /// Called from `requestAnimationFrame`: runs what is due (autosave, message expiry), draws if
    /// anything changed and hands the bytes to xterm.js.
    pub fn frame(&mut self) {
        self.turn(Vec::new());
        if self.app.needs_redraw {
            let app = &mut self.app;
            self.terminal
                .draw(|frame| mustty::view::render(frame, app))
                .unwrap_or_else(|e| match e {});
            self.app.needs_redraw = false;
        }
        let bytes = self.terminal.backend_mut().take();
        if !bytes.is_empty() {
            let _ = self
                .write
                .call1(&JsValue::NULL, &Uint8Array::from(bytes.as_slice()));
        }
    }
}

impl Session {
    fn turn(&mut self, events: Vec<Event>) {
        let turn = self.app.pump(events, Instant::now());
        let backend = self.terminal.backend_mut();
        for op in turn.ops {
            match op {
                // xterm.js reports the title through onTitleChange, the cursor shape is DECSCUSR, and
                // mouse capture (on or off, from the settings panel) is the ?1000h… / ?1000l… modes
                HostOp::Title(_) | HostOp::Cursor(_) | HostOp::MouseCapture(_) => {
                    backend.push(&op.to_ansi())
                }
                // Not wired up yet: no system clipboard
                HostOp::Clipboard(_) => {}
            }
        }
    }
}

/// Saves live in memory for the life of the page.
#[derive(Default)]
struct MemoryDocStore {
    files: HashMap<PathBuf, String>,
    revision: u64,
}

impl DocStore for MemoryDocStore {
    fn load(&self, path: &Path) -> Result<Document, IoError> {
        let text = self.files.get(path).map_or("", String::as_str);
        Ok(Document::parse(
            text,
            FileMeta::new(Some(path.to_path_buf())),
        ))
    }

    fn save(
        &mut self,
        doc: &Document,
        path: &Path,
        _check_disk: bool,
    ) -> Result<DiskStamp, IoError> {
        let text = doc.serialize();
        self.revision += 1;
        let stamp = DiskStamp {
            // Never read from a clock: a counter that only has to change on every save
            mtime: SystemTime::UNIX_EPOCH + Duration::from_secs(self.revision),
            len: text.len() as u64,
        };
        self.files.insert(path.to_path_buf(), text);
        Ok(stamp)
    }

    fn exists(&self, path: &Path) -> bool {
        self.files.contains_key(path)
    }

    fn untitled_name(&self, _dir: &Path) -> String {
        "untitled.md".into()
    }
}
