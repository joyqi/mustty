use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use anyhow::anyhow;
use clap::{CommandFactory, FromArgMatches, Parser};
use mustty::app::{self, App};
use mustty::config::{self, Config, PartialConfig};
use mustty::editor::document::{Document, FileMeta};
use mustty::i18n::{self, t, t_args};
use mustty::io::file::{IoError, load};
use mustty::terminal::TerminalGuard;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

// 说明文字按界面语言在运行时填入（见 `command`），这里不写 doc comment
#[derive(Parser)]
#[command(version)]
struct Cli {
    file: Option<PathBuf>,
    #[arg(long, value_name = "N")]
    width: Option<u16>,
    #[arg(long, overrides_with = "no_autosave")]
    autosave: bool,
    #[arg(long, overrides_with = "autosave")]
    no_autosave: bool,
    #[arg(long, overrides_with = "no_vim")]
    vim: bool,
    #[arg(long, overrides_with = "vim")]
    no_vim: bool,
    #[arg(long, overrides_with = "no_mouse")]
    mouse: bool,
    #[arg(long, overrides_with = "mouse")]
    no_mouse: bool,
}

impl Cli {
    /// 带本地化说明的 clap 命令。clap 自带的 Usage: / Options: 与解析错误无法本地化。
    fn command() -> clap::Command {
        let help = [
            ("file", t("cli-file")),
            ("width", t("cli-width")),
            ("autosave", t("cli-autosave")),
            ("no_autosave", t("cli-no-autosave")),
            ("vim", t("cli-vim")),
            ("no_vim", t("cli-no-vim")),
            ("mouse", t("cli-mouse")),
            ("no_mouse", t("cli-no-mouse")),
        ];
        help.into_iter().fold(
            <Self as CommandFactory>::command()
                .about(t("cli-about"))
                .after_help(t("cli-after-help")),
            |cmd, (id, text)| cmd.mut_arg(id, |a| a.help(text)),
        )
    }

    /// 命令行这一层配置：成对开关里后出现的生效（clap 的 overrides_with 已处理）。
    fn layer(&self) -> PartialConfig {
        let switch = |on: bool, off: bool| match (on, off) {
            (true, _) => Some(true),
            (_, true) => Some(false),
            _ => None,
        };
        PartialConfig {
            width: self.width,
            vim: switch(self.vim, self.no_vim),
            mouse: switch(self.mouse, self.no_mouse),
            autosave: switch(self.autosave, self.no_autosave),
            ..PartialConfig::default()
        }
    }
}

fn main() -> ExitCode {
    i18n::init();
    let cli = Cli::from_arg_matches(&Cli::command().get_matches()).unwrap_or_else(|e| e.exit());
    let _log = match init_log() {
        Ok(guard) => guard,
        Err(e) => {
            eprintln!("{e:#}");
            return ExitCode::FAILURE;
        }
    };
    let layer = cli.layer();
    let (config, warnings) = config::load(&layer);
    // 配置写错不阻止启动：警告打到 stderr（进入全屏前，退出后仍可见），
    // 同时汇总成一条 status bar 消息，保留到第一次按键
    let warning = config::summarize(&warnings);
    if let Some(w) = &warning {
        eprintln!("mustty: {w}");
        tracing::warn!("{w}");
    }
    match run(cli.file.as_deref(), &config, layer, warning) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e:#}");
            eprintln!("{e:#}");
            ExitCode::FAILURE
        }
    }
}

/// 只在设置 `MUSTTY_LOG=<path>` 时写日志文件，永不写 stdout / stderr。
fn init_log() -> anyhow::Result<Option<WorkerGuard>> {
    let Some(path) = std::env::var_os("MUSTTY_LOG") else {
        return Ok(None);
    };
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| {
            let path = Path::new(&path).display().to_string();
            anyhow!(t_args(
                "startup-log-file",
                &[("path", &path), ("error", &e.to_string())]
            ))
        })?;
    let (writer, guard) = tracing_appender::non_blocking(file);
    tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("debug")),
        )
        .init();
    Ok(Some(guard))
}

fn run(
    path: Option<&Path>,
    config: &Config,
    cli: PartialConfig,
    warning: Option<String>,
) -> anyhow::Result<()> {
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
    tracing::info!(path = ?path, ?config, "start");
    let mut guard = TerminalGuard::new(config.mouse)?;
    let mut app = App::with_config(doc, guard.terminal.size()?, config);
    app.set_cli_layer(cli);
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

    fn layer(args: &[&str]) -> PartialConfig {
        let argv = std::iter::once("mustty").chain(args.iter().copied());
        Cli::try_parse_from(argv).unwrap().layer()
    }

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
    fn paired_switches_last_wins() {
        assert_eq!(layer(&[]), PartialConfig::default());
        assert_eq!(layer(&["--vim"]).vim, Some(true));
        assert_eq!(layer(&["--no-vim"]).vim, Some(false));
        assert_eq!(layer(&["--vim", "--no-vim"]).vim, Some(false));
        assert_eq!(layer(&["--no-vim", "--vim"]).vim, Some(true));
        assert_eq!(layer(&["--no-mouse"]).mouse, Some(false));
        assert_eq!(
            layer(&["--autosave", "--no-autosave"]).autosave,
            Some(false)
        );
        assert_eq!(layer(&["--width", "90", "notes.md"]).width, Some(90));
    }

    #[test]
    fn cli_overrides_file() {
        let (file, _) =
            PartialConfig::from_toml_str("width = 64\nvim = true\nmouse = true\nautosave = true");
        let c = Config::default().merge(&file).merge(&layer(&[
            "--width",
            "90",
            "--no-vim",
            "--no-mouse",
            "--no-autosave",
        ]));
        assert_eq!(
            (c.width, c.vim, c.mouse, c.autosave),
            (90, false, false, false)
        );
    }
}
