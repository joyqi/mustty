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

// Help text is filled in at runtime in the UI language (see `command`), so no doc comments here.
// There is only a file argument: every other setting lives in the config file and can be changed in the Ctrl+P settings panel
#[derive(Parser)]
#[command(version)]
struct Cli {
    file: Option<PathBuf>,
}

impl Cli {
    /// The clap command with localized help text. clap's built-in Usage: / Options: and parse errors can't be localized.
    fn command() -> clap::Command {
        <Self as CommandFactory>::command()
            .about(t("cli-about"))
            .after_help(t("cli-after-help"))
            .mut_arg("file", |a| a.help(t("cli-file")))
    }
}

fn main() -> ExitCode {
    // The language must be settled before clap parses (--help is translated too): negotiate from the
    // config file's lang first, then load the config again so warnings use that language as well
    i18n::init();
    i18n::use_config(config::load().0.lang);
    let cli = Cli::from_arg_matches(&Cli::command().get_matches()).unwrap_or_else(|e| e.exit());
    let (config, warnings) = config::load();
    // A bad config doesn't block startup: warnings go to stderr (before entering full screen, so they
    // stay visible after exit) and are also summarized into one status bar message that lasts until the first keypress
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
                    "{tag}: --{} has no help text",
                    arg.get_id()
                );
            }
        }
    }

    #[test]
    fn only_file_help_and_version() {
        let mut cmd = Cli::command();
        cmd.build(); // help / version are only added at build time
        let ids: Vec<&str> = cmd.get_arguments().map(|a| a.get_id().as_str()).collect();
        assert_eq!(ids, vec!["file", "help", "version"]);
        let parse = |args: &[&str]| {
            Cli::try_parse_from(std::iter::once("mustty").chain(args.iter().copied()))
        };
        assert_eq!(parse(&["notes.md"]).unwrap().file, Some("notes.md".into()));
        assert_eq!(parse(&[]).unwrap().file, None);
        // Removed flags are rejected, not silently ignored
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
                .unwrap_or_else(|| panic!("{removed} is still accepted"));
            assert_eq!(
                err.kind(),
                clap::error::ErrorKind::UnknownArgument,
                "{removed}"
            );
        }
    }
}
