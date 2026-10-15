//! Sticky scroll: the lines that open the blocks the top of the view is
//! inside, kept at the top of the editor while it scrolls through them — VS
//! Code's, from the same regions the fold chevrons use. Pure, for both
//! windows; each draws the answer its own way.

use crate::Region;

/// At most this many lines stick, the outermost first — VS Code's default.
pub const MOST: usize = 5;

/// The headers of the regions `line` is inside, outermost first. A header is
/// not inside its own region: on screen it needs no copy of itself.
pub fn headers_over(regions: &[Region], line: u32) -> Vec<u32> {
    let mut found: Vec<u32> = regions
        .iter()
        .filter(|region| region.header < line && line <= region.last)
        .map(|region| region.header)
        .collect();
    found.sort_unstable();
    found.truncate(MOST);
    found
}

/// The lines that stick when the view's rows begin with `line_at(0)`. They
/// cover the rows they take, so what they are about is the line just under
/// them, and more stick while that line is inside more blocks. Only ever
/// more: at a block's closing brace, one line fewer would uncover a line of
/// that block and one more would cover the brace, and a count that took
/// turns between the two would flicker with every row scrolled.
pub fn stuck(regions: &[Region], line_at: impl Fn(u32) -> u32) -> Vec<u32> {
    let mut found = headers_over(regions, line_at(0));
    loop {
        let next = headers_over(regions, line_at(found.len() as u32));
        if next.len() <= found.len() {
            return found;
        }
        found = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn regions() -> Vec<Region> {
        vec![
            Region {
                header: 0,
                last: 10,
            },
            Region { header: 2, last: 5 },
            Region { header: 3, last: 4 },
            Region { header: 7, last: 9 },
        ]
    }

    #[test]
    fn the_blocks_a_line_is_in_stick_outermost_first() {
        assert_eq!(headers_over(&regions(), 4), [0, 2, 3]);
        assert_eq!(headers_over(&regions(), 8), [0, 7]);
        assert_eq!(headers_over(&regions(), 11), Vec::<u32>::new());
    }

    /// A line that opens a block is on screen already: it does not stick
    /// over itself.
    #[test]
    fn a_header_does_not_stick_over_itself() {
        assert_eq!(headers_over(&regions(), 3), [0, 2]);
        assert_eq!(headers_over(&regions(), 0), Vec::<u32>::new());
    }

    /// A view starting at line 7 has line 0 stuck over its first row, which
    /// leaves line 8 as the first one showing — inside line 7's block, so
    /// line 7 sticks too.
    #[test]
    fn what_sticks_is_about_the_line_under_it() {
        assert_eq!(stuck(&regions(), |row| 1 + row), [0]);
        assert_eq!(stuck(&regions(), |row| 3 + row), [0, 2]);
        assert_eq!(stuck(&regions(), |row| 7 + row), [0, 7]);
    }

    /// At an inner block's closing brace the answer does not take turns: with
    /// the view at line 4, lines 0, 2 and 3 are over the rows of lines 4 to
    /// 6, and line 7 — outside the inner blocks — would have only line 0.
    #[test]
    fn a_closing_brace_does_not_make_the_lines_flicker() {
        assert_eq!(stuck(&regions(), |row| 4 + row), [0, 2, 3]);
        assert_eq!(stuck(&regions(), |row| 5 + row), [0, 2]);
    }

    #[test]
    fn no_more_than_five_lines_stick() {
        let deep: Vec<Region> = (0..8)
            .map(|n| Region {
                header: n,
                last: 20,
            })
            .collect();
        assert_eq!(headers_over(&deep, 12), [0, 1, 2, 3, 4]);
    }
}
