//! Byte-for-byte golden of a whole terminal session: every byte the real binary writes to the pty
//! while typing, copying (OSC 52), retitling (OSC 2), changing the cursor shape, toggling mouse
//! capture and quitting. Refactors of the terminal layer must leave this file unchanged.
//!
//! `MUSTTY_BLESS=1 cargo test --test golden_session` rewrites `tests/golden/session.bin`.
//!
//! Recorded on macOS. Other platforms can write a different byte stream (Windows sets mouse capture
//! through the Console API instead of escape sequences, and Linux was never recorded), so the
//! byte-for-byte comparison only holds on the platform the golden came from.

#![cfg(target_os = "macos")]

mod support;

use std::time::Duration;

use support::pty::Pty;

const GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/session.bin");
/// One key per event batch: wait until the frame for the previous key has been written
const QUIET: Duration = Duration::from_millis(150);

#[test]
fn whole_session_matches_the_golden_bytes() {
    let home = tempfile::tempdir().unwrap();
    // No autosave, so no timer fires between keys; the startup hint is the only one
    std::fs::write(home.path().join(".mustty.toml"), "autosave = false\n").unwrap();
    let file = home.path().join("notes.md");
    std::fs::write(&file, "# Draft\n\nHello world\n").unwrap();
    let mut pty = Pty::spawn(home.path(), &file);
    pty.wait_for("\x1b]2;Draft\x07", 1);
    pty.wait_quiet_after(0, QUIET);
    // Let the 5 s startup hint expire before the first key, so its redraw has a fixed place
    let len = pty.bytes().len();
    std::thread::sleep(Duration::from_millis(5500));
    pty.wait_quiet_after(len, QUIET);

    let up = b"\x1b[A";
    let down = b"\x1b[B";
    let keys: Vec<&[u8]> = [
        // Type, select two characters with Shift+Left and copy them (OSC 52)
        vec![&b"a"[..], b"b", b"c", b"\x1b[1;2D", b"\x1b[1;2D", b"\x03"],
        // Into the title, append to it (OSC 2 gets the `* ` mark)
        vec![up, up, up, b"\x1b[F", b"!"],
        // Settings: mouse on, block cursor, blinking, then Enter writes ~/.mustty.toml
        vec![b"\x10", down, down, b" "],
        vec![down; 9],
        vec![b"\x1b[C", down, b" ", b"\r"],
        // Settings again: vim on, so Normal / Insert switch the cursor shape
        vec![b"\x10", down, b" ", b"\r", b"\x1b", b"i", b"x"],
        // Mouse off, then Esc restores the snapshot (mouse back on)
        vec![b"\x10", down, down, b" ", b"\x1b"],
        // Save, then quit (clears the title, restores the terminal)
        vec![b"\x13", b"\x11"],
    ]
    .concat();
    for key in keys {
        let len = pty.bytes().len();
        pty.send(key);
        pty.wait_quiet_after(len, QUIET);
    }
    assert!(pty.wait_exit(), "did not exit after Ctrl+Q");
    pty.wait_quiet_after(0, QUIET);
    let got = pty.bytes();

    if std::env::var_os("MUSTTY_BLESS").is_some() {
        std::fs::write(GOLDEN, &got).unwrap();
        return;
    }
    let want = std::fs::read(GOLDEN).expect("no golden yet: run with MUSTTY_BLESS=1");
    if got != want {
        let at = got
            .iter()
            .zip(&want)
            .position(|(a, b)| a != b)
            .unwrap_or(got.len().min(want.len()));
        let around = |b: &[u8]| {
            String::from_utf8_lossy(&b[at.saturating_sub(80)..(at + 80).min(b.len())]).into_owned()
        };
        panic!(
            "session bytes differ from the golden at byte {at} (got {} bytes, want {})\n got: {:?}\nwant: {:?}",
            got.len(),
            want.len(),
            around(&got),
            around(&want)
        );
    }
}
