//! 真实二进制的命令行：只接受 FILE、--help、--version；语言只看配置文件与 LC_* / LANG；
//! 不读任何 MUSTTY_* 环境变量。

use std::process::Command;

fn mustty(home: &std::path::Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mustty"));
    // 隔离：不读真实的 ~/.mustty.toml；语言环境固定为英文
    cmd.env("HOME", home)
        .env("USERPROFILE", home)
        .env_remove("LC_MESSAGES")
        .env_remove("LANG")
        .env("LC_ALL", "en_US.UTF-8")
        .env_remove("MUSTTY_LANG")
        .env_remove("MUSTTY_LOG");
    cmd
}

#[test]
fn removed_switches_are_rejected() {
    let home = tempfile::tempdir().unwrap();
    for arg in [
        "--width",
        "--autosave",
        "--no-autosave",
        "--vim",
        "--no-vim",
        "--mouse",
        "--no-mouse",
    ] {
        let out = mustty(home.path()).arg(arg).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{arg}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("unexpected argument"), "{arg}: {stderr}");
    }
}

#[test]
fn mustty_lang_has_no_effect() {
    let home = tempfile::tempdir().unwrap();
    let help =
        |cmd: &mut Command| String::from_utf8(cmd.arg("--help").output().unwrap().stdout).unwrap();
    let plain = help(&mut mustty(home.path()));
    let with_env = help(mustty(home.path()).env("MUSTTY_LANG", "zh-CN"));
    assert!(
        plain.contains("A distraction-free Markdown writer"),
        "{plain}"
    );
    assert_eq!(plain, with_env);
}

/// `dirs::home_dir` 在 Windows 上不看 HOME，只在 Unix 上能把配置文件隔离到临时目录。
#[cfg(unix)]
#[test]
fn config_lang_applies_to_help() {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(home.path().join(".mustty.toml"), "lang = \"zh-CN\"\n").unwrap();
    let out = mustty(home.path()).arg("--help").output().unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("终端里的极简 Markdown 写作器"), "{text}");
}

/// 日志已删除：设置 MUSTTY_LOG 不会创建文件，`--help` 的输出与退出码和不设置时完全一样。
/// 不带 `--help` 启动真实 TUI 的写法不能用：Windows 上 stdin 为 null 时 crossterm 不会报错退出，
/// 进程停在事件循环里，`.output()` 永远等不到；Unix 上子进程读终端会触发 SIGTTIN 把进程组挂起。
#[test]
fn mustty_log_has_no_effect() {
    let home = tempfile::tempdir().unwrap();
    let file = home.path().join("file.md");
    std::fs::write(&file, "hello\n").unwrap();
    let log = home.path().join("x.log");
    let help = |cmd: &mut Command| {
        let out = cmd.arg("--help").output().unwrap();
        (out.status.code(), out.stdout, out.stderr)
    };
    let plain = help(&mut mustty(home.path()));
    let with_log = help(mustty(home.path()).env("MUSTTY_LOG", &log));
    assert_eq!(plain.0, Some(0));
    assert_eq!(plain, with_log);
    assert!(!log.exists(), "MUSTTY_LOG 仍然创建了日志文件");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "hello\n");
}
