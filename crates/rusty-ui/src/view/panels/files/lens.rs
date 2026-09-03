//! Where a test's lens sits.
//!
//! The lens is the `Run Test | Debug` pair beside a `#[test]` or a test
//! module. VS Code draws it on a row of its own directly above the item; this
//! editor cannot insert a row (see `folding`: the textarea and the echo must
//! stay glyph for glyph), so the lens takes the nearest thing — the attribute
//! line above the item, after its text — and the item's own line when there is
//! no attribute above it.

/// The document line the lens is drawn on and the scalar column it starts at,
/// for the runnable declared on `item_line`. `lines` is the draft split on
/// newlines. `None` only when the item line is not in the text at all.
pub(super) fn lens_anchor(lines: &[&str], item_line: u32) -> Option<(u32, u32)> {
    let attribute_above = item_line.checked_sub(1).filter(|&above| {
        lines
            .get(above as usize)
            .is_some_and(|text| text.trim_start().starts_with("#["))
    });
    let line = attribute_above.unwrap_or(item_line);
    let text = lines.get(line as usize)?;
    // Scalars, not bytes: the column is what `col_left` measures in, and a
    // `中` in a doc comment above the test must not push the lens right.
    let end = text.trim_end().chars().count() as u32;
    Some((line, end + 2))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<&str> {
        text.lines().collect()
    }

    /// `#[test]` above the function is where VS Code draws the lens, so the
    /// lens sits on that row, past the attribute's text.
    #[test]
    fn the_attribute_above_the_item_carries_the_lens() {
        let text = "    #[test]\n    fn it_works() {}\n";
        assert_eq!(lens_anchor(&lines(text), 1), Some((0, 11 + 2)));
    }

    /// `#[cfg(test)]` is not a test attribute, but it is the row above the
    /// module and the row the lens belongs on.
    #[test]
    fn a_cfg_test_module_anchors_to_its_attribute() {
        let text = "#[cfg(test)]\nmod tests {\n}\n";
        assert_eq!(lens_anchor(&lines(text), 1), Some((0, 12 + 2)));
    }

    /// No attribute above — a `mod parsing {` after a blank line — puts the
    /// lens after the item's own text rather than on the blank row.
    #[test]
    fn without_an_attribute_the_item_line_carries_it() {
        let text = "}\n\nmod parsing {\n";
        assert_eq!(lens_anchor(&lines(text), 2), Some((2, 13 + 2)));
    }

    #[test]
    fn the_first_line_has_nothing_above_it() {
        assert_eq!(lens_anchor(&lines("mod tests {\n"), 0), Some((0, 11 + 2)));
    }

    /// A multi-byte character in the attribute line counts once.
    #[test]
    fn columns_are_scalars_not_bytes() {
        let text = "#[test] // 中文\nfn a() {}\n";
        assert_eq!(lens_anchor(&lines(text), 1), Some((0, 13 + 2)));
    }

    #[test]
    fn a_line_past_the_end_is_none() {
        assert_eq!(lens_anchor(&lines("fn a() {}\n"), 7), None);
    }
}
