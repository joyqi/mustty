//! Entering and restoring terminal modes.

use std::io;

use crossterm::cursor::SetCursorStyle;
use crossterm::event::{
    DisableFocusChange, DisableMouseCapture, EnableFocusChange, EnableMouseCapture,
};
use crossterm::execute;
use crossterm::style::Print;
use ratatui::DefaultTerminal;

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
}
