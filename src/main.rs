use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use anyhow::anyhow;
use clap::{CommandFactory, FromArgMatches, Parser};
use mustty::app::{self, App};
use mustty::config::{self, Config};
use mustty::editor::document::{Document, FileMeta};
use mustty::i18n::{self, t, t_args};
use mustty::io::file::{IoError, load};
use mustty::terminal::TerminalGuard;

// 说明文字按界面语言在运行时填入（见 `command`），这里不写 doc comment。
// 只有一个文件参数：其余设置都在配置文件里，可以用 Ctrl+P 的设置面板修改
#[derive(Parser)]
#[command(version)]
struct Cli {
    file: Option<PathBuf>,
}

impl Cli {
    /// 带本地化说明的 clap 命令。clap 自带的 Usage: / Options: 与解析错误无法本地化。
    fn command() -> clap::Command {
        <Self as CommandFactory>::command()
            .about(t("cli-about"))
            .after_help(t("cli-after-help"))
            .mut_arg("file", |a| a.help(t("cli-file")))
    }
}

fn main() -> ExitCode {
    // 语言要在 clap 解析之前定下来（--help 也要翻译）：先按配置文件的 lang 协商，
    // 再读一次配置，让警告也用这门语言
    i18n::init();
    i18n::use_config(config::load().0.lang);
    let cli = Cli::from_arg_matches(&Cli::command().get_matches()).unwrap_or_else(|e| e.exit());
    let (config, warnings) = config::load();
    // 配置写错不阻止启动：警告打到 stderr（进入全屏前，退出后仍可见），
    // 同时汇总成一条 status bar 消息，保留到第一次按键
    let warning = config::summarize(&warnings);
    if let Some(w) = &warning {
        eprintln!("mustty: {w}");
    }
    match run(cli.file.as_deref(), &config, warning) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(path: Option<&Path>, config: &Config, warning: Option<String>) -> anyhow::Result<()> {
    let doc = match path {
        Some(path) => load(path).map_err(|e| match e {
            IoError::Io(e) => {
                let path = path.display().to_string();
                anyhow!(t_args(
                    "startup-open",
                    &[("path", &path), ("error", &e.to_string())]
                ))
            }
            e => e.into(),
        })?,
        None => Document::parse("", FileMeta::new(None)),
    };
    let mut guard = TerminalGuard::new(config.mouse)?;
    let mut app = App::with_config(doc, guard.terminal.size()?, config);
    app.set_config_path(config::path());
    app.show_startup_hint(Instant::now());
    if let Some(w) = warning {
        app.show_startup_warning(w);
    }
    app::run(&mut guard.terminal, &mut app)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_arg_is_localized() {
        for tag in ["en", "zh-CN"] {
            i18n::init_for_test(tag);
            let cmd = Cli::command();
            assert_eq!(
                cmd.get_about().map(|s| s.to_string()).as_deref(),
                Some(t("cli-about"))
            );
            for arg in cmd
                .get_arguments()
                .filter(|a| !matches!(a.get_id().as_str(), "help" | "version"))
            {
                assert!(
                    arg.get_help().is_some(),
                    "{tag}: --{} 没有说明",
                    arg.get_id()
                );
            }
        }
    }

    #[test]
    fn only_file_help_and_version() {
        let mut cmd = Cli::command();
        cmd.build(); // help / version 在 build 时才加入
        let ids: Vec<&str> = cmd.get_arguments().map(|a| a.get_id().as_str()).collect();
        assert_eq!(ids, vec!["file", "help", "version"]);
        let parse = |args: &[&str]| {
            Cli::try_parse_from(std::iter::once("mustty").chain(args.iter().copied()))
        };
        assert_eq!(parse(&["notes.md"]).unwrap().file, Some("notes.md".into()));
        assert_eq!(parse(&[]).unwrap().file, None);
        // 删掉的开关被拒绝，不是悄悄忽略
        for removed in [
            "--width",
            "--autosave",
            "--no-autosave",
            "--vim",
            "--no-vim",
            "--mouse",
            "--no-mouse",
        ] {
            let err = parse(&[removed, "notes.md"])
                .err()
                .unwrap_or_else(|| panic!("{removed} 仍被接受"));
            assert_eq!(
                err.kind(),
                clap::error::ErrorKind::UnknownArgument,
                "{removed}"
            );
        }
    }
}
