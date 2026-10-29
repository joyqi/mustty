//! 真实二进制在 pty 里运行：窗口标题经 OSC 2 跟随文档标题，未保存时带 `* `，退出时清空。

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
    /// 子进程写出的全部字节（后台线程持续读取，否则子进程会阻塞在写终端上）
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
                // termp / winp 在 macOS 上是 *mut，在 Linux 上是 *const：传 *mut 裸指针两边都收
                // （*mut 自动转 *const），写 `&mut size` 在 Linux 上会触发 unnecessary_mut_passed
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
        // 新会话并把 pty 设为控制终端：/dev/tty 指向它，而不是跑测试的那个终端
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
            // 子进程退出后读到 EOF 或 EIO
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

    /// 等到输出里 `needle` 出现 `n` 次。
    fn wait_for(&self, needle: &str, n: usize) {
        let end = Instant::now() + Duration::from_secs(10);
        while self.output().matches(needle).count() < n {
            assert!(
                Instant::now() < end,
                "等不到第 {n} 个 {needle:?}：{:?}",
                self.output()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// 等子进程退出（期间输出仍由后台线程读取）。
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

    // 干净的文件：标题没有星号
    pty.wait_for("\x1b]2;Spring letter\x07", 1);
    assert!(!pty.output().contains("\x1b]2;* "), "{:?}", pty.output());
    // 正文里移动不改标题；进入标题（首行按 ↑）、到行尾，再打两个字：有未保存修改 → `* `
    pty.send(b"\x1b[B\x1b[A\x1b[A\x1b[F");
    pty.send(b"!");
    pty.wait_for("\x1b]2;* Spring letter!\x07", 1);
    pty.send(b"?");
    pty.wait_for("\x1b]2;* Spring letter!?\x07", 1);
    let titles = pty.output().matches("\x1b]2;").count();
    assert_eq!(titles, 3, "只在标题变化时发送：{:?}", pty.output());
    // 等自动保存（默认空闲 2 秒）：星号消失
    pty.wait_for("\x1b]2;Spring letter!?\x07", 1);
    assert_eq!(pty.output().matches("\x1b]2;").count(), 4);

    // 退出：已保存，一次 Ctrl+Q 即退出
    pty.send(b"\x11");
    assert!(pty.wait_exit(), "Ctrl+Q 后没有退出");
    pty.wait_for(CLEAR, 1);
    let out = pty.output();
    let last_title = out.rfind("\x1b]2;Spring letter!?\x07").unwrap();
    assert!(
        out.rfind(CLEAR).unwrap() > last_title,
        "清空在最后一次设置之后"
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "# Spring letter!?\n\nbody\n"
    );
}
