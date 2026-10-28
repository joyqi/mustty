//! Token → Style（方案 §6.1）：只用 Modifier 与一个 ANSI 基本色，不写死 RGB、不设背景。

use ratatui::style::{Color, Modifier, Style};

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

/// 选区：反显，同样不设背景色。
pub fn selection() -> Style {
    Style::default().add_modifier(Modifier::REVERSED)
}
