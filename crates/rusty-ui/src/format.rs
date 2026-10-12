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
        // Disks, not flash: the Disk section reports build directories and
        // volumes, where "92692.2 MB" is a number nobody reads.
        b if b >= K * K * K * K => format!("{:.2} TB", b as f64 / (K * K * K * K) as f64),
        b if b >= K * K * K => format!("{:.1} GB", b as f64 / (K * K * K) as f64),
        b if b >= K * K => format!("{:.1} MB", b as f64 / (K * K) as f64),
        b if b >= K => format!("{:.1} KB", b as f64 / K as f64),
        b => format!("{b} B"),
    }
}

/// A project-relative path joined onto the project's root, spelled the way
/// the root is: a Windows root gets backslashes throughout, so what lands on
/// the clipboard pastes into any Windows tool without a mixed path.
pub fn full_path(root: &str, relative: &str) -> String {
    let windows = root.contains('\\');
    let separator = if windows { '\\' } else { '/' };
    let relative = if windows {
        relative.replace('/', "\\")
    } else {
        relative.to_string()
    };
    let root = root.trim_end_matches(['/', '\\']);
    format!("{root}{separator}{relative}")
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

/// When a commit was made, the way a history reads it: how long ago for the
/// last week, the date after that. "412 days ago" is arithmetic nobody
/// wanted to do, and it was every row of an old project's log.
pub fn commit_when(epoch_secs: u64) -> String {
    let elapsed = js_sys::Date::now() / 1000.0 - epoch_secs as f64;
    if (0.0..7.0 * 86_400.0).contains(&elapsed) {
        return since(epoch_secs);
    }
    let (year, month, day, _, _) = local_parts(epoch_secs);
    format!("{year:04}-{month:02}-{day:02}")
}

/// The whole local date and time, for a tooltip.
pub fn full_time(epoch_secs: u64) -> String {
    let (year, month, day, hour, minute) = local_parts(epoch_secs);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}")
}

/// A timestamp in this machine's time zone, as calendar parts.
fn local_parts(epoch_secs: u64) -> (i64, u32, u32, u32, u32) {
    // Minutes *behind* UTC, as JavaScript counts them: -480 in China.
    let offset_minutes =
        js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(epoch_secs as f64 * 1000.0))
            .get_timezone_offset();
    civil(epoch_secs as i64 - (offset_minutes * 60.0) as i64)
}

/// Seconds since the epoch — already shifted to local time — as a calendar
/// date and a time of day. Howard Hinnant's `civil_from_days`: arithmetic
/// alone decides which day a timestamp falls on, so it is tested here
/// rather than trusted to a browser.
pub(crate) fn civil(secs: i64) -> (i64, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let rest = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (
        year,
        month,
        day,
        (rest / 3_600) as u32,
        (rest % 3_600 / 60) as u32,
    )
}

/// A percentage, for figures the user compares against a budget.
pub fn percent(fraction: f32) -> String {
    format!("{:.0}%", fraction * 100.0)
}

#[cfg(test)]
mod tests {
    use super::{bytes, bytes_parts, civil, full_path};

    /// Checked against Python's `datetime` for each: the epoch, an
    /// ordinary afternoon, a leap day, the last minute of 2099, and one
    /// second before the epoch.
    #[test]
    fn a_timestamp_falls_on_the_calendar_day_it_does() {
        assert_eq!(civil(0), (1970, 1, 1, 0, 0));
        assert_eq!(civil(1_756_940_000), (2025, 9, 3, 22, 53));
        assert_eq!(civil(951_782_400), (2000, 2, 29, 0, 0));
        assert_eq!(civil(4_102_444_799), (2099, 12, 31, 23, 59));
        assert_eq!(civil(-1), (1969, 12, 31, 23, 59));
    }

    /// The clipboard gets one spelling: the root's.
    #[test]
    fn a_full_path_follows_the_roots_separators() {
        assert_eq!(
            full_path("D:\\project\\my_fly", "src/bin/main.rs"),
            "D:\\project\\my_fly\\src\\bin\\main.rs"
        );
        assert_eq!(
            full_path("/home/me/my_fly/", "src/lib.rs"),
            "/home/me/my_fly/src/lib.rs"
        );
    }

    #[test]
    fn bytes_switch_scale_at_binary_multiples() {
        assert_eq!(bytes(512), "512 B");
        // 1023 bytes is not "1.0 KB" — rounding up across the boundary is how a
        // figure that has not reached a limit is reported as having reached it.
        assert_eq!(bytes(1023), "1023 B");
        assert_eq!(bytes(1024), "1.0 KB");
        assert_eq!(bytes(1024 * 1024), "1.0 MB");
        assert_eq!(bytes(1536 * 1024 * 1024), "1.5 GB");
        assert_eq!(bytes(895 * 1024 * 1024 * 1024), "895.0 GB");
        assert_eq!(bytes(1024u64.pow(4)), "1.00 TB");
    }

    #[test]
    fn the_unit_splits_off_for_a_readout() {
        assert_eq!(bytes_parts(2048), ("2.0".to_string(), "KB".to_string()));
        assert_eq!(bytes_parts(7), ("7".to_string(), "B".to_string()));
    }
}
