//! 文档模型：标题（首行 H1）与正文分离（方案 §4.1、§5.5、§8.3）。

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
    /// 新文件的换行符：Windows 上为 CRLF，其他平台 LF。
    pub fn platform() -> Self {
        if cfg!(windows) { Self::CrLf } else { Self::Lf }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMeta {
    pub path: Option<PathBuf>,
    pub line_ending: LineEnding,
    pub has_bom: bool,
    /// 上次读 / 写时磁盘上的修改时间与大小，用于外部修改检测
    pub stamp: Option<DiskStamp>,
}

/// mtime 粒度粗（或被同步盘还原）时，大小变化仍能发现外部修改。
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

    /// 测试用：固定 LF，让断言不随平台的默认换行变化（平台默认由专门的测试覆盖）。
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
    /// 首行 `# text`，`sep` 为紧随其后的连续换行符。
    Bound {
        text: String,
        sep: String,
    },
}

pub struct Document {
    pub title: TitleField,
    /// 只能经 `replace` 修改
    body: Rope,
    pub meta: FileMeta,
    /// 当前内容的版本号。undo 会回到旧值，所以新版本号从 `last_revision` 递增取得，永不复用。
    revision: u64,
    last_revision: u64,
}

impl Document {
    /// `raw` 已去 BOM、换行已统一为 `\n`。
    pub fn parse(raw: &str, meta: FileMeta) -> Self {
        let (first, rest) = raw.split_once('\n').unwrap_or((raw, ""));
        // 空标题不绑定：保存时会被删除，无法保证字节往返
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

    /// BOM? + 标题行 + sep + 正文，并还原换行符。
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
            // 原文件只有标题行、之后又写了正文时，补一个换行
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

    /// 正文唯一写入口。
    pub fn replace(&mut self, range: Range<usize>, text: &str) {
        self.body.remove(range.clone());
        self.body.insert(range.start, text);
        self.bump();
    }

    /// 写入标题；Unbound 时变为 `Bound { sep: "\n\n" }`。
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

    /// undo / redo 回到某个版本。
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
        // 方案 §8.3：新文件按平台默认换行（Windows CRLF，其余 LF）
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
