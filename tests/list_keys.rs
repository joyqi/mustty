//! Runs the real binary in a pty: Tab and Shift+Tab, as a terminal sends them, reach the editor.

#![cfg(unix)]

mod support;

use std::time::{Duration, Instant};

use support::pty::Pty;

fn wait_for_file(file: &std::path::Path, want: &str) {
    let end = Instant::now() + Duration::from_secs(10);
    while std::fs::read_to_string(file).unwrap() != want {
        assert!(Instant::now() < end, "{:?}", std::fs::read_to_string(file));
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn tab_and_shift_tab_move_a_list_item() {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(home.path().join(".mustty.toml"), "autosave = false\n").unwrap();
    let file = home.path().join("notes.md");
    std::fs::write(&file, "# T\n\n- a\n- b\n").unwrap();
    let mut pty = Pty::spawn(home.path(), &file);
    pty.wait_for("\x1b]2;T\x07", 1);
    // Down to `- b`, Tab nests it, Ctrl+S
    pty.send(b"\x1b[B\t\x13");
    wait_for_file(&file, "# T\n\n- a\n    - b\n");
    // Shift+Tab is CSI Z
    pty.send(b"\x1b[Z\x13");
    wait_for_file(&file, "# T\n\n- a\n- b\n");
    pty.send(b"\x11");
    assert!(pty.wait_exit(), "did not exit after Ctrl+Q");
}
