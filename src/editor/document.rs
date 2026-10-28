//! 文档模型：标题（首行 H1）与正文分离（方案 §5.5）。

use ropey::Rope;

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
    pub body: Rope,
}

impl Document {
    /// `raw` 已去 BOM、换行已统一为 `\n`。
    pub fn parse(raw: &str) -> Self {
        let (first, rest) = raw.split_once('\n').unwrap_or((raw, ""));
        let title = first.strip_prefix("# ").filter(|_| !first.ends_with('#'));
        match title {
            Some(text) => {
                let body = rest.trim_start_matches('\n');
                let newlines = raw.len() - first.len() - body.len();
                Self {
                    title: TitleField::Bound {
                        text: text.to_owned(),
                        sep: "\n".repeat(newlines),
                    },
                    body: Rope::from_str(body),
                }
            }
            None => Self {
                title: TitleField::Unbound,
                body: Rope::from_str(raw),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            ("# T\n正文", bound("T", "\n"), "正文"),
            ("# 闭合 #\n正文", TitleField::Unbound, "# 闭合 #\n正文"),
            ("## 二级\n", TitleField::Unbound, "## 二级\n"),
            ("#无空格\n", TitleField::Unbound, "#无空格\n"),
            (
                "---\ntitle: x\n---\n# T\n",
                TitleField::Unbound,
                "---\ntitle: x\n---\n# T\n",
            ),
            ("", TitleField::Unbound, ""),
        ];
        for (raw, title, body) in cases {
            let doc = Document::parse(raw);
            assert_eq!(doc.title, title, "{raw:?}");
            assert_eq!(doc.body.to_string(), body, "{raw:?}");
        }
    }
}
