//! Runs the real binary in a pty and records every byte it writes to the terminal.

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub struct Pty {
    child: Child,
    master: File,
    /// Every byte the child has written (a background thread keeps draining it, otherwise the child
    /// blocks writing to the terminal)
    out: Arc<Mutex<Vec<u8>>>,
}

impl Pty {
    pub fn spawn(home: &std::path::Path, file: &std::path::Path) -> Self {
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
            // OSC 52 is skipped under macOS Terminal: keep the output independent of where the tests run
            .env_remove("TERM_PROGRAM")
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

    pub fn bytes(&self) -> Vec<u8> {
        self.out.lock().unwrap().clone()
    }

    pub fn output(&self) -> String {
        String::from_utf8_lossy(&self.out.lock().unwrap()).into_owned()
    }

    pub fn send(&mut self, bytes: &[u8]) {
        self.master.write_all(bytes).unwrap();
    }

    /// Waits until `needle` appears `n` times in the output.
    pub fn wait_for(&self, needle: &str, n: usize) {
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

    /// Waits until the output has grown past `len` and then stayed unchanged for `quiet`.
    pub fn wait_quiet_after(&self, len: usize, quiet: Duration) {
        let end = Instant::now() + Duration::from_secs(10);
        let mut last = (self.out.lock().unwrap().len(), Instant::now());
        loop {
            assert!(
                Instant::now() < end,
                "output never settled: {:?}",
                self.output()
            );
            std::thread::sleep(Duration::from_millis(10));
            let now = self.out.lock().unwrap().len();
            if now != last.0 {
                last = (now, Instant::now());
            } else if now > len && last.1.elapsed() >= quiet {
                return;
            }
        }
    }

    /// Waits for the child to exit (the background thread keeps reading output meanwhile).
    pub fn wait_exit(&mut self) -> bool {
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
