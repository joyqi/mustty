//! A ratatui backend that writes ANSI escape sequences into a byte buffer, for hosts that hand the
//! bytes to a terminal emulator themselves (xterm.js in the browser).
//!
//! `Terminal` already keeps the previous frame and passes only the changed cells to `draw`
//! (`Buffer::diff`), so this only encodes them: move the cursor where the next cell is, switch the
//! style when it changes, print the symbol. What reaches the screen is the same as with crossterm's
//! backend; the bytes differ (named colors use SGR 30–37 / 90–97 and moves are relative when shorter).

use std::convert::Infallible;
use std::fmt::Write as _;

use ratatui::backend::{Backend, ClearType, WindowSize};
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Size};
use ratatui::style::{Color, Modifier};
use unicode_width::UnicodeWidthStr;

#[derive(Debug)]
pub struct AnsiBackend {
    out: String,
    size: Size,
    /// Where the terminal's cursor is, or None when it isn't known: at the start, after `append_lines`,
    /// and after printing into the last column (the terminal is then waiting to wrap)
    cursor: Option<Position>,
    /// The cursor position ratatui last asked for, reported back by `get_cursor_position`
    requested: Position,
}

impl AnsiBackend {
    pub fn new(size: Size) -> Self {
        Self {
            out: String::new(),
            size,
            cursor: None,
            requested: Position::ORIGIN,
        }
    }

    /// The terminal was resized; the next `Terminal::draw` notices through `size()` and redraws everything.
    pub fn resize(&mut self, size: Size) {
        self.size = size;
        self.cursor = None;
    }

    /// Appends a sequence that doesn't move the cursor (a `HostOp::to_ansi`), keeping its order relative to the frames.
    pub fn push(&mut self, seq: &str) {
        self.out.push_str(seq);
    }

    /// Takes everything written since the last call.
    pub fn take(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.out).into_bytes()
    }

    /// Moves to `to`, relatively when the cursor is known and that's shorter than an absolute move.
    fn move_to(&mut self, to: Position) {
        match self.cursor {
            Some(at) if at == to => {}
            Some(at) if at.y == to.y && at.x < to.x => match to.x - at.x {
                1 => self.out.push_str("\x1b[C"),
                n => write!(self.out, "\x1b[{n}C").unwrap(),
            },
            Some(at) if at.y == to.y && to.x == 0 => self.out.push('\r'),
            _ => write!(self.out, "\x1b[{};{}H", to.y + 1, to.x + 1).unwrap(),
        }
        self.cursor = Some(to);
    }
}

/// SGR parameters that turn the `from` modifiers into `to`. Bold and Dim share one reset (22), so
/// whichever stays on is set again afterwards, as crossterm's backend does.
fn modifier_params(from: Modifier, to: Modifier, params: &mut Vec<String>) {
    let removed = from - to;
    let added = to - from;
    let reset_intensity = removed.intersects(Modifier::BOLD | Modifier::DIM);
    let mut push = |n: u8| params.push(n.to_string());
    if reset_intensity {
        push(22);
    }
    for (modifier, off) in [
        (Modifier::ITALIC, 23),
        (Modifier::UNDERLINED, 24),
        (Modifier::REVERSED, 27),
        (Modifier::HIDDEN, 28),
        (Modifier::CROSSED_OUT, 29),
    ] {
        if removed.contains(modifier) {
            push(off);
        }
    }
    if removed.intersects(Modifier::SLOW_BLINK | Modifier::RAPID_BLINK) {
        push(25);
    }
    // After a 22 both intensities are off, so whichever is still wanted comes back on
    let on = if reset_intensity { to } else { added };
    for (modifier, code) in [
        (Modifier::BOLD, 1),
        (Modifier::DIM, 2),
        (Modifier::ITALIC, 3),
        (Modifier::UNDERLINED, 4),
        (Modifier::SLOW_BLINK, 5),
        (Modifier::RAPID_BLINK, 6),
        (Modifier::REVERSED, 7),
        (Modifier::HIDDEN, 8),
        (Modifier::CROSSED_OUT, 9),
    ] {
        let wanted = match modifier {
            Modifier::BOLD | Modifier::DIM => on.contains(modifier),
            _ => added.contains(modifier),
        };
        // A blink reset (25) clears both kinds: put back the one that stays
        let blink_back = matches!(modifier, Modifier::SLOW_BLINK | Modifier::RAPID_BLINK)
            && removed.intersects(Modifier::SLOW_BLINK | Modifier::RAPID_BLINK)
            && to.contains(modifier);
        if wanted || blink_back {
            push(code);
        }
    }
}

/// SGR parameters for a color; `base` is 30 for the foreground and 40 for the background.
fn color_params(color: Color, base: u8, params: &mut Vec<String>) {
    let named = |n: u8| (base + n).to_string();
    let bright = |n: u8| (base + 60 + n).to_string();
    params.push(match color {
        Color::Reset => (base + 9).to_string(),
        Color::Black => named(0),
        Color::Red => named(1),
        Color::Green => named(2),
        Color::Yellow => named(3),
        Color::Blue => named(4),
        Color::Magenta => named(5),
        Color::Cyan => named(6),
        Color::Gray => named(7),
        Color::DarkGray => bright(0),
        Color::LightRed => bright(1),
        Color::LightGreen => bright(2),
        Color::LightYellow => bright(3),
        Color::LightBlue => bright(4),
        Color::LightMagenta => bright(5),
        Color::LightCyan => bright(6),
        Color::White => bright(7),
        Color::Indexed(i) => format!("{};5;{i}", base + 8),
        Color::Rgb(r, g, b) => format!("{};2;{r};{g};{b}", base + 8),
    });
}

impl Backend for AnsiBackend {
    type Error = Infallible;

    fn draw<'a, I>(&mut self, content: I) -> Result<(), Self::Error>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        // Every draw starts and ends with the default style, like crossterm's backend
        let (mut fg, mut bg, mut modifier) = (Color::Reset, Color::Reset, Modifier::empty());
        let mut params = Vec::new();
        for (x, y, cell) in content {
            self.move_to(Position { x, y });
            params.clear();
            if cell.modifier != modifier {
                modifier_params(modifier, cell.modifier, &mut params);
                modifier = cell.modifier;
            }
            if cell.fg != fg {
                color_params(cell.fg, 30, &mut params);
                fg = cell.fg;
            }
            if cell.bg != bg {
                color_params(cell.bg, 40, &mut params);
                bg = cell.bg;
            }
            if !params.is_empty() {
                write!(self.out, "\x1b[{}m", params.join(";")).unwrap();
            }
            let symbol = cell.symbol();
            self.out.push_str(symbol);
            // Printing into the last column leaves the terminal waiting to wrap: don't guess where it is
            let end = x.saturating_add(symbol.width() as u16);
            self.cursor = (end < self.size.width).then_some(Position { x: end, y });
        }
        if (fg, bg, modifier) != (Color::Reset, Color::Reset, Modifier::empty()) {
            self.out.push_str("\x1b[0m");
        }
        Ok(())
    }

    fn append_lines(&mut self, n: u16) -> Result<(), Self::Error> {
        for _ in 0..n {
            self.out.push('\n');
        }
        self.cursor = None;
        Ok(())
    }

    fn hide_cursor(&mut self) -> Result<(), Self::Error> {
        self.out.push_str("\x1b[?25l");
        Ok(())
    }

    fn show_cursor(&mut self) -> Result<(), Self::Error> {
        self.out.push_str("\x1b[?25h");
        Ok(())
    }

    fn get_cursor_position(&mut self) -> Result<Position, Self::Error> {
        Ok(self.requested)
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> Result<(), Self::Error> {
        let to = position.into();
        self.requested = to;
        // Always absolute: this is where the IME draws its candidates, so don't depend on tracking
        self.cursor = None;
        self.move_to(to);
        Ok(())
    }

    fn clear(&mut self) -> Result<(), Self::Error> {
        self.clear_region(ClearType::All)
    }

    fn clear_region(&mut self, clear_type: ClearType) -> Result<(), Self::Error> {
        self.out.push_str(match clear_type {
            ClearType::All => "\x1b[2J",
            ClearType::AfterCursor => "\x1b[J",
            ClearType::BeforeCursor => "\x1b[1J",
            ClearType::CurrentLine => "\x1b[2K",
            ClearType::UntilNewLine => "\x1b[K",
        });
        Ok(())
    }

    fn size(&self) -> Result<Size, Self::Error> {
        Ok(self.size)
    }

    fn window_size(&mut self) -> Result<WindowSize, Self::Error> {
        Ok(WindowSize {
            columns_rows: self.size,
            pixels: Size::default(),
        })
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::Style;

    fn sgr(from: Modifier, to: Modifier) -> String {
        let mut params = Vec::new();
        modifier_params(from, to, &mut params);
        params.join(";")
    }

    /// vt100 doesn't track hidden, crossed-out or blink, so the differential test can't see them
    #[test]
    fn modifiers_vt100_ignores_still_switch() {
        let m = |s: &str| Modifier::from_name(s).unwrap();
        assert_eq!(
            sgr(Modifier::empty(), m("HIDDEN") | m("CROSSED_OUT")),
            "8;9"
        );
        assert_eq!(
            sgr(m("HIDDEN") | m("CROSSED_OUT"), Modifier::empty()),
            "28;29"
        );
        assert_eq!(sgr(Modifier::empty(), m("SLOW_BLINK")), "5");
        // 25 ends both blinks, so the one that stays is set again
        assert_eq!(
            sgr(m("SLOW_BLINK") | m("RAPID_BLINK"), m("RAPID_BLINK")),
            "25;6"
        );
        // 22 ends both intensities, likewise
        assert_eq!(sgr(m("BOLD") | m("DIM"), m("DIM")), "22;2");
        assert_eq!(sgr(m("BOLD"), m("DIM")), "22;2");
    }

    fn drawn(buf: &Buffer) -> String {
        let mut b = AnsiBackend::new(Size::new(buf.area.width, buf.area.height));
        b.draw(Buffer::empty(buf.area).diff(buf).into_iter())
            .unwrap();
        String::from_utf8(b.take()).unwrap()
    }

    #[test]
    fn moves_are_relative_when_shorter_and_absolute_after_the_last_column() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 6, 2));
        buf.set_string(0, 0, "中", Style::default());
        buf.set_string(3, 0, "ab", Style::default());
        buf.set_string(5, 0, "c", Style::default());
        buf.set_string(0, 1, "d", Style::default());
        // After 中 the cursor is at column 2: one step right to 3. After the last column it is unknown
        assert_eq!(drawn(&buf), "\x1b[1;1H中\x1b[Cabc\x1b[2;1Hd");
    }

    #[test]
    fn pushed_sequences_keep_their_place_between_frames() {
        let mut b = AnsiBackend::new(Size::new(4, 1));
        b.push("\x1b[2 q");
        b.hide_cursor().unwrap();
        b.set_cursor_position((2, 0)).unwrap();
        assert_eq!(
            String::from_utf8(b.take()).unwrap(),
            "\x1b[2 q\x1b[?25l\x1b[1;3H"
        );
        assert!(b.take().is_empty());
        assert_eq!(b.get_cursor_position().unwrap(), Position::new(2, 0));
    }
}
