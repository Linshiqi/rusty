//! Turning model numbers into something readable.
//!
//! Presentation, so it lives here rather than on the wire types — the CLI
//! formats the same figures differently, and a model type that carries a
//! pre-rendered string forces every consumer to accept one house style.

use rusty_i18n::t;

/// Bytes at the scale an embedded developer thinks in.
///
/// Binary multiples, because that is what a datasheet, a linker script and
/// `cargo size` all mean by K. Labelled `KB` rather than `KiB` for the same
/// reason: it is what the rest of the toolchain prints, and being pedantically
/// correct in a column next to `espflash` output just looks like a discrepancy.
pub fn bytes(value: u64) -> String {
    const K: u64 = 1024;
    match value {
        b if b >= K * K => format!("{:.1} MB", b as f64 / (K * K) as f64),
        b if b >= K => format!("{:.1} KB", b as f64 / K as f64),
        b => format!("{b} B"),
    }
}

/// Bytes split into a number and its unit, for a [`Readout`](crate::view::components::Readout).
///
/// The unit is set smaller and lighter there, which only works if it arrives
/// separately.
pub fn bytes_parts(value: u64) -> (String, String) {
    let rendered = bytes(value);
    match rendered.rsplit_once(' ') {
        Some((number, unit)) => (number.to_string(), unit.to_string()),
        None => (rendered, String::new()),
    }
}

/// Roughly how long ago, from a Unix timestamp in seconds.
///
/// Deliberately coarse. The question this answers is "is this the build I just
/// made, or one from last week" — a clock time would make the reader do that
/// subtraction themselves.
pub fn since(epoch_secs: u64) -> String {
    let now = js_sys::Date::now() / 1000.0;
    let elapsed = now - epoch_secs as f64;

    // A build in the future means a clock skew — a network share, a container,
    // a machine that just changed timezone. Saying "in 3 hours" would be worse
    // than admitting the timestamp is not usable.
    if elapsed < 0.0 {
        return t!("misc.clock-skew");
    }

    match elapsed as u64 {
        s if s < 90 => t!("misc.just-now"),
        s if s < 3_600 => t!("misc.minutes-ago", count = (s / 60).to_string()),
        s if s < 86_400 => t!("misc.hours-ago", count = (s / 3_600).to_string()),
        s => t!("misc.days-ago", count = (s / 86_400).to_string()),
    }
}

/// A percentage, for figures the user compares against a budget.
pub fn percent(fraction: f32) -> String {
    format!("{:.0}%", fraction * 100.0)
}

#[cfg(test)]
mod tests {
    use super::{bytes, bytes_parts};

    #[test]
    fn bytes_switch_scale_at_binary_multiples() {
        assert_eq!(bytes(512), "512 B");
        // 1023 bytes is not "1.0 KB" — rounding up across the boundary is how a
        // figure that has not reached a limit is reported as having reached it.
        assert_eq!(bytes(1023), "1023 B");
        assert_eq!(bytes(1024), "1.0 KB");
        assert_eq!(bytes(1024 * 1024), "1.0 MB");
    }

    #[test]
    fn the_unit_splits_off_for_a_readout() {
        assert_eq!(bytes_parts(2048), ("2.0".to_string(), "KB".to_string()));
        assert_eq!(bytes_parts(7), ("7".to_string(), "B".to_string()));
    }
}
