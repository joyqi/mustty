# Changelog

All notable changes to mustty are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- One-line installers: `mustty-installer.sh` for macOS and Linux and `mustty-installer.ps1` for Windows PowerShell pick the right archive for your platform.

## [0.1.2] - 2026-10-30

### Changed

- The settings panel shows its cursor preview on the help line, so the selected row no longer hides the cursor shape and the preview stays put while you move between rows.
- With vim keys on, the cursor settings are named "Insert cursor" and "Insert cursor blink", since Normal and Visual mode always use a block.
- The `scrolloff` setting has a clearer label and help text in both English and Simplified Chinese.

### Fixed

- Tab in the title moves to the start of the body, like Enter, instead of padding the title with spaces.

## [0.1.1] - 2026-10-29

### Added

- The terminal window title follows the document title (or the file name, or Untitled) as you type, with a leading `*` while there are unsaved changes.
- `cursor` (`bar`, `block` or `underline`) and `cursor_blink` settings, previewed live in the `Ctrl+P` settings panel.
- Saving a new document with `Ctrl+S` offers `untitled.md` (or the next free `untitled-N.md`) as the file name, so Enter saves it right away.

## [0.1.0] - 2026-10-28

### Added

- A single Markdown file on a centered 72-column page (adjustable from 40 to 120) with only the title, the body and a one-line status bar.
- Markdown headings, emphasis, code, links, lists and quotes styled in place, with their markers dimmed instead of hidden.
- Autosave that writes atomically after 2 seconds of idle time, on focus loss and on quit, and pauses when another program changes the file on disk.
- A status bar with the file name, a green dot once everything is saved, the word count and the cursor position.
- Undo and redo, selection, word motions, bracketed paste, and OSC 52 clipboard support over SSH.
- Soft wrapping that follows Unicode line-breaking rules, so mixed English and CJK text wraps cleanly.
- The real terminal cursor, so IME candidate windows stay next to the text you are typing.
- Typewriter mode (`Ctrl+T`), Focus mode (`Ctrl+F`), optional vim keys and optional mouse support.
- A settings panel (`Ctrl+P`) that applies changes immediately and saves them to `~/.mustty.toml`.
- A shortcuts panel (`Ctrl+G`, or `?` in vim Normal mode).
- An English and Simplified Chinese interface, chosen automatically or set with `lang`.
- Prebuilt archives for macOS, Linux and Windows, and `brew install joyqi/tap/mustty`.

[Unreleased]: https://github.com/joyqi/mustty/compare/v0.1.2...HEAD
[0.1.2]: https://github.com/joyqi/mustty/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/joyqi/mustty/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/joyqi/mustty/releases/tag/v0.1.0
