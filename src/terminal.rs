//! 进入 / 恢复终端模式。

use std::io;

use crossterm::cursor::SetCursorStyle;
use crossterm::event::{
    DisableFocusChange, DisableMouseCapture, EnableFocusChange, EnableMouseCapture,
};
use crossterm::execute;
use crossterm::style::Print;
use ratatui::DefaultTerminal;

/// 持有终端；`Drop` 时恢复鼠标、粘贴、焦点事件、光标形状、窗口标题、raw mode 与 alternate screen，
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
            let _ = execute!(
                out,
                DisableFocusChange,
                SetCursorStyle::DefaultUserShape,
                Print(CLEAR_TITLE)
            );
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
            SetCursorStyle::DefaultUserShape,
            Print(CLEAR_TITLE)
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

/// 退出时清空窗口标题（OSC 2 空标题），免得编辑器关掉后窗口还挂着文章标题。
pub const CLEAR_TITLE: &str = "\x1b]2;\x07";

/// OSC 2 设置窗口标题：`ESC ] 2 ; <title> BEL`。控制字符一律去掉（`BEL` / `ESC` 会提前结束序列或注入新的序列），
/// 调用方已过滤时这里不改变任何东西。
pub fn osc2(title: &str) -> String {
    format!("\x1b]2;{}\x07", strip_controls(title))
}

/// 去掉 C0、DEL 与 C1 控制字符（C1 里的 0x9B / 0x9C 在 8 位终端上就是 CSI / ST）。
pub fn strip_controls(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect()
}

/// 窗口标题去重：同一个标题只写一次序列。标题编辑时每敲一个字都会变，其余时候不发。
#[derive(Debug, Default)]
pub struct WindowTitle {
    last: Option<String>,
}

impl WindowTitle {
    /// 标题与上次写出的不同时返回要写的 OSC 2 序列，并记下它。
    pub fn update(&mut self, title: &str) -> Option<String> {
        if self.last.as_deref() == Some(title) {
            return None;
        }
        self.last = Some(title.to_owned());
        Some(osc2(title))
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
        assert_eq!(w.update("Notes"), None, "同一个标题不再发");
        assert_eq!(w.update("Notes!").as_deref(), Some("\x1b]2;Notes!\x07"));
        assert_eq!(w.update("Notes!"), None);
        // 改回去也是变化
        assert_eq!(w.update("Notes").as_deref(), Some("\x1b]2;Notes\x07"));
        // 空标题（理论上不会出现）也照常去重
        assert_eq!(w.update("").as_deref(), Some(CLEAR_TITLE));
        assert_eq!(w.update(""), None);
    }
}
