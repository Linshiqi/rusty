//! Text objects: `iw`, `a"`, `i(`, `ip`.
//!
//! In phase one rather than deferred because `ciw` and `ci"` are what a Vim
//! user reaches for before they reach for anything else. A modal editor
//! without them has the keys but not the leverage, and "直接上手" fails on
//! the first rename.
//!
//! `i` is the inside; `a` takes the delimiters too — and for a word, the
//! trailing whitespace, which is why `daw` leaves a sentence spaced correctly
//! and `diw` does not.

use super::motion::Span;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Object {
    /// `i` (inside) versus `a` (around).
    pub inside: bool,
    pub kind: Kind,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Word,
    BigWord,
    Paragraph,
    /// A delimiter pair, given by its opening character.
    Pair(char),
    /// A quote, where opening and closing are the same character — which is
    /// why it cannot share the pair scan.
    Quote(char),
}

/// The object a key names, or `None` for one this does not implement — which
/// the caller reports rather than swallowing.
pub fn of(inside: bool, key: char) -> Option<Object> {
    let kind = match key {
        'w' => Kind::Word,
        'W' => Kind::BigWord,
        'p' => Kind::Paragraph,
        '(' | ')' | 'b' => Kind::Pair('('),
        '[' | ']' => Kind::Pair('['),
        '{' | '}' | 'B' => Kind::Pair('{'),
        '<' | '>' => Kind::Pair('<'),
        '"' => Kind::Quote('"'),
        '\'' => Kind::Quote('\''),
        '`' => Kind::Quote('`'),
        _ => return None,
    };
    Some(Object { inside, kind })
}

pub fn apply(object: Object, text: &str, cursor: usize) -> Option<Span> {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return None;
    }
    let cursor = cursor.min(chars.len().saturating_sub(1));

    let (start, end, linewise) = match object.kind {
        Kind::Word => word(&chars, cursor, object.inside, false)?,
        Kind::BigWord => word(&chars, cursor, object.inside, true)?,
        Kind::Paragraph => paragraph(&chars, cursor, object.inside)?,
        Kind::Pair(open) => pair(&chars, cursor, open, object.inside)?,
        Kind::Quote(quote) => quoted(&chars, cursor, quote, object.inside)?,
    };
    Some(Span {
        start,
        end,
        cursor: start,
        linewise,
    })
}

fn is_word(c: char, big: bool) -> bool {
    !c.is_whitespace() && (big || c.is_alphanumeric() || c == '_')
}

fn word(chars: &[char], cursor: usize, inside: bool, big: bool) -> Option<(usize, usize, bool)> {
    let on_word = is_word(chars[cursor], big);
    let mut start = cursor;
    let mut end = cursor;

    if on_word {
        while start > 0 && is_word(chars[start - 1], big) {
            start -= 1;
        }
        while end + 1 < chars.len() && is_word(chars[end + 1], big) {
            end += 1;
        }
    } else if chars[cursor].is_whitespace() {
        // On blanks, the object is the run of blanks.
        while start > 0 && chars[start - 1].is_whitespace() && chars[start - 1] != '\n' {
            start -= 1;
        }
        while end + 1 < chars.len() && chars[end + 1].is_whitespace() && chars[end + 1] != '\n' {
            end += 1;
        }
    } else {
        // Punctuation is its own object, the way `w` treats it.
        let punct = |c: char| !c.is_whitespace() && !is_word(c, false);
        while start > 0 && punct(chars[start - 1]) {
            start -= 1;
        }
        while end + 1 < chars.len() && punct(chars[end + 1]) {
            end += 1;
        }
    }

    let mut end = end + 1;
    if !inside {
        // `aw` swallows the whitespace after the word, or before it when
        // there is none after — so deleting the last word of a line does not
        // leave a dangling space.
        let before = end;
        while end < chars.len() && chars[end] != '\n' && chars[end].is_whitespace() {
            end += 1;
        }
        if end == before {
            while start > 0 && chars[start - 1] != '\n' && chars[start - 1].is_whitespace() {
                start -= 1;
            }
        }
    }
    Some((start, end, false))
}

fn paragraph(chars: &[char], cursor: usize, inside: bool) -> Option<(usize, usize, bool)> {
    let blank = |at: usize| {
        let start = line_start(chars, at);
        let end = line_end(chars, at);
        chars[start..end].iter().all(|c| c.is_whitespace())
    };
    let mut start = line_start(chars, cursor);
    let mut end = line_end(chars, cursor);
    while start > 0 && !blank(start - 1) {
        start = line_start(chars, start - 1);
    }
    while end + 1 < chars.len() && !blank(end + 1) {
        end = line_end(chars, end + 1);
    }
    if !inside {
        while end + 1 < chars.len() && blank(end + 1) {
            end = line_end(chars, end + 1);
        }
    }
    Some((start, (end + 1).min(chars.len()), true))
}

fn line_start(chars: &[char], at: usize) -> usize {
    let mut at = at.min(chars.len());
    while at > 0 && chars[at - 1] != '\n' {
        at -= 1;
    }
    at
}

fn line_end(chars: &[char], at: usize) -> usize {
    let mut at = at.min(chars.len());
    while at < chars.len() && chars[at] != '\n' {
        at += 1;
    }
    at
}

/// The pair enclosing the cursor, counting depth so `di(` inside nested
/// parentheses takes the inner one.
fn pair(chars: &[char], cursor: usize, open: char, inside: bool) -> Option<(usize, usize, bool)> {
    let close = match open {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        '<' => '>',
        _ => return None,
    };

    let start = if chars[cursor] == open {
        cursor
    } else {
        let mut depth = 0i32;
        let mut at = cursor;
        loop {
            if chars[at] == close && at != cursor {
                depth += 1;
            } else if chars[at] == open {
                if depth == 0 {
                    break at;
                }
                depth -= 1;
            }
            at = at.checked_sub(1)?;
        }
    };

    let mut depth = 0i32;
    let mut end = None;
    for (index, c) in chars.iter().enumerate().skip(start) {
        if *c == open {
            depth += 1;
        } else if *c == close {
            depth -= 1;
            if depth == 0 {
                end = Some(index);
                break;
            }
        }
    }
    let end = end?;
    if inside {
        // An empty pair has no inside; taking `start + 1 .. start + 1` is an
        // empty span, which deletes nothing rather than eating a delimiter.
        Some((start + 1, end, false))
    } else {
        Some((start, end + 1, false))
    }
}

/// Quotes cannot be scanned by depth — the same character opens and closes —
/// so they are counted from the start of the line, which is what Vim does and
/// what makes `ci"` land on the string the cursor is inside.
fn quoted(
    chars: &[char],
    cursor: usize,
    quote: char,
    inside: bool,
) -> Option<(usize, usize, bool)> {
    let from = line_start(chars, cursor);
    let to = line_end(chars, cursor);

    let mut open: Option<usize> = None;
    let mut at = from;
    while at < to {
        if chars[at] == quote && (at == from || chars[at - 1] != '\\') {
            match open {
                None => open = Some(at),
                Some(start) => {
                    if cursor <= at {
                        return Some(if inside {
                            (start + 1, at, false)
                        } else {
                            (start, at + 1, false)
                        });
                    }
                    open = None;
                }
            }
        }
        at += 1;
    }
    None
}
