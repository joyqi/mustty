//! Token → Style（方案 §6.1）：只用 Modifier 与一个 ANSI 基本色，不写死 RGB、不设背景。

use ratatui::style::{Color, Modifier, Style};

use crate::keymap::Mode;
use crate::markdown::Token;

/// `code` 为行内代码与代码块的前景色（配置项 code_color，限 8 个 ANSI 基本色）。
pub fn style(token: Token, code: Color) -> Style {
    let m = |m: Modifier| Style::default().add_modifier(m);
    match token {
        Token::HeadingMark
        | Token::EmphMark
        | Token::StrongMark
        | Token::CodeMark
        | Token::FenceLine
        | Token::QuoteMark
        | Token::ListMark
        | Token::LinkPunct
        | Token::LinkUrl
        | Token::Rule => m(Modifier::DIM),
        Token::Heading(_) | Token::Strong => m(Modifier::BOLD),
        Token::Emph | Token::Quote => m(Modifier::ITALIC),
        Token::Code | Token::FencedCode => Style::default().fg(code),
        Token::LinkText => m(Modifier::UNDERLINED),
    }
}

/// vim 模式标识：INS 默认前景色（最醒目，表示能打字），NOR 与周围 status bar 一样 DIM（待命），
/// VIS 黄色（选区是特殊状态，vim 的通行约定）。文字本身已不同，颜色只是冗余强化。
pub fn mode(mode: Mode) -> Style {
    match mode {
        Mode::Insert => Style::default(),
        Mode::Normal(s) if s.visual => Style::default().fg(Color::Yellow),
        Mode::Normal(_) => Style::from(Modifier::DIM),
    }
}

/// 选区：反显，同样不设背景色。
pub fn selection() -> Style {
    Style::default().add_modifier(Modifier::REVERSED)
}
