//! Command line of the real binary: it accepts only FILE, --help and --version. The language
//! comes from the config file and LC_* / LANG alone; no MUSTTY_* variable is read.

use std::process::Command;

fn mustty(home: &std::path::Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mustty"));
    // Isolation: never read the real ~/.mustty.toml, and pin the locale to English
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

/// `dirs::home_dir` ignores HOME on Windows, so only on Unix can the config file be redirected
/// to a temp dir.
#[cfg(unix)]
#[test]
fn config_lang_applies_to_help() {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(home.path().join(".mustty.toml"), "lang = \"zh-CN\"\n").unwrap();
    let out = mustty(home.path()).arg("--help").output().unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("终端里的极简 Markdown 写作器"), "{text}");
}

/// Logging is gone: setting MUSTTY_LOG creates no file, and `--help` prints the same output with
/// the same exit code as without it. Launching the real TUI without `--help` won't work here: on
/// Windows crossterm doesn't fail on a null stdin, so the process sits in the event loop and
/// `.output()` never returns; on Unix the child reading the terminal gets SIGTTIN, which stops
/// the process group.
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
    assert!(!log.exists(), "MUSTTY_LOG still created a log file");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "hello\n");
}
