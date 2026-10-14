//! Numbers as an embedded developer reads them.

/// Bytes in binary multiples, labelled `KB` as a datasheet, a linker script
/// and `cargo size` all label them.
pub fn bytes(value: u64) -> String {
    const K: u64 = 1024;
    match value {
        b if b >= K * K * K => format!("{:.1} GB", b as f64 / (K * K * K) as f64),
        b if b >= K * K => format!("{:.1} MB", b as f64 / (K * K) as f64),
        b if b >= K => format!("{:.1} KB", b as f64 / K as f64),
        b => format!("{b} B"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_part_reads_in_the_units_its_datasheet_uses() {
        assert_eq!(bytes(400 * 1024), "400.0 KB");
        assert_eq!(bytes(4 * 1024 * 1024), "4.0 MB");
        assert_eq!(bytes(512), "512 B");
    }
}
