# Changelog

All notable changes to mustty are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- A checked task item (`- [x] …` or `- [X] …`) shows its text dimmed, so finished items recede while staying readable and editable; the characters are untouched. Selections, find matches and the cursor show on it as anywhere else. A box inside a code block or a quote is not a task and stays as it is.
- `Ctrl+D` checks or unchecks the task box on the cursor's line (`- [ ]` / `- [x]`, any marker, any indent), in the default keys and in every vim mode. On a list item without a box it adds `[ ] ` after the marker; on any other line, empty or not, it puts `- [ ] ` in front of the text. Each is one undo step. On a heading, a thematic break (`---`), or inside a code block or a quote it does nothing. The help panel (`Ctrl+G`) lists it.
- vim: `d` takes a motion ([#9](https://github.com/joyqi/mustty/issues/9)): `dw`, `de`, `db`, `d0`, `d$`, and `dh`, `dl`, `←`, `→`, `Home`, `End`, delete characters with vim's ranges; `dgg` and `dG` delete whole lines up to the start or end of the document. `dw` on the last word of a line stops at the line end instead of joining the next line. The deleted text goes to the register, so `p` puts it back, and each delete is one undo step.
- vim: `r` followed by a character replaces the character under the cursor (a CJK character or an accented letter counts as one) and stays in Normal mode, as one undo step. `r` followed by `Enter`, `Esc` or a movement key does nothing.
- The help panel (`Ctrl+G`) lists `dw / de / db`, `d0 / d$`, `dgg / dG` and `r{char}` under vim.

## [0.1.7] - 2026-11-25

### Added

- Lists continue as you write ([#2](https://github.com/joyqi/mustty/issues/2)). On a bullet, checkbox or numbered item, `Enter` at the end starts the next item (a new checkbox starts unchecked) and `Enter` in the middle splits the item, moving the text after the cursor to the new one. `Enter` on an empty item ends the list, or moves a nested item out one level.
- `Tab` makes a list item a sub-item of the one above, bringing its own sub-items along, and `Shift+Tab` moves it back out. A moved item takes the marker of the list it joins. Outside lists `Tab` still inserts `tab_width` spaces.
- `Backspace` at the start of an item's text removes the marker, or moves a nested item out one level. At the very start of the line it joins with the line above as before, marker and all.
- Numbered lists renumber themselves when items are added, deleted, moved, pasted or nested, each level counting on its own from the number of its first item; undo puts the old numbers back with the edit. A list written as `1.` on every line becomes 1, 2, 3 once you edit it.
- Deleting the first item of a numbered list keeps the list's start: the next item takes its number, so `1, 2, 3` becomes `1, 2` and a list starting at 5 still starts at 5.
- vim: `O` opens a new line above and enters Insert mode. On a list item, `o` and `O` open an empty item with the right marker.
- The help panel (`Ctrl+G`) lists `Tab / Shift+Tab`, and `o / O` under vim.
- Plain URLs (`http://…`, `https://…`, `www.…`) are underlined like Markdown link text ([#4](https://github.com/joyqi/mustty/issues/4)). A URL may contain CJK and other letters (`…/wiki/春`); trailing punctuation, an unmatched `)` and CJK punctuation (`x.io，还有`) are left out; URLs inside inline code or inside `[text](url)` are not styled twice.
- A `link_color` setting colors link text and plain URLs with one of the 8 terminal colors, so it follows your light or dark theme. It is `none` by default, which keeps links in the text color, underlined. Set it in `~/.mustty.toml` or with the `Ctrl+P` settings panel (the row under Code highlighting).
- A `rule_lines` setting, off by default ([#3](https://github.com/joyqi/mustty/issues/3)). On, a `---`, `***` or `___` line is drawn as a line across the text column, and follows the window when it resizes. Put the cursor on it, select into it or find text on it and it shows its characters again.
- `Ctrl+L` opens the link under the cursor in your browser: the `(url)` of a Markdown link when the cursor is anywhere on it, or a plain URL. With vim keys, `gx` or `Enter` in Normal mode does the same. Only `http://` and `https://` addresses open (`www.` gets `https://`); for anything else, or when the system has no opener (`open`, `xdg-open`), the status bar shows the address instead. On mustty.com the link opens in a new tab.
- The help panel (`Ctrl+G`) lists `Ctrl+L`, and `gx / Enter` under vim.

## [0.1.6] - 2026-11-18

### Added

- Find: `Ctrl+F` opens a find bar in the status line. Matches are found as you type and the first one after the cursor is shown, with every match underlined in yellow, the current one highlighted, and a `3/12` count on the right. `Enter` keeps the cursor on the match; `Esc` puts the cursor, the selection and the view back where they were. After that, `Ctrl+N` / `Ctrl+B` jump to the next / previous match, wrapping around at either end. A lowercase query ignores case, one with a capital letter doesn't; only the body is searched, never the title. With vim keys, `/` opens find and `n` / `N` step through the matches (extending the selection in Visual mode).

### Changed

- Focus mode moves from `Ctrl+F` to `Ctrl+O`, since `Ctrl+F` now opens find. The old key isn't kept. The help panel (`Ctrl+G`) lists both on one line with typewriter mode, `Ctrl+T / Ctrl+O`.

## [0.1.5] - 2026-11-08

### Added

- Code blocks are syntax highlighted by language. A fence that names one of 30 languages (Rust, JavaScript, TypeScript, Python, Go, Bash, PowerShell, JSON, YAML, TOML, SQL, HTML, CSS, C, C++, Java, C#, PHP, Kotlin, Lua, Ruby, Dart, Swift, Markdown, Diff, XML, SCSS, Dockerfile, Makefile and Protobuf) keeps your `code_color`, with keywords in bold, comments dimmed and strings in green; in a `diff` block, added lines are green and removed lines red. The usual names and aliases work (` ```js `, ` ```ts `, ` ```cs `, ` ```sh `, ` ```cpp `, ` ```Rust,no_run `, ` ```{.python} `); other blocks stay one color, as before.
- A `code_highlight` setting, on by default. Turn it off in `~/.mustty.toml` or with the `Ctrl+P` settings panel (the row under Code color) to get one-color code blocks back.
- On mustty.com, the embedded editor has Open and Download buttons for loading a local Markdown file and saving what you wrote, and the site has a favicon.

### Changed

- Highlighting is done by mustty's own small line-by-line lexer, with no new dependencies: each keystroke rescans at most to the end of the code block it is in. Its keyword and type lists are generated from highlight.js and corrected against each language's official specification; the generator lives in `tools/lexer-tables/`, and `LICENSE` now includes highlight.js's BSD-3-Clause notice.

## [0.1.4] - 2026-11-02

### Added

- mustty runs in the browser: [mustty.com](https://mustty.com) embeds the full editor compiled to WebAssembly. Type, click to place the cursor, scroll with the wheel and maximize it to the whole window; mixed English, Chinese, Japanese and Korean text lines up and wraps as in a terminal. Nothing is saved there.
- The mustty.com website: what mustty is, the install command for each platform with a copy button, and the live demo, in light and dark.

### Changed

- The terminal layer sits behind a platform-independent interface, with its own ANSI renderer and input parser, so the same editor core drives both the terminal and the browser. Nothing changes in the terminal: a byte-for-byte recording of a whole session guards it.
- The crate's homepage is now https://mustty.com.

### Fixed

- With vim keys, undoing (`u`) after deleting across lines no longer leaves a reversed-video selection nobody made; nor does `d`, `x`, `y` or `p` in Visual mode at the end of the document.

## [0.1.3] - 2026-11-01

### Added

- One-line installers: `mustty-installer.sh` for macOS and Linux and `mustty-installer.ps1` for Windows PowerShell download the right archive for your platform. The shell installer also checks the archive's SHA-256 checksum.
- Release archives include this changelog next to `LICENSE` and `README.md`.

### Changed

- Release notes on GitHub now lead with the changes from this changelog, followed by the install commands and downloads.
- The README's Install section lists each way to install (installer script, PowerShell, Homebrew, from source) as its own short command.

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

[Unreleased]: https://github.com/joyqi/mustty/compare/v0.1.3...HEAD
[0.1.3]: https://github.com/joyqi/mustty/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/joyqi/mustty/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/joyqi/mustty/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/joyqi/mustty/releases/tag/v0.1.0
