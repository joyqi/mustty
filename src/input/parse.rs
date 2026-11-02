//! Terminal input bytes → `Event`, for hosts that receive what a terminal emulator sends instead of
//! reading a tty (xterm.js's `onData`).
//!
//! Ported from crossterm 0.29, `src/event/sys/unix/parse.rs` and the `Parser` driver in
//! `src/event/source/unix/mio.rs`
//! (<https://github.com/crossterm-rs/crossterm/blob/0.29/src/event/sys/unix/parse.rs>), so the
//! keymap sees exactly the events it sees natively. Kept: what xterm.js sends with the modes mustty
//! enables (keys, SS3, `CSI ~` / modifier codes, SGR mouse, focus, bracketed paste). Dropped: the kitty
//! keyboard protocol, rxvt / X10 mouse and replies to terminal queries, which mustty never asks for.
//! Input arrives as in raw mode: `\n` is Ctrl+J.
//!
//! crossterm is MIT licensed:
//!
//! > MIT License
//! >
//! > Copyright (c) 2019 Timon
//! >
//! > Permission is hereby granted, free of charge, to any person obtaining a copy of this software
//! > and associated documentation files (the "Software"), to deal in the Software without
//! > restriction, including without limitation the rights to use, copy, modify, merge, publish,
//! > distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the
//! > Software is furnished to do so, subject to the following conditions:
//! >
//! > The above copyright notice and this permission notice shall be included in all copies or
//! > substantial portions of the Software.
//! >
//! > THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING
//! > BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
//! > NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,
//! > DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
//! > OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

use super::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};

/// Incremental parser. Bytes of an unfinished sequence (including a bracketed paste still waiting
/// for its end marker) are kept until the next `feed`.
#[derive(Debug, Default)]
pub struct InputParser {
    buffer: Vec<u8>,
}

impl InputParser {
    /// Parses one chunk of input. A chunk is one delivery from the terminal, so an `ESC` that ends
    /// it is the Esc key rather than the start of a sequence still on its way.
    pub fn feed(&mut self, bytes: &[u8], out: &mut Vec<Event>) {
        for (i, &byte) in bytes.iter().enumerate() {
            self.buffer.push(byte);
            match parse_event(&self.buffer, i + 1 < bytes.len()) {
                Ok(Some(ev)) => {
                    out.push(ev);
                    self.buffer.clear();
                }
                // A sequence that isn't complete yet
                Ok(None) => {}
                // Not something we understand: drop it and start over with the next byte
                Err(Malformed) => self.buffer.clear(),
            }
        }
    }
}

/// The buffer can't become an event (crossterm's `could_not_parse_event_error`)
#[derive(Debug)]
struct Malformed;

/// `Ok(None)`: wait for more bytes; `Err`: discard the buffer; `Ok(Some)`: an event, clear the buffer
type Parsed = Result<Option<Event>, Malformed>;

fn key(code: KeyCode) -> Parsed {
    Ok(Some(Event::Key(code.into())))
}

fn ctrl(c: char) -> Parsed {
    Ok(Some(Event::Key(KeyEvent::new(
        KeyCode::Char(c),
        KeyModifiers::CONTROL,
    ))))
}

fn parse_event(buffer: &[u8], input_available: bool) -> Parsed {
    let Some(&first) = buffer.first() else {
        return Ok(None);
    };
    match first {
        b'\x1B' => match buffer.get(1) {
            // Possibly the start of a sequence
            None if input_available => Ok(None),
            None => key(KeyCode::Esc),
            Some(b'O') => match buffer.get(2) {
                None => Ok(None),
                Some(b'D') => key(KeyCode::Left),
                Some(b'C') => key(KeyCode::Right),
                Some(b'A') => key(KeyCode::Up),
                Some(b'B') => key(KeyCode::Down),
                Some(b'H') => key(KeyCode::Home),
                Some(b'F') => key(KeyCode::End),
                // F1-F4
                Some(&val @ b'P'..=b'S') => key(KeyCode::F(1 + val - b'P')),
                Some(_) => Err(Malformed),
            },
            Some(b'[') => parse_csi(buffer),
            Some(b'\x1B') => key(KeyCode::Esc),
            // ESC + anything else: that key with Alt
            Some(_) => parse_event(&buffer[1..], input_available).map(|ev| {
                ev.map(|ev| match ev {
                    Event::Key(mut k) => {
                        k.modifiers |= KeyModifiers::ALT;
                        Event::Key(k)
                    }
                    other => other,
                })
            }),
        },
        b'\r' => key(KeyCode::Enter),
        b'\t' => key(KeyCode::Tab),
        b'\x7F' => key(KeyCode::Backspace),
        c @ b'\x01'..=b'\x1A' => ctrl((c - 0x1 + b'a') as char),
        c @ b'\x1C'..=b'\x1F' => ctrl((c - 0x1C + b'4') as char),
        b'\0' => ctrl(' '),
        _ => parse_utf8_char(buffer).map(|c| c.map(|c| Event::Key(char_code_to_event(c)))),
    }
}

/// Uppercase characters come with SHIFT.
fn char_code_to_event(c: char) -> KeyEvent {
    let modifiers = if c.is_uppercase() {
        KeyModifiers::SHIFT
    } else {
        KeyModifiers::NONE
    };
    KeyEvent::new(KeyCode::Char(c), modifiers)
}

fn parse_csi(buffer: &[u8]) -> Parsed {
    debug_assert!(buffer.starts_with(b"\x1B["));
    let Some(&third) = buffer.get(2) else {
        return Ok(None);
    };
    match third {
        // Linux console F1-F5
        b'[' => match buffer.get(3) {
            None => Ok(None),
            Some(&val @ b'A'..=b'E') => key(KeyCode::F(1 + val - b'A')),
            Some(_) => Err(Malformed),
        },
        b'D' => key(KeyCode::Left),
        b'C' => key(KeyCode::Right),
        b'A' => key(KeyCode::Up),
        b'B' => key(KeyCode::Down),
        b'H' => key(KeyCode::Home),
        b'F' => key(KeyCode::End),
        b'Z' => Ok(Some(Event::Key(KeyEvent::new(
            KeyCode::BackTab,
            KeyModifiers::SHIFT,
        )))),
        b'<' => parse_csi_sgr_mouse(buffer),
        b'I' => Ok(Some(Event::FocusGained)),
        b'O' => Ok(Some(Event::FocusLost)),
        b';' => parse_csi_modifier_key_code(buffer),
        // The kitty protocol's CSI P / Q / S, where the 1 is left out without modifiers
        b'P' => key(KeyCode::F(1)),
        b'Q' => key(KeyCode::F(2)),
        b'S' => key(KeyCode::F(4)),
        // Replies to terminal queries (`CSI ? … u`, `CSI ? … c`): skip the whole sequence
        b'?' => match buffer[buffer.len() - 1] {
            64..=126 if buffer.len() > 3 => Err(Malformed),
            _ => Ok(None),
        },
        // Numbered escape code
        b'0'..=b'9' => {
            let last = buffer[buffer.len() - 1];
            // The final byte of a CSI sequence is in 64..=126: keep reading until it arrives
            if buffer.len() == 3 || !(64..=126).contains(&last) {
                return Ok(None);
            }
            if buffer.starts_with(b"\x1B[200~") {
                return parse_csi_bracketed_paste(buffer);
            }
            match last {
                b'~' => parse_csi_special_key_code(buffer),
                // Kitty `CSI u` and rxvt mouse are never requested. `CSI 1 ; mod R` is F3 with
                // modifiers but also a cursor position report: crossterm reads it as the report, so
                // natively those keys never arrive, and they don't here either
                b'u' | b'R' | b'M' => Err(Malformed),
                _ => parse_csi_modifier_key_code(buffer),
            }
        }
        _ => Err(Malformed),
    }
}

fn next_parsed<T: std::str::FromStr>(iter: &mut dyn Iterator<Item = &str>) -> Result<T, Malformed> {
    iter.next()
        .ok_or(Malformed)?
        .parse::<T>()
        .map_err(|_| Malformed)
}

/// A `modifiers[:kind]` parameter
fn modifier_and_kind_parsed(iter: &mut dyn Iterator<Item = &str>) -> Result<(u8, u8), Malformed> {
    let mut sub_split = iter.next().ok_or(Malformed)?.split(':');
    let modifier_mask = next_parsed::<u8>(&mut sub_split)?;
    let kind = next_parsed::<u8>(&mut sub_split).unwrap_or(1);
    Ok((modifier_mask, kind))
}

/// xterm's modifier parameter is 1 + a bit mask
fn parse_modifiers(mask: u8) -> KeyModifiers {
    let mask = mask.saturating_sub(1);
    let mut modifiers = KeyModifiers::NONE;
    for (bit, modifier) in [
        (1, KeyModifiers::SHIFT),
        (2, KeyModifiers::ALT),
        (4, KeyModifiers::CONTROL),
        (8, KeyModifiers::SUPER),
        (16, KeyModifiers::HYPER),
        (32, KeyModifiers::META),
    ] {
        if mask & bit != 0 {
            modifiers |= modifier;
        }
    }
    modifiers
}

fn parse_modifiers_to_state(mask: u8) -> KeyEventState {
    let mask = mask.saturating_sub(1);
    let mut state = KeyEventState::NONE;
    if mask & 64 != 0 {
        state |= KeyEventState::CAPS_LOCK;
    }
    if mask & 128 != 0 {
        state |= KeyEventState::NUM_LOCK;
    }
    state
}

fn parse_key_event_kind(kind: u8) -> KeyEventKind {
    match kind {
        2 => KeyEventKind::Repeat,
        3 => KeyEventKind::Release,
        _ => KeyEventKind::Press,
    }
}

/// `CSI 1 ; mod X` for arrows, Home / End and F1-F4
fn parse_csi_modifier_key_code(buffer: &[u8]) -> Parsed {
    let s = std::str::from_utf8(&buffer[2..buffer.len() - 1]).map_err(|_| Malformed)?;
    let mut split = s.split(';');
    split.next();
    let (modifiers, kind) = if let Ok((mask, kind)) = modifier_and_kind_parsed(&mut split) {
        (parse_modifiers(mask), parse_key_event_kind(kind))
    } else if buffer.len() > 3 {
        let digit = (buffer[buffer.len() - 2] as char)
            .to_digit(10)
            .ok_or(Malformed)?;
        (parse_modifiers(digit as u8), KeyEventKind::Press)
    } else {
        (KeyModifiers::NONE, KeyEventKind::Press)
    };
    let code = match buffer[buffer.len() - 1] {
        b'A' => KeyCode::Up,
        b'B' => KeyCode::Down,
        b'C' => KeyCode::Right,
        b'D' => KeyCode::Left,
        b'F' => KeyCode::End,
        b'H' => KeyCode::Home,
        b'P' => KeyCode::F(1),
        b'Q' => KeyCode::F(2),
        b'R' => KeyCode::F(3),
        b'S' => KeyCode::F(4),
        _ => return Err(Malformed),
    };
    Ok(Some(Event::Key(KeyEvent::new_with_kind(
        code, modifiers, kind,
    ))))
}

/// `CSI n [; mod] ~` for Insert, Delete, Home / End, PageUp / PageDown and F5-F20
fn parse_csi_special_key_code(buffer: &[u8]) -> Parsed {
    let s = std::str::from_utf8(&buffer[2..buffer.len() - 1]).map_err(|_| Malformed)?;
    let mut split = s.split(';');
    let first = next_parsed::<u8>(&mut split)?;
    let (modifiers, kind, state) = match modifier_and_kind_parsed(&mut split) {
        Ok((mask, kind)) => (
            parse_modifiers(mask),
            parse_key_event_kind(kind),
            parse_modifiers_to_state(mask),
        ),
        Err(_) => (KeyModifiers::NONE, KeyEventKind::Press, KeyEventState::NONE),
    };
    let code = match first {
        1 | 7 => KeyCode::Home,
        2 => KeyCode::Insert,
        3 => KeyCode::Delete,
        4 | 8 => KeyCode::End,
        5 => KeyCode::PageUp,
        6 => KeyCode::PageDown,
        v @ 11..=15 => KeyCode::F(v - 10),
        v @ 17..=21 => KeyCode::F(v - 11),
        v @ 23..=26 => KeyCode::F(v - 12),
        v @ 28..=29 => KeyCode::F(v - 15),
        v @ 31..=34 => KeyCode::F(v - 17),
        _ => return Err(Malformed),
    };
    Ok(Some(Event::Key(KeyEvent::new_with_kind_and_state(
        code, modifiers, kind, state,
    ))))
}

/// `CSI < Cb ; Cx ; Cy M` (press, drag, scroll) or `… m` (release), 1-based coordinates
fn parse_csi_sgr_mouse(buffer: &[u8]) -> Parsed {
    let release = match buffer[buffer.len() - 1] {
        b'm' => true,
        b'M' => false,
        _ => return Ok(None),
    };
    let s = std::str::from_utf8(&buffer[3..buffer.len() - 1]).map_err(|_| Malformed)?;
    let mut split = s.split(';');
    let (kind, modifiers) = parse_cb(next_parsed::<u8>(&mut split)?)?;
    let column = next_parsed::<u16>(&mut split)?
        .checked_sub(1)
        .ok_or(Malformed)?;
    let row = next_parsed::<u16>(&mut split)?
        .checked_sub(1)
        .ok_or(Malformed)?;
    let kind = match kind {
        MouseEventKind::Down(button) if release => MouseEventKind::Up(button),
        other => other,
    };
    Ok(Some(Event::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers,
    })))
}

/// Cb, from low to high bit: button (2 bits), Shift, Alt, Ctrl, dragging, button (2 more bits)
fn parse_cb(cb: u8) -> Result<(MouseEventKind, KeyModifiers), Malformed> {
    let button = (cb & 0b0000_0011) | ((cb & 0b1100_0000) >> 4);
    let dragging = cb & 0b0010_0000 != 0;
    let kind = match (button, dragging) {
        (0, false) => MouseEventKind::Down(MouseButton::Left),
        (1, false) => MouseEventKind::Down(MouseButton::Middle),
        (2, false) => MouseEventKind::Down(MouseButton::Right),
        (0, true) => MouseEventKind::Drag(MouseButton::Left),
        (1, true) => MouseEventKind::Drag(MouseButton::Middle),
        (2, true) => MouseEventKind::Drag(MouseButton::Right),
        (3, false) => MouseEventKind::Up(MouseButton::Left),
        (3..=5, true) => MouseEventKind::Moved,
        (4, false) => MouseEventKind::ScrollUp,
        (5, false) => MouseEventKind::ScrollDown,
        (6, false) => MouseEventKind::ScrollLeft,
        (7, false) => MouseEventKind::ScrollRight,
        _ => return Err(Malformed),
    };
    let mut modifiers = KeyModifiers::NONE;
    for (bit, modifier) in [
        (0b0000_0100, KeyModifiers::SHIFT),
        (0b0000_1000, KeyModifiers::ALT),
        (0b0001_0000, KeyModifiers::CONTROL),
    ] {
        if cb & bit != 0 {
            modifiers |= modifier;
        }
    }
    Ok((kind, modifiers))
}

/// `CSI 200 ~ text CSI 201 ~`; until the end marker arrives, the text keeps accumulating
fn parse_csi_bracketed_paste(buffer: &[u8]) -> Parsed {
    if !buffer.ends_with(b"\x1b[201~") {
        return Ok(None);
    }
    let text = String::from_utf8_lossy(&buffer[6..buffer.len() - 6]).into_owned();
    Ok(Some(Event::Paste(text)))
}

/// One UTF-8 character, waiting while the bytes so far are a valid prefix of one.
fn parse_utf8_char(buffer: &[u8]) -> Result<Option<char>, Malformed> {
    if let Ok(s) = std::str::from_utf8(buffer) {
        return s.chars().next().map(Some).ok_or(Malformed);
    }
    let required = match buffer[0] {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        0x80..=0xBF | 0xF8..=0xFF => return Err(Malformed),
    };
    let continuation = buffer[1..].iter().all(|b| b & 0b1100_0000 == 0b1000_0000);
    if continuation && buffer.len() < required {
        Ok(None)
    } else {
        Err(Malformed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn parse(chunks: &[&[u8]]) -> Vec<Event> {
        let mut parser = InputParser::default();
        let mut out = Vec::new();
        for chunk in chunks {
            parser.feed(chunk, &mut out);
        }
        out
    }

    fn k(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::Key(KeyEvent::new(code, modifiers))
    }

    /// What xterm.js sends for each key (`src/common/input/Keyboard.ts`, normal cursor mode)
    #[test]
    fn xterm_key_sequences() {
        use KeyCode::*;
        let none = KeyModifiers::NONE;
        let shift = KeyModifiers::SHIFT;
        let alt = KeyModifiers::ALT;
        let ctrl = KeyModifiers::CONTROL;
        let cases: &[(&[u8], Event)] = &[
            (b"a", k(Char('a'), none)),
            (b"A", k(Char('A'), shift)),
            ("中".as_bytes(), k(Char('中'), none)),
            (b"\r", k(Enter, none)),
            (b"\x1b\r", k(Enter, alt)),
            (b"\t", k(Tab, none)),
            (b"\x1b[Z", k(BackTab, shift)),
            (b"\x7f", k(Backspace, none)),
            // Ctrl+Backspace is ^H, which the keymap treats as delete-word, as in native terminals
            (b"\x08", k(Char('h'), ctrl)),
            (b"\x1b\x7f", k(Backspace, alt)),
            (b"\x1b", k(Esc, none)),
            (b"\x1b\x1b", k(Esc, none)),
            (b"\x1b[A", k(Up, none)),
            (b"\x1b[B", k(Down, none)),
            (b"\x1b[C", k(Right, none)),
            (b"\x1b[D", k(Left, none)),
            (b"\x1b[H", k(Home, none)),
            (b"\x1b[F", k(End, none)),
            (b"\x1bOH", k(Home, none)),
            (b"\x1b[1;2D", k(Left, shift)),
            (b"\x1b[1;5C", k(Right, ctrl)),
            (b"\x1b[1;6A", k(Up, shift | ctrl)),
            (b"\x1b[1;3B", k(Down, alt)),
            (b"\x1b[1;9D", k(Left, KeyModifiers::SUPER)),
            (b"\x1b[1;5H", k(Home, ctrl)),
            (b"\x1b[1;5F", k(End, ctrl)),
            (b"\x1b[2~", k(Insert, none)),
            (b"\x1b[3~", k(Delete, none)),
            (b"\x1b[3;5~", k(Delete, ctrl)),
            (b"\x1b[5~", k(PageUp, none)),
            (b"\x1b[6~", k(PageDown, none)),
            (b"\x1b[6;2~", k(PageDown, shift)),
            (b"\x1bOP", k(F(1), none)),
            (b"\x1bOS", k(F(4), none)),
            (b"\x1b[1;5P", k(F(1), ctrl)),
            (b"\x1b[15~", k(F(5), none)),
            (b"\x1b[21~", k(F(10), none)),
            (b"\x1b[24~", k(F(12), none)),
            (b"\x1b[24;2~", k(F(12), shift)),
            (b"\x01", k(Char('a'), ctrl)),
            (b"\x13", k(Char('s'), ctrl)),
            (b"\x1a", k(Char('z'), ctrl)),
            (b"\0", k(Char(' '), ctrl)),
            (b"\x1c", k(Char('4'), ctrl)),
            (b"\x1f", k(Char('7'), ctrl)),
            // Raw mode: \n is Ctrl+J, not Enter
            (b"\n", k(Char('j'), ctrl)),
            (b"\x1bb", k(Char('b'), alt)),
            (b"\x1bB", k(Char('B'), alt | shift)),
            (b"\x1b\x02", k(Char('b'), alt | ctrl)),
            (b"\x1b ", k(Char(' '), alt)),
            (b"\x1b[I", Event::FocusGained),
            (b"\x1b[O", Event::FocusLost),
        ];
        for (bytes, want) in cases {
            assert_eq!(parse(&[bytes]), vec![want.clone()], "{bytes:?}");
        }
    }

    #[test]
    fn sgr_mouse() {
        let at = |kind, column, row, modifiers| {
            Event::Mouse(MouseEvent {
                kind,
                column,
                row,
                modifiers,
            })
        };
        let left = MouseButton::Left;
        let none = KeyModifiers::NONE;
        let cases: &[(&[u8], Event)] = &[
            (b"\x1b[<0;10;5M", at(MouseEventKind::Down(left), 9, 4, none)),
            (b"\x1b[<0;10;5m", at(MouseEventKind::Up(left), 9, 4, none)),
            (
                b"\x1b[<32;11;5M",
                at(MouseEventKind::Drag(left), 10, 4, none),
            ),
            (b"\x1b[<64;1;1M", at(MouseEventKind::ScrollUp, 0, 0, none)),
            (b"\x1b[<65;1;1M", at(MouseEventKind::ScrollDown, 0, 0, none)),
            (
                b"\x1b[<4;2;3M",
                at(MouseEventKind::Down(left), 1, 2, KeyModifiers::SHIFT),
            ),
            (
                b"\x1b[<2;1;1M",
                at(MouseEventKind::Down(MouseButton::Right), 0, 0, none),
            ),
        ];
        for (bytes, want) in cases {
            assert_eq!(parse(&[bytes]), vec![want.clone()], "{bytes:?}");
        }
        // 0 is not a 1-based coordinate: dropped instead of underflowing
        assert_eq!(
            parse(&[b"\x1b[<0;0;1M", b"x"]),
            vec![k(KeyCode::Char('x'), none)]
        );
    }

    #[test]
    fn paste_spans_chunks() {
        let paste = |s: &str| Event::Paste(s.into());
        assert_eq!(
            parse(&[b"\x1b[200~first line\n", "第二行 \x1b".as_bytes(), b"[201~"]),
            vec![paste("first line\n第二行 ")]
        );
        // The markers themselves split across chunks, and keys on both sides
        assert_eq!(
            parse(&[b"a\x1b[20", b"0~x\x1b[2", b"01", b"~b"]),
            vec![
                k(KeyCode::Char('a'), KeyModifiers::NONE),
                paste("x"),
                k(KeyCode::Char('b'), KeyModifiers::NONE)
            ]
        );
        // Escape sequences inside a paste are text, not keys
        assert_eq!(
            parse(&[b"\x1b[200~\x1b[A\x1b\x1b[201~"]),
            vec![paste("\x1b[A\x1b")]
        );
        assert_eq!(parse(&[b"\x1b[200~\x1b[201~"]), vec![paste("")]);
    }

    #[test]
    fn chunk_ends_decide_esc() {
        let none = KeyModifiers::NONE;
        // At the end of a chunk ESC is the key; mid-chunk it starts a sequence
        assert_eq!(
            parse(&[b"\x1b", b"b"]),
            vec![k(KeyCode::Esc, none), k(KeyCode::Char('b'), none)]
        );
        assert_eq!(
            parse(&[b"\x1bb"]),
            vec![k(KeyCode::Char('b'), KeyModifiers::ALT)]
        );
        // A sequence cut anywhere after ESC [ just waits for the rest
        assert_eq!(
            parse(&[b"\x1b[", b"1;5", b"C"]),
            vec![k(KeyCode::Right, KeyModifiers::CONTROL)]
        );
        // UTF-8 split between chunks
        let bytes = "春".as_bytes();
        assert_eq!(
            parse(&[&bytes[..1], &bytes[1..]]),
            vec![k(KeyCode::Char('春'), none)]
        );
    }

    #[test]
    fn unknown_input_is_dropped_and_parsing_recovers() {
        let none = KeyModifiers::NONE;
        let x = k(KeyCode::Char('x'), none);
        for junk in [
            &b"\x1b[99~"[..],
            b"\x1bOZ",
            b"\x1b[?1;2c",
            b"\x1b[12;5R",
            b"\xff",
            b"\x1b[97u",
        ] {
            assert_eq!(parse(&[junk, b"x"]), vec![x.clone()], "{junk:?}");
        }
    }

    /// xterm.js's encoding (Keyboard.ts and its mouse and paste reports), the inverse of the parser
    /// for the events it can produce.
    fn encode(ev: &Event) -> Vec<u8> {
        let csi = |s: String| format!("\x1b[{s}").into_bytes();
        let param = |m: KeyModifiers| 1 + parse_modifiers_inverse(m);
        match ev {
            Event::Key(key) => {
                let m = key.modifiers;
                let alt = m.contains(KeyModifiers::ALT);
                let plain = |bytes: Vec<u8>| {
                    if alt {
                        [b"\x1b".to_vec(), bytes].concat()
                    } else {
                        bytes
                    }
                };
                match key.code {
                    KeyCode::Char(c) if m.contains(KeyModifiers::CONTROL) => plain(vec![match c {
                        'a'..='z' => c as u8 - b'a' + 1,
                        '4'..='7' => c as u8 - b'4' + 0x1C,
                        ' ' => 0,
                        _ => unreachable!("{c:?}"),
                    }]),
                    KeyCode::Char(c) => plain(c.to_string().into_bytes()),
                    KeyCode::Enter => plain(b"\r".to_vec()),
                    KeyCode::Backspace => plain(b"\x7f".to_vec()),
                    KeyCode::Tab => b"\t".to_vec(),
                    KeyCode::BackTab => b"\x1b[Z".to_vec(),
                    KeyCode::Esc => b"\x1b".to_vec(),
                    code @ (KeyCode::Up
                    | KeyCode::Down
                    | KeyCode::Right
                    | KeyCode::Left
                    | KeyCode::Home
                    | KeyCode::End
                    | KeyCode::F(1..=4)) => {
                        let last = match code {
                            KeyCode::Up => 'A',
                            KeyCode::Down => 'B',
                            KeyCode::Right => 'C',
                            KeyCode::Left => 'D',
                            KeyCode::Home => 'H',
                            KeyCode::End => 'F',
                            KeyCode::F(n) => (b'P' + n - 1) as char,
                            _ => unreachable!(),
                        };
                        match (m.is_empty(), code) {
                            (true, KeyCode::F(_)) => format!("\x1bO{last}").into_bytes(),
                            (true, _) => csi(last.to_string()),
                            (false, _) => csi(format!("1;{}{last}", param(m))),
                        }
                    }
                    code => {
                        let n = match code {
                            KeyCode::Insert => 2,
                            KeyCode::Delete => 3,
                            KeyCode::PageUp => 5,
                            KeyCode::PageDown => 6,
                            KeyCode::F(n @ 5) => 10 + n,
                            KeyCode::F(n @ 6..=10) => 11 + n,
                            KeyCode::F(n @ 11..=12) => 12 + n,
                            other => unreachable!("{other:?}"),
                        };
                        match m.is_empty() {
                            true => csi(format!("{n}~")),
                            false => csi(format!("{n};{}~", param(m))),
                        }
                    }
                }
            }
            Event::Mouse(mouse) => {
                let (button, release) = match mouse.kind {
                    MouseEventKind::Down(b) => (button_code(b), false),
                    MouseEventKind::Up(b) => (button_code(b), true),
                    MouseEventKind::Drag(b) => (32 + button_code(b), false),
                    MouseEventKind::Moved => (35, false),
                    MouseEventKind::ScrollUp => (64, false),
                    MouseEventKind::ScrollDown => (65, false),
                    MouseEventKind::ScrollLeft => (66, false),
                    MouseEventKind::ScrollRight => (67, false),
                };
                let m = mouse.modifiers;
                let cb = button
                    + 4 * u8::from(m.contains(KeyModifiers::SHIFT))
                    + 8 * u8::from(m.contains(KeyModifiers::ALT))
                    + 16 * u8::from(m.contains(KeyModifiers::CONTROL));
                let end = if release { 'm' } else { 'M' };
                csi(format!("<{cb};{};{}{end}", mouse.column + 1, mouse.row + 1))
            }
            Event::Paste(text) => format!("\x1b[200~{text}\x1b[201~").into_bytes(),
            Event::FocusGained => b"\x1b[I".to_vec(),
            Event::FocusLost => b"\x1b[O".to_vec(),
            Event::Resize(..) => unreachable!("not input"),
        }
    }

    fn parse_modifiers_inverse(m: KeyModifiers) -> u8 {
        [
            (1, KeyModifiers::SHIFT),
            (2, KeyModifiers::ALT),
            (4, KeyModifiers::CONTROL),
            (8, KeyModifiers::SUPER),
        ]
        .into_iter()
        .filter(|&(_, flag)| m.contains(flag))
        .map(|(bit, _)| bit)
        .sum()
    }

    fn button_code(b: MouseButton) -> u8 {
        match b {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
        }
    }

    /// Events in the form the parser produces them (SHIFT exactly on uppercase characters, Ctrl
    /// only on the characters a C0 byte stands for, no Ctrl+I / Ctrl+M, which are Tab / Enter, and no
    /// Alt+[ / Alt+O).
    fn event() -> impl Strategy<Value = Event> {
        let mods = (any::<bool>(), any::<bool>(), any::<bool>(), any::<bool>()).prop_map(
            |(s, a, c, w)| {
                let mut m = KeyModifiers::NONE;
                for (on, flag) in [
                    (s, KeyModifiers::SHIFT),
                    (a, KeyModifiers::ALT),
                    (c, KeyModifiers::CONTROL),
                    (w, KeyModifiers::SUPER),
                ] {
                    if on {
                        m |= flag;
                    }
                }
                m
            },
        );
        let alt = any::<bool>().prop_map(|a| {
            if a {
                KeyModifiers::ALT
            } else {
                KeyModifiers::NONE
            }
        });
        let text_char = prop_oneof![
            proptest::char::range('a', 'z'),
            proptest::char::range('A', 'Z'),
            proptest::sample::select(vec!['中', '文', '，', '「', 'é', 'Ω', '😀', '1', '@', '{']),
        ];
        // Alt+[ and Alt+O are ESC [ and ESC O, the CSI and SS3 introducers: no terminal can tell them apart
        let text_char = text_char.prop_filter("CSI / SS3 introducer", |&c| !matches!(c, '[' | 'O'));
        let ctrl_char = proptest::sample::select(
            ('a'..='z')
                .filter(|c| !matches!(c, 'i' | 'm'))
                .chain(['4', '5', '6', '7', ' '])
                .collect::<Vec<_>>(),
        );
        let named = proptest::sample::select(vec![
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::Insert,
            KeyCode::Delete,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::F(1),
            KeyCode::F(2),
            KeyCode::F(3),
            KeyCode::F(4),
            KeyCode::F(5),
            KeyCode::F(8),
            KeyCode::F(12),
        ]);
        let mouse_kind = prop_oneof![
            proptest::sample::select(vec![
                MouseButton::Left,
                MouseButton::Middle,
                MouseButton::Right
            ])
            .prop_flat_map(|b| {
                proptest::sample::select(vec![
                    MouseEventKind::Down(b),
                    MouseEventKind::Up(b),
                    MouseEventKind::Drag(b),
                ])
            }),
            proptest::sample::select(vec![
                MouseEventKind::Moved,
                MouseEventKind::ScrollUp,
                MouseEventKind::ScrollDown,
                MouseEventKind::ScrollLeft,
                MouseEventKind::ScrollRight,
            ]),
        ];
        let mouse_mods = (any::<bool>(), any::<bool>(), any::<bool>()).prop_map(|(s, a, c)| {
            let mut m = KeyModifiers::NONE;
            for (on, flag) in [
                (s, KeyModifiers::SHIFT),
                (a, KeyModifiers::ALT),
                (c, KeyModifiers::CONTROL),
            ] {
                if on {
                    m |= flag;
                }
            }
            m
        });
        prop_oneof![
            (text_char, alt.clone()).prop_map(|(c, a)| Event::Key(KeyEvent::new(
                KeyCode::Char(c),
                a | char_code_to_event(c).modifiers
            ))),
            (ctrl_char, alt.clone())
                .prop_map(|(c, a)| k(KeyCode::Char(c), KeyModifiers::CONTROL | a)),
            (named, mods).prop_map(|(code, m)| {
                // xterm sends Insert without modifiers; F3 with modifiers reads as a cursor position report
                let m = if matches!(code, KeyCode::Insert | KeyCode::F(3)) {
                    KeyModifiers::NONE
                } else {
                    m
                };
                k(code, m)
            }),
            alt.clone().prop_map(|a| k(KeyCode::Enter, a)),
            alt.prop_map(|a| k(KeyCode::Backspace, a)),
            Just(k(KeyCode::Tab, KeyModifiers::NONE)),
            Just(k(KeyCode::BackTab, KeyModifiers::SHIFT)),
            Just(k(KeyCode::Esc, KeyModifiers::NONE)),
            "[^\x1b]{0,20}".prop_map(Event::Paste),
            Just(Event::FocusGained),
            Just(Event::FocusLost),
            (mouse_kind, 0u16..300, 0u16..100, mouse_mods).prop_map(
                |(kind, column, row, modifiers)| Event::Mouse(MouseEvent {
                    kind,
                    column,
                    row,
                    modifiers
                })
            ),
        ]
    }

    proptest! {
        /// Each event as its own chunk, the way xterm.js delivers keystrokes
        #[test]
        fn round_trip_one_chunk_per_event(events in proptest::collection::vec(event(), 0..30)) {
            let encoded: Vec<Vec<u8>> = events.iter().map(encode).collect();
            let chunks: Vec<&[u8]> = encoded.iter().map(Vec::as_slice).collect();
            prop_assert_eq!(parse(&chunks), events);
        }

        /// Everything in one stream, cut at arbitrary points. A cut right after an ESC makes it the
        /// Esc key (that is what a chunk boundary means), so cuts land elsewhere and lone Esc keys
        /// stay out: inside one chunk ESC + key is Alt+key.
        #[test]
        fn round_trip_any_split(
            events in proptest::collection::vec(event().prop_filter("lone Esc", |e| *e != k(KeyCode::Esc, KeyModifiers::NONE)), 0..30),
            cuts in proptest::collection::vec(any::<prop::sample::Index>(), 0..8),
        ) {
            let stream: Vec<u8> = events.iter().flat_map(encode).collect();
            let mut at: Vec<usize> = cuts.iter().map(|i| i.index(stream.len() + 1)).filter(|&i| i == 0 || stream[i - 1] != 0x1b).collect();
            at.push(stream.len());
            at.sort_unstable();
            let mut chunks = Vec::new();
            let mut start = 0;
            for end in at {
                chunks.push(&stream[start..end]);
                start = end;
            }
            prop_assert_eq!(parse(&chunks), events);
        }
    }
}
