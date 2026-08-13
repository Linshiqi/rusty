//! The serial protocol the simulated board speaks.
//!
//! One line, one fact, both directions. The firmware announces what it set
//! (`[rusty:gpio] 26=1,27=0`) and what it wants shown (`[rusty:disp] hello`);
//! the panel injects presses (`B14=1`) and knob positions (`P34=128`) into
//! the same serial line. Parsing lives here rather than in `model` because
//! the wire types describe *what crosses the IPC boundary*, and this
//! describes what crosses the *serial* one — a different contract, with a
//! different audience: anyone writing firmware for the simulator.
//!
//! Compiled unconditionally: the frontend parses these lines as they stream
//! past, so nothing here may touch IO.

/// Parse one serial line of the firmware's pin reports.
///
/// The convention is `[rusty:gpio] 26=1,27=0` — the firmware announcing what
/// it just set. The board view mirrors the firmware's word, and says so; the
/// QEMU peripheral models here do not expose register readback to do better.
pub fn parse_gpio_report(line: &str) -> Option<Vec<(u8, bool)>> {
    let rest = line.trim().strip_prefix("[rusty:gpio]")?;
    let mut out = Vec::new();
    for pair in rest.trim().split(',') {
        let (pin, level) = pair.trim().split_once('=')?;
        let pin: u8 = pin.trim().parse().ok()?;
        let level = matches!(level.trim(), "1" | "true" | "high");
        out.push((pin, level));
    }
    (!out.is_empty()).then_some(out)
}

/// Parse one `[rusty:disp]` line: the text the firmware wants shown.
/// An empty payload clears the screen.
pub fn parse_display_report(line: &str) -> Option<String> {
    let rest = line.trim().strip_prefix("[rusty:disp]")?;
    Some(rest.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpio_reports_parse_and_reject_noise() {
        assert_eq!(
            parse_gpio_report("[rusty:gpio] 26=1,27=0"),
            Some(vec![(26, true), (27, false)]),
        );
        assert_eq!(parse_gpio_report("  [rusty:gpio] 4=high "), Some(vec![(4, true)]));
        assert_eq!(parse_gpio_report("I (44) boot: Loaded app"), None);
        assert_eq!(parse_gpio_report("[rusty:gpio] nonsense"), None);
    }

    #[test]
    fn display_reports_carry_their_text() {
        use super::parse_display_report;
        assert_eq!(
            parse_display_report("[rusty:disp] tick 42"),
            Some("tick 42".to_string()),
        );
        assert_eq!(parse_display_report("[rusty:disp]"), Some(String::new()));
        assert_eq!(parse_display_report("I (44) boot: x"), None);
    }
}
