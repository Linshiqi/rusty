//! Which bracket pairs with the one at the caret.
//!
//! Read off the painted lines rather than the text, because the painting
//! already knows what a string and a comment are: a `{` inside `"{}"` or
//! after `//` is not a bracket, and a scan over characters would pair it
//! with something. A line typed on and not yet repainted is plain, so its
//! strings count as code for the quarter of a second before its colours
//! come back — the one place this can be wrong, and only briefly.

use rusty_edit::{Line, Token};

use super::pairs::closer;

/// A document position: line, and scalar column.
pub(super) type At = (u32, u32);

/// How far a scan for the other half goes, in lines. Past this a bracket is
/// shown unmatched rather than every keystroke in a vast block reading to
/// its end.
const SCAN_LINES: usize = 20_000;

/// The bracket at `caret` and the one it pairs with — the bracket just after
/// the caret, or failing that the one just before it, as VS Code chooses.
/// `None` when neither is a bracket in code, or its other half is not found.
pub(super) fn bracket_pair(lines: &[Line], caret: At) -> Option<(At, At)> {
    let (line, col) = caret;
    let after = Some((line, col));
    let before = col.checked_sub(1).map(|col| (line, col));
    [after, before].into_iter().flatten().find_map(|at| {
        let ch = code_char(lines.get(at.0 as usize)?, at.1)?;
        let other = if let Some(close) = closer(ch).filter(|_| "([{".contains(ch)) {
            scan(lines, at, ch, close, true)
        } else if let Some(open) = opener(ch) {
            scan(lines, at, open, ch, false)
        } else {
            None
        }?;
        Some((at, other))
    })
}

fn opener(close: char) -> Option<char> {
    match close {
        ')' => Some('('),
        ']' => Some('['),
        '}' => Some('{'),
        _ => None,
    }
}

/// The character at a column of a painted line, when it is code.
fn code_char(line: &Line, col: u32) -> Option<char> {
    let mut at = 0u32;
    for span in &line.spans {
        let count = span.text.chars().count() as u32;
        if col < at + count {
            if !is_code(span.token) {
                return None;
            }
            return span.text.chars().nth((col - at) as usize);
        }
        at += count;
    }
    None
}

fn is_code(token: Token) -> bool {
    !matches!(token, Token::Str | Token::Comment)
}

/// From the bracket at `from`, the one that closes (or opens) it: the first
/// of the same pair where the depth comes back to nothing.
fn scan(lines: &[Line], from: At, open: char, close: char, forward: bool) -> Option<At> {
    let mut depth = 0i64;
    let visit = |line: u32, chars: Vec<(u32, char)>, depth: &mut i64| -> Option<At> {
        for (col, ch) in chars {
            if ch == open {
                *depth += if forward { 1 } else { -1 };
            } else if ch == close {
                *depth += if forward { -1 } else { 1 };
            } else {
                continue;
            }
            if *depth == 0 {
                return Some((line, col));
            }
        }
        None
    };
    let code_chars = |line: &Line| -> Vec<(u32, char)> {
        let mut out = Vec::new();
        let mut col = 0u32;
        for span in &line.spans {
            for ch in span.text.chars() {
                if is_code(span.token) && (ch == open || ch == close) {
                    out.push((col, ch));
                }
                col += 1;
            }
        }
        out
    };
    let (start, start_col) = (from.0 as usize, from.1);
    if forward {
        for (index, line) in lines.iter().enumerate().skip(start).take(SCAN_LINES) {
            let chars = code_chars(line)
                .into_iter()
                .filter(|(col, _)| index > start || *col >= start_col)
                .collect();
            if let Some(found) = visit(index as u32, chars, &mut depth) {
                return Some(found);
            }
        }
    } else {
        for (index, line) in lines
            .iter()
            .enumerate()
            .take(start + 1)
            .rev()
            .take(SCAN_LINES)
        {
            let chars = code_chars(line)
                .into_iter()
                .rev()
                .filter(|(col, _)| index < start || *col <= start_col)
                .collect();
            if let Some(found) = visit(index as u32, chars, &mut depth) {
                return Some(found);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_edit::Span;

    /// Lines as painted: `code` runs, with a `"string"` or a `// comment`
    /// painted as one.
    fn painted(text: &str) -> Vec<Line> {
        text.split('\n')
            .map(|line| {
                let mut spans = Vec::new();
                let mut rest = line;
                while !rest.is_empty() {
                    let (token, end) = if rest.starts_with("//") {
                        (Token::Comment, rest.len())
                    } else if let Some(body) = rest.strip_prefix('"') {
                        (Token::Str, body.find('"').map_or(rest.len(), |at| at + 2))
                    } else {
                        let end = rest
                            .find(['"', '/'])
                            .filter(|&at| at > 0)
                            .unwrap_or(rest.len());
                        (Token::Plain, end)
                    };
                    spans.push(Span {
                        text: rest[..end].to_string(),
                        token,
                    });
                    rest = &rest[end..];
                }
                Line { spans }
            })
            .collect()
    }

    #[test]
    fn the_bracket_after_the_caret_pairs_across_lines() {
        let lines = painted("fn a() {\n    if x {\n        y();\n    }\n}");
        assert_eq!(bracket_pair(&lines, (0, 7)), Some(((0, 7), (4, 0))));
        assert_eq!(bracket_pair(&lines, (4, 0)), Some(((4, 0), (0, 7))));
        assert_eq!(bracket_pair(&lines, (1, 9)), Some(((1, 9), (3, 4))));
    }

    /// With no bracket after the caret, the one before it: the caret just past
    /// a closing parenthesis shows its opener.
    #[test]
    fn the_bracket_before_the_caret_is_the_fallback() {
        let lines = painted("call(a, (b))");
        assert_eq!(bracket_pair(&lines, (0, 12)), Some(((0, 11), (0, 4))));
        assert_eq!(bracket_pair(&lines, (0, 2)), None, "nothing at or before");
    }

    #[test]
    fn brackets_in_strings_and_comments_are_not_brackets() {
        let lines = painted("f(\"(\", x) // )\ng()");
        assert_eq!(bracket_pair(&lines, (0, 1)), Some(((0, 1), (0, 8))));
        assert_eq!(bracket_pair(&lines, (0, 3)), None, "the one in the string");
    }

    #[test]
    fn an_unclosed_bracket_pairs_with_nothing() {
        let lines = painted("fn a() {\n    b(");
        assert_eq!(bracket_pair(&lines, (0, 7)), None);
    }
}
