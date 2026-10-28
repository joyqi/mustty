use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::anyhow;
use clap::Parser;
use mustty::app::{self, App};
use mustty::editor::document::{Document, FileMeta};
use mustty::io::file::{IoError, load};
use mustty::terminal::TerminalGuard;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

/// 终端里的极简 Markdown 写作器
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// 要编辑的文件；不存在则在首次保存时创建，省略则为 Untitled
    file: Option<PathBuf>,
    /// 关闭自动保存（默认停手 2 秒后保存）
    #[arg(long)]
    no_autosave: bool,
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
    match run(cli.file.as_deref(), !cli.no_autosave) {
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

fn run(path: Option<&Path>, autosave: bool) -> anyhow::Result<()> {
    let doc = match path {
        Some(path) => load(path).map_err(|e| match e {
            IoError::Io(e) => anyhow!("无法打开 {}：{e}", path.display()),
            e => e.into(),
        })?,
        None => Document::parse("", FileMeta::new(None)),
    };
    tracing::info!(path = ?path, autosave, "start");
    let mut guard = TerminalGuard::new()?;
    let mut app = App::new(doc, guard.terminal.size()?, autosave);
    app::run(&mut guard.terminal, &mut app)
}
