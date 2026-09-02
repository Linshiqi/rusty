//! The simulated board: what is on it, and how a run of it is planned.

use serde::{Deserialize, Serialize};

use super::CommandPlan;

/// A tool the simulator needs and cannot find, with the way to get it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimTool {
    pub name: String,
    pub install: String,
}

/// A display pin nobody has wired yet.
///
/// One value, named once: the file format and the wire model both need it,
/// and two spellings of 255 is how one of them ends up meaning something else.
pub const UNWIRED_PIN: u8 = 255;

fn unwired_pin() -> u8 {
    UNWIRED_PIN
}

/// Serde's skip test for the common case: most parts are never turned.
pub(crate) fn is_upright(rot: &u16) -> bool {
    *rot == 0
}

/// Where a part sits on the sheet, how it is turned, and how its wires run.
///
/// One struct rather than the same five fields on every part. They were copied
/// six times, comments and all, and the copies drifted the moment one of them
/// gained a field: `flip` was added to all six wire types and to *neither*
/// half of the file format, so mirroring a part survived until the project was
/// reopened and then silently was not there. A field added here reaches every
/// part or none of them.
///
/// A nested field rather than `#[serde(flatten)]`: flatten routes the whole
/// struct through serde's buffering path, and the frontend decodes this from a
/// JS value, where a buffered number is not reliably the integer `rot` needs.
/// The JSON shape is internal — both sides `use` this same type — so nesting
/// costs nothing and cannot misdecode.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Placement {
    /// Canvas position, when the editor has placed it. Absent means "lay it
    /// out automatically", which is what hand-written files get.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    /// User-drawn waypoints per wire, world coordinates. Empty means "route
    /// automatically". routes[0] belongs to pins[0] (or the only pin).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routes: Vec<Vec<(f64, f64)>>,
    /// Quarter turns on the sheet: 0, 90, 180 or 270 degrees. A schematic
    /// nobody can rotate is a diagram that fights its own wiring.
    #[serde(default, skip_serializing_if = "is_upright")]
    pub rot: u16,
    /// Mirrored left-to-right — what a part on the chip's right wants.
    /// Mirrored rather than turned because a 180° turn also reverses the stub
    /// order, and seven wires to a seven-segment then cross on the way in.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flip: bool,
}

/// One LED on the simulated board view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimLed {
    pub pin: u8,
    /// `green`, `blue`, `red`, `yellow` — the stylesheet's palette names.
    pub color: String,
    pub label: String,
    #[serde(default)]
    pub place: Placement,
}

/// A push button on the board. Pressing it sends `B<pin>=1` (and release
/// `=0`) into the firmware's UART through the simulator's stdin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimButton {
    pub pin: u8,
    pub label: String,
    #[serde(default)]
    pub place: Placement,
}

/// An RGB LED: three pins, one lens. The lit colour is the additive mix of
/// whichever channels the firmware reports high.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimRgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub label: String,
    #[serde(default)]
    pub place: Placement,
}

/// A seven-segment digit: seven GPIO pins, one per segment a..g. Lit
/// segment by segment from the same gpio report channel as every lamp —
/// the most honest display there is, because it is not a display at all,
/// just seven LEDs in a font.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimSeven {
    /// Segments a, b, c, d, e, f, g in order.
    pub pins: [u8; 7],
    pub label: String,
    #[serde(default)]
    pub place: Placement,
}

/// A small text screen fed by the `[rusty:disp]` serial channel — the
/// firmware prints what the screen shows. Stands in for OLED/LCD until a
/// protocol decoder exists; the caption on the panel says whose word it is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimDisplay {
    pub label: String,
    /// The I2C pins the module hangs on. 255 = not wired yet — old board
    /// files carry no pins at all, and an unwired screen still shows text.
    #[serde(default = "unwired_pin")]
    pub sda: u8,
    #[serde(default = "unwired_pin")]
    pub scl: u8,
    /// `routes` here is (sda, scl), in that order.
    #[serde(default)]
    pub place: Placement,
}

/// A potentiometer: a slider in the UI that sends `P<pin>=<0..255>` into
/// the firmware's UART as it moves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimPot {
    pub pin: u8,
    pub label: String,
    #[serde(default)]
    pub place: Placement,
}

/// An analog source on a pin: a battery through a divider, a thermistor, any
/// voltage the firmware reads with its ADC.
///
/// Distinct from [`SimPot`], which is a *knob a person turns* and sends 8
/// bits. This is a *voltage that is there*, at the resolution the chip's ADC
/// actually has — a 1S cell sagging from 4.2 V to 3.3 V under throttle is
/// four counts of an 8-bit range and eighty of a 12-bit one, and a low-battery
/// cutoff cannot be tested against four.
///
/// **Counts, not volts.** rusty does not know the divider on your board, so
/// it does not claim a voltage. [`Self::note`] is where you write what the
/// count means to *you*, and it is shown verbatim as your words, not rusty's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimAnalog {
    pub pin: u8,
    pub label: String,
    /// Full scale for this source. 4095 is the ESP32's 12-bit ADC; a part
    /// wired to something else can say so.
    #[serde(default = "full_scale")]
    pub max: u16,
    /// Where the slider sits when the board loads.
    #[serde(default)]
    pub start: u16,
    /// What the count means on this board — "4095 = 4.2 V through 100k/27k".
    /// Yours to write and yours to be right about; rusty only repeats it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default)]
    pub place: Placement,
}

fn full_scale() -> u16 {
    4095
}

/// A motor: a toy car's drive through an H-bridge, or a fan.
///
/// One part rather than two, because a fan *is* a motor with one direction —
/// and which one you have is a property of what you wired, not a mode to
/// pick from a menu. Wire only [`Self::pwm`] and it turns one way; wire the
/// two direction pins as well and it is an H-bridge.
///
/// The direction pins are ordinary GPIO and arrive on the boolean channel.
/// The speed cannot: a duty cycle is not a level, which is why
/// [`crate::protocol::parse_pwm_report`] exists at all.
///
/// **This shows commanded drive, never a measured shaft speed.** There is no
/// inertia here, no load, no back-EMF — a motor that has been told 40% shows
/// 40% the instant it is told, and a real one takes time to get there under
/// a load rusty knows nothing about. The panel says so rather than implying
/// a physics it does not have.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimMotor {
    /// The pin carrying the duty cycle — the enable leg of an H-bridge, or
    /// the fan's single control wire.
    #[serde(default = "unwired_pin")]
    pub pwm: u8,
    /// The H-bridge's two direction inputs. Both [`UNWIRED_PIN`] for a fan.
    #[serde(default = "unwired_pin")]
    pub in1: u8,
    #[serde(default = "unwired_pin")]
    pub in2: u8,
    pub label: String,
    /// `routes` here is (pwm, in1, in2), in that order.
    #[serde(default)]
    pub place: Placement,
}

/// What an H-bridge is doing, from its two direction inputs.
///
/// Worth naming rather than leaving as two booleans in the view, because the
/// table is the thing people get wrong: `1,1` is not "full speed", it is a
/// brake — both low-side transistors on, the winding shorted, the motor
/// fighting its own momentum. Someone who reaches for it expecting speed
/// gets a stop, and nothing in a datasheet page of timing diagrams says so
/// as plainly as a board that shows BRAKE.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Drive {
    Forward,
    Reverse,
    /// Both inputs low: the bridge is open and the motor freewheels.
    Coast,
    /// Both inputs high: the winding is shorted and the motor is held.
    Brake,
}

impl Drive {
    /// The H-bridge truth table, and the whole reason this type exists.
    pub fn from_inputs(in1: bool, in2: bool) -> Self {
        match (in1, in2) {
            (true, false) => Drive::Forward,
            (false, true) => Drive::Reverse,
            (false, false) => Drive::Coast,
            (true, true) => Drive::Brake,
        }
    }

    /// What the panel writes beside the rotor.
    pub fn label(self) -> &'static str {
        match self {
            Drive::Forward => "FWD",
            Drive::Reverse => "REV",
            Drive::Coast => "COAST",
            Drive::Brake => "BRAKE",
        }
    }

    /// Whether the shaft turns at all — a duty of 90% into a braked bridge
    /// still goes nowhere, and a rotor that spun anyway would be teaching
    /// the wrong thing.
    pub fn turns(self) -> bool {
        matches!(self, Drive::Forward | Drive::Reverse)
    }
}

/// A user-defined part from `.rusty/parts/*.toml` — how a device rusty never
/// heard of still gets drawn and driven. v1 parts behave as lamps on the
/// gpio report channel; richer behaviours grow on this same record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartDef {
    pub name: String,
    /// Glow hue, one of the palette names.
    pub color: String,
}

/// The board view beside the serial output, when the project describes one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimBoard {
    /// The chip whose pins the board is drawn with. Always the chip the
    /// project builds for — `.rusty/sim.toml` may name another, and then
    /// [`SimPlan::notes`] says so rather than the header of a part that is
    /// not being simulated getting drawn.
    pub chip: String,
    /// Where the devkit itself sits on the canvas — it is a part like any
    /// other, and a schematic whose chip cannot move is a poster.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kit_x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kit_y: Option<f64>,
    pub leds: Vec<SimLed>,
    #[serde(default)]
    pub buttons: Vec<SimButton>,
    #[serde(default)]
    pub rgbs: Vec<SimRgb>,
    #[serde(default)]
    pub sevens: Vec<SimSeven>,
    #[serde(default)]
    pub displays: Vec<SimDisplay>,
    #[serde(default)]
    pub pots: Vec<SimPot>,
    #[serde(default)]
    pub motors: Vec<SimMotor>,
    #[serde(default)]
    pub analogs: Vec<SimAnalog>,
}

/// Everything the frontend needs to attach a debugger to a frozen boot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimDebug {
    /// The full command line to type into the terminal: gdb, the ELF, and
    /// `target remote` — composed here so the frontend never builds paths.
    /// Kept for the terminal path, which is still the way to reach gdb's
    /// own REPL for anything the panel does not model.
    pub gdb_command: String,
    /// The image with the symbols in it — what the in-app debugger loads.
    #[serde(default)]
    pub elf: String,
    /// Where QEMU's gdbstub listens.
    #[serde(default = "gdbstub_port")]
    pub port: u16,
}

fn gdbstub_port() -> u16 {
    1234
}

/// How this project would be simulated, or exactly why it cannot be.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimPlan {
    pub supported: bool,
    /// Set when `supported` is false — the refusal, in actionable terms.
    pub reason: Option<String>,
    /// Tools to install before the steps can run.
    pub missing: Vec<SimTool>,
    /// build → image → boot, each inspectable before anything runs.
    pub steps: Vec<CommandPlan>,
    /// Drawn beside the serial output when `.rusty/sim.toml` describes one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board: Option<SimBoard>,
    /// User-defined parts from `.rusty/parts/`, offered in the library.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<PartDef>,
    /// Present when the right gdb is installed; the Debug button needs it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug: Option<SimDebug>,
    /// The gdb to install when `debug` is absent — same card, same one-click
    /// installer as every other missing tool, but it only gates Debug.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug_tool: Option<SimTool>,
    /// Things the plan wants read that are not refusals: a board file that
    /// names a chip other than the one this project builds for, most
    /// obviously. Worth its own list because a note buried in `reason` would
    /// have to make the plan unsupported to be seen.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

impl SimPlan {
    /// A plan that cannot run, and says why.
    ///
    /// One constructor rather than the same eight-field literal at every
    /// refusal: the third copy is the one that forgets to reset a field when
    /// the struct grows one.
    pub fn refused(reason: impl Into<String>) -> Self {
        SimPlan {
            supported: false,
            reason: Some(reason.into()),
            missing: Vec::new(),
            steps: Vec::new(),
            board: None,
            parts: Vec::new(),
            debug: None,
            debug_tool: None,
            notes: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The H-bridge table, which is the one piece of real hardware knowledge
    /// this type carries. `1,1` is the entry worth having a test for: it is
    /// the one people reach for expecting full speed, and it is a brake.
    #[test]
    fn both_inputs_high_is_a_brake_and_not_full_speed() {
        assert_eq!(Drive::from_inputs(true, true), Drive::Brake);
        assert!(!Drive::Brake.turns(), "a braked bridge holds the shaft");

        assert_eq!(Drive::from_inputs(false, false), Drive::Coast);
        assert!(!Drive::Coast.turns(), "an open bridge freewheels");

        assert_eq!(Drive::from_inputs(true, false), Drive::Forward);
        assert_eq!(Drive::from_inputs(false, true), Drive::Reverse);
        assert!(Drive::Forward.turns() && Drive::Reverse.turns());
    }

    /// Reversing is swapping the two inputs, and nothing else. Firmware that
    /// drives one pin and leaves the other alone gets brake or coast rather
    /// than the reverse it wanted, which is exactly the mistake the board is
    /// meant to make visible.
    #[test]
    fn reverse_is_the_mirror_of_forward() {
        for (a, b) in [(true, false), (false, true)] {
            let one = Drive::from_inputs(a, b);
            let other = Drive::from_inputs(b, a);
            assert_ne!(one, other);
            assert!(one.turns() && other.turns());
        }
    }

    /// A refusal carries its reason and nothing else — no steps to run, no
    /// board to draw, no debugger to offer.
    #[test]
    fn a_refused_plan_is_unsupported_and_names_why() {
        let plan = SimPlan::refused("no chip");
        assert!(!plan.supported);
        assert_eq!(plan.reason.as_deref(), Some("no chip"));
        assert!(plan.steps.is_empty() && plan.board.is_none() && plan.debug.is_none());
    }
}
