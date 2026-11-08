//! Lightweight line-by-line Markdown scanner. It only produces style ranges and never changes the
//! text. Each line depends only on the block state it starts in (inside a fenced code block or
//! not, and where the code lexer stands), so results can be cached per line and rescanned
//! incrementally.

use std::ops::Range;

use ropey::Rope;

use crate::code::{self, CodeState, LangId};
use crate::text::wrap::line_text;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BlockState {
    #[default]
    Normal,
    Fenced {
        fence: char,
        len: u8,
        /// From the opening fence's info string; `None` when it names no language we know
        lang: Option<LangId>,
        /// The code lexer's state at the start of the line. The closing fence drops it, so an edit
        /// inside a block rescans at most to the block's end.
        code: CodeState,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    HeadingMark,
    Heading(u8),
    EmphMark,
    Emph,
    StrongMark,
    Strong,
    CodeMark,
    Code,
    FenceLine,
    FencedCode,
    /// A code lexer token inside a fenced block, layered over `FencedCode`
    Syntax(code::Token),
    QuoteMark,
    Quote,
    ListMark,
    LinkPunct,
    LinkText,
    LinkUrl,
    Rule,
}

/// Char offsets `[start, end)` within the line. Ranges may nest; the renderer layers styles in
/// order (outer first).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StyledRange {
    pub start: usize,
    pub end: usize,
    pub token: Token,
}

pub fn scan_line(line: &str, state_in: BlockState) -> (Vec<StyledRange>, BlockState) {
    let chars: Vec<char> = line.chars().collect();
    let n = chars.len();
    let whole = |token| {
        vec![StyledRange {
            start: 0,
            end: n,
            token,
        }]
    };
    let indent = chars.iter().take_while(|&&c| c == ' ').count();
    let fence_run = |c: char| {
        let run = chars[indent..].iter().take_while(|&&x| x == c).count();
        (indent <= 3 && run >= 3).then_some(run)
    };

    if let BlockState::Fenced {
        fence,
        len,
        lang,
        code,
    } = state_in
    {
        let closes = fence_run(fence).is_some_and(|run| {
            run >= len as usize && chars[indent + run..].iter().all(|c| c.is_whitespace())
        });
        if closes {
            return (whole(Token::FenceLine), BlockState::Normal);
        }
        let mut out = whole(Token::FencedCode);
        let Some(lang) = lang else {
            return (out, state_in);
        };
        let (ranges, code) = code::scan(lang, line, code);
        out.extend(ranges.into_iter().map(|r| StyledRange {
            start: r.start,
            end: r.end,
            token: Token::Syntax(r.token),
        }));
        let state_out = BlockState::Fenced {
            fence,
            len,
            lang: Some(lang),
            code,
        };
        return (out, state_out);
    }
    for fence in ['`', '~'] {
        if let Some(run) = fence_run(fence) {
            let info = &chars[indent + run..];
            // A backtick fence's info string must not contain backticks
            if fence == '~' || !info.contains(&'`') {
                let len = run.min(u8::MAX as usize) as u8;
                let lang = code::aliases::from_info(&info.iter().collect::<String>());
                let state_out = BlockState::Fenced {
                    fence,
                    len,
                    lang,
                    code: CodeState::Normal,
                };
                return (whole(Token::FenceLine), state_out);
            }
        }
    }
    if is_rule(&chars) {
        return (whole(Token::Rule), BlockState::Normal);
    }

    let mut out = Vec::new();
    let mut pos = 0;
    // Quotes can nest: `> > text`
    while pos < n && chars[pos] == '>' {
        let end = pos + 1 + usize::from(chars.get(pos + 1) == Some(&' '));
        out.push(StyledRange {
            start: pos,
            end,
            token: Token::QuoteMark,
        });
        pos = end;
    }
    if pos > 0 {
        out.insert(
            0,
            StyledRange {
                start: pos,
                end: n,
                token: Token::Quote,
            },
        );
    }
    if let Some(level) = heading_level(&chars[pos..]) {
        let mark_end = pos
            + level
            + chars[pos + level..]
                .iter()
                .take_while(|&&c| c == ' ')
                .count();
        out.push(StyledRange {
            start: pos,
            end: mark_end,
            token: Token::HeadingMark,
        });
        out.push(StyledRange {
            start: mark_end,
            end: n,
            token: Token::Heading(level as u8),
        });
        pos = mark_end;
    } else if let Some(end) = list_marker(&chars, pos) {
        out.push(StyledRange {
            start: pos,
            end,
            token: Token::ListMark,
        });
        pos = end;
    }
    scan_inline(&chars, pos, &mut out);
    (out, BlockState::Normal)
}

/// `---`, `***`, `___` (spaces allowed in between, at least 3 marks)
fn is_rule(chars: &[char]) -> bool {
    let marks: Vec<char> = chars
        .iter()
        .copied()
        .filter(|c| !c.is_whitespace())
        .collect();
    marks.len() >= 3
        && matches!(marks[0], '-' | '*' | '_')
        && marks.iter().all(|&c| c == marks[0])
        && chars.iter().take_while(|&&c| c == ' ').count() <= 3
}

fn heading_level(chars: &[char]) -> Option<usize> {
    let level = chars.iter().take_while(|&&c| c == '#').count();
    let follows = chars.get(level).is_none_or(|&c| c == ' ');
    ((1..=6).contains(&level) && follows).then_some(level)
}

/// Where the list marker ends, including indent and task box:
/// `- `, `* `, `+ `, `1. `, `1) `, `- [ ] `.
fn list_marker(chars: &[char], from: usize) -> Option<usize> {
    let mut i = from + chars[from..].iter().take_while(|&&c| c == ' ').count();
    match chars.get(i) {
        Some('-' | '*' | '+') => i += 1,
        Some(c) if c.is_ascii_digit() => {
            let digits = chars[i..].iter().take_while(|c| c.is_ascii_digit()).count();
            if digits > 9 || !matches!(chars.get(i + digits), Some('.' | ')')) {
                return None;
            }
            i += digits + 1;
        }
        _ => return None,
    }
    if chars.get(i) != Some(&' ') {
        return None;
    }
    i += 1;
    if matches!(chars.get(i..i + 3), Some(['[', ' ' | 'x' | 'X', ']']))
        && chars.get(i + 3).is_none_or(|&c| c == ' ')
    {
        i = (i + 4).min(chars.len());
    }
    Some(i)
}

/// Inline passes: code → links → `**`/`__` → `*`/`_`. Matching is sequential, never backtracks and
/// never spans lines; unclosed markers stay unstyled.
fn scan_inline(chars: &[char], from: usize, out: &mut Vec<StyledRange>) {
    let n = chars.len();
    // Positions claimed by code, link URLs or delimiters; later passes skip them
    let mut taken = vec![false; n];
    let push =
        |out: &mut Vec<StyledRange>, taken: &mut [bool], r: Range<usize>, token, take: bool| {
            if take {
                taken[r.clone()].iter_mut().for_each(|t| *t = true);
            }
            out.push(StyledRange {
                start: r.start,
                end: r.end,
                token,
            });
        };

    // Inline code: a run of n backticks pairs with the next run of exactly n backticks
    let mut i = from;
    while i < n {
        if chars[i] != '`' {
            i += 1;
            continue;
        }
        let len = run_len(chars, i, '`');
        let close = (i + len..n).find(|&j| {
            chars[j] == '`' && (j == 0 || chars[j - 1] != '`') && run_len(chars, j, '`') == len
        });
        match close {
            Some(j) => {
                push(out, &mut taken, i..i + len, Token::CodeMark, true);
                push(out, &mut taken, i + len..j, Token::Code, true);
                push(out, &mut taken, j..j + len, Token::CodeMark, true);
                i = j + len;
            }
            None => i += len,
        }
    }

    // Links: [text](url)
    let mut i = from;
    while i < n {
        if chars[i] != '[' || taken[i] {
            i += 1;
            continue;
        }
        let close = (i + 1..n).find(|&j| chars[j] == ']' && !taken[j]);
        let Some(j) = close.filter(|&j| chars.get(j + 1) == Some(&'(')) else {
            i += 1;
            continue;
        };
        let Some(k) = (j + 2..n).find(|&k| chars[k] == ')' && !taken[k]) else {
            i += 1;
            continue;
        };
        push(out, &mut taken, i..i + 1, Token::LinkPunct, true);
        push(out, &mut taken, i + 1..j, Token::LinkText, false);
        push(out, &mut taken, j..j + 2, Token::LinkPunct, true);
        push(out, &mut taken, j + 2..k, Token::LinkUrl, true);
        push(out, &mut taken, k..k + 1, Token::LinkPunct, true);
        i = k + 1;
    }

    // Strong before emphasis, so emphasis can nest inside strong text
    for (width, mark, content) in [
        (2, Token::StrongMark, Token::Strong),
        (1, Token::EmphMark, Token::Emph),
    ] {
        let mut i = from;
        while i + width <= n {
            let Some(d) = delimiter(chars, &taken, i, width).filter(|_| opens(chars, i, width))
            else {
                i += 1;
                continue;
            };
            let close = (i + width + 1..=n - width).find(|&j| {
                delimiter(chars, &taken, j, width) == Some(d) && closes(chars, j, width, d)
            });
            match close {
                Some(j) => {
                    push(out, &mut taken, i..i + width, mark, true);
                    push(out, &mut taken, i + width..j, content, false);
                    push(out, &mut taken, j..j + width, mark, true);
                    i = j + width;
                }
                None => i += 1,
            }
        }
    }
}

fn run_len(chars: &[char], at: usize, c: char) -> usize {
    chars[at..].iter().take_while(|&&x| x == c).count()
}

/// Whether `at` holds exactly `width` identical `*` or `_` (not part of a longer run, and not yet
/// claimed).
fn delimiter(chars: &[char], taken: &[bool], at: usize, width: usize) -> Option<char> {
    let d = *chars.get(at)?;
    let exact = matches!(d, '*' | '_')
        && (at..at + width).all(|k| chars.get(k) == Some(&d) && !taken[k])
        && (at == 0 || chars[at - 1] != d || taken[at - 1])
        && chars
            .get(at + width)
            .is_none_or(|&c| c != d || taken[at + width]);
    exact.then_some(d)
}

/// Opening delimiter: non-whitespace on the right; line start or a non-alphanumeric on the left
/// (so `snake_case` is not mistaken for emphasis).
fn opens(chars: &[char], at: usize, width: usize) -> bool {
    let right_ok = chars.get(at + width).is_some_and(|c| !c.is_whitespace());
    let left_ok = at == 0 || !chars[at - 1].is_alphanumeric();
    right_ok && left_ok
}

/// Closing delimiter: non-whitespace on the left; for `_`, the right side must also be line end or
/// a non-alphanumeric.
fn closes(chars: &[char], at: usize, width: usize, d: char) -> bool {
    let left_ok = at > 0 && !chars[at - 1].is_whitespace();
    let right_ok = d == '*' || chars.get(at + width).is_none_or(|c| !c.is_alphanumeric());
    left_ok && right_ok
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    state_in: BlockState,
    ranges: Vec<StyledRange>,
    state_out: BlockState,
}

/// Per-line cache of scan results. It only keeps a contiguous valid prefix starting at line 0 and
/// extends it forward when a later line is requested.
#[derive(Debug, Default)]
pub struct HighlightCache {
    entries: Vec<Entry>,
}

impl HighlightCache {
    pub fn ranges(&mut self, rope: &Rope, line: usize) -> &[StyledRange] {
        while self.entries.len() <= line {
            let l = self.entries.len();
            let state_in = self
                .entries
                .last()
                .map_or(BlockState::Normal, |e| e.state_out);
            self.entries.push(scan(rope, l, state_in));
        }
        &self.entries[line].ranges
    }

    /// Rescans after an edit. `lines` are the affected logical lines before the edit, `inserted` is
    /// the net change in line count, and `rope` is the body after the edit. Rescanning starts at
    /// the edited line and stops at the first line whose entry state matches the cache, since every
    /// line after it is unchanged. Returns the number of lines rescanned.
    pub fn invalidate(&mut self, rope: &Rope, lines: Range<usize>, inserted: isize) -> usize {
        let first = lines.start;
        if first >= self.entries.len() {
            return 0;
        }
        if lines.end > self.entries.len() {
            self.entries.truncate(first);
            return 0;
        }
        let new_count = (lines.len() as isize + inserted).max(0) as usize;
        let mut state = self.entries[first].state_in;
        let fresh: Vec<Entry> = (first..first + new_count)
            .map(|l| {
                let e = scan(rope, l, state);
                state = e.state_out;
                e
            })
            .collect();
        let mut rescanned = fresh.len();
        self.entries.splice(lines, fresh);
        for l in first + new_count..self.entries.len() {
            if self.entries[l].state_in == state {
                break;
            }
            self.entries[l] = scan(rope, l, state);
            state = self.entries[l].state_out;
            rescanned += 1;
        }
        rescanned
    }

    /// Every cached line in `lines` starts in the previous line's output state and matches a fresh
    /// scan (for tests and debug assertions).
    pub fn is_consistent_in(&self, rope: &Rope, lines: Range<usize>) -> bool {
        self.entries.len() <= rope.len_lines()
            && lines.filter(|&l| l < self.entries.len()).all(|l| {
                let e = &self.entries[l];
                let prev = if l == 0 {
                    BlockState::Normal
                } else {
                    self.entries[l - 1].state_out
                };
                e.state_in == prev && *e == scan(rope, l, e.state_in)
            })
    }

    pub fn is_consistent(&self, rope: &Rope) -> bool {
        self.is_consistent_in(rope, 0..self.entries.len())
    }
}

fn scan(rope: &Rope, line: usize, state_in: BlockState) -> Entry {
    let (ranges, state_out) = scan_line(&line_text(rope, line), state_in);
    Entry {
        state_in,
        ranges,
        state_out,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code::LangId;

    const FENCED: BlockState = BlockState::Fenced {
        fence: '`',
        len: 3,
        lang: None,
        code: CodeState::Normal,
    };

    fn fenced(fence: char, len: u8) -> BlockState {
        BlockState::Fenced {
            fence,
            len,
            lang: None,
            code: CodeState::Normal,
        }
    }

    /// Renders ranges in a snapshot-friendly form: one `Token  "text"` line per range.
    fn show(line: &str) -> String {
        let chars: Vec<char> = line.chars().collect();
        let (ranges, _) = scan_line(line, BlockState::Normal);
        let mut s = format!("{line}\n");
        for r in ranges {
            let text: String = chars[r.start..r.end].iter().collect();
            s += &format!("  {:?} {text:?}\n", r.token);
        }
        s
    }

    #[test]
    fn elements_snapshot() {
        let lines = [
            "# 一级标题",
            "### 三级 *强调* 标题",
            "*强调* 与 _下划线强调_",
            "**加粗** 与 __加粗__",
            "行内 `code` 和 ``含 ` 的代码``",
            "> 引用的一段话",
            "> > 嵌套引用",
            "- 无序列表",
            "* 星号列表",
            "12. 有序列表",
            "- [ ] 待办",
            "- [x] 已完成",
            "[链接文字](https://example.com/a_b_c)",
            "---",
            "* * *",
            "**加粗里有 *强调* 嵌套**",
            "*强调里有 **加粗** 嵌套*",
        ];
        insta::assert_snapshot!(lines.map(show).concat());
    }

    #[test]
    fn unclosed_and_false_positives_snapshot() {
        let lines = [
            "**未闭合的加粗",
            "*未闭合的强调",
            "`未闭合的代码",
            "[未闭合链接](http://x",
            "snake_case_name 不是强调",
            "2 * 3 * 4 不是强调",
            "** 两侧有空格 ** 不是加粗",
            "`*代码里*不是强调`",
            "#不是标题",
            "-不是列表",
        ];
        insta::assert_snapshot!(lines.map(show).concat());
    }

    #[test]
    fn fence_state_transitions() {
        let tilde = fenced('~', 4);
        let rust = BlockState::Fenced {
            fence: '`',
            len: 3,
            lang: code::aliases::from_info("rust"),
            code: CodeState::Normal,
        };
        let cases = [
            ("```rust", BlockState::Normal, Token::FenceLine, rust),
            ("```", BlockState::Normal, Token::FenceLine, FENCED),
            ("   ```", BlockState::Normal, Token::FenceLine, FENCED),
            (
                "    ```",
                BlockState::Normal,
                Token::Code,
                BlockState::Normal,
            ), // indented 4: not a fence
            (
                "``` a`b",
                BlockState::Normal,
                Token::Code,
                BlockState::Normal,
            ), // info contains a backtick: not a fence
            ("~~~~", BlockState::Normal, Token::FenceLine, tilde),
            ("code **not bold**", FENCED, Token::FencedCode, FENCED),
            ("```", FENCED, Token::FenceLine, BlockState::Normal),
            ("````  ", FENCED, Token::FenceLine, BlockState::Normal),
            ("``", FENCED, Token::FencedCode, FENCED),
            ("```", tilde, Token::FencedCode, tilde),
            ("~~~", tilde, Token::FencedCode, tilde), // shorter than the opening fence
            ("~~~~~", tilde, Token::FenceLine, BlockState::Normal),
            ("``` info", FENCED, Token::FencedCode, FENCED), // no info after a closing fence
        ];
        for (line, state_in, first_token, state_out) in cases {
            let (ranges, out) = scan_line(line, state_in);
            assert_eq!(out, state_out, "{line:?}");
            if first_token == Token::Code {
                assert!(
                    ranges.iter().all(|r| r.token != Token::FenceLine),
                    "{line:?}"
                );
            } else {
                assert_eq!(
                    ranges.first().map(|r| r.token),
                    Some(first_token),
                    "{line:?}"
                );
            }
        }
    }

    fn cached(rope: &Rope) -> HighlightCache {
        let mut hl = HighlightCache::default();
        hl.ranges(rope, rope.len_lines() - 1);
        hl
    }

    #[test]
    fn incremental_rescan_stops_when_state_matches() {
        let text: String = ["a", "```", "code", "```", "b", "c", "d"]
            .map(|l| format!("{l}\n"))
            .concat();
        let mut rope = Rope::from_str(&text);
        let mut hl = cached(&rope);
        // A plain inline edit rescans only that line
        rope.insert(rope.line_to_char(4), "*x* ");
        assert_eq!(hl.invalidate(&rope, 4..5, 0), 1);
        assert!(hl.is_consistent(&rope));
        // A fence inserted before line 0 flips every later block state; the rescan runs to the end
        rope.insert(0, "```\n");
        let n = hl.invalidate(&rope, 0..1, 1);
        assert_eq!(n, rope.len_lines());
        assert!(hl.is_consistent(&rope));
        assert_eq!(hl.ranges(&rope, 5)[0].token, Token::FencedCode);
        // Typing a language into a fence line's info string changes the block's state, but the
        // closing fence resets it: the rescan stops after that (lines 0-2), not at the document end
        rope.insert(3, "rust");
        assert_eq!(hl.invalidate(&rope, 0..1, 0), 3);
        assert!(hl.is_consistent(&rope));
        // Delete a closing fence line (two lines merge into one)
        let at = rope.line_to_char(4);
        rope.remove(at - 1..at + 3);
        hl.invalidate(&rope, 3..5, -1);
        assert!(hl.is_consistent(&rope));
    }

    /// The code lexer's state rides in the block state, which the cache compares to stop
    /// rescanning: an edit above a code block, or inside it, must leave the block's coloring
    /// exactly as a fresh scan would, and an edit inside a block rescans no further than its end
    #[test]
    fn code_state_survives_incremental_rescans() {
        let text = [
            "para",
            "```rust",
            "let s = \"one",
            "two\"; /* open",
            "still comment */ fn x() {}",
            "```",
            "after",
            "```py",
            "x = 1",
            "```",
        ]
        .map(|l| format!("{l}\n"))
        .concat();
        let mut rope = Rope::from_str(&text);
        let mut hl = cached(&rope);
        let syntax = |hl: &mut HighlightCache, rope: &Rope, line: usize| -> Vec<Token> {
            hl.ranges(rope, line)
                .iter()
                .map(|r| r.token)
                .filter(|t| matches!(t, Token::Syntax(_)))
                .collect()
        };
        let before: Vec<_> = (0..rope.len_lines())
            .map(|l| syntax(&mut hl, &rope, l))
            .collect();
        assert_eq!(
            syntax(&mut hl, &rope, 4),
            [
                Token::Syntax(code::Token::Comment),
                Token::Syntax(code::Token::Keyword)
            ]
        );
        // Edit the line above the block: only that line is rescanned, the block keeps its colors
        rope.insert(0, "more ");
        assert_eq!(hl.invalidate(&rope, 0..1, 0), 1);
        assert!(hl.is_consistent(&rope));
        for (l, want) in before.iter().enumerate() {
            assert_eq!(&syntax(&mut hl, &rope, l), want, "line {l}");
        }
        // Close the string early inside the block: the lines after it change state, and the
        // rescan stops at the closing fence (lines 2-5), not at the next block
        let at = rope.line_to_char(2) + "let s = \"one".chars().count();
        rope.insert(at, "\"");
        assert_eq!(hl.invalidate(&rope, 2..3, 0), 4);
        assert!(hl.is_consistent(&rope));
        assert_eq!(syntax(&mut hl, &rope, 8), before[8]);
        // Undo it: back to the original coloring
        rope.remove(at..at + 1);
        hl.invalidate(&rope, 2..3, 0);
        assert!(hl.is_consistent(&rope));
        for (l, want) in before.iter().enumerate() {
            assert_eq!(&syntax(&mut hl, &rope, l), want, "line {l}");
        }
    }

    /// Every line of `text` from the top, as (text, code token) for the Syntax ranges
    fn syntax_of(text: &str) -> Vec<Vec<(String, code::Token)>> {
        let mut state = BlockState::Normal;
        text.lines()
            .map(|line| {
                let (ranges, next) = scan_line(line, state);
                state = next;
                let chars: Vec<char> = line.chars().collect();
                ranges
                    .iter()
                    .filter_map(|r| match r.token {
                        Token::Syntax(t) => Some((chars[r.start..r.end].iter().collect(), t)),
                        _ => None,
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn fence_info_string_picks_the_language() {
        let lang = |line: &str| match scan_line(line, BlockState::Normal).1 {
            BlockState::Fenced { lang, .. } => lang,
            BlockState::Normal => panic!("{line:?} is not a fence"),
        };
        let rust = code::aliases::from_info("rust");
        assert!(rust.is_some());
        for line in [
            "```rust",
            "```Rust,no_run",
            "  ``` rs ignore",
            "~~~rust title=\"x\"",
        ] {
            assert_eq!(lang(line), rust, "{line:?}");
        }
        assert_eq!(lang("```{.python}"), code::aliases::from_info("py"));
        assert_eq!(lang("```text"), Some(LangId::PLAIN));
        for line in ["```", "```haskell", "~~~ {r}"] {
            assert_eq!(lang(line), None, "{line:?}");
        }
    }

    /// Without a known language, a block is one FencedCode range per line, as before the lexer
    #[test]
    fn unknown_or_plain_blocks_stay_whole_line() {
        for info in ["", "haskell", "text", "console"] {
            let text = format!("```{info}\nlet s = \"x\"; // c\n```");
            let mut state = BlockState::Normal;
            for line in text.lines() {
                let (ranges, next) = scan_line(line, state);
                assert_eq!(ranges.len(), 1, "{info:?} {line:?}");
                state = next;
            }
            assert_eq!(state, BlockState::Normal);
        }
    }

    /// An unclosed comment or string ends with the block: the closing fence resets the lexer, the
    /// text after it is Markdown again, and the next block starts fresh
    #[test]
    fn closing_fence_resets_the_code_state() {
        let text = "```c\n/* never closed\nint x;\n```\n*emphasis* after\n```js\nlet y = 1;\n```";
        let syntax = syntax_of(text);
        assert_eq!(syntax[2], [("int x;".to_string(), code::Token::Comment)]);
        let (ranges, _) = scan_line("*emphasis* after", BlockState::Normal);
        assert!(ranges.iter().any(|r| r.token == Token::Emph));
        assert_eq!(
            syntax[6],
            [
                ("let".to_string(), code::Token::Keyword),
                ("1".to_string(), code::Token::Number)
            ]
        );
    }

    /// A longer fence holds a shorter one as code: a Markdown block that shows a fence
    #[test]
    fn shorter_fence_inside_a_block_is_code() {
        let text = "````markdown\n# Title\n```rust\nfn main() {}\n```\n````\nafter";
        let syntax = syntax_of(text);
        assert_eq!(syntax[1], [("# Title".to_string(), code::Token::Keyword)]);
        // Markdown's lexer leaves the inner fence and code plain, rather than lexing it as Rust
        assert!(syntax[3].is_empty());
        let mut state = BlockState::Normal;
        for line in text.lines().take(6) {
            state = scan_line(line, state).1;
        }
        assert_eq!(state, BlockState::Normal);
    }

    #[test]
    fn invalidate_beyond_cached_prefix_is_lazy() {
        let mut rope = Rope::from_str("a\nb\nc\nd\n");
        let mut hl = HighlightCache::default();
        hl.ranges(&rope, 1);
        rope.insert(rope.line_to_char(3), "```\n");
        assert_eq!(hl.invalidate(&rope, 3..4, 1), 0);
        assert!(hl.is_consistent(&rope));
        assert_eq!(hl.ranges(&rope, 4)[0].token, Token::FencedCode);
    }
}
