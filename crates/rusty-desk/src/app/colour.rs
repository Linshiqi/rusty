//! A colour written as a page writes one, `#rrggbb`, as Slint draws it —
//! one reading, where the draw, lab, math and board pages each kept their
//! own three lines of it.

/// `#rrggbb` (the `#` optional) as a colour; black for anything else.
pub(crate) fn hex(hex: &str) -> slint::Color {
    let value = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0);
    slint::Color::from_rgb_u8((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

#[cfg(test)]
mod tests {
    use super::hex;

    #[test]
    fn a_hex_triplet_is_its_three_channels_and_nonsense_is_black() {
        let rust = hex("#e5484d");
        assert_eq!((rust.red(), rust.green(), rust.blue()), (0xe5, 0x48, 0x4d));
        assert_eq!(hex("30a46c").green(), 0xa4);
        assert_eq!(hex("#nope").red(), 0);
    }
}
