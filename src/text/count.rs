//! 字数统计（方案 §9）：每个汉字计 1，英文按词计，标点不计。

use unicode_segmentation::UnicodeSegmentation;

pub fn count_words(s: &str) -> usize {
    s.unicode_words().count()
}

/// 千分位：`1284` → `1,284`。
pub fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts() {
        let cases = [
            ("", 0),
            ("hello world", 2),
            ("don't stop", 2),
            ("中文字数", 4),
            ("Rust 是一门语言", 6),
            ("，。！？——「」", 0),
            ("v1.2 版本，3,000 行", 5),
            ("ひらがな", 4),
            ("emoji 🎉 不计", 3),
        ];
        for (s, want) in cases {
            assert_eq!(count_words(s), want, "{s:?}");
        }
    }

    #[test]
    fn thousands_separator() {
        for (n, want) in [
            (0, "0"),
            (999, "999"),
            (1284, "1,284"),
            (1_000_000, "1,000,000"),
        ] {
            assert_eq!(thousands(n), want);
        }
    }
}
