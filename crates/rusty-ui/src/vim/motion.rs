//! Where a motion lands, and what an operator applied to it covers.
//!
//! Two different answers, which is the trap Vim's own documentation spends a
//! page on: `e` puts the cursor *on* the last character of the word, and `de`
//! deletes *through* it. A motion that returned one number would get one of
//! the two wrong, so [`Span`] carries both.
//!
//! Indices are Unicode scalars. Each call materialises the buffer as a
//! `Vec<char>` — O(n) per keystroke, which on a 300 KB file is under a
//! millisecond and buys arithmetic that is obviously right rather than
//! obviously fast.

/// What a motion covers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Span {
    /// Operator range, half-open: `d` removes `start..end`.
    pub start: usize,
    pub end: usize,
    /// Where a bare motion leaves the cursor. Not always `end`: inclusive
    /// motions put the cursor on the last character they cover.
    pub cursor: usize,
    /// Whole lines, which paste and `p` treat differently.
    pub linewise: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    LineStart,
    FirstWord,
    LineEnd,
    WordForward,
    BigWordForward,
    WordBack,
    BigWordBack,
    WordEnd,
    FirstLine,
    LastLine,
    ParagraphForward,
    ParagraphBack,
    MatchPair,
    /// `f`, `F`, `t`, `T` and the character they were given.
    Find {
        key: char,
        target: char,
    },
    /// `;` and `,`.
    RepeatFind {
        reverse: bool,
    },
}

/// Apply a motion. `None` means it had nowhere to go — Vim beeps, this does
/// nothing, and either way the buffer is untouched.
pub fn apply(
    motion: Motion,
    text: &str,
    cursor: usize,
    count: usize,
    last_find: &Option<(char, char)>,
) -> Option<Span> {
    let chars: Vec<char> = text.chars().collect();
    let count = count.max(1);

    let (target, inclusive, linewise) = match motion {
        Motion::Left => (left(text, cursor, count), false, false),
        Motion::Right => (right(&chars, cursor, count), false, false),
        Motion::Up => (vertical(&chars, cursor, count, false)?, false, true),
        Motion::Down => (vertical(&chars, cursor, count, true)?, false, true),
        Motion::LineStart => (line_start(text, cursor), false, false),
        Motion::FirstWord => (line_first_word(text, cursor), false, false),
        Motion::LineEnd => {
            let mut at = cursor;
            for _ in 1..count {
                let next = (line_end_at(&chars, at) + 1).min(chars.len());
                if next >= chars.len() {
                    break;
                }
                at = next;
            }
            // The last character, not the newline: normal mode's cursor sits
            // on a character. Inclusive, so `d$` still takes that character.
            let end = line_end_at(&chars, at);
            (end.saturating_sub(1).max(line_start_at(&chars, at)), true, false)
        }
        Motion::WordForward => (word_forward(&chars, cursor, count, false), false, false),
        Motion::BigWordForward => (word_forward(&chars, cursor, count, true), false, false),
        Motion::WordBack => (word_back(&chars, cursor, count, false), false, false),
        Motion::BigWordBack => (word_back(&chars, cursor, count, true), false, false),
        Motion::WordEnd => (word_end(&chars, cursor, count), true, false),
        Motion::FirstLine => (first_word_at(&chars, 0), false, true),
        Motion::LastLine => {
            // `G` with a count is "go to that line"; bare it is the last.
            let at = if count > 1 {
                line_number_start(&chars, count - 1)
            } else {
                line_number_start(&chars, line_count(&chars).saturating_sub(1))
            };
            (first_word_at(&chars, at), false, true)
        }
        Motion::ParagraphForward => (paragraph(&chars, cursor, count, true), false, false),
        Motion::ParagraphBack => (paragraph(&chars, cursor, count, false), false, false),
        Motion::MatchPair => (match_pair(&chars, cursor)?, true, false),
        Motion::Find { key, target } => find(&chars, cursor, count, key, target, false)?,
        Motion::RepeatFind { reverse } => {
            let (key, target) = (*last_find)?;
            let key = if reverse { flip_find(key) } else { key };
            find(&chars, cursor, count, key, target, true)?
        }
    };

    Some(span(text, cursor, target, inclusive, linewise))
}

/// Turn "the cursor was here, the motion points there" into both answers.
fn span(text: &str, cursor: usize, target: usize, inclusive: bool, linewise: bool) -> Span {
    if linewise {
        let (from, to) = (cursor.min(target), cursor.max(target));
        return Span {
            start: line_start(text, from),
            end: (line_end(text, to) + 1).min(text.chars().count()),
            cursor: target,
            linewise: true,
        };
    }
    let (start, end) = if target >= cursor {
        (cursor, if inclusive { target + 1 } else { target })
    } else {
        (target, cursor)
    };
    Span {
        start,
        end: end.min(text.chars().count()),
        cursor: target,
        linewise: false,
    }
}

/// The whole lines `dd` and its friends cover.
pub fn whole_lines(text: &str, cursor: usize, count: usize) -> Span {
    let chars: Vec<char> = text.chars().collect();
    let start = line_start(text, cursor);
    let mut end = line_end_at(&chars, cursor);
    // One line's end is already found; each extra count steps over the
    // newline and finds the next. Adding one *inside* the loop as well
    // counted every newline twice, and `dd` took a character off the line
    // below.
    for _ in 1..count.max(1) {
        if end + 1 >= chars.len() {
            break;
        }
        end = line_end_at(&chars, end + 1);
    }
    Span {
        start,
        end: (end + 1).min(chars.len()),
        cursor: start,
        linewise: true,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Lines
// ─────────────────────────────────────────────────────────────────────────────

pub fn line_start(text: &str, cursor: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let mut at = cursor.min(chars.len());
    while at > 0 && chars[at - 1] != '\n' {
        at -= 1;
    }
    at
}

pub fn line_end(text: &str, cursor: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    line_end_at(&chars, cursor)
}

fn line_end_at(chars: &[char], cursor: usize) -> usize {
    let mut at = cursor.min(chars.len());
    while at < chars.len() && chars[at] != '\n' {
        at += 1;
    }
    at
}

/// The first non-blank on this line — `^`, and where `G` and `gg` land.
pub fn line_first_word(text: &str, cursor: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    first_word_at(&chars, line_start(text, cursor))
}

fn first_word_at(chars: &[char], start: usize) -> usize {
    let mut at = start;
    while at < chars.len() && chars[at] != '\n' && chars[at].is_whitespace() {
        at += 1;
    }
    at
}

/// The last column the *normal-mode* cursor may occupy: one before the
/// newline, because the cursor sits on a character rather than after it.
pub fn last_column(text: &str, cursor: usize) -> usize {
    let end = line_end(text, cursor);
    let start = line_start(text, cursor);
    end.saturating_sub(1).max(start)
}

fn line_count(chars: &[char]) -> usize {
    chars.iter().filter(|c| **c == '\n').count() + 1
}

fn line_number_start(chars: &[char], line: usize) -> usize {
    let mut seen = 0;
    for (index, c) in chars.iter().enumerate() {
        if seen == line {
            return index;
        }
        if *c == '\n' {
            seen += 1;
        }
    }
    chars.len().saturating_sub(usize::from(!chars.is_empty()))
}

pub fn left(text: &str, cursor: usize, count: usize) -> usize {
    let start = line_start(text, cursor);
    cursor.saturating_sub(count).max(start)
}

fn right(chars: &[char], cursor: usize, count: usize) -> usize {
    let end = line_end_at(chars, cursor);
    (cursor + count).min(end.saturating_sub(1).max(line_start_at(chars, cursor)))
}

fn line_start_at(chars: &[char], cursor: usize) -> usize {
    let mut at = cursor.min(chars.len());
    while at > 0 && chars[at - 1] != '\n' {
        at -= 1;
    }
    at
}

/// Where `a` puts the caret: after the character, which is one past what
/// normal mode allows.
pub fn right_for_append(text: &str, cursor: usize) -> usize {
    (cursor + 1).min(line_end(text, cursor))
}

/// `j` and `k`, keeping the column. `None` at the buffer's ends.
fn vertical(chars: &[char], cursor: usize, count: usize, down: bool) -> Option<usize> {
    let column = cursor - line_start_at(chars, cursor);
    let mut at = cursor;
    for _ in 0..count {
        if down {
            let end = line_end_at(chars, at);
            if end >= chars.len() {
                return (at != cursor).then_some(at);
            }
            at = end + 1;
        } else {
            let start = line_start_at(chars, at);
            if start == 0 {
                return (at != cursor).then_some(at);
            }
            at = line_start_at(chars, start - 1);
        }
    }
    let start = line_start_at(chars, at);
    let end = line_end_at(chars, at);
    Some((start + column).min(end))
}

/// `J`: pull the next `count - 1` lines onto this one, with one space where
/// each newline was — Vim's own rule, not a bare concatenation.
pub fn join(text: &str, cursor: usize, count: usize) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<char> = chars.clone();
    let mut at = line_end_at(&chars, cursor);
    let mut joined = 0;
    for _ in 1..count.max(2) {
        if at >= out.len() {
            break;
        }
        // Drop the newline and the next line's indent, leave one space.
        let mut end = at + 1;
        while end < out.len() && out[end] != '\n' && out[end].is_whitespace() {
            end += 1;
        }
        out.splice(at..end, [' ']);
        joined += 1;
        at = line_end_at(&out, at);
    }
    (joined > 0).then(|| out.into_iter().collect())
}

// ─────────────────────────────────────────────────────────────────────────────
// Words
// ─────────────────────────────────────────────────────────────────────────────

/// Vim's three classes: word characters, punctuation, and blanks. `w` stops
/// at a change between the first two; `W` only at blanks.
#[derive(PartialEq, Eq, Clone, Copy)]
enum Class {
    Blank,
    Word,
    Punct,
}

fn class(c: char, big: bool) -> Class {
    if c.is_whitespace() {
        Class::Blank
    } else if big || c.is_alphanumeric() || c == '_' {
        Class::Word
    } else {
        Class::Punct
    }
}

fn word_forward(chars: &[char], cursor: usize, count: usize, big: bool) -> usize {
    let mut at = cursor;
    for _ in 0..count {
        if at >= chars.len() {
            break;
        }
        let start = class(chars[at], big);
        if start != Class::Blank {
            while at < chars.len() && class(chars[at], big) == start {
                at += 1;
            }
        }
        while at < chars.len() && class(chars[at], big) == Class::Blank {
            at += 1;
        }
    }
    at.min(chars.len())
}

fn word_back(chars: &[char], cursor: usize, count: usize, big: bool) -> usize {
    let mut at = cursor;
    for _ in 0..count {
        if at == 0 {
            break;
        }
        at -= 1;
        while at > 0 && class(chars[at], big) == Class::Blank {
            at -= 1;
        }
        if class(chars[at], big) == Class::Blank {
            break;
        }
        let here = class(chars[at], big);
        while at > 0 && class(chars[at - 1], big) == here {
            at -= 1;
        }
    }
    at
}

fn word_end(chars: &[char], cursor: usize, count: usize) -> usize {
    let mut at = cursor;
    for _ in 0..count {
        if at + 1 >= chars.len() {
            break;
        }
        at += 1;
        while at < chars.len() && class(chars[at], false) == Class::Blank {
            at += 1;
        }
        if at >= chars.len() {
            break;
        }
        let here = class(chars[at], false);
        while at + 1 < chars.len() && class(chars[at + 1], false) == here {
            at += 1;
        }
    }
    at.min(chars.len().saturating_sub(1))
}

// ─────────────────────────────────────────────────────────────────────────────
// Paragraphs, pairs, find
// ─────────────────────────────────────────────────────────────────────────────

/// `{` and `}` — the next blank line, which is what a paragraph is here.
fn paragraph(chars: &[char], cursor: usize, count: usize, forward: bool) -> usize {
    let mut at = cursor;
    for _ in 0..count.max(1) {
        loop {
            if forward {
                if at >= chars.len() {
                    break;
                }
                at = (line_end_at(chars, at) + 1).min(chars.len());
                if at >= chars.len() || is_blank_line(chars, at) {
                    break;
                }
            } else {
                if at == 0 {
                    break;
                }
                let start = line_start_at(chars, at);
                if start == 0 {
                    at = 0;
                    break;
                }
                at = line_start_at(chars, start - 1);
                if is_blank_line(chars, at) {
                    break;
                }
            }
        }
    }
    at
}

fn is_blank_line(chars: &[char], at: usize) -> bool {
    let start = line_start_at(chars, at);
    let end = line_end_at(chars, at);
    chars[start..end].iter().all(|c| c.is_whitespace())
}

/// `%` — the bracket matching the one under the cursor, or the next one on
/// the line if the cursor is not on one, which is what Vim does.
fn match_pair(chars: &[char], cursor: usize) -> Option<usize> {
    const PAIRS: [(char, char); 3] = [('(', ')'), ('[', ']'), ('{', '}')];

    let end = line_end_at(chars, cursor);
    let at = (cursor..end).find(|index| {
        PAIRS
            .iter()
            .any(|(open, close)| chars[*index] == *open || chars[*index] == *close)
    })?;
    let here = chars[at];
    let (open, close, forward) = PAIRS
        .iter()
        .find_map(|(open, close)| {
            if here == *open {
                Some((*open, *close, true))
            } else if here == *close {
                Some((*open, *close, false))
            } else {
                None
            }
        })?;

    let mut depth = 0i32;
    if forward {
        for (index, c) in chars.iter().enumerate().skip(at) {
            if *c == open {
                depth += 1;
            } else if *c == close {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
        }
    } else {
        for index in (0..=at).rev() {
            if chars[index] == close {
                depth += 1;
            } else if chars[index] == open {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
        }
    }
    None
}

fn flip_find(key: char) -> char {
    match key {
        'f' => 'F',
        'F' => 'f',
        't' => 'T',
        'T' => 't',
        other => other,
    }
}

/// `f`/`F`/`t`/`T`, within the line — Vim never crosses one for these.
fn find(
    chars: &[char],
    cursor: usize,
    count: usize,
    key: char,
    target: char,
    // `;` and `,` only. A fresh `t-` stops in front of the *first* match, so
    // stepping off here would skip it; a repeat that did not step off would
    // never move again.
    repeat: bool,
) -> Option<(usize, bool, bool)> {
    let forward = matches!(key, 'f' | 't');
    let till = matches!(key, 't' | 'T');
    let start = line_start_at(chars, cursor);
    let end = line_end_at(chars, cursor);

    let mut at = cursor;
    for _ in 0..count.max(1) {
        if forward {
            // `t` repeated has to step off the character it stopped before,
            // or `;` never moves.
            let from = if repeat && till && at + 1 < end && chars[at + 1] == target {
                at + 2
            } else {
                at + 1
            };
            at = (from..end).find(|index| chars[*index] == target)?;
        } else {
            let from = if repeat && till && at > start && chars[at - 1] == target {
                at.checked_sub(2)?
            } else {
                at.checked_sub(1)?
            };
            at = (start..=from).rev().find(|index| chars[*index] == target)?;
        }
    }
    let at = if till {
        if forward { at - 1 } else { at + 1 }
    } else {
        at
    };
    // Forward finds are inclusive, backward ones are not — the asymmetry is
    // Vim's, and `dfx` versus `dFx` is where anyone would notice it missing.
    Some((at, forward, false))
}
