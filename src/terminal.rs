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
