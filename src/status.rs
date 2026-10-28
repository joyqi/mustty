//! status bar 的内容：文件名 + `行:列`（方案 §9，M0 子集）。

use std::path::Path;

use ropey::Rope;

use crate::text::grapheme::graphemes;
use crate::text::wrap::line_text;

pub struct StatusModel {
    pub file_name: String,
}

impl StatusModel {
    pub fn new(path: &Path) -> Self {
        let file_name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        Self { file_name }
    }

    /// 行 = 逻辑行号，列 = 行内 grapheme 序号，均从 1 起。
    pub fn position(rope: &Rope, head: usize) -> String {
        let line = rope.char_to_line(head);
        let local = head - rope.line_to_char(line);
        let col = graphemes(&line_text(rope, line))
            .take_while(|&(c, _)| c < local)
            .count();
        format!("{}:{}", line + 1, col + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_counts_graphemes() {
        let rope = Rope::from_str("a\n中e\u{301}x");
        assert_eq!(StatusModel::position(&rope, 0), "1:1");
        assert_eq!(StatusModel::position(&rope, 2), "2:1");
        assert_eq!(StatusModel::position(&rope, 5), "2:3");
    }
}
