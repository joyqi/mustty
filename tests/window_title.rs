//! Runs the real binary in a pty: the window title follows the document title via OSC 2, gets a
//! `* ` prefix while unsaved, and is cleared on exit.

#![cfg(unix)]

mod support;

use support::pty::Pty;

const CLEAR: &str = "\x1b]2;\x07";

#[test]
fn window_title_follows_the_h1_marks_unsaved_and_is_cleared_on_exit() {
    let home = tempfile::tempdir().unwrap();
    let file = home.path().join("notes.md");
    std::fs::write(&file, "# Spring letter\n\nbody\n").unwrap();
    let mut pty = Pty::spawn(home.path(), &file);

    // Clean file: no asterisk in the title
    pty.wait_for("\x1b]2;Spring letter\x07", 1);
    assert!(!pty.output().contains("\x1b]2;* "), "{:?}", pty.output());
    // Moving within the body leaves the title alone. Move into the title (↑ on the first
    // line), go to the end of the line and type two characters: unsaved changes → `* `
    pty.send(b"\x1b[B\x1b[A\x1b[A\x1b[F");
    pty.send(b"!");
    pty.wait_for("\x1b]2;* Spring letter!\x07", 1);
    pty.send(b"?");
    pty.wait_for("\x1b]2;* Spring letter!?\x07", 1);
    let titles = pty.output().matches("\x1b]2;").count();
    assert_eq!(titles, 3, "sent only on change: {:?}", pty.output());
    // Wait for autosave (2 s idle by default): the asterisk goes away
    pty.wait_for("\x1b]2;Spring letter!?\x07", 1);
    assert_eq!(pty.output().matches("\x1b]2;").count(), 4);

    // Quit: the file is saved, so a single Ctrl+Q exits
    pty.send(b"\x11");
    assert!(pty.wait_exit(), "did not exit after Ctrl+Q");
    pty.wait_for(CLEAR, 1);
    let out = pty.output();
    let last_title = out.rfind("\x1b]2;Spring letter!?\x07").unwrap();
    assert!(
        out.rfind(CLEAR).unwrap() > last_title,
        "the clear comes after the last title set"
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "# Spring letter!?\n\nbody\n"
    );
}
