//! Lightweight line-by-line Markdown scanner. It only produces style ranges and never changes the
//! text. Each line depends only on the block state it starts in (inside a fenced code block or
//! not), so results can be cached per line and rescanned incrementally.

use std::ops::Range;

use ropey::Rope;

use crate::text::wrap::line_text;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BlockState {
    #[default]
    Normal,
    Fenced {
        fence: char,
        len: u8,
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

    if let BlockState::Fenced { fence, len } = state_in {
        let closes = fence_run(fence).is_some_and(|run| {
            run >= len as usize && chars[indent + run..].iter().all(|c| c.is_whitespace())
        });
        return if closes {
            (whole(Token::FenceLine), BlockState::Normal)
        } else {
            (whole(Token::FencedCode), state_in)
        };
    }
    for fence in ['`', '~'] {
        if let Some(run) = fence_run(fence) {
            // A backtick fence's info string must not contain backticks
            if fence == '~' || !chars[indent + run..].contains(&'`') {
                let len = run.min(u8::MAX as usize) as u8;
                return (whole(Token::FenceLine), BlockState::Fenced { fence, len });
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

    const FENCED: BlockState = BlockState::Fenced { fence: '`', len: 3 };

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
        let tilde = BlockState::Fenced { fence: '~', len: 4 };
        let cases = [
            ("```rust", BlockState::Normal, Token::FenceLine, FENCED),
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
        // Typing in a fence line's info string keeps the block state; the rescan stops after 1 line
        rope.insert(3, "rust");
        assert_eq!(hl.invalidate(&rope, 0..1, 0), 1);
        assert!(hl.is_consistent(&rope));
        // Delete a closing fence line (two lines merge into one)
        let at = rope.line_to_char(4);
        rope.remove(at - 1..at + 3);
        hl.invalidate(&rope, 3..5, -1);
        assert!(hl.is_consistent(&rope));
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
