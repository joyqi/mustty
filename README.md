<p align="center"><picture><source media="(prefers-color-scheme: dark)" srcset="assets/logo-dark.svg"><img src="assets/logo-light.svg" alt="mustty" width="140"></picture></p>

<h1 align="center">mustty</h1>

<p align="center">An old-fashioned Markdown writer for the terminal</p>

<p align="center">
  <a href="https://github.com/joyqi/mustty/actions/workflows/ci.yml"><img src="https://github.com/joyqi/mustty/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT"></a>
  <img src="https://img.shields.io/badge/rust-1.88%2B-orange.svg" alt="Rust 1.88+">
</p>

mustty is an unhurried place to write by hand, one word at a time, inside the terminal. It opens a single Markdown file on a centered 72-column sheet with nothing on screen but the title, the body, and a one-line status bar. Syntax markers stay visible but dimmed, so you always edit the source in front of you.

<p align="center"><picture><source media="(prefers-color-scheme: dark)" srcset="assets/screenshot-dark.svg"><img src="assets/screenshot-light.svg" alt="Screenshot of the mustty interface: a bold centered title, a 72-column body of Markdown with dimmed markers, and a status bar showing notes.md with a green saved dot, the word count and the cursor position" width="760"></picture></p>

## Why mustty

- One file at a time on a 72-column centered page (40–120), with no file tree, splits, borders, or chrome.
- Markdown headings, emphasis, code, links, lists, and quotes are styled in place while their `#`, `*`, and `` ` `` markers dim instead of disappearing.
- Autosave writes atomically after 2 seconds of idle time, on focus loss, and on quit, and pauses if another program modifies the file on disk.
- Soft wrapping follows Unicode line-breaking rules (UAX #14), so mixed English and CJK text wraps cleanly and closing punctuation never starts a line.
- Positioning the terminal's hardware cursor—rather than drawing a fake one—keeps IME candidate windows attached to the character you are typing.
- The terminal window title follows the document title (or its file name) as you type, with a leading `*` while there are unsaved changes.
- Typewriter mode (`Ctrl+T`), Focus mode (`Ctrl+F`), optional vim keys, optional mouse support, OSC 52 clipboard over SSH, and an English / Simplified Chinese UI.

## Install

```sh
curl -LsSf https://github.com/joyqi/mustty/releases/latest/download/mustty-installer.sh | sh
powershell -c "irm https://github.com/joyqi/mustty/releases/latest/download/mustty-installer.ps1 | iex"
brew install joyqi/tap/mustty
cargo install --git https://github.com/joyqi/mustty
```

Or grab an archive from [Releases](https://github.com/joyqi/mustty/releases).

## Usage

```text
mustty [FILE]
```

Pass a file to open or create it on first save, or omit `FILE` to start an Untitled document (`-h` / `--help` and `-V` / `--version` are the only flags).

- Press `Ctrl+G` inside the app (or `?` in vim Normal mode) to see the shortcuts worth knowing.
- Press `Ctrl+P` to open the live settings panel. Changes apply immediately and write back to `~/.mustty.toml` (`width`, `vim`, `mouse`, `autosave`, `autosave_idle_ms`, `typewriter`, `focus_mode`, `scrolloff`, `tab_width`, `code_color`, `cursor`, `cursor_blink`, `lang`).
  `cursor` is `bar`, `block`, or `underline`, previewed live in the panel; with vim on, `cursor` and `cursor_blink` only affect Insert mode (Normal and Visual always use a block).

## License

[MIT](LICENSE)
