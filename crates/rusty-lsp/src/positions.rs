//! Converting between byte offsets, protocol positions, and scalar columns.
//!
//! Three unit systems meet here and every off-by-one is invisible in ASCII:
//!
//! - Rust strings index by **UTF-8 bytes**.
//! - LSP positions count in the **negotiated encoding** — UTF-16 code units by
//!   default, UTF-8 bytes when the server accepts our offer. rust-analyzer
//!   accepts, but the conversion still has to exist for the day it does not.
//! - The frontend wants **Unicode scalars**, so it can slice a line with
//!   `chars()` and be right.
//!
//! A `// 中文注释` before the error line is enough to shift every diagnostic on
//! it by a third under the wrong system, which is why the tests here are mostly
//! non-ASCII.

/// The unit the server counts `character` in, from the `initialize` handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf16,
}

impl Encoding {
    fn cost(self, ch: char) -> usize {
        match self {
            Encoding::Utf8 => ch.len_utf8(),
            Encoding::Utf16 => ch.len_utf16(),
        }
    }
}

/// A byte offset in `text` as an LSP `(line, character)`.
pub fn offset_to_position(text: &str, offset: usize, encoding: Encoding) -> (u32, u32) {
    let offset = offset.min(text.len());
    let before = &text[..offset];
    let line = before.bytes().filter(|b| *b == b'\n').count() as u32;
    let line_start = before.rfind('\n').map(|at| at + 1).unwrap_or(0);
    let character: usize = text[line_start..offset]
        .chars()
        .map(|ch| encoding.cost(ch))
        .sum();
    (line, character as u32)
}

/// An LSP `character` as a scalar column within one line.
///
/// Clamped: a position past the end of the line lands at the end, and one in
/// the middle of a UTF-16 surrogate pair stops before it. The server should
/// send neither; a stale document version means it sometimes does.
pub fn character_to_scalar(line: &str, character: u32, encoding: Encoding) -> u32 {
    let mut budget = character as usize;
    let mut scalars = 0u32;
    for ch in line.chars() {
        let cost = encoding.cost(ch);
        if budget < cost {
            break;
        }
        budget -= cost;
        scalars += 1;
        if budget == 0 {
            break;
        }
    }
    scalars
}

/// A scalar column within one line as an LSP `character`.
pub fn scalar_to_character(line: &str, scalar: u32, encoding: Encoding) -> u32 {
    line.chars()
        .take(scalar as usize)
        .map(|ch| encoding.cost(ch))
        .sum::<usize>() as u32
}

/// The smallest single-range edit that turns `old` into `new`, as LSP wants it:
/// start position, end position (both in `old`), and the replacement text.
///
/// Common prefix and suffix are stripped so a keystroke travels as a keystroke
/// rather than the whole file — rust-analyzer re-checks on every change, and
/// feeding it full documents makes typing latency track file size.
pub fn content_change(old: &str, new: &str, encoding: Encoding) -> ((u32, u32), (u32, u32), String) {
    let old_bytes = old.as_bytes();
    let new_bytes = new.as_bytes();

    let mut prefix = old_bytes
        .iter()
        .zip(new_bytes)
        .take_while(|(a, b)| a == b)
        .count();
    // Equal bytes up to `prefix` do not make `prefix` a boundary in either
    // string — the differing byte decides — so back up until both agree.
    while !(old.is_char_boundary(prefix) && new.is_char_boundary(prefix)) {
        prefix -= 1;
    }

    let mut suffix = old_bytes[prefix..]
        .iter()
        .rev()
        .zip(new_bytes[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    while !(old.is_char_boundary(old.len() - suffix) && new.is_char_boundary(new.len() - suffix)) {
        suffix -= 1;
    }

    let start = offset_to_position(old, prefix, encoding);
    let end = offset_to_position(old, old.len() - suffix, encoding);
    (start, end, new[prefix..new.len() - suffix].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cjk_counts_differently_per_encoding() {
        // "中" is 3 bytes in UTF-8 and 1 code unit in UTF-16.
        let line = "let 中文 = 1;";
        let offset = line.find('=').unwrap();
        let text = format!("//\n{line}");

        let (l8, c8) = offset_to_position(&text, 3 + offset, Encoding::Utf8);
        let (l16, c16) = offset_to_position(&text, 3 + offset, Encoding::Utf16);
        assert_eq!((l8, l16), (1, 1));
        assert_eq!(c8, offset as u32, "utf-8 characters are bytes: let␣=4 + 中文=6 + ␣=1");
        assert_eq!(c16, 7, "utf-16 units: let␣=4 + 中文=2 + ␣=1");

        // And back to scalars: both encodings land on the same column.
        assert_eq!(character_to_scalar(line, c8, Encoding::Utf8), 7);
        assert_eq!(character_to_scalar(line, c16, Encoding::Utf16), 7);
    }

    #[test]
    fn emoji_take_two_utf16_units() {
        let line = "a🦀b";
        // After the crab: byte 5, utf16 unit 3, scalar 2.
        assert_eq!(scalar_to_character(line, 2, Encoding::Utf16), 3);
        assert_eq!(scalar_to_character(line, 2, Encoding::Utf8), 5);
        assert_eq!(character_to_scalar(line, 3, Encoding::Utf16), 2);
        // A position inside the surrogate pair stops before the pair rather
        // than splitting it.
        assert_eq!(character_to_scalar(line, 2, Encoding::Utf16), 1);
    }

    #[test]
    fn positions_clamp_rather_than_panic() {
        assert_eq!(offset_to_position("ab", 99, Encoding::Utf8), (0, 2));
        assert_eq!(character_to_scalar("ab", 99, Encoding::Utf16), 2);
    }

    #[test]
    fn a_keystroke_travels_as_one_character() {
        let old = "fn main() {}\n";
        let new = "fn main() {x}\n";
        let (start, end, text) = content_change(old, new, Encoding::Utf16);
        assert_eq!(start, (0, 11));
        assert_eq!(end, (0, 11));
        assert_eq!(text, "x");
    }

    #[test]
    fn deleting_a_line_is_a_range_with_empty_text() {
        let old = "a\nbb\nc\n";
        let new = "a\nc\n";
        let (start, end, text) = content_change(old, new, Encoding::Utf16);
        assert_eq!(text, "");
        assert_eq!(start, (1, 0));
        assert_eq!(end, (2, 0));
    }

    #[test]
    fn multibyte_edits_stay_on_char_boundaries() {
        // 中 → 史 shares the first UTF-8 byte (0xE4), so a byte-wise prefix
        // lands mid-character and must back up.
        let old = "let a = \"中\";";
        let new = "let a = \"史\";";
        let (start, end, text) = content_change(old, new, Encoding::Utf16);
        assert_eq!(start, (0, 9));
        assert_eq!(end, (0, 10));
        assert_eq!(text, "史");
    }
}
