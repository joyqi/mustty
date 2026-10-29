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

    /// For tests: pin LF so assertions don't depend on the platform default (a dedicated test covers that).
    #[cfg(test)]
    pub(crate) fn lf() -> Self {
        Self {
            line_ending: LineEnding::Lf,
            ..Self::new(None)
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
    /// `raw` has its BOM stripped and line endings normalized to `\n`.
    pub fn parse(raw: &str, meta: FileMeta) -> Self {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(raw: &str) -> Document {
        Document::parse(raw, FileMeta::lf())
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

    #[test]
    fn new_documents_use_platform_line_ending() {
        // New files use the platform line ending (CRLF on Windows, LF elsewhere)
        assert_eq!(FileMeta::new(None).line_ending, LineEnding::platform());
        let meta = FileMeta {
            line_ending: LineEnding::platform(),
            ..FileMeta::new(None)
        };
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
