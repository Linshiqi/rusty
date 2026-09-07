//! How a run of the simulated board is planned, and the parts of the board
//! that are not the sheet itself (`sheet.rs` is the board).

use serde::{Deserialize, Serialize};

use super::{CommandPlan, Sheet, Symbol};

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
