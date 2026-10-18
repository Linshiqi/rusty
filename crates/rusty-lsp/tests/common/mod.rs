//! What the end-to-end tests share.

/// The zero-based line holding `needle`, or a panic naming it.
pub fn line_of(text: &str, needle: &str) -> u32 {
    text.lines()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("no line contains {needle}")) as u32
}
