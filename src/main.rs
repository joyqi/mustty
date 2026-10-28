use std::path::Path;
use std::process::ExitCode;

use anyhow::anyhow;
use mustty::app::{self, App};
use mustty::editor::document::{Document, FileMeta};
use mustty::io::file::{IoError, load};
use mustty::terminal::TerminalGuard;

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() > 1 {
        eprintln!("用法: mustty [FILE]");
        return ExitCode::from(2);
    }
    match run(args.first().map(Path::new)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(path: Option<&Path>) -> anyhow::Result<()> {
    let doc = match path {
        Some(path) => load(path).map_err(|e| match e {
            IoError::Io(e) => anyhow!("无法打开 {}：{e}", path.display()),
            e => e.into(),
        })?,
        None => Document::parse("", FileMeta::new(None)),
    };
    let mut guard = TerminalGuard::new()?;
    let mut app = App::new(doc, guard.terminal.size()?);
    app::run(&mut guard.terminal, &mut app)
}
