//! 进入 / 恢复终端模式（方案 §10.6）。

use std::io;

use crossterm::cursor::SetCursorStyle;
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
        execute!(io::stdout(), SetCursorStyle::SteadyBar)?;
        Ok(guard)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), SetCursorStyle::DefaultUserShape);
        ratatui::restore();
    }
}
