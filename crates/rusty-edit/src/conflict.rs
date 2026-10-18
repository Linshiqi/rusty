//! Merge conflicts in a file, as git leaves them: where each block of
//! markers stands, and the text that resolves it one way or the other.
//!
//! A pull or a merge that stops leaves `<<<<<<< ours` … `=======` …
//! `>>>>>>> theirs` in the file, and with `merge.conflictStyle = diff3` a
//! `||||||| base` section between. VS Code puts *Accept Current Change |
//! Accept Incoming Change | Accept Both Changes* over each block and tints
//! the two sides; the editor draws the same from what this finds. Pure, so
//! what a block resolves to is a test rather than something found by
//! clicking on one.

/// One block of markers, by the document line (from zero) each marker is
/// on. The current side is the lines after `start` up to `base` or
/// `middle`; the incoming side the lines after `middle` up to `end`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// The `<<<<<<<` line.
    pub start: u32,
    /// The `|||||||` line of a diff3-style block, when there is one.
    pub base: Option<u32>,
    /// The `=======` line.
    pub middle: u32,
    /// The `>>>>>>>` line.
    pub end: u32,
}

/// Which side a block is resolved to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    /// Ours: the lines above `=======`.
    Current,
    /// Theirs: the lines below it.
    Incoming,
    /// Ours, then theirs.
    Both,
}

impl Choice {
    /// The verb the editor's lens names a choice by.
    pub fn parse(verb: &str) -> Option<Choice> {
        match verb {
            "current" => Some(Choice::Current),
            "incoming" => Some(Choice::Incoming),
            "both" => Some(Choice::Both),
            _ => None,
        }
    }
}

/// Whether `line` is a marker: seven of `mark` followed by a space, or
/// nothing but the seven. git writes a label after the first and last
/// (`<<<<<<< HEAD`), and may write `=======` bare.
fn is_marker(line: &str, mark: char) -> bool {
    let line = line.strip_suffix('\r').unwrap_or(line);
    let mut chars = line.chars();
    for _ in 0..7 {
        if chars.next() != Some(mark) {
            return false;
        }
    }
    matches!(chars.next(), None | Some(' '))
}

/// Every complete block of `text`, in order: a `<<<<<<<` line, then the
/// next `=======` (with at most one `|||||||` before it), then the next
/// `>>>>>>>`. A `<<<<<<<` with no `=======` and `>>>>>>>` after it is not a
/// conflict, and a file that merely mentions the markers in prose has none
/// in that order.
///
/// Cheap for a file with none in it: a text without `<<<<<<<` returns
/// before splitting a line.
pub fn find(text: &str) -> Vec<Conflict> {
    if !text.contains("<<<<<<<") {
        return Vec::new();
    }
    let mut found = Vec::new();
    let mut open: Option<(u32, Option<u32>)> = None;
    let mut middle: Option<u32> = None;
    for (at, line) in text.split('\n').enumerate() {
        let at = at as u32;
        if is_marker(line, '<') {
            // A block begun inside another begins again: the outer one had
            // no end and is no block.
            open = Some((at, None));
            middle = None;
        } else if let Some((start, base)) = open {
            if middle.is_none() && base.is_none() && is_marker(line, '|') {
                open = Some((start, Some(at)));
            } else if middle.is_none() && is_marker(line, '=') {
                middle = Some(at);
            } else if let Some(mid) = middle
                && is_marker(line, '>')
            {
                found.push(Conflict {
                    start,
                    base,
                    middle: mid,
                    end: at,
                });
                open = None;
                middle = None;
            }
        }
    }
    found
}

/// The byte range of lines `from..=to` of `text`, the line break after the
/// last of them included when there is one.
fn lines_range(text: &str, from: u32, to: u32) -> Option<(usize, usize)> {
    let mut starts = vec![0];
    for (at, b) in text.bytes().enumerate() {
        if b == b'\n' {
            starts.push(at + 1);
        }
    }
    let begin = *starts.get(from as usize)?;
    let end = match starts.get(to as usize + 1) {
        Some(next) => *next,
        None if (to as usize) < starts.len() => text.len(),
        None => return None,
    };
    Some((begin, end))
}

/// The lines of `text` from `from` up to `to` (exclusive), as one string
/// with their breaks.
fn lines_text(text: &str, from: u32, to: u32) -> String {
    if from >= to {
        return String::new();
    }
    match lines_range(text, from, to - 1) {
        Some((begin, end)) => text[begin..end].to_string(),
        None => String::new(),
    }
}

/// What resolving `conflict` one way does to `text`: the byte range of the
/// whole block, markers included, and the text that goes in its place.
/// `None` when the block is not in the text — it was found on another
/// version.
pub fn resolve(
    text: &str,
    conflict: &Conflict,
    choice: Choice,
) -> Option<((usize, usize), String)> {
    let lines: Vec<&str> = text.split('\n').collect();
    let on = |at: u32, mark: char| {
        lines
            .get(at as usize)
            .is_some_and(|line| is_marker(line, mark))
    };
    if !(on(conflict.start, '<') && on(conflict.middle, '=') && on(conflict.end, '>'))
        || conflict.base.is_some_and(|base| !on(base, '|'))
    {
        return None;
    }
    let range = lines_range(text, conflict.start, conflict.end)?;
    let current_end = conflict.base.unwrap_or(conflict.middle);
    let current = lines_text(text, conflict.start + 1, current_end);
    let incoming = lines_text(text, conflict.middle + 1, conflict.end);
    let mut replacement = match choice {
        Choice::Current => current,
        Choice::Incoming => incoming,
        Choice::Both => current + &incoming,
    };
    // The block ran to the end of the file with no break after its last
    // marker: what replaces it ends the file the same way.
    let ends_file = range.1 == text.len() && !text.ends_with('\n');
    if ends_file && replacement.ends_with('\n') {
        replacement.pop();
        if replacement.ends_with('\r') {
            replacement.pop();
        }
    }
    Some((range, replacement))
}

/// Which lines of `conflict` are the current side's (the `<<<<<<<` line
/// through the line before `|||||||` or `=======`) and which the incoming
/// side's (the line after `=======` through the `>>>>>>>` line), for
/// tinting them.
pub fn sides(conflict: &Conflict) -> (std::ops::Range<u32>, std::ops::Range<u32>) {
    let current_end = conflict.base.unwrap_or(conflict.middle);
    (
        conflict.start..current_end,
        conflict.middle + 1..conflict.end + 1,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "a\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> origin/main\nz\n";

    #[test]
    fn a_block_is_found_by_its_three_markers() {
        assert_eq!(
            find(TEXT),
            vec![Conflict {
                start: 1,
                base: None,
                middle: 3,
                end: 5
            }]
        );
    }

    #[test]
    fn each_side_resolves_to_its_own_lines_and_both_to_the_pair() {
        let c = find(TEXT)[0];
        let apply = |choice| {
            let ((from, to), with) = resolve(TEXT, &c, choice).unwrap();
            format!("{}{}{}", &TEXT[..from], with, &TEXT[to..])
        };
        assert_eq!(apply(Choice::Current), "a\nours\nz\n");
        assert_eq!(apply(Choice::Incoming), "a\ntheirs\nz\n");
        assert_eq!(apply(Choice::Both), "a\nours\ntheirs\nz\n");
    }

    /// `merge.conflictStyle = diff3` writes the base between the two: it is
    /// no part of either answer.
    #[test]
    fn a_diff3_base_is_left_out_of_every_choice() {
        let text = "<<<<<<< HEAD\nours\n||||||| base\nwas\n=======\ntheirs\n>>>>>>> theirs\n";
        let c = find(text)[0];
        assert_eq!(c.base, Some(2));
        let ((from, to), with) = resolve(text, &c, Choice::Both).unwrap();
        assert_eq!((from, to), (0, text.len()));
        assert_eq!(with, "ours\ntheirs\n");
        assert_eq!(sides(&c), (0..2, 5..7));
    }

    /// An empty side is an ordinary side: one branch deleted what the other
    /// changed.
    #[test]
    fn an_empty_side_resolves_to_nothing() {
        let text = "<<<<<<< HEAD\n=======\ntheirs\n>>>>>>> x\nz\n";
        let c = find(text)[0];
        let ((_, to), with) = resolve(text, &c, Choice::Current).unwrap();
        assert_eq!(with, "");
        assert_eq!(&text[to..], "z\n");
    }

    /// A block at the end of a file with no final newline keeps the file
    /// that way.
    #[test]
    fn a_block_that_ends_the_file_without_a_break_keeps_it_so() {
        let text = "a\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> x";
        let c = find(text)[0];
        let ((from, to), with) = resolve(text, &c, Choice::Incoming).unwrap();
        assert_eq!(
            format!("{}{}{}", &text[..from], with, &text[to..]),
            "a\ntheirs"
        );
    }

    #[test]
    fn markers_out_of_order_or_unfinished_are_no_conflict() {
        assert!(find("=======\n>>>>>>> x\n<<<<<<< y\n").is_empty());
        assert!(find("<<<<<<< y\nours\n=======\n").is_empty());
        assert!(find("the markers <<<<<<< and ======= and >>>>>>> in prose\n").is_empty());
        assert!(
            find("<<<<<<<<\n=======\n>>>>>>>\n").is_empty(),
            "eight is not a marker"
        );
    }

    /// Two blocks are two, and a restarted one is only the second.
    #[test]
    fn two_blocks_are_two_and_a_restarted_one_counts_once() {
        let text =
            "<<<<<<< a\n1\n=======\n2\n>>>>>>> b\n<<<<<<< a\n<<<<<<< a\n3\n=======\n4\n>>>>>>> b\n";
        let found = find(text);
        assert_eq!(found.len(), 2);
        assert_eq!(found[1].start, 6);
    }

    /// Windows line endings are markers too, and resolve with their `\r`.
    #[test]
    fn crlf_markers_are_markers() {
        let text = "<<<<<<< HEAD\r\nours\r\n=======\r\ntheirs\r\n>>>>>>> x\r\n";
        let c = find(text)[0];
        let (_, with) = resolve(text, &c, Choice::Current).unwrap();
        assert_eq!(with, "ours\r\n");
    }

    /// A block from another version of the text is refused rather than
    /// applied to the wrong lines.
    #[test]
    fn a_block_the_text_no_longer_holds_is_refused() {
        let c = find(TEXT)[0];
        assert!(resolve("a\nb\nc\nd\ne\nf\ng\n", &c, Choice::Current).is_none());
    }
}
