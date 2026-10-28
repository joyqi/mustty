use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use anyhow::anyhow;
use clap::Parser;
use mustty::app::{self, App};
use mustty::config::{self, Config, PartialConfig};
use mustty::editor::document::{Document, FileMeta};
use mustty::io::file::{IoError, load};
use mustty::terminal::TerminalGuard;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

/// 终端里的极简 Markdown 写作器
#[derive(Parser)]
#[command(
    version,
    about,
    after_help = "配置文件：~/.mustty.toml（Windows 为用户主目录下的 .mustty.toml）。\n优先级：命令行参数 > 配置文件 > 内置默认值。"
)]
struct Cli {
    /// 要编辑的文件；不存在则在首次保存时创建，省略则为 Untitled
    file: Option<PathBuf>,
    /// 正文宽度（列），钳制在 40..=120，默认 72
    #[arg(long, value_name = "N")]
    width: Option<u16>,
    /// 自动保存（默认开启：停手 2 秒后保存）
    #[arg(long, overrides_with = "no_autosave")]
    autosave: bool,
    /// 关闭自动保存
    #[arg(long, overrides_with = "autosave")]
    no_autosave: bool,
    /// 启用 vim 风格的 Normal / Insert 键位
    #[arg(long, overrides_with = "no_vim")]
    vim: bool,
    /// 关闭 vim 键位（覆盖配置文件）
    #[arg(long, overrides_with = "vim")]
    no_vim: bool,
    /// 启用鼠标：单击定位、拖拽选择、滚轮滚动。开启后终端原生拖选复制需按住 Shift
    #[arg(long, overrides_with = "no_mouse")]
    mouse: bool,
    /// 关闭鼠标（覆盖配置文件）
    #[arg(long, overrides_with = "mouse")]
    no_mouse: bool,
}

impl Cli {
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
    let cli = Cli::parse();
    let _log = match init_log() {
        Ok(guard) => guard,
        Err(e) => {
            eprintln!("{e:#}");
            return ExitCode::FAILURE;
        }
    };
    let (config, warnings) = config::load(&cli.layer());
    // 配置写错不阻止启动：警告打到 stderr（进入全屏前，退出后仍可见），
    // 同时汇总成一条 status bar 消息，保留到第一次按键
    let warning = config::summarize(&warnings);
    if let Some(w) = &warning {
        eprintln!("mustty: {w}");
        tracing::warn!("{w}");
    }
    match run(cli.file.as_deref(), &config, warning) {
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
        .map_err(|e| anyhow!("无法打开日志文件 {}：{e}", Path::new(&path).display()))?;
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

fn run(path: Option<&Path>, config: &Config, warning: Option<String>) -> anyhow::Result<()> {
    let doc = match path {
        Some(path) => load(path).map_err(|e| match e {
            IoError::Io(e) => anyhow!("无法打开 {}：{e}", path.display()),
            e => e.into(),
        })?,
        None => Document::parse("", FileMeta::new(None)),
    };
    tracing::info!(path = ?path, ?config, "start");
    let mut guard = TerminalGuard::new(config.mouse)?;
    let mut app = App::with_config(doc, guard.terminal.size()?, config);
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
