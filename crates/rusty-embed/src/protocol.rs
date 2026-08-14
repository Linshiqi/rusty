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
    out.push_str(
        "$version rusty simulator $end
",
    );
    out.push_str(
        "$timescale 1 us $end
",
    );
    out.push_str(
        "$scope module board $end
",
    );
    for (index, pin) in pins.iter().enumerate() {
        out.push_str(&format!(
            "$var wire 1 {} GPIO{pin} $end
",
            id_of(index)
        ));
    }
    out.push_str(
        "$upscope $end
$enddefinitions $end
",
    );

    let mut last_stamp: Option<u64> = None;
    for (at, pin, level) in events {
        if last_stamp != Some(*at) {
            out.push_str(&format!(
                "#{at}
"
            ));
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

/// One `[rusty:tel]` line: named numeric channels at a moment.
///
/// The analog sibling of [`GpioReport`], and the reason the board view is not
/// the whole story. A pin is on or off; a gyro rate, a PID term or a motor
/// output is a number, and what you need to see is its *shape over time*.
/// Firmware that prints one of these per control loop gets a rolling plot
/// without a debugger, which is the only way to watch a loop that cannot be
/// stopped — stopping a flight controller means the craft falls.
#[derive(Debug, Clone, PartialEq)]
pub struct Telemetry {
    /// Microseconds on the firmware's clock, from `[rusty:tel@1234]`. The
    /// same contract as the pin reports: `None` means the consumer must fall
    /// back to arrival time and say so.
    pub at_us: Option<u64>,
    /// Channel name to value, in the order the firmware wrote them.
    pub channels: Vec<(String, f32)>,
}

/// Parse `[rusty:tel] gyro_x=1.25,pid_p=-0.5` or the `@`-stamped form.
///
/// Channel names are whatever the firmware calls them — no registry, no
/// declaration step. A plot of a channel nobody predicted is exactly the
/// point: you add a `println!` and watch it, the way a `printf` is added
/// today, except the result is a curve rather than a wall of numbers.
///
/// A value that does not parse drops that channel rather than the line: one
/// `NaN`-printing sensor must not take the other eleven with it.
pub fn parse_telemetry(line: &str) -> Option<Telemetry> {
    let rest = line.trim().strip_prefix("[rusty:tel")?;
    let (at_us, rest) = match rest.strip_prefix('@') {
        Some(stamped) => {
            let (stamp, tail) = stamped.split_once(']')?;
            (Some(stamp.trim().parse::<u64>().ok()?), tail)
        }
        None => (None, rest.strip_prefix(']')?),
    };

    let channels: Vec<(String, f32)> = rest
        .split(',')
        .filter_map(|field| {
            let (name, value) = field.split_once('=')?;
            let name = name.trim();
            if name.is_empty() {
                return None;
            }
            Some((name.to_string(), value.trim().parse::<f32>().ok()?))
        })
        .collect();
    (!channels.is_empty()).then_some(Telemetry { at_us, channels })
}

/// A tunable the firmware exposes, as it announces itself.
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub value: f32,
    /// The range the firmware will accept, when it says. A slider needs
    /// bounds, and bounds invented by the panel are how somebody sends a
    /// gain of 500 to a motor loop.
    pub min: Option<f32>,
    pub max: Option<f32>,
}

/// Parse `[rusty:param] pid_roll_p=12.5 0..50` — value, and optionally the
/// range the firmware accepts.
///
/// The firmware announces its own tunables, so the panel needs no config
/// file and cannot drift from the binary that is actually running. Re-sending
/// the line after a change is how the firmware confirms what it took, which
/// is not always what was asked for: a clamp is information.
pub fn parse_param(line: &str) -> Option<Param> {
    let rest = line.trim().strip_prefix("[rusty:param]")?.trim();
    let (name, tail) = rest.split_once('=')?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }

    let mut parts = tail.split_whitespace();
    let value = parts.next()?.parse::<f32>().ok()?;
    let (min, max) = match parts.next().and_then(|range| range.split_once("..")) {
        Some((low, high)) => (low.trim().parse().ok(), high.trim().parse().ok()),
        None => (None, None),
    };
    Some(Param {
        name: name.to_string(),
        value,
        min,
        max,
    })
}

/// The line that sets a parameter, for writing into the firmware's serial
/// input — `Spid_roll_p=12.5`.
///
/// One letter and a name, the shape `B14=1` and `P34=128` already use, so a
/// firmware that reads one reads all three with the same three lines of
/// parsing. Tuning without reflashing is the difference between a change
/// costing thirty seconds and costing a build, a flash and a re-arm.
pub fn set_param_line(name: &str, value: f32) -> String {
    format!("S{name}={value}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telemetry_carries_named_channels_and_a_stamp() {
        assert_eq!(
            parse_telemetry("[rusty:tel@4210] gyro_x=1.25,gyro_y=-0.5"),
            Some(Telemetry {
                at_us: Some(4210),
                channels: vec![("gyro_x".to_string(), 1.25), ("gyro_y".to_string(), -0.5),],
            }),
        );
        // Unstamped is legal; the consumer then times by arrival and says so.
        assert_eq!(
            parse_telemetry("[rusty:tel] throttle=0.5").map(|t| t.at_us),
            Some(None),
        );
        assert_eq!(parse_telemetry("[rusty:gpio] 4=1"), None);
        assert_eq!(parse_telemetry("just a log line"), None);
    }

    /// One bad channel must not cost the others: a sensor printing `nan` in
    /// the middle of twelve is the normal case, not the exception.
    #[test]
    fn an_unparseable_channel_drops_itself_and_nothing_else() {
        let parsed = parse_telemetry("[rusty:tel] a=1.0,b=oops,c=3.0").expect("kept the line");
        assert_eq!(
            parsed.channels,
            vec![("a".to_string(), 1.0), ("c".to_string(), 3.0)],
        );
        // But a line with nothing usable in it is not a sample.
        assert_eq!(parse_telemetry("[rusty:tel] b=oops"), None);
    }

    #[test]
    fn a_parameter_announces_its_value_and_optionally_its_range() {
        assert_eq!(
            parse_param("[rusty:param] pid_roll_p=12.5 0..50"),
            Some(Param {
                name: "pid_roll_p".to_string(),
                value: 12.5,
                min: Some(0.0),
                max: Some(50.0),
            }),
        );
        let bare = parse_param("[rusty:param] hover_throttle=0.42").expect("no range is legal");
        assert_eq!((bare.min, bare.max), (None, None), "and invents none");
        assert_eq!(parse_param("[rusty:param] broken"), None);
    }

    /// The write side is the same shape as the presses and knobs that came
    /// before it, so firmware parses all three the same way.
    #[test]
    fn setting_a_parameter_uses_the_family_the_firmware_already_reads() {
        assert_eq!(set_param_line("pid_roll_p", 12.5), "Spid_roll_p=12.5");
        assert_eq!(set_param_line("rate", -1.0), "Srate=-1");
    }

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
