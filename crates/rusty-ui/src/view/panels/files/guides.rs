//! Indent guides: a faint line at every indentation stop a line's text starts
//! past, as VS Code draws them, and the one for the block the caret is in
//! drawn brighter.
//!
//! Drawn inside each echo row, so they cost nothing past the rows in view and
//! move nothing: a guide is a line one pixel wide in whitespace, and the
//! textarea over the echo never knows it is there. Pure, so the rule for a
//! blank line — which has no indentation of its own — is under tests.

use super::TAB_SIZE;

/// How many guides each line gets: one per indentation stop its text starts
/// after. A line indented eight spaces has two, at columns 0 and 4; one
/// indented six has two as well, since its text starts past the second stop.
/// A blank line takes the deeper of the lines around it, so a guide runs
/// unbroken through the blank lines inside a block and stops at the ones
/// between two blocks.
pub(super) fn indent_levels(text: &str) -> Vec<u8> {
    let tab = TAB_SIZE as usize;
    let own: Vec<Option<usize>> = text.split('\n').map(|line| indent_of(line, tab)).collect();
    // The nearest line below each that has text.
    let mut below = vec![None; own.len()];
    let mut next = None;
    for (at, indent) in own.iter().enumerate().rev() {
        below[at] = next;
        if indent.is_some() {
            next = *indent;
        }
    }
    let mut above = None;
    own.iter()
        .zip(below)
        .map(|(indent, below)| {
            let columns = match indent {
                Some(columns) => {
                    above = Some(*columns);
                    *columns
                }
                None => above.unwrap_or(0).max(below.unwrap_or(0)),
            };
            columns.div_ceil(tab).min(usize::from(u8::MAX)) as u8
        })
        .collect()
}

/// The columns a line's leading whitespace covers, a tab reaching the next
/// stop; `None` for a line with nothing else on it.
fn indent_of(line: &str, tab: usize) -> Option<usize> {
    let mut columns = 0;
    for ch in line.chars() {
        match ch {
            ' ' => columns += 1,
            '\t' => columns += tab - columns % tab,
            '\r' => {}
            _ => return Some(columns),
        }
    }
    None
}

/// The guide of the block the caret is in: its stop, and the first and last
/// line it runs through. On a line that opens a block — the next line is
/// deeper — it is the block that line opens, as VS Code highlights it; on any
/// other, the block the line sits in. `None` at the outermost level.
pub(super) fn active_guide(levels: &[u8], caret_line: usize) -> Option<(u8, usize, usize)> {
    let level = *levels.get(caret_line)?;
    let next = levels.get(caret_line + 1).copied().unwrap_or(0);
    let (stop, first) = if next > level {
        (level, caret_line + 1)
    } else {
        (level.checked_sub(1)?, caret_line)
    };
    // The run of lines deeper than the stop, around where it starts.
    let inside = |at: usize| levels.get(at).is_some_and(|&l| l > stop);
    if !inside(first) {
        return None;
    }
    let mut start = first;
    while start > 0 && inside(start - 1) {
        start -= 1;
    }
    let mut end = first;
    while inside(end + 1) {
        end += 1;
    }
    Some((stop, start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CODE: &str =
        "fn a() {\n    let x = 1;\n\n    if x {\n        y();\n\n    }\n}\n\nfn b() {}";

    #[test]
    fn a_line_has_a_guide_per_stop_its_text_starts_past() {
        assert_eq!(
            indent_levels("a\n    b\n        c\n      d\n\te\n  \tf"),
            [0, 1, 2, 2, 1, 1]
        );
    }

    /// Blank lines inside a block keep its guides; the one between two
    /// blocks has none.
    #[test]
    fn a_blank_line_takes_the_deeper_of_its_neighbours() {
        assert_eq!(indent_levels(CODE), [0, 1, 1, 1, 2, 2, 1, 0, 0, 0]);
    }

    /// On a line inside a block, the block's guide; on the line that opens
    /// one, the block it opens.
    #[test]
    fn the_caret_lights_the_guide_of_its_block() {
        let levels = indent_levels(CODE);
        // `let x = 1;`: the function's body, lines 1 to 6.
        assert_eq!(active_guide(&levels, 1), Some((0, 1, 6)));
        // `if x {` opens the block on lines 4 and 5.
        assert_eq!(active_guide(&levels, 3), Some((1, 4, 5)));
        // `y();` sits in that block.
        assert_eq!(active_guide(&levels, 4), Some((1, 4, 5)));
        // `fn a() {` opens the function's body.
        assert_eq!(active_guide(&levels, 0), Some((0, 1, 6)));
        // `fn b() {}` is at the outermost level and opens nothing.
        assert_eq!(active_guide(&levels, 9), None);
        assert_eq!(active_guide(&levels, 99), None);
    }
}
