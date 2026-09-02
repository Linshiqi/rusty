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

/// The optional `@stamp` that closes a report's header, and what follows the
/// `]`: `@1234] 26=1` is `(Some(1234), " 26=1")`, `] 26=1` is `(None, " 26=1")`.
///
/// One reader for the three stamped reports — gpio, pwm, telemetry — so the
/// three cannot come to disagree about what a stamp looks like. `None` when
/// the header does not close or the stamp is not a number: a line that is
/// nearly a report is not one.
fn split_stamp(rest: &str) -> Option<(Option<u64>, &str)> {
    match rest.strip_prefix('@') {
        Some(stamped) => {
            let (stamp, tail) = stamped.split_once(']')?;
            Some((Some(stamp.trim().parse::<u64>().ok()?), tail))
        }
        None => Some((None, rest.strip_prefix(']')?)),
    }
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
    let (at_us, rest) = split_stamp(rest)?;
    let mut pins = Vec::new();
    for pair in rest.trim().split(',') {
        let (pin, level) = pair.trim().split_once('=')?;
        let pin: u8 = pin.trim().parse().ok()?;
        let level = matches!(level.trim(), "1" | "true" | "high");
        pins.push((pin, level));
    }
    (!pins.is_empty()).then_some(GpioReport { at_us, pins })
}

/// One `[rusty:pwm]` line: how hard a pin is being driven, not merely
/// whether it is high.
///
/// The analogue sibling of [`GpioReport`], and the reason a motor needs one.
/// A lamp is on or off and `[rusty:gpio]` says so; a motor is a *speed*, and
/// the speed lives in a duty cycle that a boolean channel cannot carry.
#[derive(Debug, Clone, PartialEq)]
pub struct PwmReport {
    /// Microseconds on the firmware's systimer, from `[rusty:pwm@1234]`, on
    /// the same terms as [`GpioReport::at_us`].
    pub at_us: Option<u64>,
    /// Pin, and the fraction of full drive on it — `0.0` to `1.0`.
    pub pins: Vec<(u8, f32)>,
}

/// Parse `[rusty:pwm] 5=0.75,6=0` — the duty the firmware set on each pin.
///
/// **Why a channel of its own rather than timing the `[rusty:gpio]` edges.**
/// That would be the more honest measurement, and it is not available: a
/// motor driven at anything from 1 to 20 kHz produces thousands of edges a
/// second, and reporting each one would flood the same serial line the
/// console is on and drown everything else the firmware has to say. So this
/// is reported per *change* rather than per cycle — one line when the
/// firmware writes a new duty, and silence while it holds.
///
/// **A fraction, not a percentage and not 0..255.** Firmware counts duty in
/// whatever its timer's bit width gives it, so any integer convention here
/// would be one more thing to get wrong at 3am; `0.0..=1.0` cannot be
/// misread. Values outside the range are clamped rather than dropped —
/// a `set_duty` that overshot its maximum is a real bug worth *seeing* as
/// full drive rather than as silence.
///
/// What this does not carry is a shaft speed. Nothing here measures a motor;
/// it reports what the firmware said it commanded, which is the same footing
/// the board view stands on everywhere else, and the panel says so.
pub fn parse_pwm_report(line: &str) -> Option<PwmReport> {
    let rest = line.trim().strip_prefix("[rusty:pwm")?;
    let (at_us, rest) = split_stamp(rest)?;
    let mut pins = Vec::new();
    for pair in rest.trim().split(',') {
        let (pin, duty) = pair.trim().split_once('=')?;
        let pin: u8 = pin.trim().parse().ok()?;
        let duty: f32 = duty.trim().parse().ok()?;
        if !duty.is_finite() {
            continue;
        }
        pins.push((pin, duty.clamp(0.0, 1.0)));
    }
    (!pins.is_empty()).then_some(PwmReport { at_us, pins })
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
    out.push_str("$version rusty simulator $end\n");
    out.push_str("$timescale 1 us $end\n");
    out.push_str("$scope module board $end\n");
    for (index, pin) in pins.iter().enumerate() {
        out.push_str(&format!("$var wire 1 {} GPIO{pin} $end\n", id_of(index)));
    }
    out.push_str("$upscope $end\n$enddefinitions $end\n");

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

/// Parse one `[rusty:pins]` line: who the pin levels are coming from.
///
/// Not a line any firmware writes — rusty emits it once per run, because the
/// answer is a property of the *emulator* rather than of the code being
/// simulated. With rusty's QEMU the levels come from the GPIO registers and a
/// LED lights because a pin went high; with Espressif's stock build the write
/// handler is an empty function, so a pin has no state and the board can only
/// repeat what the firmware printed about itself.
///
/// The board has to say which, in as many words. A user whose LED stays dark
/// needs to know whether to suspect their wiring or their `println!`, and the
/// two answers send them to completely different places.
pub fn parse_pin_source(line: &str) -> Option<PinSource> {
    let rest = line.trim().strip_prefix("[rusty:pins]")?;
    // The first word decides; the rest of the line is for whoever is reading
    // the dock. An unknown word is not "firmware" — it is a newer rusty
    // talking to an older frontend, and guessing would put a confident wrong
    // caption under the board.
    match rest.split_whitespace().next()? {
        "emulator" => Some(PinSource::Emulator),
        "firmware" => Some(PinSource::Firmware),
        _ => None,
    }
}

/// Where the board's pin levels come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinSource {
    /// The emulator's GPIO registers — true whatever the firmware says or
    /// does not say about itself.
    Emulator,
    /// The firmware's own `[rusty:gpio]` lines. Code that does not print
    /// tells the board nothing.
    Firmware,
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
    let (at_us, rest) = split_stamp(rest)?;

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

/// A sensor the firmware wants fed, as it announces itself.
///
/// The mirror of [`Param`], and for the same reason: a panel that invents a
/// sensor's name or its range is a panel that will one day inject 2000°/s
/// into a loop written for 250. The firmware declares; the panel offers
/// exactly what was declared and nothing else.
#[derive(Debug, Clone, PartialEq)]
pub struct SensorDef {
    pub name: String,
    /// How many numbers one sample carries — 3 for a gyro, 1 for a range
    /// finder. The injection line must carry exactly this many.
    pub components: u8,
    /// `rad/s`, `m/s^2`, `V` — decoration for the panel, and the thing that
    /// stops somebody feeding degrees to a loop that wanted radians.
    pub unit: Option<String>,
    /// The range each component accepts, when the firmware says.
    pub min: Option<f32>,
    pub max: Option<f32>,
}

/// Parse `[rusty:sensor] gyro=3 rad/s -35..35` — a sensor, how many numbers
/// it takes, and optionally its unit and range.
///
/// After the count the tokens are order-free: anything containing `..` is the
/// range, anything else is the unit. Order-free because this line is written
/// by hand in firmware and a format that fails on a swapped pair would fail
/// silently — the sensor simply would not appear, and nothing would say why.
pub fn parse_sensor_def(line: &str) -> Option<SensorDef> {
    let rest = line.trim().strip_prefix("[rusty:sensor]")?.trim();
    let (name, tail) = rest.split_once('=')?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }

    let mut parts = tail.split_whitespace();
    let components: u8 = parts.next()?.parse().ok()?;
    // A sample with no numbers in it is not a sample, and one with more
    // components than a line can sensibly carry is a typo rather than a
    // sensor. Refusing beats offering a card nothing can fill.
    if !(1..=8).contains(&components) {
        return None;
    }

    let (mut unit, mut min, mut max) = (None, None, None);
    for token in parts {
        match token.split_once("..") {
            Some((low, high)) => {
                min = low.trim().parse().ok();
                max = high.trim().parse().ok();
            }
            None => unit = Some(token.to_string()),
        }
    }
    Some(SensorDef {
        name: name.to_string(),
        components,
        unit,
        min,
        max,
    })
}

/// The line that injects one sensor sample — `Igyro=1.25,-0.5,0.02`.
///
/// **Every component on one line, always.** An IMU sample is atomic: split
/// across three lines, the firmware can read x from one moment and y from the
/// next, and an attitude fused from a torn sample is wrong in a way that
/// looks exactly like drift. One line, one sample, or the loop this exists to
/// serve cannot be trusted.
///
/// `I` for the same reason `B`, `P` and `S` are single letters: firmware
/// reads all four with the same three lines of parsing.
pub fn sensor_line(name: &str, values: &[f32]) -> String {
    let mut out = format!("I{name}=");
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&value.to_string());
    }
    out
}

/// The line that puts a raw ADC count on a pin — `A34=2048`.
///
/// **Counts, not volts.** rusty does not know your divider: a battery through
/// 100k/27k reads one number and the same battery direct reads another, and a
/// panel that claimed "3.7 V" while the firmware's arithmetic said otherwise
/// would be the confident wrong answer this workbench exists to avoid. The
/// firmware already owns that conversion; this hands it the number its ADC
/// would have produced.
pub fn analog_line(pin: u8, count: u16) -> String {
    format!("A{pin}={count}")
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

    #[test]
    fn a_sensor_declares_its_shape_and_optionally_its_range() {
        assert_eq!(
            parse_sensor_def("[rusty:sensor] gyro=3 rad/s -35..35"),
            Some(SensorDef {
                name: "gyro".to_string(),
                components: 3,
                unit: Some("rad/s".to_string()),
                min: Some(-35.0),
                max: Some(35.0),
            }),
        );

        // Order-free after the count: this line is typed by hand in firmware,
        // and a swapped pair must not make the sensor silently not appear.
        let swapped = parse_sensor_def("[rusty:sensor] gyro=3 -35..35 rad/s").expect("parsed");
        assert_eq!(swapped.unit.as_deref(), Some("rad/s"));
        assert_eq!(swapped.min, Some(-35.0));

        let bare = parse_sensor_def("[rusty:sensor] range=1").expect("a count is enough");
        assert_eq!((bare.components, bare.unit, bare.min), (1, None, None));
    }

    /// A sample with no numbers is not a sample, and a count in the hundreds
    /// is a typo. Both refuse rather than offering a card nothing can fill.
    #[test]
    fn a_component_count_outside_what_a_sample_can_be_is_refused() {
        assert_eq!(parse_sensor_def("[rusty:sensor] gyro=0"), None);
        assert_eq!(parse_sensor_def("[rusty:sensor] gyro=99"), None);
        assert_eq!(parse_sensor_def("[rusty:sensor] =3"), None);
        assert_eq!(parse_sensor_def("[rusty:sensor] gyro"), None);
        assert_eq!(parse_sensor_def("[rusty:param] kp=1"), None);
    }

    /// The whole sample on one line. Split across three, the firmware can
    /// read x from one moment and y from the next, and an attitude fused from
    /// a torn sample drifts in a way nothing in the code explains.
    #[test]
    fn a_sample_travels_whole() {
        assert_eq!(
            sensor_line("gyro", &[1.25, -0.5, 0.02]),
            "Igyro=1.25,-0.5,0.02"
        );
        assert_eq!(sensor_line("range", &[0.42]), "Irange=0.42");
        // The same single-letter shape the presses and knobs already use, so
        // firmware parses all of them with one branch.
        assert!(sensor_line("gyro", &[0.0]).starts_with('I'));
    }

    /// Counts rather than volts: rusty does not know the divider, and a panel
    /// that claimed a voltage the firmware's arithmetic disagreed with would
    /// be exactly the confident wrong answer this workbench refuses to give.
    #[test]
    fn an_analog_pin_carries_the_count_the_adc_would_have_produced() {
        assert_eq!(analog_line(34, 2048), "A34=2048");
        assert_eq!(analog_line(0, 0), "A0=0");
        assert_eq!(analog_line(39, 4095), "A39=4095");
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
    fn a_duty_report_carries_a_fraction_per_pin() {
        assert_eq!(
            parse_pwm_report("[rusty:pwm] 5=0.75,6=0"),
            Some(PwmReport {
                at_us: None,
                pins: vec![(5, 0.75), (6, 0.0)],
            }),
        );
        assert_eq!(
            parse_pwm_report("[rusty:pwm@4210] 5=1").map(|r| r.at_us),
            Some(Some(4210)),
        );
    }

    /// A `set_duty` that overshot its timer's maximum is a real bug, and one
    /// worth seeing as full drive rather than as silence — the motor really
    /// is pinned. Same for a negative, which is a wrapped subtraction.
    #[test]
    fn a_duty_outside_the_range_is_clamped_rather_than_dropped() {
        let over = parse_pwm_report("[rusty:pwm] 5=1.4").expect("kept");
        assert_eq!(over.pins, vec![(5, 1.0)]);
        let under = parse_pwm_report("[rusty:pwm] 5=-0.2").expect("kept");
        assert_eq!(under.pins, vec![(5, 0.0)]);
    }

    /// The two channels must not read each other's lines: a boolean pin
    /// report arriving as a duty of 1.0 would make every lit LED look like a
    /// motor at full throttle.
    #[test]
    fn the_duty_channel_and_the_pin_channel_stay_apart() {
        assert_eq!(parse_pwm_report("[rusty:gpio] 5=1"), None);
        assert_eq!(parse_gpio_report("[rusty:pwm] 5=0.5"), None);
        assert_eq!(parse_pwm_report("[rusty:pwm] nonsense"), None);
        assert_eq!(parse_pwm_report("I (44) boot: Loaded app"), None);
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

    #[test]
    fn the_pin_source_line_names_which_emulator_is_running() {
        use super::{PinSource, parse_pin_source};

        assert_eq!(
            parse_pin_source("[rusty:pins] emulator — pin state read from the GPIO registers"),
            Some(PinSource::Emulator),
        );
        assert_eq!(
            parse_pin_source("[rusty:pins] firmware"),
            Some(PinSource::Firmware),
        );

        // A word this frontend does not know is a *newer* rusty talking to it.
        // Falling back to "firmware" would put a confident wrong caption under
        // the board, which is the failure this line exists to prevent.
        assert_eq!(parse_pin_source("[rusty:pins] something-new"), None);

        // And an ordinary firmware line is not an announcement.
        assert_eq!(parse_pin_source("[rusty:gpio] 0=1"), None);
        assert_eq!(parse_pin_source("I (44) boot: x"), None);
    }
}
