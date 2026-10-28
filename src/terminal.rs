//! 进入 / 恢复终端模式（方案 §10.6）。

use std::io;

use crossterm::cursor::SetCursorStyle;
use crossterm::event::{
    DisableFocusChange, DisableMouseCapture, EnableFocusChange, EnableMouseCapture,
};
use crossterm::execute;
use ratatui::DefaultTerminal;

/// 持有终端；`Drop` 时恢复鼠标、粘贴、焦点事件、光标形状、raw mode 与 alternate screen，
/// `?` 早退也不例外。panic 时由下面包装的 hook 先恢复终端再打印信息。
///
/// 鼠标捕获可能在运行中由设置面板打开，所以退出时**无条件**关闭（对没开过的终端无副作用）。
pub struct TerminalGuard {
    pub terminal: DefaultTerminal,
}

impl TerminalGuard {
    /// `mouse` 为 false 时不执行 EnableMouseCapture，终端原生的拖选复制照常可用。
    pub fn new(mouse: bool) -> io::Result<Self> {
        let guard = Self {
            terminal: ratatui::try_init()?,
        };
        if mouse {
            execute!(io::stdout(), EnableMouseCapture)?;
        }
        // ratatui 的 hook 只恢复 raw mode 与 alternate screen：先关掉其余模式，再交给它打印 panic 信息
        let ratatui_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let mut out = io::stdout();
            let _ = execute!(out, DisableMouseCapture);
            #[cfg(not(windows))]
            let _ = execute!(out, crossterm::event::DisableBracketedPaste);
            let _ = execute!(out, DisableFocusChange, SetCursorStyle::DefaultUserShape);
            ratatui_hook(info);
        }));
        // Windows 的 Console API 不产生 Event::Paste，改由 app::coalesce_paste 合并
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
            SetCursorStyle::DefaultUserShape
        );
        ratatui::restore();
    }
}

/// 运行中开关鼠标捕获。用 `execute!` 而不是预先拼好的 ANSI 字符串：
/// Windows 上 crossterm 的鼠标捕获走 WinAPI 的控制台模式，不是转义序列。
pub fn set_mouse_capture(out: &mut impl io::Write, on: bool) -> io::Result<()> {
    if on {
        execute!(out, EnableMouseCapture)
    } else {
        execute!(out, DisableMouseCapture)
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
