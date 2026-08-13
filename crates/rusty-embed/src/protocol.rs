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

/// One `[rusty:gpio]` line, parsed: which pins changed, and — when the
/// firmware stamped the line — the moment on its own clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpioReport {
    /// Microseconds on the firmware's systimer, from `[rusty:gpio@1234]`.
    /// `None` for the unstamped form; the consumer falls back to its own
    /// clock and must say so — mixing two time bases silently is how a
    /// waveform lies.
    pub at_us: Option<u64>,
    pub pins: Vec<(u8, bool)>,
}

/// Parse one serial line of the firmware's pin reports.
///
/// Two spellings: `[rusty:gpio] 26=1,27=0` and `[rusty:gpio@12345] 26=1` —
/// the `@` carries the firmware's systimer in microseconds, which is what
/// makes a waveform panel honest about *when* rather than merely *that*.
/// The board view mirrors the firmware's word either way; the QEMU
/// peripheral models here do not expose register readback to do better.
pub fn parse_gpio_report(line: &str) -> Option<GpioReport> {
    let rest = line.trim().strip_prefix("[rusty:gpio")?;
    let (at_us, rest) = match rest.strip_prefix('@') {
        Some(stamped) => {
            let (stamp, tail) = stamped.split_once(']')?;
            (Some(stamp.trim().parse::<u64>().ok()?), tail)
        }
        None => (None, rest.strip_prefix(']')?),
    };
    let mut pins = Vec::new();
    for pair in rest.trim().split(',') {
        let (pin, level) = pair.trim().split_once('=')?;
        let pin: u8 = pin.trim().parse().ok()?;
        let level = matches!(level.trim(), "1" | "true" | "high");
        pins.push((pin, level));
    }
    (!pins.is_empty()).then_some(GpioReport { at_us, pins })
}

/// A captured trace as a Value Change Dump, the format every waveform tool
/// opens — PulseView, GTKWave, Surfer.
///
/// Events are `(microseconds, pin, level)` and must be time-ordered; pins
/// appear in the header in first-seen order. Two events on the same
/// timestamp share one `#` block, as the format expects.
pub fn to_vcd(events: &[(u64, u8, bool)]) -> String {
    let mut pins: Vec<u8> = Vec::new();
    for (_, pin, _) in events {
        if !pins.contains(pin) {
            pins.push(*pin);
        }
    }

    // VCD identifiers are printable ASCII; one char each is plenty for the
    // pin count a board has.
    let id_of = |index: usize| -> char { (b'!' + index as u8) as char };

    let mut out = String::new();
    out.push_str("$version rusty simulator $end
");
    out.push_str("$timescale 1 us $end
");
    out.push_str("$scope module board $end
");
    for (index, pin) in pins.iter().enumerate() {
        out.push_str(&format!("$var wire 1 {} GPIO{pin} $end
", id_of(index)));
    }
    out.push_str("$upscope $end
$enddefinitions $end
");

    let mut last_stamp: Option<u64> = None;
    for (at, pin, level) in events {
        if last_stamp != Some(*at) {
            out.push_str(&format!("#{at}
"));
            last_stamp = Some(*at);
        }
        let index = pins.iter().position(|p| p == pin).unwrap_or(0);
        out.push_str(&format!(
            "{}{}
",
            if *level { '1' } else { '0' },
            id_of(index),
        ));
    }
    out
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
            Some(GpioReport {
                at_us: None,
                pins: vec![(26, true), (27, false)],
            }),
        );
        assert_eq!(
            parse_gpio_report("  [rusty:gpio] 4=high "),
            Some(GpioReport {
                at_us: None,
                pins: vec![(4, true)],
            }),
        );
        assert_eq!(parse_gpio_report("I (44) boot: Loaded app"), None);
        assert_eq!(parse_gpio_report("[rusty:gpio] nonsense"), None);
    }

    #[test]
    fn a_stamped_report_carries_its_microseconds() {
        assert_eq!(
            parse_gpio_report("[rusty:gpio@1500000] 26=0,27=1"),
            Some(GpioReport {
                at_us: Some(1_500_000),
                pins: vec![(26, false), (27, true)],
            }),
        );
        // A mangled stamp is noise, not a zero-time event.
        assert_eq!(parse_gpio_report("[rusty:gpio@abc] 26=1"), None);
        assert_eq!(parse_gpio_report("[rusty:gpio@] 26=1"), None);
    }

    #[test]
    fn vcd_output_is_what_pulseview_expects() {
        let events = [
            (1000, 26, true),
            (1000, 27, false),
            (501_000, 26, false),
            (501_000, 27, true),
        ];
        let vcd = to_vcd(&events);
        assert!(vcd.contains("$timescale 1 us $end"));
        assert!(vcd.contains("$var wire 1 ! GPIO26 $end"));
        assert!(vcd.contains("$var wire 1 \" GPIO27 $end"));
        // One # block per timestamp, both changes inside it.
        assert!(vcd.contains("#1000\n1!\n0\""));
        assert!(vcd.contains("#501000\n0!\n1\""));
        assert_eq!(vcd.matches("#1000\n").count(), 1);
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
