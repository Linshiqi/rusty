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

/// One `[rusty:adc]` line: what the firmware's converter actually took off
/// a pin, in the converter's own counts.
///
/// The emulator says this; no firmware does. It is the return half of
/// `A<pin>=<counts>` — the host said what was on the pin, and this says what
/// was read from it and when — and it is what tells "the slider does
/// nothing" from "the firmware is not reading". Only rusty's build of QEMU
/// emits it, so its absence is the ordinary state of a run on Espressif's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdcReport {
    /// Microseconds on the emulator's virtual clock.
    pub at_us: Option<u64>,
    pub pin: u8,
    pub counts: u16,
}

/// Parse `[rusty:adc@1234] 3=2048`.
///
/// A conversion of a channel with no pin behind it is reported as
/// `adc1ch7=?`, which this refuses rather than inventing a pin for — the
/// line exists to be read by a human in the dock, and a panel has nothing
/// to do with it.
pub fn parse_adc_report(line: &str) -> Option<AdcReport> {
    let rest = line.trim().strip_prefix("[rusty:adc")?;
    let (at_us, rest) = split_stamp(rest)?;
    let (pin, counts) = rest.trim().split_once('=')?;
    Some(AdcReport {
        at_us,
        pin: pin.trim().parse().ok()?,
        counts: counts.trim().parse().ok()?,
    })
}

/// One `[rusty:i2c]` line: a transaction on the emulator's bus.
///
/// The emulator says this; no firmware does. It is what turns "the driver
/// returned an error" into "the address never acknowledged", and what lets
/// a panel show a display's traffic without the firmware being written to
/// narrate it. `verb` is `w`, `r`, `nak` or `full`, kept as text because
/// the set is the emulator's to grow and an unknown one should reach the
/// dock unchanged rather than be dropped as unparseable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct I2cReport {
    pub at_us: Option<u64>,
    pub address: u8,
    pub verb: String,
    pub bytes: Vec<u8>,
}

/// Parse `[rusty:i2c@1234] 3c w 00ae`.
pub fn parse_i2c_report(line: &str) -> Option<I2cReport> {
    let rest = line.trim().strip_prefix("[rusty:i2c")?;
    let (at_us, rest) = split_stamp(rest)?;
    let mut parts = rest.split_whitespace();
    let address = u8::from_str_radix(parts.next()?, 16).ok()?;
    let verb = parts.next()?.to_string();
    let hex = parts.next().unwrap_or("");
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    for pair in hex.as_bytes().chunks(2) {
        bytes.push(u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?);
    }
    Some(I2cReport {
        at_us,
        address,
        verb,
        bytes,
    })
}

/// One `[rusty:spi]` line: a transfer on the emulator's SPI2.
///
/// The wire's answer to [`I2cReport`], and simpler because SPI is: a chip
/// select instead of an address, and no acknowledgement to report. `verb` is
/// `w` for what went out and `r` for what came back; a full-duplex transfer
/// produces one of each.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpiReport {
    pub at_us: Option<u64>,
    pub select: u8,
    pub verb: String,
    pub bytes: Vec<u8>,
}

/// Parse `[rusty:spi@1234] 0 w aea501`.
pub fn parse_spi_report(line: &str) -> Option<SpiReport> {
    let rest = line.trim().strip_prefix("[rusty:spi")?;
    let (at_us, rest) = split_stamp(rest)?;
    let mut parts = rest.split_whitespace();
    let select = parts.next()?.parse().ok()?;
    let verb = parts.next()?.to_string();
    let hex = parts.next().unwrap_or("");
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    for pair in hex.as_bytes().chunks(2) {
        bytes.push(u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?);
    }
    Some(SpiReport {
        at_us,
        select,
        verb,
        bytes,
    })
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
    /// Pin, and what is on it.
    pub pins: Vec<(u8, Duty)>,
}

/// How hard a pin is being driven, and how fast it is being switched.
///
/// The fraction alone is what firmware narrating its own line can say, and
/// it is enough for a lamp and a motor: both answer to *how hard*. A servo
/// does not. Its horn follows the width of the high part of each cycle —
/// 1.5 ms is the middle whatever the period is — so a fraction without the
/// frequency beside it cannot be read as an angle at all. rusty's emulator
/// knows the carrier, because the timer that sets it is the one it models,
/// so it says it; `None` is the honest answer everywhere else.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Duty {
    /// The fraction of full drive — `0.0` to `1.0`.
    pub duty: f32,
    /// The carrier in hertz, when whoever reported it knew.
    pub hz: Option<f32>,
}

impl Duty {
    /// How long the pin is high in each cycle, in microseconds.
    pub fn pulse_us(&self) -> Option<f32> {
        let hz = self.hz?;
        (hz > 0.0).then(|| self.duty / hz * 1_000_000.0)
    }

    /// Where a hobby servo's horn stands, in degrees from 0 to 180, given
    /// the two pulse widths the part answers to.
    ///
    /// **The ends are the part's, not a constant here.** 500 and 2500
    /// microseconds are what most hobby servos take and 1000 to 2000 is the
    /// other common pair; the difference is forty degrees at each end,
    /// which is a horn against its stop rather than where the firmware
    /// asked. So the sheet says, and this is handed what it said.
    ///
    /// With no carrier the fraction is read straight, as it always was:
    /// firmware that prints its own `[rusty:pwm]` line is saying how hard,
    /// not how often, and reading that as a pulse width would put every
    /// hand-narrated servo hard against one end.
    pub fn servo_angle(&self, min_us: f32, max_us: f32) -> f32 {
        let span = max_us - min_us;
        match self.pulse_us() {
            Some(us) if span > 0.0 => ((us - min_us) / span).clamp(0.0, 1.0) * 180.0,
            _ => self.duty.clamp(0.0, 1.0) * 180.0,
        }
    }
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
/// **The carrier comes after an `@`, and only from something that knows
/// it.** `5=0.075@50` is a servo at the middle of its travel; `5=0.075` is
/// a pin driven at seven and a half percent and nothing about how often.
/// The emulator models the timer, so it can say; firmware narrating its own
/// line cannot, and must not be read as though it had.
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
        let (duty, hz) = match duty.trim().split_once('@') {
            Some((duty, hz)) => (
                duty,
                hz.trim().parse::<f32>().ok().filter(|h| h.is_finite()),
            ),
            None => (duty.trim(), None),
        };
        let duty: f32 = duty.trim().parse().ok()?;
        if !duty.is_finite() {
            continue;
        }
        pins.push((
            pin,
            Duty {
                duty: duty.clamp(0.0, 1.0),
                hz,
            },
        ));
    }
    (!pins.is_empty()).then_some(PwmReport { at_us, pins })
}

/// One `[rusty:rmt]` line: what a transmission put on a pin, as bytes.
///
/// RMT sends pulse codes, and every one-wire LED protocol — WS2812, SK6812,
/// WS2811 — carries a bit as the shape of one: a long high then a short low
/// is a one, and the other way round a zero. The emulator reads that shape
/// and hands over the bytes, so a host reading this does not have to know
/// anybody's timings. What it *does* have to know is what the bytes mean,
/// which is the part's business: three bytes a pixel, green first, for the
/// family above.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RmtReport {
    pub at_us: Option<u64>,
    /// The pin the matrix sends the channel to. A transmission the matrix
    /// sends nowhere is never reported.
    pub pin: u8,
    pub bytes: Vec<u8>,
    /// Bits beyond what one report carries. A strip longer than the
    /// emulator's buffer is said rather than silently cut, because a
    /// shortened strip and a short one look the same on a board.
    pub dropped: u32,
}

/// Parse `[rusty:rmt@63834] 8 100000002000000030`, and the `+12` a
/// transmission too long to carry whole ends with.
pub fn parse_rmt_report(line: &str) -> Option<RmtReport> {
    let rest = line.trim().strip_prefix("[rusty:rmt")?;
    let (at_us, rest) = split_stamp(rest)?;
    let mut parts = rest.split_whitespace();
    let pin: u8 = parts.next()?.parse().ok()?;
    let hex = parts.next()?;
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    for pair in hex.as_bytes().chunks(2) {
        bytes.push(u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?);
    }
    let dropped = parts
        .next()
        .and_then(|more| more.strip_prefix('+'))
        .and_then(|count| count.parse().ok())
        .unwrap_or(0);
    Some(RmtReport {
        at_us,
        pin,
        bytes,
        dropped,
    })
}

/// The colours a run of `[rusty:rmt]` bytes drives, as `(r, g, b)`.
///
/// Green, red, blue is the order the wire carries for the WS2812 family,
/// which is the one thing about these bytes that is not obvious and the one
/// thing everybody gets wrong first. A trailing part-pixel is dropped: two
/// bytes is a transmission caught mid-flight, not a colour.
pub fn strip_colours(bytes: &[u8]) -> Vec<(u8, u8, u8)> {
    bytes
        .as_chunks::<3>()
        .0
        .iter()
        .map(|[green, red, blue]| (*red, *green, *blue))
        .collect()
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

// ── The pin channel's inbound lines ───────────────────────────────────────
//
// What the host says to rusty's QEMU on the channel the pin reports come back
// on. Beside the console's messages because two of them are the same number
// said twice — `A3=2048` to firmware reading this protocol and to the
// converter the firmware actually samples — and a firmware reading the text
// and a firmware reading its ADC must not be shown different worlds.

/// `<pin>=<level>` — a level the host drives onto a pin, which unmodified
/// firmware reads through `GPIO_IN`.
pub fn pin_line(pin: u32, level: u8) -> String {
    format!("{pin}={level}\n")
}

/// `A<pin>=<counts>` — an analog value on a pin, in the converter's own
/// counts: the same spelling and the same number as [`analog_line`].
pub fn analog_pin_line(pin: u32, count: u16) -> String {
    format!("A{pin}={count}\n")
}

/// `i2c 68:3b=0102…` for each run of registers, without declaring the
/// device — what a sensor's moving reading writes.
pub fn bus_register_lines(address: u8, runs: &[(u8, Vec<u8>)]) -> Vec<String> {
    runs.iter()
        .map(|(at, bytes)| {
            let mut line = format!("i2c {address:02x}:{at:02x}=");
            for byte in bytes {
                line.push_str(&format!("{byte:02x}"));
            }
            line.push('\n');
            line
        })
        .collect()
}

/// One I2C device: `i2c 68=+` and then its registers.
///
/// The bare declaration first and always, even for a device with
/// registers. A device that only appeared through a register write would
/// not exist until it had one, and a display — which nobody reads from —
/// would then never be on the bus at all.
pub fn bus_lines(address: u8, runs: &[(u8, Vec<u8>)]) -> Vec<String> {
    let mut lines = vec![format!("i2c {address:02x}=+\n")];
    lines.extend(bus_register_lines(address, runs));
    lines
}

/// `spi <cs>=<hex>` — what a chip select answers with. A device with
/// nothing to say still needs a token the model can read: `-` is not hex,
/// and anything that is not hex clears the buffer.
pub fn wire_line(select: u8, miso: &[u8]) -> String {
    let mut line = format!("spi {select}=");
    for byte in miso {
        line.push_str(&format!("{byte:02x}"));
    }
    if miso.is_empty() {
        line.push('-');
    }
    line.push('\n');
    line
}

/// `B<pin>=<pressed>` — the board's button message, and nothing else.
///
/// Any non-zero value is pressed, because the message is a state and not a
/// count.
pub fn button_press(text: &str) -> Option<(u32, u8)> {
    let (pin, level) = text.trim().strip_prefix('B')?.split_once('=')?;
    let level: u8 = level.trim().parse().ok()?;
    Some((pin.trim().parse().ok()?, u8::from(level != 0)))
}

/// `A<pin>=<counts>` — an analog source on the board, in the converter's own
/// counts.
///
/// Deliberately not the potentiometer's `P34=128`. That message is rusty's
/// own eight-bit convention, and what a wiper at a given position converts to
/// depends on what its two ends are connected to; turning 128 into counts
/// would be asserting a rail-to-rail divider nobody stated. `A` already
/// carries the number the firmware's own ADC would have produced, so it needs
/// no conversion to reach the model — which is why it is the one that goes
/// down the pin channel.
pub fn analog_set(text: &str) -> Option<(u32, u16)> {
    let (pin, count) = text.trim().strip_prefix('A')?.split_once('=')?;
    Some((pin.trim().parse().ok()?, count.trim().parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two inbound messages a console line can carry to a pin as well.
    #[test]
    fn a_button_press_is_read_off_the_console_and_written_to_the_pin_channel() {
        assert_eq!(button_press("B14=1"), Some((14, 1)));
        assert_eq!(button_press("B14=0"), Some((14, 0)));
        assert_eq!(
            button_press(" B2 = 7 "),
            Some((2, 1)),
            "a level is a level: non-zero is high, whitespace is noise",
        );
        assert_eq!(
            button_press("P34=128"),
            None,
            "the potentiometer is analog, and a GPIO carries one bit"
        );
        assert_eq!(button_press("14=1"), None, "the prefix is the message");
        assert_eq!(button_press("B=1"), None);
        assert_eq!(button_press("Bx=1"), None);
        assert_eq!(button_press("Skp=8.5"), None, "a tunable is not a pin");

        assert_eq!(pin_line(14, 1), "14=1\n");
        assert_eq!(
            button_press("B14=1").map(|(pin, level)| pin_line(pin, level)),
            Some("14=1\n".to_string()),
            "the console message and the pin line name the same pin at the same level",
        );
    }

    /// The two messages that reach the emulator's pins are told apart by
    /// their first letter and by nothing else, so each has to refuse the
    /// other's traffic — and both have to refuse the potentiometer's `P`,
    /// which stays on the console because what a wiper converts to depends
    /// on what its ends are wired to.
    #[test]
    fn the_pin_channel_takes_buttons_and_analog_sources_and_nothing_else() {
        assert_eq!(button_press("B14=7"), Some((14, 1)), "any non-zero is down");
        assert_eq!(button_press("A3=2048"), None);

        assert_eq!(analog_set("A3=2048"), Some((3, 2048)));
        assert_eq!(analog_set(" A3=0 \n"), Some((3, 0)));
        assert_eq!(analog_set("B14=1"), None);
        assert_eq!(analog_set("P34=128"), None, "the pot stays on the console");
        assert_eq!(analog_set("A3=notanumber"), None);
        assert_eq!(analog_set("A3"), None);
    }

    /// A device is announced before its registers, always.
    #[test]
    fn a_bus_device_is_declared_before_the_registers_behind_it() {
        assert_eq!(
            bus_lines(0x68, &[(0x75, vec![0x68]), (0x3b, vec![0x01, 0x02])]),
            vec![
                "i2c 68=+\n".to_string(),
                "i2c 68:75=68\n".to_string(),
                "i2c 68:3b=0102\n".to_string(),
            ]
        );
        assert_eq!(bus_lines(0x3c, &[]), vec!["i2c 3c=+\n".to_string()]);
        assert_eq!(
            bus_register_lines(0x68, &[(0x3b, vec![0xff])]),
            vec!["i2c 68:3b=ff\n".to_string()],
            "a moving reading writes registers and declares nothing",
        );
    }

    /// A chip select with nothing to say still has to send a token: the
    /// model reads a run of hex and clears the buffer on anything else, so
    /// an empty line would be a device that kept the previous run's answer.
    #[test]
    fn a_chip_select_with_no_answer_still_says_so() {
        assert_eq!(wire_line(0, &[0x1a, 0x68]), "spi 0=1a68\n");
        assert_eq!(wire_line(1, &[]), "spi 1=-\n");
    }

    /// One value said twice, to two readers that must not be shown
    /// different worlds: the console's message and the pin channel's line
    /// carry the same number for the same pin.
    #[test]
    fn the_console_and_the_pin_channel_agree_about_an_analog_value() {
        let console = analog_line(3, 2048);
        assert_eq!(console, "A3=2048");
        assert_eq!(analog_pin_line(3, 2048), format!("{console}\n"));
        assert_eq!(analog_set(&console), Some((3, 2048)));
    }

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

    fn duty(duty: f32) -> Duty {
        Duty { duty, hz: None }
    }

    #[test]
    fn a_duty_report_carries_a_fraction_per_pin() {
        assert_eq!(
            parse_pwm_report("[rusty:pwm] 5=0.75,6=0"),
            Some(PwmReport {
                at_us: None,
                pins: vec![(5, duty(0.75)), (6, duty(0.0))],
            }),
        );
        assert_eq!(
            parse_pwm_report("[rusty:pwm@4210] 5=1").map(|r| r.at_us),
            Some(Some(4210)),
        );
    }

    /// The emulator knows the carrier because it models the timer that sets
    /// it, and firmware narrating its own line does not. Both spellings
    /// have to read, and the second must not be given a frequency it never
    /// claimed — a servo would then be drawn hard against one end.
    #[test]
    fn a_carrier_is_carried_when_it_is_there_and_absent_when_it_is_not() {
        let with = parse_pwm_report("[rusty:pwm@58311] 5=0.0750@50.0").expect("kept");
        assert_eq!(
            with.pins,
            vec![(
                5,
                Duty {
                    duty: 0.075,
                    hz: Some(50.0)
                }
            )]
        );
        assert_eq!(with.pins[0].1.pulse_us(), Some(1500.0));
        let without = parse_pwm_report("[rusty:pwm] 5=0.075").expect("kept");
        assert_eq!(without.pins[0].1.hz, None);
        assert_eq!(without.pins[0].1.pulse_us(), None);
    }

    /// A servo's middle is 1.5 ms whatever the period, and the ends are the
    /// part's to state. The reading with no carrier is the fraction, which
    /// is what the panel has always drawn for hand-narrated firmware.
    #[test]
    fn a_servos_angle_comes_from_the_pulse_width_when_there_is_one() {
        let middle = Duty {
            duty: 0.075,
            hz: Some(50.0),
        };
        assert!((middle.servo_angle(500.0, 2500.0) - 90.0).abs() < 0.01);
        // The same pulse against the other common pair of ends is another
        // fifty degrees along, which is why the ends are not a constant.
        assert!((middle.servo_angle(1000.0, 2000.0) - 90.0).abs() < 0.01);
        let low = Duty {
            duty: 0.05,
            hz: Some(50.0),
        };
        assert!((low.servo_angle(1000.0, 2000.0) - 0.0).abs() < 0.01);
        assert!((low.servo_angle(500.0, 2500.0) - 45.0).abs() < 0.01);
        // Past the ends is a horn against its stop, not an angle beyond it.
        let over = Duty {
            duty: 0.2,
            hz: Some(50.0),
        };
        assert!((over.servo_angle(500.0, 2500.0) - 180.0).abs() < 0.01);
        // And with nothing said about the carrier, the fraction reads as it
        // always did.
        assert!((duty(0.5).servo_angle(500.0, 2500.0) - 90.0).abs() < 0.01);
    }

    /// The bytes a strip's driver clocked out, and the pin they went to.
    /// The `+N` is a transmission longer than one report carries — said,
    /// because a strip shortened in silence looks like a shorter strip.
    #[test]
    fn a_strip_transmission_carries_its_pin_and_its_bytes() {
        assert_eq!(
            parse_rmt_report("[rusty:rmt@63834] 8 100000002000000030"),
            Some(RmtReport {
                at_us: Some(63834),
                pin: 8,
                bytes: vec![0x10, 0x00, 0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x30],
                dropped: 0,
            }),
        );
        let long = parse_rmt_report("[rusty:rmt@10] 2 0102030405060708090a0b0c +24").expect("kept");
        assert_eq!(long.dropped, 24);
        assert_eq!(parse_rmt_report("[rusty:rmt@10] 2 010"), None);
        assert_eq!(parse_rmt_report("[rusty:gpio@10] 2=1"), None);
    }

    /// Green first. Reading the bytes in the order they look like — red,
    /// green, blue — swaps two channels of every pixel, which is a picture
    /// that is plainly wrong and plainly working at the same time.
    #[test]
    fn a_strips_bytes_are_green_red_blue() {
        assert_eq!(
            strip_colours(&[0x10, 0x00, 0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x30]),
            vec![(0x00, 0x10, 0x00), (0x20, 0x00, 0x00), (0x00, 0x00, 0x30)],
        );
        // A part-pixel is not a colour.
        assert_eq!(strip_colours(&[0x10, 0x20]), vec![]);
    }

    /// A `set_duty` that overshot its timer's maximum is a real bug, and one
    /// worth seeing as full drive rather than as silence — the motor really
    /// is pinned. Same for a negative, which is a wrapped subtraction.
    #[test]
    fn a_duty_outside_the_range_is_clamped_rather_than_dropped() {
        let over = parse_pwm_report("[rusty:pwm] 5=1.4").expect("kept");
        assert_eq!(over.pins, vec![(5, duty(1.0))]);
        let under = parse_pwm_report("[rusty:pwm] 5=-0.2").expect("kept");
        assert_eq!(under.pins, vec![(5, duty(0.0))]);
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

    /// The emulator's account of what its converter handed the firmware.
    /// Counts, so it can be compared with the counts the host sent; and a
    /// channel with no pin behind it is refused rather than given one, since
    /// there is no pin the panel could put it on.
    #[test]
    fn an_adc_report_carries_the_pin_and_the_counts_it_read() {
        assert_eq!(
            parse_adc_report("[rusty:adc@2233716] 3=2048"),
            Some(AdcReport {
                at_us: Some(2_233_716),
                pin: 3,
                counts: 2048,
            })
        );
        assert_eq!(
            parse_adc_report("[rusty:adc] 0=0"),
            Some(AdcReport {
                at_us: None,
                pin: 0,
                counts: 0,
            })
        );
        assert_eq!(
            parse_adc_report("[rusty:adc@10] adc1ch7=?"),
            None,
            "a channel with no pin is not a pin"
        );
        assert_eq!(parse_adc_report("[rusty:gpio] 3=1"), None);
        assert_eq!(parse_adc_report("I (44) boot: Loaded app"), None);
    }

    /// A transaction on the bus, hex throughout, with the verb kept as text
    /// so a new one reaches the dock rather than being dropped.
    #[test]
    fn an_i2c_report_carries_the_address_the_verb_and_the_bytes() {
        assert_eq!(
            parse_i2c_report("[rusty:i2c@2233716] 3c w 00ae"),
            Some(I2cReport {
                at_us: Some(2_233_716),
                address: 0x3c,
                verb: "w".into(),
                bytes: vec![0x00, 0xae],
            })
        );
        assert_eq!(
            parse_i2c_report("[rusty:i2c@10] 68 nak"),
            Some(I2cReport {
                at_us: Some(10),
                address: 0x68,
                verb: "nak".into(),
                bytes: Vec::new(),
            }),
            "an address that answered nothing carries no bytes"
        );
        assert_eq!(
            parse_i2c_report("[rusty:i2c@10] 68 r 0102030405"),
            Some(I2cReport {
                at_us: Some(10),
                address: 0x68,
                verb: "r".into(),
                bytes: vec![1, 2, 3, 4, 5],
            })
        );
        assert_eq!(
            parse_i2c_report("[rusty:i2c@10] 68 r 010"),
            None,
            "half a byte is not a byte"
        );
        assert_eq!(parse_i2c_report("[rusty:adc@10] 3=2048"), None);
        assert_eq!(parse_i2c_report("I (44) boot: Loaded app"), None);
    }

    /// A transfer on the wire: a chip select in decimal, because it is a
    /// line number, and the bytes in hex like everything else.
    #[test]
    fn a_spi_report_carries_the_chip_select_and_the_bytes() {
        assert_eq!(
            parse_spi_report("[rusty:spi@2233716] 0 w aea501"),
            Some(SpiReport {
                at_us: Some(2_233_716),
                select: 0,
                verb: "w".into(),
                bytes: vec![0xae, 0xa5, 0x01],
            })
        );
        assert_eq!(
            parse_spi_report("[rusty:spi@10] 2 r 1a68"),
            Some(SpiReport {
                at_us: Some(10),
                select: 2,
                verb: "r".into(),
                bytes: vec![0x1a, 0x68],
            })
        );
        assert_eq!(parse_spi_report("[rusty:spi@10] 0 w aea50"), None);
        assert_eq!(parse_spi_report("[rusty:i2c@10] 68 r 68"), None);
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
        // The stock build's line carries a paragraph for the dock; the word
        // still decides.
        assert_eq!(
            parse_pin_source(
                "[rusty:pins] firmware — Espressif's stock QEMU: its GPIO write handler is empty"
            ),
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
