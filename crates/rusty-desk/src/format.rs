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

/// [`bytes`] as its number and its unit, for a readout that sets them apart.
pub fn bytes_parts(value: u64) -> (String, String) {
    let text = bytes(value);
    match text.rsplit_once(' ') {
        Some((number, unit)) => (number.to_string(), unit.to_string()),
        None => (text, String::new()),
    }
}

/// A fraction as a whole percentage, for figures held to a budget.
pub fn percent(fraction: f32) -> String {
    format!("{:.0}%", fraction * 100.0)
}

/// Roughly how long ago, from seconds since the epoch, against `now`.
/// Coarse on purpose: the question is "the commit I just made, or one from
/// last week", and a clock time leaves the subtraction to the reader.
pub fn since(secs: u64, now: u64) -> String {
    use rusty_i18n::t;
    // A time in the future is a skewed clock, and "in 3 hours" would be
    // worse than saying the time cannot be used.
    let Some(elapsed) = now.checked_sub(secs) else {
        return t!("misc.clock-skew");
    };
    match elapsed {
        s if s < 90 => t!("misc.just-now"),
        s if s < 3_600 => t!("misc.minutes-ago", count = s / 60),
        s if s < 86_400 => t!("misc.hours-ago", count = s / 3_600),
        s => t!("misc.days-ago", count = s / 86_400),
    }
}

/// When a commit was made, as a history reads it: how long ago within the
/// week, the date after that. "412 d ago" is arithmetic nobody wanted.
pub fn commit_when(secs: u64) -> String {
    let now = now_secs();
    if now >= secs && now - secs < 7 * 86_400 {
        return since(secs, now);
    }
    local(secs)
        .map(|z| format!("{:04}-{:02}-{:02}", z.year(), z.month(), z.day()))
        .unwrap_or_default()
}

/// The whole local date and time.
pub fn full_time(secs: u64) -> String {
    local(secs)
        .map(|z| {
            format!(
                "{:04}-{:02}-{:02} {:02}:{:02}",
                z.year(),
                z.month(),
                z.day(),
                z.hour(),
                z.minute()
            )
        })
        .unwrap_or_default()
}

fn local(secs: u64) -> Option<jiff::Zoned> {
    let at = jiff::Timestamp::from_second(i64::try_from(secs).ok()?).ok()?;
    Some(at.to_zoned(jiff::tz::TimeZone::system()))
}

fn now_secs() -> u64 {
    u64::try_from(jiff::Timestamp::now().as_second()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Coarse, and a time from the future is said to be one rather than
    /// counted backwards.
    #[test]
    fn a_time_ago_is_coarse_and_never_negative() {
        use rusty_i18n::t;
        assert_eq!(since(1_000, 1_030), t!("misc.just-now"));
        assert_eq!(since(1_000, 1_000 + 7_200), t!("misc.hours-ago", count = 2));
        assert_eq!(since(1_000 + 60, 1_000), t!("misc.clock-skew"));
    }

    #[test]
    fn a_part_reads_in_the_units_its_datasheet_uses() {
        assert_eq!(bytes(400 * 1024), "400.0 KB");
        assert_eq!(bytes(4 * 1024 * 1024), "4.0 MB");
        assert_eq!(bytes(512), "512 B");
    }
}
