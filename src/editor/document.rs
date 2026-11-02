//! Document model: the title (a first-line H1) is kept separate from the body.

use std::ops::Range;
use std::path::PathBuf;
use std::time::SystemTime;

use ropey::Rope;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    CrLf,
}

impl LineEnding {
    /// Line ending for new files: CRLF on Windows, LF elsewhere.
    pub fn platform() -> Self {
        if cfg!(windows) { Self::CrLf } else { Self::Lf }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMeta {
    pub path: Option<PathBuf>,
    pub line_ending: LineEnding,
    pub has_bom: bool,
    /// On-disk mtime and size at the last read / write, used to detect external changes
    pub stamp: Option<DiskStamp>,
}

/// When mtime is coarse (or restored by a sync drive), a size change still reveals an external edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskStamp {
    pub mtime: SystemTime,
    pub len: u64,
}

impl FileMeta {
    pub fn new(path: Option<PathBuf>) -> Self {
        Self {
            path,
            line_ending: LineEnding::platform(),
            has_bom: false,
            stamp: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TitleField {
    Unbound,
    /// A first line of `# text`; `sep` holds the run of newlines right after it.
    Bound {
        text: String,
        sep: String,
    },
}

pub struct Document {
    pub title: TitleField,
    /// Modified only through `replace`
    body: Rope,
    pub meta: FileMeta,
    /// Revision of the current content. Undo goes back to old values, so new revisions come from incrementing `last_revision` and are never reused.
    revision: u64,
    last_revision: u64,
}

impl Document {
    /// `raw` has its BOM stripped and line endings normalized to `\n` (what `io::file::decode` does);
    /// `meta.line_ending` says what `serialize` turns them back into.
    pub fn parse(raw: &str, meta: FileMeta) -> Self {
        debug_assert!(
            !raw.contains("\r\n"),
            "Document::parse takes \\n-normalized text; decode CRLF input with io::file::decode"
        );
        let (first, rest) = raw.split_once('\n').unwrap_or((raw, ""));
        // Don't bind an empty title: saving would drop it, breaking the byte-exact round trip
        let title = first
            .strip_prefix("# ")
            .filter(|t| !t.is_empty() && !first.ends_with('#'));
        let (title, body) = match title {
            Some(text) => {
                let body = rest.trim_start_matches('\n');
                let newlines = raw.len() - first.len() - body.len();
                let title = TitleField::Bound {
                    text: text.to_owned(),
                    sep: "\n".repeat(newlines),
                };
                (title, body)
            }
            None => (TitleField::Unbound, raw),
        };
        Self {
            title,
            body: Rope::from_str(body),
            meta,
            revision: 0,
            last_revision: 0,
        }
    }

    /// BOM? + title line + sep + body, with the original line endings restored.
    pub fn serialize(&self) -> String {
        let mut out = String::new();
        if self.meta.has_bom {
            out.push('\u{feff}');
        }
        if let TitleField::Bound { text, sep } = &self.title
            && !text.is_empty()
        {
            out.push_str("# ");
            out.push_str(text);
            // If the file had only a title line and a body was written later, add a newline
            out.push_str(if sep.is_empty() && self.body.len_chars() > 0 {
                "\n"
            } else {
                sep
            });
        }
        out.push_str(&self.body.to_string());
        match self.meta.line_ending {
            LineEnding::Lf => out,
            LineEnding::CrLf => out.replace('\n', "\r\n"),
        }
    }

    /// The body's single write path.
    pub fn replace(&mut self, range: Range<usize>, text: &str) {
        self.body.remove(range.clone());
        self.body.insert(range.start, text);
        self.bump();
    }

    /// Set the title; an Unbound title becomes `Bound { sep: "\n\n" }`.
    pub fn set_title(&mut self, text: String) {
        match &mut self.title {
            TitleField::Bound { text: t, .. } => *t = text,
            TitleField::Unbound => {
                self.title = TitleField::Bound {
                    text,
                    sep: "\n\n".into(),
                }
            }
        }
        self.bump();
    }

    fn bump(&mut self) {
        self.last_revision += 1;
        self.revision = self.last_revision;
    }

    /// Undo / redo jumps back to a given revision.
    pub(crate) fn set_revision(&mut self, revision: u64) {
        self.revision = revision;
    }

    pub fn body(&self) -> &Rope {
        &self.body
    }

    pub fn title_text(&self) -> &str {
        match &self.title {
            TitleField::Bound { text, .. } => text,
            TitleField::Unbound => "",
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }
}

/// For tests: metadata with LF line endings, so no assertion depends on the platform default (CRLF
/// on Windows). Tests use this rather than `FileMeta::new`, which `test_meta_pins_line_endings`
/// checks; tests/support/mod.rs has the same helper for the integration tests.
#[cfg(test)]
pub(crate) fn lf_meta(path: Option<&str>) -> FileMeta {
    FileMeta {
        path: path.map(PathBuf::from),
        line_ending: LineEnding::Lf,
        has_bom: false,
        stamp: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn parse(raw: &str) -> Document {
        Document::parse(raw, lf_meta(None))
    }

    fn bound(text: &str, sep: &str) -> TitleField {
        TitleField::Bound {
            text: text.into(),
            sep: sep.into(),
        }
    }

    #[test]
    fn title_rules() {
        let cases = [
            ("# 标题\n\n正文\n", bound("标题", "\n\n"), "正文\n"),
            ("# 只有标题", bound("只有标题", ""), ""),
            ("# 只有标题\n", bound("只有标题", "\n"), ""),
            ("# T\n正文", bound("T", "\n"), "正文"),
            ("# T\n\n\n\n正文", bound("T", "\n\n\n\n"), "正文"),
            ("# ", TitleField::Unbound, "# "),
            ("# 闭合 #\n正文", TitleField::Unbound, "# 闭合 #\n正文"),
            ("## 二级\n", TitleField::Unbound, "## 二级\n"),
            ("#无空格\n", TitleField::Unbound, "#无空格\n"),
            (" # 前导空格\n", TitleField::Unbound, " # 前导空格\n"),
            (
                "正文\n# 不在首行\n",
                TitleField::Unbound,
                "正文\n# 不在首行\n",
            ),
            (
                "---\ntitle: x\n---\n# T\n",
                TitleField::Unbound,
                "---\ntitle: x\n---\n# T\n",
            ),
            ("", TitleField::Unbound, ""),
        ];
        for (raw, title, body) in cases {
            let doc = parse(raw);
            assert_eq!(doc.title, title, "{raw:?}");
            assert_eq!(doc.body().to_string(), body, "{raw:?}");
            assert_eq!(doc.serialize(), raw, "round trip {raw:?}");
        }
    }

    #[test]
    fn title_edits_serialize() {
        let mut doc = parse("正文\n");
        doc.set_title("新标题".into());
        assert_eq!(doc.serialize(), "# 新标题\n\n正文\n");
        doc.set_title(String::new());
        assert_eq!(doc.serialize(), "正文\n");

        let mut doc = parse("# 只有标题");
        doc.replace(0..0, "补上正文");
        assert_eq!(doc.serialize(), "# 只有标题\n补上正文");
    }

    #[test]
    fn replace_bumps_revision() {
        let mut doc = parse("abc");
        doc.replace(1..2, "中文");
        assert_eq!(doc.body().to_string(), "a中文c");
        assert_eq!(doc.revision(), 1);
    }

    /// `FileMeta::new` picks the platform line ending (CRLF on Windows), so a test that builds its
    /// metadata that way passes on macOS and Linux and fails only on Windows CI. Test code (a file's
    /// part after `#[cfg(test)]` in src/, everything in tests/ but the support helpers) builds it with
    /// `lf_meta` instead. A line that wants the platform default says so with the marker below.
    #[test]
    fn test_meta_pins_line_endings() {
        fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    sources(&path, out);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    out.push(path);
                }
            }
        }
        // Spelled in two parts so this file's own scan doesn't find it here
        let needle = concat!("FileMeta", "::new(");
        let marker = "platform default on purpose";
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let (mut src, mut tests) = (Vec::new(), Vec::new());
        sources(&root.join("src"), &mut src);
        sources(&root.join("tests"), &mut tests);
        assert!(
            src.len() > 20 && tests.len() > 5,
            "found only {src:?} {tests:?}"
        );
        let support = root.join("tests/support");
        let test_code = src
            .iter()
            .filter_map(|f| {
                let text = std::fs::read_to_string(f).unwrap();
                let (_, tests) = text.split_once("#[cfg(test)]")?;
                Some((f, tests.to_owned()))
            })
            .chain(
                tests
                    .iter()
                    .filter(|f| !f.starts_with(&support))
                    .map(|f| (f, std::fs::read_to_string(f).unwrap())),
            );
        let mut found = Vec::new();
        for (file, code) in test_code {
            for line in code.lines() {
                if line.contains(needle) && !line.contains(marker) {
                    found.push(format!("{}: {}", file.display(), line.trim()));
                }
            }
        }
        assert!(
            found.is_empty(),
            "use lf_meta in tests:\n{}",
            found.join("\n")
        );
    }

    /// The other half of the guard: CRLF text handed straight to parse (not through decode) is caught
    /// on every platform, not just where serialize would double the \r
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "normalized")]
    fn parse_rejects_crlf_text() {
        Document::parse("# T\r\n\r\na\r\n", lf_meta(None));
    }

    #[test]
    fn new_documents_use_platform_line_ending() {
        // New files use the platform line ending (CRLF on Windows, LF elsewhere)
        let meta = FileMeta::new(None); // platform default on purpose
        assert_eq!(meta.line_ending, LineEnding::platform());
        let doc = Document::parse("# T\n\na\n", meta);
        let want = if cfg!(windows) {
            "# T\r\n\r\na\r\n"
        } else {
            "# T\n\na\n"
        };
        assert_eq!(doc.serialize(), want);
    }

    #[test]
    fn crlf_and_bom_restored() {
        let meta = FileMeta {
            path: None,
            line_ending: LineEnding::CrLf,
            has_bom: true,
            stamp: None,
        };
        let doc = Document::parse("# T\n\na\nb", meta);
        assert_eq!(doc.serialize(), "\u{feff}# T\r\n\r\na\r\nb");
    }
}
