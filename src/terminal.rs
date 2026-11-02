//! Entering and restoring terminal modes.

use std::io;

use crossterm::cursor::SetCursorStyle;
use crossterm::event::{
    DisableFocusChange, DisableMouseCapture, EnableFocusChange, EnableMouseCapture,
};
use crossterm::execute;
use crossterm::style::Print;
use ratatui::DefaultTerminal;

use crate::host::CLEAR_TITLE;

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
