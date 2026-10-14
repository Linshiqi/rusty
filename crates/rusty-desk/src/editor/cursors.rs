//! Several cursors at once — VS Code's multi-cursor editing — as arithmetic
//! over the text: where each cursor is, what one keystroke does at all of
//! them, and where they are afterwards. The editor's textarea holds one
//! selection, the first cursor's; the others are drawn beside it
//! (`view/panels/files/multi.rs`).
//!
//! Positions are document bytes on character boundaries. The first cursor of
//! a list is the one the textarea shows, and every operation keeps it first:
//! when two cursors run into each other it is the other one that goes.

/// One cursor: where its selection was started and where it is now — the
/// same place for a caret.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub anchor: usize,
    pub head: usize,
}

impl Cursor {
    pub fn caret(at: usize) -> Cursor {
        Cursor {
            anchor: at,
            head: at,
        }
    }

    pub fn start(self) -> usize {
        self.anchor.min(self.head)
    }

    pub fn end(self) -> usize {
        self.anchor.max(self.head)
    }

    pub fn is_caret(self) -> bool {
        self.anchor == self.head
    }
}

/// Which way a key moves every cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    /// The line's first non-blank, or its start from there.
    Home,
    End,
    WordLeft,
    WordRight,
}

/// Replace, at every cursor, `from..to` with what `change` answers for it,
/// and answer the new text and the cursors after it — each a caret after
/// what it wrote, in the order given. A replacement that runs into one before
/// it in the text is dropped with its cursor, unless it is the first cursor's,
/// which then drops the other.
pub fn edit(
    text: &str,
    cursors: &[Cursor],
    change: impl Fn(&str, Cursor) -> (usize, usize, String),
) -> (String, Vec<Cursor>) {
    let mut changes: Vec<(usize, usize, usize, String)> = cursors
        .iter()
        .enumerate()
        .map(|(index, cursor)| {
            let (from, to, with) = change(text, *cursor);
            let from = boundary(text, from.min(text.len()));
            let to = boundary(text, to.clamp(from, text.len()));
            (index, from, to, with)
        })
        .collect();
    changes.sort_by_key(|(index, from, to, _)| (*from, *to, *index));
    let mut kept: Vec<(usize, usize, usize, String)> = Vec::new();
    'changes: for change in changes {
        while let Some(last) = kept.last() {
            let meets = change.1 < last.2 || (change.1 == last.1 && change.2 == last.2);
            if !meets {
                break;
            }
            if change.0 == 0 {
                kept.pop();
            } else {
                continue 'changes;
            }
        }
        kept.push(change);
    }
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    let mut placed: Vec<(usize, usize)> = Vec::new();
    for (index, from, to, with) in &kept {
        out.push_str(&text[at..*from]);
        out.push_str(with);
        placed.push((*index, out.len()));
        at = *to;
    }
    out.push_str(&text[at..]);
    placed.sort_by_key(|(index, _)| *index);
    let after = placed
        .into_iter()
        .map(|(_, at)| Cursor::caret(at))
        .collect();
    (out, after)
}

/// `text` typed at every cursor, over whatever each has selected.
pub fn typed(text: &str, cursors: &[Cursor], typed: &str) -> (String, Vec<Cursor>) {
    edit(text, cursors, |_, cursor| {
        (cursor.start(), cursor.end(), typed.to_string())
    })
}

/// Backspace, or Delete when `forward`, at every cursor: a selection goes
/// whole, a caret takes the character (or with `word`, the word) beside it.
pub fn erased(text: &str, cursors: &[Cursor], forward: bool, word: bool) -> (String, Vec<Cursor>) {
    edit(text, cursors, |text, cursor| {
        if !cursor.is_caret() {
            return (cursor.start(), cursor.end(), String::new());
        }
        let at = cursor.head;
        match (forward, word) {
            (false, false) => (previous(text, at), at, String::new()),
            (true, false) => (at, next(text, at), String::new()),
            (false, true) => (word_left(text, at), at, String::new()),
            (true, true) => (at, word_right(text, at), String::new()),
        }
    })
}

/// A new line at every cursor, indented as far as the line it breaks.
pub fn broken(text: &str, cursors: &[Cursor]) -> (String, Vec<Cursor>) {
    edit(text, cursors, |text, cursor| {
        let start = line_start(text, cursor.start());
        let indent: String = text[start..cursor.start()]
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        (cursor.start(), cursor.end(), format!("\n{indent}"))
    })
}

/// What copying takes with several cursors: every selection, in the order
/// they are in the text, a line each — or, with nothing selected anywhere,
/// the line each cursor is on.
pub fn copied(text: &str, cursors: &[Cursor]) -> String {
    let mut ordered: Vec<Cursor> = cursors.to_vec();
    ordered.sort_by_key(|cursor| cursor.start());
    if ordered.iter().all(|cursor| cursor.is_caret()) {
        let mut lines: Vec<(usize, &str)> = ordered
            .iter()
            .map(|cursor| {
                let start = line_start(text, cursor.head);
                (start, &text[start..line_end(text, cursor.head)])
            })
            .collect();
        lines.dedup_by_key(|(start, _)| *start);
        return lines.iter().map(|(_, line)| format!("{line}\n")).collect();
    }
    ordered
        .iter()
        .filter(|cursor| !cursor.is_caret())
        .map(|cursor| &text[cursor.start()..cursor.end()])
        .collect::<Vec<_>>()
        .join("\n")
}

/// Cutting with several cursors: every selection goes — or, with nothing
/// selected anywhere, the line each cursor is on, as [`copied`] took them.
pub fn cut(text: &str, cursors: &[Cursor]) -> (String, Vec<Cursor>) {
    if cursors.iter().all(|cursor| cursor.is_caret()) {
        return edit(text, cursors, |text, cursor| {
            let end = line_end(text, cursor.head);
            let through = if end < text.len() { end + 1 } else { end };
            (line_start(text, cursor.head), through, String::new())
        });
    }
    edit(text, cursors, |_, cursor| {
        (cursor.start(), cursor.end(), String::new())
    })
}

/// Pasting with several cursors: a clipboard with one line per cursor gives
/// each cursor its line, in the order they are in the text, as VS Code does;
/// anything else goes in whole at every cursor.
pub fn pasted(text: &str, cursors: &[Cursor], clip: &str) -> (String, Vec<Cursor>) {
    let clip = clip.replace("\r\n", "\n");
    let lines: Vec<&str> = clip
        .strip_suffix('\n')
        .unwrap_or(&clip)
        .split('\n')
        .collect();
    if lines.len() == cursors.len() && cursors.len() > 1 {
        let mut order: Vec<usize> = (0..cursors.len()).collect();
        order.sort_by_key(|&index| cursors[index].start());
        let mut line_of = vec![""; cursors.len()];
        for (line, index) in lines.iter().zip(order) {
            line_of[index] = line;
        }
        let owned: Vec<Cursor> = cursors.to_vec();
        return edit(text, &owned, |_, cursor| {
            let index = owned.iter().position(|c| *c == cursor).unwrap_or(0);
            (cursor.start(), cursor.end(), line_of[index].to_string())
        });
    }
    typed(text, cursors, &clip)
}

/// Every cursor moved, or its selection grown when `extend`. Without
/// `extend`, Left and Right first collapse a selection to its near end, as
/// in every editor; cursors that end up in one place become one.
pub fn moved(text: &str, cursors: &[Cursor], motion: Motion, extend: bool) -> Vec<Cursor> {
    let each: Vec<Cursor> = cursors
        .iter()
        .map(|cursor| {
            if !extend && !cursor.is_caret() {
                match motion {
                    Motion::Left => return Cursor::caret(cursor.start()),
                    Motion::Right => return Cursor::caret(cursor.end()),
                    _ => {}
                }
            }
            let head = step(text, cursor.head, motion);
            if extend {
                Cursor {
                    anchor: cursor.anchor,
                    head,
                }
            } else {
                Cursor::caret(head)
            }
        })
        .collect();
    merged(&each)
}

/// Cursors that overlap or stand in one place made one, the first cursor's
/// place kept first.
pub fn merged(cursors: &[Cursor]) -> Vec<Cursor> {
    let mut order: Vec<usize> = (0..cursors.len()).collect();
    order.sort_by_key(|&index| (cursors[index].start(), cursors[index].end()));
    let mut groups: Vec<(Vec<usize>, Cursor)> = Vec::new();
    for index in order {
        let cursor = cursors[index];
        if let Some((members, last)) = groups.last_mut()
            && (cursor.start() < last.end()
                || cursor.start() == last.start() && cursor.end() == last.end())
        {
            members.push(index);
            if cursor.end() > last.end() {
                // Grown to cover both, facing the way the later one did.
                *last = if last.head >= last.anchor {
                    Cursor {
                        anchor: last.start(),
                        head: cursor.end(),
                    }
                } else {
                    Cursor {
                        anchor: cursor.end(),
                        head: last.start(),
                    }
                };
            }
            continue;
        }
        groups.push((vec![index], cursor));
    }
    let first = groups
        .iter()
        .position(|(members, _)| members.contains(&0))
        .unwrap_or(0);
    let mut out = vec![groups[first].1];
    out.extend(
        groups
            .iter()
            .enumerate()
            .filter(|(at, _)| *at != first)
            .map(|(_, (_, cursor))| *cursor),
    );
    out
}

/// The word around `at`, or the one just before it: what Ctrl+D starts from
/// when nothing is selected.
pub fn word_at(text: &str, at: usize) -> Option<(usize, usize)> {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let at = boundary(text, at.min(text.len()));
    let start = text[..at]
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_word(*c))
        .last()
        .map_or(at, |(i, _)| i);
    let end = at
        + text[at..]
            .char_indices()
            .take_while(|(_, c)| is_word(*c))
            .last()
            .map_or(0, |(i, c)| i + c.len_utf8());
    (start < end).then_some((start, end))
}

/// Where `needle` occurs in `text`, by byte. A needle that is a word is
/// found only as a whole word, as VS Code's Ctrl+D finds the word it
/// selected itself: `gain` is not in `gain2`.
fn matches<'a>(text: &'a str, needle: &'a str) -> impl Iterator<Item = usize> + 'a {
    let word = needle.chars().all(|c| c.is_alphanumeric() || c == '_');
    let is_word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    text.match_indices(needle)
        .map(|(i, _)| i)
        .filter(move |&i| {
            !word
                || !(is_word(text[..i].chars().next_back())
                    || is_word(text[i + needle.len()..].chars().next()))
        })
}

/// Ctrl+D: the next place the last cursor's selection occurs after it,
/// wrapping past the end, as a selection — none when every place already has
/// a cursor.
pub fn next_match(text: &str, cursors: &[Cursor]) -> Option<Cursor> {
    let last = cursors.last()?;
    let needle = &text[last.start()..last.end()];
    if needle.is_empty() {
        return None;
    }
    let taken = |start: usize| cursors.iter().any(|cursor| cursor.start() == start);
    let found: Vec<usize> = matches(text, needle).collect();
    let after = found
        .iter()
        .filter(|&&start| start >= last.end())
        .chain(found.iter().filter(|&&start| start < last.end()))
        .copied()
        .find(|start| !taken(*start))?;
    Some(Cursor {
        anchor: after,
        head: after + needle.len(),
    })
}

/// Ctrl+Shift+L: every place `cursor`'s selection occurs, as selections, the
/// one it was first.
pub fn every_match(text: &str, cursor: Cursor) -> Vec<Cursor> {
    let needle = &text[cursor.start()..cursor.end()];
    if needle.is_empty() {
        return vec![cursor];
    }
    let mut all = vec![cursor];
    all.extend(
        matches(text, needle)
            .filter(|start| *start != cursor.start())
            .map(|start| Cursor {
                anchor: start,
                head: start + needle.len(),
            }),
    );
    all
}

/// Ctrl+Alt+Up or Down: a caret on the line above the topmost cursor, or
/// below the lowest, at the same column as far as that line goes.
pub fn beside_vertically(text: &str, cursors: &[Cursor], up: bool) -> Option<Cursor> {
    let from = if up {
        cursors.iter().map(|c| c.head).min()?
    } else {
        cursors.iter().map(|c| c.head).max()?
    };
    let head = step(text, from, if up { Motion::Up } else { Motion::Down });
    (line_start(text, head) != line_start(text, from)).then(|| Cursor::caret(head))
}

/// Where one step of `motion` takes a position.
fn step(text: &str, at: usize, motion: Motion) -> usize {
    match motion {
        Motion::Left => previous(text, at),
        Motion::Right => next(text, at),
        Motion::Up | Motion::Down => {
            let start = line_start(text, at);
            let column = text[start..at].chars().count();
            let target = if motion == Motion::Up {
                if start == 0 {
                    return 0;
                }
                line_start(text, start - 1)
            } else {
                let end = line_end(text, at);
                if end == text.len() {
                    return text.len();
                }
                end + 1
            };
            let line = &text[target..line_end(text, target)];
            target
                + line
                    .char_indices()
                    .nth(column)
                    .map_or(line.len(), |(i, _)| i)
        }
        Motion::Home => {
            let start = line_start(text, at);
            let blank = text[start..line_end(text, at)]
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .map(char::len_utf8)
                .sum::<usize>();
            if at == start + blank {
                start
            } else {
                start + blank
            }
        }
        Motion::End => line_end(text, at),
        Motion::WordLeft => word_left(text, at),
        Motion::WordRight => word_right(text, at),
    }
}

fn line_start(text: &str, at: usize) -> usize {
    text[..at].rfind('\n').map_or(0, |i| i + 1)
}

fn line_end(text: &str, at: usize) -> usize {
    text[at..].find('\n').map_or(text.len(), |i| at + i)
}

fn previous(text: &str, at: usize) -> usize {
    text[..at].char_indices().next_back().map_or(0, |(i, _)| i)
}

fn next(text: &str, at: usize) -> usize {
    text[at..]
        .chars()
        .next()
        .map_or(text.len(), |c| at + c.len_utf8())
}

/// What kind of character, for word motion: a word, punctuation, or space.
fn class(c: char) -> u8 {
    if c.is_alphanumeric() || c == '_' {
        0
    } else if c.is_whitespace() {
        2
    } else {
        1
    }
}

fn word_left(text: &str, at: usize) -> usize {
    let mut chars = text[..at].char_indices().rev().peekable();
    while chars
        .next_if(|(_, c)| class(*c) == 2 && *c != '\n')
        .is_some()
    {}
    let Some(&(_, first)) = chars.peek() else {
        return 0;
    };
    let kind = class(first);
    let mut start = at;
    for (i, c) in chars {
        if class(c) != kind || c == '\n' && kind == 2 {
            break;
        }
        start = i;
    }
    if start == at {
        previous(text, at)
    } else {
        start
    }
}

fn word_right(text: &str, at: usize) -> usize {
    let mut chars = text[at..].char_indices().peekable();
    while chars
        .next_if(|(_, c)| class(*c) == 2 && *c != '\n')
        .is_some()
    {}
    let Some(&(_, first)) = chars.peek() else {
        return text.len();
    };
    let kind = class(first);
    let mut end = at;
    for (i, c) in chars {
        if class(c) != kind {
            break;
        }
        end = at + i + c.len_utf8();
    }
    if end == at { next(text, at) } else { end }
}

fn boundary(text: &str, mut at: usize) -> usize {
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

#[cfg(test)]
mod tests {
    use super::*;

    fn carets(at: &[usize]) -> Vec<Cursor> {
        at.iter().map(|&at| Cursor::caret(at)).collect()
    }

    #[test]
    fn typing_goes_in_at_every_cursor_and_the_first_stays_first() {
        let text = "a\nb\nc";
        let (out, after) = typed(text, &carets(&[4, 0, 2]), "x");
        assert_eq!(out, "xa\nxb\nxc");
        // In the order given, each after what it wrote.
        assert_eq!(after, carets(&[7, 1, 4]));
    }

    #[test]
    fn typing_replaces_every_selection() {
        let text = "let a = a + a;";
        let cursors = every_match(text, Cursor { anchor: 4, head: 5 });
        assert_eq!(cursors.len(), 3);
        let (out, _) = typed(text, &cursors, "bb");
        assert_eq!(out, "let bb = bb + bb;");
    }

    #[test]
    fn backspace_and_delete_take_a_character_or_a_selection() {
        let text = "ab\ncd";
        let (out, after) = erased(text, &carets(&[1, 4]), false, false);
        assert_eq!(out, "b\nd");
        assert_eq!(after, carets(&[0, 2]));
        let (out, _) = erased(text, &carets(&[0, 3]), true, false);
        assert_eq!(out, "b\nd");
        let selection = [Cursor { anchor: 0, head: 2 }, Cursor::caret(5)];
        let (out, _) = erased(text, &selection, false, false);
        assert_eq!(out, "\nc");
    }

    /// Two carets that run into each other become one; the first wins.
    #[test]
    fn cursors_that_meet_become_one() {
        let text = "ab";
        let (out, after) = erased(text, &carets(&[1, 1]), false, false);
        assert_eq!(out, "b");
        assert_eq!(after.len(), 1);
        let moved = moved("abc", &carets(&[1, 2]), Motion::Home, false);
        assert_eq!(moved, carets(&[0]));
    }

    #[test]
    fn a_new_line_keeps_the_indentation_of_the_line_it_breaks() {
        let text = "    a\n  b";
        let (out, _) = broken(text, &carets(&[5, 9]));
        assert_eq!(out, "    a\n    \n  b\n  ");
    }

    #[test]
    fn a_character_boundary_is_never_split() {
        let text = "中文";
        let (out, after) = erased(text, &carets(&[6]), false, false);
        assert_eq!(out, "中");
        assert_eq!(after, carets(&[3]));
        assert_eq!(
            moved(text, &carets(&[3]), Motion::Right, false),
            carets(&[6])
        );
    }

    #[test]
    fn every_cursor_moves_and_a_selection_collapses_first() {
        let text = "one two\nthree";
        assert_eq!(
            moved(text, &carets(&[0, 8]), Motion::End, false),
            carets(&[7, 13])
        );
        assert_eq!(
            moved(text, &carets(&[2]), Motion::Down, false),
            carets(&[10])
        );
        assert_eq!(moved(text, &carets(&[12]), Motion::Up, false), carets(&[4]));
        let selected = [Cursor { anchor: 0, head: 3 }];
        assert_eq!(moved(text, &selected, Motion::Left, false), carets(&[0]));
        assert_eq!(
            moved(text, &selected, Motion::Right, true),
            [Cursor { anchor: 0, head: 4 }]
        );
        assert_eq!(
            moved(text, &carets(&[0]), Motion::WordRight, false),
            carets(&[3])
        );
        assert_eq!(
            moved(text, &carets(&[7]), Motion::WordLeft, false),
            carets(&[4])
        );
    }

    #[test]
    fn home_goes_to_the_first_non_blank_and_then_to_the_start() {
        let text = "    let x";
        assert_eq!(step(text, 8, Motion::Home), 4);
        assert_eq!(step(text, 4, Motion::Home), 0);
    }

    /// Ctrl+D: the word first, then the next place it occurs as a word,
    /// wrapping past the end, and never a place that already has a cursor.
    #[test]
    fn ctrl_d_finds_the_next_occurrence_and_wraps() {
        let text = "x gain(); gain2(); gain();";
        assert_eq!(word_at(text, 4), Some((2, 6)));
        assert_eq!(word_at(text, 6), Some((2, 6)), "just after the word");
        let from_last = [Cursor {
            anchor: 19,
            head: 23,
        }];
        let wrapped = next_match(text, &from_last).unwrap();
        assert_eq!(
            wrapped.start(),
            2,
            "past the end it starts again, skipping gain2"
        );
        let both = [from_last[0], wrapped];
        assert_eq!(next_match(text, &both), None, "every place has a cursor");
        // Not a word, not a whole-word search.
        let brackets = [Cursor { anchor: 6, head: 8 }];
        assert_eq!(next_match(text, &brackets).unwrap().start(), 15);
    }

    #[test]
    fn a_cursor_is_added_on_the_line_above_or_below() {
        let text = "abcd\nab\nabcd";
        assert_eq!(
            beside_vertically(text, &carets(&[3]), false),
            Some(Cursor::caret(7))
        );
        assert_eq!(beside_vertically(text, &carets(&[3]), true), None);
        assert_eq!(
            beside_vertically(text, &carets(&[3, 7]), false),
            Some(Cursor::caret(10))
        );
    }

    #[test]
    fn copying_takes_the_selections_or_the_lines() {
        let text = "one\ntwo\nthree";
        let selections = [Cursor { anchor: 4, head: 7 }, Cursor { anchor: 0, head: 3 }];
        assert_eq!(copied(text, &selections), "one\ntwo");
        assert_eq!(copied(text, &carets(&[5, 1, 6])), "one\ntwo\n");
    }

    /// A clipboard with a line per cursor gives each its own, in text order.
    #[test]
    fn pasting_a_line_per_cursor_distributes_them() {
        let text = "a\nb";
        let (out, _) = pasted(text, &carets(&[3, 1]), "1\n2\n");
        assert_eq!(out, "a1\nb2");
        let (out, _) = pasted(text, &carets(&[1, 3]), "x");
        assert_eq!(out, "ax\nbx");
    }
}
