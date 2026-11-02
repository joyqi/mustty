//! What the App needs from wherever it runs, and what it asks that host to do.
//!
//! The App never writes to the terminal or the file system itself. Side effects it only announces
//! (clipboard, window title, cursor shape, mouse capture) are `HostOp` values the host takes after
//! each batch of events; the two things it needs an answer from (documents and the config file) are
//! the `DocStore` and `ConfigStore` traits. Time comes in as the `now` every entry point takes.

use std::io;
use std::path::Path;

use web_time::Instant;

use crate::config::CursorShape;
use crate::editor::document::{DiskStamp, Document};
use crate::io::file::IoError;

#[cfg(feature = "native")]
pub mod native;

/// A side effect the App asks the host to carry out, in the order it should happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostOp {
    /// Put text on the system clipboard
    Clipboard(String),
    /// Set the window title (control characters are stripped when encoding)
    Title(String),
    Cursor(CursorStyle),
    MouseCapture(bool),
}

impl HostOp {
    /// The escape sequence a terminal understands for this operation. The native host sends mouse
    /// capture through crossterm instead, since Windows sets it through the console API.
    pub fn to_ansi(&self) -> String {
        match self {
            Self::Clipboard(text) => osc52(text),
            Self::Title(title) => osc2(title),
            Self::Cursor(style) => style.to_ansi(),
            Self::MouseCapture(true) => ENABLE_MOUSE_CAPTURE.to_owned(),
            Self::MouseCapture(false) => DISABLE_MOUSE_CAPTURE.to_owned(),
        }
    }
}

/// Same sequences, in the same order, as crossterm's `EnableMouseCapture` / `DisableMouseCapture`.
pub const ENABLE_MOUSE_CAPTURE: &str = "\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1015h\x1b[?1006h";
pub const DISABLE_MOUSE_CAPTURE: &str = "\x1b[?1006l\x1b[?1015l\x1b[?1003l\x1b[?1002l\x1b[?1000l";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorStyle {
    pub shape: CursorShape,
    pub blink: bool,
}

impl CursorStyle {
    /// vim Normal mode's cursor
    pub const STEADY_BLOCK: Self = Self {
        shape: CursorShape::Block,
        blink: false,
    };

    /// DECSCUSR: `CSI Ps SP q`, blinking odd and steady even.
    pub fn to_ansi(self) -> String {
        let n = match self.shape {
            CursorShape::Block => 1,
            CursorShape::Underline => 3,
            CursorShape::Bar => 5,
        } + u8::from(!self.blink);
        format!("\x1b[{n} q")
    }
}

/// What the host can do, decided by the host rather than probed from inside the App.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostCaps {
    /// Whether `HostOp::Clipboard` reaches the system clipboard; without it, the first copy warns once
    pub clipboard: bool,
    /// Whether Quit can end the program (a browser tab can't close itself)
    pub can_quit: bool,
}

/// Where documents are loaded from and saved to.
pub trait DocStore {
    /// Opens a document; one that doesn't exist yet comes back empty and remembers the path.
    fn load(&self, path: &Path) -> Result<Document, IoError>;
    /// Saves `doc` to `path` and returns the new stamp. With `check_disk`, gives up with
    /// `ExternallyModified` when what is stored no longer matches `doc.meta.stamp`.
    fn save(&mut self, doc: &Document, path: &Path, check_disk: bool)
    -> Result<DiskStamp, IoError>;
    fn exists(&self, path: &Path) -> bool;
    /// The name prefilled when saving a document that has no path yet.
    fn untitled_name(&self, dir: &Path) -> String;
}

/// The config file's text. Parsing and editing it stay in `config`, so every store keeps the
/// user's comments and key order the same way.
pub trait ConfigStore {
    /// None when there is no config file yet.
    fn read(&self) -> io::Result<Option<String>>;
    fn write(&mut self, text: &str) -> io::Result<()>;
}

/// What one turn of the event loop leaves for the host.
#[derive(Debug)]
pub struct Pump {
    pub ops: Vec<HostOp>,
    /// Draw a frame (the host clears `App::needs_redraw` once it has)
    pub redraw: bool,
    /// When to run the next turn even if no event arrives
    pub next: Option<Instant>,
}

/// Clear the window title on exit (an empty OSC 2 title) so the window doesn't keep showing the document title after the editor closes.
pub const CLEAR_TITLE: &str = "\x1b]2;\x07";

/// Set the window title with OSC 2: `ESC ] 2 ; <title> BEL`. All control characters are stripped (`BEL` / `ESC` would end the sequence early or inject a new one);
/// if the caller already filtered them, this changes nothing.
pub fn osc2(title: &str) -> String {
    format!("\x1b]2;{}\x07", strip_controls(title))
}

/// Strip C0, DEL and C1 control characters (C1's 0x9B / 0x9C act as CSI / ST on 8-bit terminals).
pub fn strip_controls(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect()
}

/// Window title dedup: write the sequence only once per title. While editing the title it changes on every keystroke; otherwise nothing is sent.
#[derive(Debug, Default)]
pub struct WindowTitle {
    last: Option<String>,
}

impl WindowTitle {
    /// When the title differs from the last one written, return the OSC 2 sequence to write and remember it.
    pub fn update(&mut self, title: &str) -> Option<String> {
        if self.last.as_deref() == Some(title) {
            return None;
        }
        self.last = Some(title.to_owned());
        Some(osc2(title))
    }
}

/// Write the system clipboard with OSC 52: `ESC ] 52 ; c ; <base64> BEL`.
pub fn osc52(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", base64(text.as_bytes()))
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | (b as u32) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc4648() {
        let cases = [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foobar", "Zm9vYmFy"),
            ("中文", "5Lit5paH"),
        ];
        for (input, want) in cases {
            assert_eq!(base64(input.as_bytes()), want);
        }
        assert_eq!(osc52("hi"), "\x1b]52;c;aGk=\x07");
    }

    #[test]
    fn osc2_strips_control_characters() {
        assert_eq!(osc2("春天"), "\x1b]2;春天\x07");
        assert_eq!(osc2("a\x07b\x1b]2;evil\x07c"), "\x1b]2;ab]2;evilc\x07");
        assert_eq!(
            osc2("tab\there\r\n\u{7f}\u{9b}31m"),
            "\x1b]2;tabhere31m\x07"
        );
        assert_eq!(CLEAR_TITLE, osc2(""));
    }

    #[test]
    fn window_title_is_sent_only_when_it_changes() {
        let mut w = WindowTitle::default();
        assert_eq!(w.update("Notes").as_deref(), Some("\x1b]2;Notes\x07"));
        assert_eq!(w.update("Notes"), None, "the same title is not sent again");
        assert_eq!(w.update("Notes!").as_deref(), Some("\x1b]2;Notes!\x07"));
        assert_eq!(w.update("Notes!"), None);
        // Changing back counts as a change too
        assert_eq!(w.update("Notes").as_deref(), Some("\x1b]2;Notes\x07"));
        // An empty title (shouldn't happen in practice) is deduplicated the same way
        assert_eq!(w.update("").as_deref(), Some(CLEAR_TITLE));
        assert_eq!(w.update(""), None);
    }

    #[cfg(feature = "native")]
    #[test]
    fn ops_encode_like_crossterm() {
        fn ansi(c: impl crossterm::Command) -> String {
            let mut out = String::new();
            c.write_ansi(&mut out).unwrap();
            out
        }
        for shape in CursorShape::ALL {
            for blink in [false, true] {
                let style = CursorStyle { shape, blink };
                assert_eq!(
                    HostOp::Cursor(style).to_ansi(),
                    ansi(shape.style(blink)),
                    "{style:?}"
                );
            }
        }
        assert_eq!(
            CursorStyle::STEADY_BLOCK.to_ansi(),
            ansi(crossterm::cursor::SetCursorStyle::SteadyBlock)
        );
        assert_eq!(
            HostOp::MouseCapture(true).to_ansi(),
            ansi(crossterm::event::EnableMouseCapture)
        );
        assert_eq!(
            HostOp::MouseCapture(false).to_ansi(),
            ansi(crossterm::event::DisableMouseCapture)
        );
        assert_eq!(HostOp::Title("a\x07b".into()).to_ansi(), osc2("a\x07b"));
        assert_eq!(HostOp::Clipboard("hi".into()).to_ansi(), osc52("hi"));
    }

    /// `std::time::Instant::now()` panics on wasm and `SystemTime::now()` has no clock there: outside
    /// the native host (and the native binary), time comes from `web_time` or from the caller's `now`.
    /// Only code before a file's `#[cfg(test)]` is checked, since tests always run natively.
    #[test]
    fn platform_clock_stays_in_the_native_host() {
        fn sources(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    sources(&path, out);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    out.push(path);
                }
            }
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let allowed = [root.join("host/native.rs"), root.join("main.rs")];
        let mut files = Vec::new();
        sources(&root, &mut files);
        assert!(files.len() > 20, "found only {files:?}");
        for file in files.iter().filter(|f| !allowed.contains(f)) {
            let text = std::fs::read_to_string(file).unwrap();
            let code = text.split("#[cfg(test)]").next().unwrap();
            // Also catches `use std::time::{Duration, Instant}`
            let std_instant = code.lines().any(|line| {
                line.split_once("std::time::").is_some_and(|(_, rest)| {
                    rest.split(|c: char| !c.is_alphanumeric() && c != '_')
                        .any(|w| w == "Instant")
                })
            });
            assert!(
                !std_instant,
                "{}: std::time::Instant (use web_time::Instant)",
                file.display()
            );
            assert!(
                !code.contains("SystemTime::now"),
                "{}: SystemTime::now belongs in host/native.rs",
                file.display()
            );
        }
    }
}
