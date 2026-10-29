//! Runs the real binary in a pty: the window title follows the document title via OSC 2, gets a
//! `* ` prefix while unsaved, and is cleared on exit.

#![cfg(unix)]

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const CLEAR: &str = "\x1b]2;\x07";

struct Pty {
    child: Child,
    master: File,
    /// Every byte the child has written (a background thread keeps draining it, otherwise the child
    /// blocks writing to the terminal)
    out: Arc<Mutex<Vec<u8>>>,
}

impl Pty {
    fn spawn(home: &std::path::Path, file: &std::path::Path) -> Self {
        let (mut master, mut slave) = (0, 0);
        let mut size = libc::winsize {
            ws_row: 24,
            ws_col: 80,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        let rc = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                // termp / winp are *mut on macOS and *const on Linux. A raw *mut pointer works
                // for both (*mut coerces to *const); `&mut size` would trip
                // unnecessary_mut_passed on Linux
                &raw mut size,
            )
        };
        assert_eq!(rc, 0, "openpty: {}", std::io::Error::last_os_error());
        let (master, slave) = unsafe { (File::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_mustty"));
        cmd.arg(file)
            .env("HOME", home)
            .env("TERM", "xterm-256color")
            .env("LC_ALL", "en_US.UTF-8")
            .env_remove("LANG")
            .env_remove("LC_MESSAGES")
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave));
        // New session with the pty as controlling terminal, so /dev/tty points at it rather than at
        // the terminal running the tests
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = cmd.spawn().unwrap();
        let out = Arc::new(Mutex::new(Vec::new()));
        let mut reader = master.try_clone().unwrap();
        let sink = Arc::clone(&out);
        std::thread::spawn(move || {
            let mut buf = [0; 4096];
            // Once the child exits, reads return EOF or EIO
            while let Ok(n @ 1..) = reader.read(&mut buf) {
                sink.lock().unwrap().extend_from_slice(&buf[..n]);
            }
        });
        Self { child, master, out }
    }

    fn output(&self) -> String {
        String::from_utf8_lossy(&self.out.lock().unwrap()).into_owned()
    }

    fn send(&mut self, bytes: &[u8]) {
        self.master.write_all(bytes).unwrap();
    }

    /// Waits until `needle` appears `n` times in the output.
    fn wait_for(&self, needle: &str, n: usize) {
        let end = Instant::now() + Duration::from_secs(10);
        while self.output().matches(needle).count() < n {
            assert!(
                Instant::now() < end,
                "timed out waiting for occurrence {n} of {needle:?}: {:?}",
                self.output()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Waits for the child to exit (the background thread keeps reading output meanwhile).
    fn wait_exit(&mut self) -> bool {
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end {
            if self.child.try_wait().unwrap().is_some() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

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
