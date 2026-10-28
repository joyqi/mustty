//! 进入 / 恢复终端模式（方案 §10.6）。

use std::io;

use crossterm::cursor::SetCursorStyle;
use crossterm::event::{DisableFocusChange, EnableFocusChange};
use crossterm::execute;
use ratatui::DefaultTerminal;

/// 持有终端；`Drop` 时恢复光标形状、raw mode 与 alternate screen，`?` 早退也不例外。
/// panic 时由 `ratatui::try_init` 安装的 hook 先恢复终端再打印信息。
pub struct TerminalGuard {
    pub terminal: DefaultTerminal,
}

impl TerminalGuard {
    pub fn new() -> io::Result<Self> {
        let guard = Self {
            terminal: ratatui::try_init()?,
        };
        // Windows 的 Console API 不产生 Event::Paste，改由 app::coalesce_paste 合并
        #[cfg(not(windows))]
        execute!(io::stdout(), crossterm::event::EnableBracketedPaste)?;
        execute!(io::stdout(), EnableFocusChange, SetCursorStyle::SteadyBar)?;
        Ok(guard)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        #[cfg(not(windows))]
        let _ = execute!(io::stdout(), crossterm::event::DisableBracketedPaste);
        let _ = execute!(
            io::stdout(),
            DisableFocusChange,
            SetCursorStyle::DefaultUserShape
        );
        ratatui::restore();
    }
}

/// OSC 52 写系统剪贴板：`ESC ] 52 ; c ; <base64> BEL`。
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
}
