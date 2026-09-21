//! How a run of the simulated board is planned, and the parts of the board
//! that are not the sheet itself (`sheet.rs` is the board).

use serde::{Deserialize, Serialize};

use super::{CommandPlan, Sheet, Symbol};

/// Something the emulator cannot do on this chip, said before the run so
/// that a hang or a silence is not blamed on the firmware.
///
/// A stable `kind` beside the English, as a `Problem` carries one: the
/// frontend says it in the reader's language, the CLI prints the English.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimLimit {
    /// `esp32-outdated`, `s3-unproven`, and the one diagnosis a run can
    /// end on, `cpu-fpu-off`.
    pub kind: String,
    pub text: String,
}

impl SimLimit {
    fn new(kind: &str, text: &str) -> Self {
        SimLimit {
            kind: kind.to_string(),
            text: text.to_string(),
        }
    }

    /// What the emulator this plan will boot is known not to do on `chip`.
    ///
    /// The C3 is the chip every model was written and proven against, and
    /// the ESP32 now has every one of them in its own layout — the pads'
    /// pulls in IO_MUX's pad-name order, the converter at `SENS`, the
    /// buses, LEDC's two halves, RMT's eight channels, every interrupt
    /// source reaching its handler (a timer's and a software interrupt's as
    /// well as a GPIO edge's), and the FPU on from reset as the silicon has
    /// it. So neither has a limit **with rusty's current emulator**, and
    /// the ESP32 has one with an older copy of it (`outdated_emulator`):
    /// there, each of those fails in a way of its own, and the panel's
    /// Upgrade is the fix. The text says what the copy one generation back
    /// cannot do and then what the ones before it could not either, since
    /// the plan knows only that the copy is not current.
    pub fn for_chip(chip: &str, outdated_emulator: bool) -> Vec<SimLimit> {
        match chip {
            "esp32" if outdated_emulator => vec![SimLimit::new(
                "esp32-outdated",
                "This emulator predates rusty's current ESP32 models, so on an ESP32 a timer's \
                 interrupt and a software interrupt never reach their handlers: an Embassy \
                 application's Timer::after() never returns, and anything waiting on an alarm \
                 waits for ever. Builds older still leave the FPU switched off at reset, where \
                 the silicon has it on, so an interrupt that does arrive faults inside its own \
                 context save and the run goes quiet — and lay the converter, buses, LEDC, RMT \
                 and pad pulls out as the C3's, so a read_oneshot() or a bus transaction waits \
                 for ever and every Pull::Up button reads as held down. Upgrade the emulator \
                 from this panel.",
            )],
            "esp32s3" => vec![SimLimit::new(
                "s3-unproven",
                "Nothing in rusty's emulator has been checked on the ESP32-S3: its pins, \
                 converter and buses are whatever Espressif's machine does, and the board \
                 shows only what the firmware prints.",
            )],
            _ => Vec::new(),
        }
    }

    /// A line the emulator printed that explains where a run stopped, so
    /// the explanation lands there rather than only on a panel somebody may
    /// not be looking at.
    ///
    /// `[rusty:cpu] coprocessor 0 is disabled …` is the emulator's own
    /// account, printed at the exception: something switched the FPU off —
    /// xtensa-lx-rt does inside every interrupt when esp-hal's
    /// `float-save-restore` is off — and a float followed. It is the CPU's,
    /// so it is not chip-specific. `divide by zero` on an ESP32 is what a
    /// guest spinning in the double-exception vector eventually produced on
    /// an emulator from before the FPU was on at reset, so it names the
    /// outdated emulator rather than anybody's code.
    pub fn explaining(chip: &str, line: &str) -> Option<SimLimit> {
        if line.contains("[rusty:cpu] coprocessor 0 is disabled") {
            return Some(SimLimit::new(
                "cpu-fpu-off",
                "The application switched the FPU off and then used it. CPENABLE bit 0 was \
                 cleared — xtensa-lx-rt clears it inside every interrupt unless esp-hal's \
                 float-save-restore feature is on, and firmware can clear it itself — so the \
                 floating-point instruction faulted, the exception handler that saves the \
                 floating-point registers faulted too, and the CPU is spinning in the \
                 double-exception vector. Keep float-save-restore on (it is esp-hal's default) \
                 or keep floats out of interrupt handlers.",
            ));
        }
        if chip == "esp32" && line.contains("divide by zero") {
            return SimLimit::for_chip("esp32", true).into_iter().next();
        }
        None
    }
}

/// A tool the simulator needs and cannot find, with the way to get it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimTool {
    pub name: String,
    pub install: String,
}

/// The emulator the plan will boot, once one was found — and whether it is
/// rusty's build, which models the GPIO registers, or Espressif's stock one,
/// whose GPIO write handler is empty: a pin read back there is always 0, so
/// `led.toggle()` followed by `led.is_set_high()` prints `false` for ever
/// and reads as a broken driver. The panel says which beside Run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Emulator {
    /// `qemu-system-riscv32` or `qemu-system-xtensa`.
    pub name: String,
    pub path: String,
    pub gpio_model: bool,
    /// Whether it also models the converter and both buses — the ADC, the
    /// I2C master and SPI2. An early build of rusty's has the pins and none
    /// of these, and firmware reading any of them there waits for ever in
    /// its own `read`, so the panel offers the upgrade for that too.
    #[serde(default)]
    pub peripherals: bool,
}

/// Serde's skip test for the common case: most parts are never turned.
pub(crate) fn is_upright(rot: &u16) -> bool {
    *rot == 0
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
    /// The emulator found, and whether it models the pins. Absent when it
    /// is among `missing`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emulator: Option<Emulator>,
    /// build → image → boot, each inspectable before anything runs.
    pub steps: Vec<CommandPlan>,
    /// Drawn beside the serial output when `.rusty/sim.toml` describes one,
    /// with the symbols its parts use resolved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board: Option<Sheet>,
    /// Every symbol the sheet may place: the built-in libraries, the parts
    /// imported from LCSC, and the project's own `.rusty/symbols/`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub library: Vec<Symbol>,
    /// Every part a sheet may answer for on the bus: the ones rusty ships
    /// and the ones the project declared in `.rusty/parts/`. The sliders
    /// under a sensor are its channels, so a part nobody declared gets no
    /// slider — the tunables' rule, for the same reason.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<crate::sensor::Spec>,
    /// Present when the right gdb is installed; the Debug button needs it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug: Option<SimDebug>,
    /// The gdb to install when `debug` is absent — same card, same one-click
    /// installer as every other missing tool, but it only gates Debug.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug_tool: Option<SimTool>,
    /// Things the plan wants read that are not refusals: a board file that
    /// names a chip other than the one this project builds for, a symbol
    /// library that could not be read, a migration from the first board
    /// format. Worth its own list because a note buried in `reason` would
    /// have to make the plan unsupported to be seen.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    /// What the emulator is known not to do on this chip ([`SimLimit`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub limits: Vec<SimLimit>,
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
            emulator: None,
            steps: Vec::new(),
            board: None,
            library: Vec::new(),
            parts: Vec::new(),
            debug: None,
            debug_tool: None,
            notes: Vec::new(),
            limits: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The C3 is the chip every model was proven on and has no limits; the
    /// others each name what they cost, and the one that ends a run is
    /// recognised in the emulator's own last words.
    #[test]
    fn each_chip_names_what_the_emulator_cannot_do_on_it() {
        // With rusty's current emulator neither the C3 nor the ESP32 has a
        // limit left; the S3 has never been checked, whatever the build.
        for outdated in [false, true] {
            assert!(SimLimit::for_chip("esp32c3", outdated).is_empty());
            assert_eq!(
                SimLimit::for_chip("esp32s3", outdated)[0].kind,
                "s3-unproven"
            );
        }
        assert!(SimLimit::for_chip("esp32", false).is_empty());
        // An older copy is the one case the ESP32 still has, because every
        // model it needs arrived after that copy was built.
        let outdated: Vec<String> = SimLimit::for_chip("esp32", true)
            .into_iter()
            .map(|l| l.kind)
            .collect();
        assert_eq!(outdated, ["esp32-outdated"]);

        // The emulator's own account, which names the cause at the
        // exception rather than leaving it to be inferred from what the
        // guest did afterwards. It is not chip-specific: CPENABLE is the
        // CPU's, and an S3 firmware hits the same wall the same way.
        let said = "[rusty:cpu] coprocessor 0 is disabled and the application used it at \
                    pc=0x400d14ad: something wrote CPENABLE with bit 0 clear, and the FPU is \
                    on from reset.";
        assert_eq!(
            SimLimit::explaining("esp32", said).map(|l| l.kind),
            Some("cpu-fpu-off".to_string())
        );
        assert_eq!(
            SimLimit::explaining("esp32s3", said).map(|l| l.kind),
            Some("cpu-fpu-off".to_string()),
            "the register is the CPU's, not the machine's"
        );

        // And what a guest spinning in the double-exception vector
        // eventually produced on an emulator that left the FPU off at reset
        // — which is an outdated emulator, not anybody's code.
        let fatal = "qemu-system-xtensa: Fatal error: divide by zero";
        assert_eq!(
            SimLimit::explaining("esp32", fatal).map(|l| l.kind),
            Some("esp32-outdated".to_string())
        );
        assert_eq!(
            SimLimit::explaining("esp32c3", fatal),
            None,
            "not this chip's symptom"
        );
        assert_eq!(
            SimLimit::explaining("esp32", "ets Jun  8 2016 00:22:57"),
            None
        );
    }

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
