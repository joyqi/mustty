//! Token → Style: only Modifiers and one basic ANSI color. No hard-coded RGB, no background.

use ratatui::style::{Color, Modifier, Style};

use crate::keymap::Mode;
use crate::markdown::Token;

/// `code` is the foreground color for inline code and code blocks (the code_color setting, limited to the 8 basic ANSI colors).
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

/// vim mode indicator: INS uses the default foreground (most prominent: you can type), NOR is DIM
/// like the rest of the status bar (idle), and VIS is yellow (a selection is a special state, as vim
/// conventionally shows it). The labels already differ; the color is only redundant reinforcement.
pub fn mode(mode: Mode) -> Style {
    match mode {
        Mode::Insert => Style::default(),
        Mode::Normal(s) if s.visual => Style::default().fg(Color::Yellow),
        Mode::Normal(_) => Style::from(Modifier::DIM),
    }
}

/// Selection: reversed, again with no background color.
pub fn selection() -> Style {
    Style::default().add_modifier(Modifier::REVERSED)
}
