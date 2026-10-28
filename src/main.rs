use std::path::Path;
use std::process::ExitCode;

use anyhow::anyhow;
use mustty::app::{self, App};
use mustty::io::file::{IoError, load};
use mustty::terminal::TerminalGuard;

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let (Some(path), None) = (args.next(), args.next()) else {
        eprintln!("用法: mustty FILE");
        return ExitCode::from(2);
    };
    match run(Path::new(&path)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(path: &Path) -> anyhow::Result<()> {
    let doc = load(path).map_err(|e| match e {
        IoError::Io(e) => anyhow!("无法打开 {}：{e}", path.display()),
        e => e.into(),
    })?;
    let mut guard = TerminalGuard::new()?;
    let mut app = App::new(doc, path, guard.terminal.size()?);
    app::run(&mut guard.terminal, &mut app)
}
