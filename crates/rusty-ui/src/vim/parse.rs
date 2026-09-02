//! The grammar of normal and visual mode: which keys typed so far are a
//! complete command, a prefix of one, or nothing at all.
//!
//! Deliberately a re-parse of the whole pending string on every key rather
//! than a stepping machine: the grammar is small, the strings are three
//! characters long, and a pure `&str -> Parsed` is a function a test can
//! enumerate. `mod.rs` owns the state that acts on what this returns; this
//! file owns only the reading.

use super::{
    Ask, View,
    motion::Motion,
    object::{self, Object},
};

/// One level of indent. Four spaces, because that is what rustfmt emits and
/// what every file this editor opens is already using; reading it from the
/// buffer would guess wrong on the first blank file.
pub(super) const SHIFT: usize = 4;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Op {
    Delete,
    Change,
    Yank,
    Indent,
    Outdent,
    /// `gc` — handed to the editor, which knows the language.
    Comment,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Target {
    Motion(Motion),
    /// `dd`, `cc`, `yy` — the whole line, count of them.
    Line,
    Object(Object),
    /// An operator pressed in visual mode acts on what is selected.
    Selection,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Simple {
    Insert,
    InsertLineStart,
    Append,
    AppendLineEnd,
    OpenBelow,
    OpenAbove,
    DeleteChar,
    Paste { after: bool },
    Visual,
    VisualLine,
    Replace(char),
    JoinLines,
    Repeat,
    Ask(Ask),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Command {
    Move(Motion, usize),
    Operate {
        op: Op,
        target: Target,
        count: usize,
    },
    Simple(Simple, usize),
}

impl Command {
    pub(super) fn changes_buffer(&self) -> bool {
        match self {
            Command::Move(..) => false,
            Command::Operate { op, .. } => *op != Op::Yank,
            Command::Simple(simple, _) => !matches!(
                simple,
                Simple::Visual | Simple::VisualLine | Simple::Ask(_) | Simple::Repeat
            ),
        }
    }
}

pub(super) enum Parsed {
    /// A prefix of something real — keep waiting.
    Incomplete,
    /// Nothing starts like this.
    Unknown,
    Command(Command),
}

/// Parse the keys typed so far.
///
/// Deliberately re-parses the whole pending string on every key rather than
/// stepping a machine: the grammar is small, the strings are three characters
/// long, and a pure `&str -> Parsed` is a function a test can enumerate.
pub(super) fn parse(pending: &str) -> Parsed {
    let (count, rest) = take_count(pending);
    if rest.is_empty() {
        return Parsed::Incomplete;
    }
    let count = count.unwrap_or(1);
    let mut chars = rest.chars();
    let first = chars.next().expect("rest is not empty");
    let tail: String = chars.collect();

    // Operators, which take a second half.
    if let Some(op) = operator(first) {
        return parse_operator_target(op, first, count, &tail);
    }

    if !tail.is_empty() {
        // Two-key commands that are not operators.
        return match (first, tail.as_str()) {
            ('g', "g") => Parsed::Command(Command::Move(Motion::FirstLine, count)),
            ('g', "c") => Parsed::Incomplete,
            ('g', "cc") => Parsed::Command(Command::Operate {
                op: Op::Comment,
                target: Target::Line,
                count,
            }),
            ('g', rest) if rest.starts_with('c') => {
                parse_operator_target(Op::Comment, 'c', count, &rest[1..])
            }
            ('g', _) => Parsed::Unknown,
            ('z', "z") => Parsed::Command(Command::Simple(
                Simple::Ask(Ask::Centre { at: View::Middle }),
                1,
            )),
            ('z', "t") => Parsed::Command(Command::Simple(
                Simple::Ask(Ask::Centre { at: View::Top }),
                1,
            )),
            ('z', "b") => Parsed::Command(Command::Simple(
                Simple::Ask(Ask::Centre { at: View::Bottom }),
                1,
            )),
            ('z', _) => Parsed::Unknown,
            ('Z', "Z") => Parsed::Command(Command::Simple(Simple::Ask(Ask::SaveAndClose), 1)),
            ('Z', _) => Parsed::Unknown,
            ('f' | 'F' | 't' | 'T', target) => {
                let Some(target) = target.chars().next() else {
                    return Parsed::Incomplete;
                };
                Parsed::Command(Command::Move(Motion::Find { key: first, target }, count))
            }
            ('r', with) => {
                let Some(with) = with.chars().next() else {
                    return Parsed::Incomplete;
                };
                Parsed::Command(Command::Simple(Simple::Replace(with), count))
            }
            _ => Parsed::Unknown,
        };
    }

    match first {
        // Waiting for a second key.
        'g' | 'Z' | 'z' | 'f' | 'F' | 't' | 'T' | 'r' => Parsed::Incomplete,

        // The word under the cursor, and where the view sits.
        '*' => Parsed::Command(Command::Simple(
            Simple::Ask(Ask::SearchWord { backwards: false }),
            1,
        )),
        '#' => Parsed::Command(Command::Simple(
            Simple::Ask(Ask::SearchWord { backwards: true }),
            1,
        )),

        'h' => Parsed::Command(Command::Move(Motion::Left, count)),
        'l' | ' ' => Parsed::Command(Command::Move(Motion::Right, count)),
        'j' => Parsed::Command(Command::Move(Motion::Down, count)),
        'k' => Parsed::Command(Command::Move(Motion::Up, count)),
        '0' => Parsed::Command(Command::Move(Motion::LineStart, 1)),
        '^' => Parsed::Command(Command::Move(Motion::FirstWord, 1)),
        '$' => Parsed::Command(Command::Move(Motion::LineEnd, count)),
        'w' => Parsed::Command(Command::Move(Motion::WordForward, count)),
        'W' => Parsed::Command(Command::Move(Motion::BigWordForward, count)),
        'b' => Parsed::Command(Command::Move(Motion::WordBack, count)),
        'B' => Parsed::Command(Command::Move(Motion::BigWordBack, count)),
        'e' => Parsed::Command(Command::Move(Motion::WordEnd, count)),
        'G' => Parsed::Command(Command::Move(Motion::LastLine, count)),
        '{' => Parsed::Command(Command::Move(Motion::ParagraphBack, count)),
        '}' => Parsed::Command(Command::Move(Motion::ParagraphForward, count)),
        '%' => Parsed::Command(Command::Move(Motion::MatchPair, 1)),
        ';' => Parsed::Command(Command::Move(Motion::RepeatFind { reverse: false }, count)),
        ',' => Parsed::Command(Command::Move(Motion::RepeatFind { reverse: true }, count)),

        'i' => Parsed::Command(Command::Simple(Simple::Insert, count)),
        'I' => Parsed::Command(Command::Simple(Simple::InsertLineStart, count)),
        'a' => Parsed::Command(Command::Simple(Simple::Append, count)),
        'A' => Parsed::Command(Command::Simple(Simple::AppendLineEnd, count)),
        'o' => Parsed::Command(Command::Simple(Simple::OpenBelow, count)),
        'O' => Parsed::Command(Command::Simple(Simple::OpenAbove, count)),
        'x' => Parsed::Command(Command::Simple(Simple::DeleteChar, count)),
        'p' => Parsed::Command(Command::Simple(Simple::Paste { after: true }, count)),
        'P' => Parsed::Command(Command::Simple(Simple::Paste { after: false }, count)),
        'v' => Parsed::Command(Command::Simple(Simple::Visual, 1)),
        'V' => Parsed::Command(Command::Simple(Simple::VisualLine, 1)),
        'J' => Parsed::Command(Command::Simple(Simple::JoinLines, count)),
        '.' => Parsed::Command(Command::Simple(Simple::Repeat, count)),
        'u' => Parsed::Command(Command::Simple(Simple::Ask(Ask::Undo), count)),
        '/' => Parsed::Command(Command::Simple(
            Simple::Ask(Ask::Search { backwards: false }),
            1,
        )),
        '?' => Parsed::Command(Command::Simple(
            Simple::Ask(Ask::Search { backwards: true }),
            1,
        )),
        'n' => Parsed::Command(Command::Simple(Simple::Ask(Ask::SearchNext), count)),
        'N' => Parsed::Command(Command::Simple(Simple::Ask(Ask::SearchPrevious), count)),

        // `D`, `C`, `S` are Vim's own shorthands, and people type them
        // constantly — spelling them out here is cheaper than explaining
        // their absence.
        'D' => Parsed::Command(Command::Operate {
            op: Op::Delete,
            target: Target::Motion(Motion::LineEnd),
            count,
        }),
        'C' => Parsed::Command(Command::Operate {
            op: Op::Change,
            target: Target::Motion(Motion::LineEnd),
            count,
        }),
        'Y' => Parsed::Command(Command::Operate {
            op: Op::Yank,
            target: Target::Line,
            count,
        }),
        'S' => Parsed::Command(Command::Operate {
            op: Op::Change,
            target: Target::Line,
            count,
        }),
        's' => Parsed::Command(Command::Operate {
            op: Op::Change,
            target: Target::Motion(Motion::Right),
            count,
        }),

        _ => Parsed::Unknown,
    }
}

/// Add or remove one level of indent on every line the range touches.
///
/// Blank lines are left alone when indenting — Vim does, and a file full of
/// trailing whitespace is what happens when they are not.
pub(super) fn shift(text: &str, from: usize, to: usize, deeper: bool) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut at = 0usize;
    let mut line_start = 0usize;

    while at <= chars.len() {
        let ends = at == chars.len() || chars[at] == '\n';
        if ends {
            let line: String = chars[line_start..at].iter().collect();
            let touched = line_start <= to && at >= from;
            if touched && !(deeper && line.trim().is_empty()) {
                if deeper {
                    out.push_str(&" ".repeat(SHIFT));
                    out.push_str(&line);
                } else {
                    let drop = line.chars().take(SHIFT).take_while(|c| *c == ' ').count();
                    out.push_str(&line[drop..]);
                }
            } else {
                out.push_str(&line);
            }
            if at < chars.len() {
                out.push('\n');
            }
            line_start = at + 1;
        }
        at += 1;
    }
    out
}

/// True when the cursor is on whitespace — the one thing `cw` asks before
/// deciding whether it is really `ce`.
pub(super) fn on_blank(text: &str, cursor: usize) -> bool {
    text.chars().nth(cursor).is_none_or(char::is_whitespace)
}

pub(super) fn operator(key: char) -> Option<Op> {
    match key {
        'd' => Some(Op::Delete),
        'c' => Some(Op::Change),
        'y' => Some(Op::Yank),
        '>' => Some(Op::Indent),
        '<' => Some(Op::Outdent),
        _ => None,
    }
}

/// The half of an operator command after the operator: `w`, `d` (the line),
/// `iw`, `2j`, and so on.
fn parse_operator_target(op: Op, key: char, count: usize, tail: &str) -> Parsed {
    if tail.is_empty() {
        return Parsed::Incomplete;
    }
    let (inner, rest) = take_count(tail);
    if rest.is_empty() {
        return Parsed::Incomplete;
    }
    let count = count * inner.unwrap_or(1);
    let mut chars = rest.chars();
    let first = chars.next().expect("rest is not empty");
    let tail: String = chars.collect();

    // `dd` / `cc` / `yy`: the operator doubled means whole lines.
    if first == key && tail.is_empty() {
        return Parsed::Command(Command::Operate {
            op,
            target: Target::Line,
            count,
        });
    }

    if matches!(first, 'i' | 'a') {
        let Some(kind) = tail.chars().next() else {
            return Parsed::Incomplete;
        };
        return match object::of(first == 'i', kind) {
            Some(object) => Parsed::Command(Command::Operate {
                op,
                target: Target::Object(object),
                count,
            }),
            None => Parsed::Unknown,
        };
    }

    // Anything else has to be a motion, and it is the same table as a bare
    // motion — `dw` and `w` must never disagree about what a word is.
    let sub = format!("{first}{tail}");
    match parse(&sub) {
        Parsed::Command(Command::Move(motion, inner)) => Parsed::Command(Command::Operate {
            op,
            target: Target::Motion(motion),
            count: count * inner,
        }),
        Parsed::Incomplete => Parsed::Incomplete,
        _ => Parsed::Unknown,
    }
}

/// Split a leading count off. `0` is a motion, not a count, so a count never
/// starts with it — `d0` deletes to the line start.
fn take_count(keys: &str) -> (Option<usize>, &str) {
    let digits: String = keys
        .chars()
        .enumerate()
        .take_while(|(index, c)| c.is_ascii_digit() && !(*index == 0 && *c == '0'))
        .map(|(_, c)| c)
        .collect();
    if digits.is_empty() {
        return (None, keys);
    }
    (digits.parse().ok(), &keys[digits.len()..])
}

/// Vim's operators applied to a visual selection: `d`, `c`, `y`, `x`, `s`.
///
/// Separate from [`parse`] because in visual mode the target is already
/// chosen, and routing it through the operator grammar would wait for a
/// motion that is never coming.
pub(super) fn visual_command(key: char) -> Option<Command> {
    let op = match key {
        'd' | 'x' => Op::Delete,
        'c' | 's' => Op::Change,
        'y' => Op::Yank,
        '>' => Op::Indent,
        '<' => Op::Outdent,
        _ => return None,
    };
    Some(Command::Operate {
        op,
        target: Target::Selection,
        count: 1,
    })
}
