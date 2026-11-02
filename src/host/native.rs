//! The terminal host: the event loop, entering and restoring terminal modes, and documents and the
//! config file on disk.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::cursor::SetCursorStyle;
use crossterm::event::{
    self, DisableFocusChange, DisableMouseCapture, EnableFocusChange, EnableMouseCapture,
};
use crossterm::execute;
use crossterm::style::Print;
use ratatui::DefaultTerminal;
use web_time::Instant;

use crate::app::{App, coalesce_paste};
use crate::editor::document::{DiskStamp, Document};
use crate::host::{CLEAR_TITLE, ConfigStore, DocStore, HostCaps, HostOp};
use crate::io::file::{self, IoError, free_name};
use crate::view;

/// The terminal event loop: apply what the last turn left, draw, wait for input or the next deadline, pump.
pub fn run(terminal: &mut DefaultTerminal, app: &mut App) -> anyhow::Result<()> {
    // No event yet: the first turn only collects what startup left for the terminal
    let mut turn = app.pump(Vec::new(), Instant::now());
    loop {
        let mut written = false;
        for op in turn.ops {
            let backend = terminal.backend_mut();
            match op {
                // Through crossterm: on Windows, mouse capture is a console mode, not an escape sequence
                HostOp::MouseCapture(on) => set_mouse_capture(backend, on)?,
                // The window title (OSC 2) also goes straight to stdout: it doesn't move the cursor and bypasses ratatui's Buffer
                op => {
                    backend.write_all(op.to_ansi().as_bytes())?;
                    written = true;
                }
            }
        }
        if written {
            terminal.backend_mut().flush()?;
        }
        if turn.redraw {
            terminal.draw(|frame| view::render(frame, app))?;
            app.needs_redraw = false;
        }
        let timeout = turn
            .next
            .map(|d| d.saturating_duration_since(Instant::now()));
        let mut batch = Vec::new();
        let ready = match timeout {
            Some(t) => event::poll(t)?,
            None => true, // Nothing to wait for: block on read, so idle uses zero CPU
        };
        if ready {
            batch.push(event::read()?.into());
            // Drain the whole batch before redrawing, so several characters committed by the IME at once cost one redraw
            while event::poll(Duration::ZERO)? {
                batch.push(event::read()?.into());
            }
        }
        if cfg!(windows) {
            batch = coalesce_paste(batch);
        }
        turn = app.pump(batch, Instant::now());
        if app.should_quit {
            return Ok(());
        }
    }
}

/// Owns the terminal. `Drop` restores mouse, paste, focus events, cursor shape, window title, raw
/// mode and the alternate screen, even on an early `?` return. On panic, the hook wrapped below
/// restores the terminal before printing the message.
///
/// The settings panel can turn on mouse capture at runtime, so exit disables it **unconditionally**
/// (a no-op on a terminal where it was never enabled).
pub struct TerminalGuard {
    pub terminal: DefaultTerminal,
}

impl TerminalGuard {
    /// With `mouse` false, skip EnableMouseCapture so the terminal's native drag-to-select copy keeps working.
    pub fn new(mouse: bool) -> io::Result<Self> {
        let guard = Self {
            terminal: ratatui::try_init()?,
        };
        if mouse {
            execute!(io::stdout(), EnableMouseCapture)?;
        }
        // ratatui's hook only restores raw mode and the alternate screen: turn off the other modes first, then let it print the panic message
        let ratatui_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let mut out = io::stdout();
            let _ = execute!(out, DisableMouseCapture);
            #[cfg(not(windows))]
            let _ = execute!(out, crossterm::event::DisableBracketedPaste);
            let _ = execute!(
                out,
                DisableFocusChange,
                SetCursorStyle::DefaultUserShape,
                Print(CLEAR_TITLE)
            );
            ratatui_hook(info);
        }));
        // The Windows Console API never produces Event::Paste; app::coalesce_paste merges pasted input instead
        #[cfg(not(windows))]
        execute!(io::stdout(), crossterm::event::EnableBracketedPaste)?;
        execute!(io::stdout(), EnableFocusChange, SetCursorStyle::SteadyBar)?;
        Ok(guard)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), DisableMouseCapture);
        #[cfg(not(windows))]
        let _ = execute!(io::stdout(), crossterm::event::DisableBracketedPaste);
        let _ = execute!(
            io::stdout(),
            DisableFocusChange,
            SetCursorStyle::DefaultUserShape,
            Print(CLEAR_TITLE)
        );
        ratatui::restore();
    }
}

/// Toggle mouse capture at runtime. Uses `execute!` rather than a prebuilt ANSI string because on
/// Windows crossterm sets mouse capture through the WinAPI console mode, not escape sequences.
pub fn set_mouse_capture(out: &mut impl io::Write, on: bool) -> io::Result<()> {
    if on {
        execute!(out, EnableMouseCapture)
    } else {
        execute!(out, DisableMouseCapture)
    }
}

impl HostCaps {
    /// A terminal: OSC 52 reaches the clipboard except in macOS Terminal, and Quit exits.
    pub fn native() -> Self {
        Self {
            clipboard: std::env::var_os("TERM_PROGRAM").is_none_or(|t| t != "Apple_Terminal"),
            can_quit: true,
        }
    }
}

/// Documents as files: atomic saves, with the mtime and size as the stamp.
#[derive(Debug, Default)]
pub struct FsDocStore;

impl DocStore for FsDocStore {
    fn load(&self, path: &Path) -> Result<Document, IoError> {
        file::load(path)
    }

    fn save(
        &mut self,
        doc: &Document,
        path: &Path,
        check_disk: bool,
    ) -> Result<DiskStamp, IoError> {
        file::save(doc, path, check_disk)
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn untitled_name(&self, dir: &Path) -> String {
        untitled_name(dir)
    }
}

/// The first free `untitled*.md` in `dir`. A dangling symlink counts as taken (a write would follow it somewhere else).
pub fn untitled_name(dir: &Path) -> String {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    free_name(
        "untitled",
        "md",
        |name| fs::symlink_metadata(dir.join(name)).is_ok(),
        stamp,
    )
}

/// The config file on disk. A symlinked config (dotfiles kept in git) is resolved to its target,
/// so the atomic replace rewrites the target and the link itself survives.
#[derive(Debug)]
pub struct FsConfigStore {
    path: PathBuf,
}

impl FsConfigStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The real file behind `path`; `path` itself while nothing exists there yet.
    fn target(&self) -> io::Result<PathBuf> {
        match fs::canonicalize(&self.path) {
            Ok(p) => Ok(p),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(self.path.clone()),
            Err(e) => Err(e),
        }
    }
}

impl ConfigStore for FsConfigStore {
    fn read(&self) -> io::Result<Option<String>> {
        match fs::read_to_string(self.target()?) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn write(&mut self, text: &str) -> io::Result<()> {
        file::atomic_write(&self.target()?, text.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untitled_name_checks_the_directory() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(untitled_name(dir.path()), "untitled.md");
        fs::write(dir.path().join("untitled.md"), "").unwrap();
        assert_eq!(untitled_name(dir.path()), "untitled-2.md");
        // A directory counts as taken
        fs::create_dir(dir.path().join("untitled-2.md")).unwrap();
        assert_eq!(untitled_name(dir.path()), "untitled-3.md");
        // So does a dangling symlink
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("nowhere", dir.path().join("untitled-3.md")).unwrap();
            assert_eq!(untitled_name(dir.path()), "untitled-4.md");
        }
    }
}
