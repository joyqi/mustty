//! 与按键解耦的编辑语义。全部变体在 M1 定稿，M4 不再增删；
//! 有意的修订只有两处：设置面板的 `OpenSettings` 与键位帮助的 `OpenHelp`。
//! M1 未绑定按键或未实现的变体由 `Editor::apply` 忽略。

use crossterm::event::KeyEvent;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Insert(String),
    Paste(String),
    Newline,
    DeleteBack,
    DeleteForward,
    DeleteWordBack,
    /// `bool`：是否扩展选区（M3）
    Move(Motion, bool),
    Undo,
    Redo,
    Save,
    Quit,
    Cancel,
    SelectAll,
    Copy,
    Cut,
    /// Ctrl+V / vim `p`：粘贴内部寄存器（行级寄存器粘贴到下一行）
    PasteRegister,
    /// vim `dd`：剪切整个逻辑行进寄存器（行级）
    DeleteLine,
    /// vim `o`：移到逻辑行尾并插入 `\n`，作为一个 Transaction
    OpenLineBelow,
    ToggleTypewriter,
    ToggleFocusMode,
    /// Ctrl+P：打开设置面板，由 App 消费
    OpenSettings,
    /// Ctrl+G（nano 的 ^G help）/ vim Normal 态 `?`：打开键位帮助，由 App 消费
    OpenHelp,
    PromptInput(KeyEvent),
    /// 鼠标单击 / 拖拽：直接定位到 char 索引；`bool` 为是否扩展选区
    MoveTo(Target, bool),
    /// 鼠标滚轮：由 App / Viewport 消费，不进入 `Editor::apply`
    ScrollView(i32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    LineStart,
    LineEnd,
    PageUp,
    PageDown,
    DocStart,
    DocEnd,
    /// 上一个词首（Ctrl+← / vim `b`）
    WordLeft,
    /// 下一个词首（Ctrl+→ / vim `w`）
    WordRight,
    /// 当前或下一个词尾（仅 vim `e` 绑定）
    WordEnd,
}

/// 鼠标定位的目标：正文全局 char 索引，或标题内 char 索引。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Body(usize),
    Title(usize),
}
