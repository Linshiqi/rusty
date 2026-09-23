//! The chips the emulator models and where this machine keeps what a plan
//! runs: the emulator — the most capable copy, not the first — the
//! debuggers, and QEMU's data directory.

use std::path::{Path, PathBuf};

use super::models::models_carried;
use crate::model::EmbeddedProject;
use crate::tools;

/// Chips Espressif's QEMU actually models, with the system emulator each
/// needs. Kept small and honest — c6/h2/p4 have no machine model yet.
pub(super) const MACHINES: &[(&str, &str)] = &[
    ("esp32c3", "qemu-system-riscv32"),
    ("esp32", "qemu-system-xtensa"),
    ("esp32s3", "qemu-system-xtensa"),
];

/// What the machine has that a plan depends on, resolved once and handed in.
///
/// Handed in rather than read inside [`plan`] so a plan can be tested against
/// a directory a test made: the positive half of "the debugger reads the ELF
/// the run built" needs a gdb to exist, and the machine running the test may
/// have none — CI has none. [`Machine::here`] is what the app uses.
pub(crate) struct Machine {
    /// The data directory's `tools/`, where the installer unpacks QEMU and the
    /// debuggers. `None` when there is no data directory.
    pub(super) tools: Option<PathBuf>,
    /// The tools the installer shipped beside the app, searched after
    /// `tools` — rusty's own QEMU, for a machine that never downloaded one.
    pub(super) bundled: Option<PathBuf>,
    /// `CARGO_TARGET_DIR`, which outranks `[build] target-dir` for the cargo
    /// this plan will spawn — it inherits rusty's environment.
    pub(super) target_dir: Option<String>,
}

impl Machine {
    pub(crate) fn here() -> Self {
        Machine {
            tools: tools::data_tools_dir(),
            bundled: tools::bundled_dir(),
            target_dir: std::env::var("CARGO_TARGET_DIR")
                .ok()
                .filter(|dir| !dir.trim().is_empty()),
        }
    }

    /// The emulator to run, which is the **most capable copy** rather than
    /// the first one on the ladder.
    ///
    /// For every other tool the first copy found wins, because somebody put
    /// it there. The emulator is the one binary whose copies are told apart
    /// by what they can do, and the copy in the data directory is usually
    /// rusty's own download from whenever it was installed: a release from
    /// before the converter and the buses were modelled beat the current
    /// build sitting in the bundle, and firmware reading a knob hung in its
    /// own `read_oneshot()` with the right emulator installed one directory
    /// away.
    ///
    /// Generations pile up: a build somebody installed a year ago sits in
    /// the data directory, the installer's sits in the bundle, and each
    /// carries the models of its own moment. Taking the first found put a
    /// GPIO-only build in front of a bundled one with the converter and
    /// both buses — the v0.6.46 bug in a second costume, and the same
    /// symptom: a `read_oneshot()` that never returns. Ranked by what each
    /// one actually carries, with the ladder breaking ties, the answer is
    /// right whatever is lying about.
    pub(super) fn find_emulator(&self, binary: &str) -> Option<PathBuf> {
        let roots: Vec<PathBuf> = [self.tools.clone(), self.bundled.clone()]
            .into_iter()
            .flatten()
            .collect();
        let found = tools::candidates(binary, &roots);
        let mut best: Option<(usize, &PathBuf)> = None;
        for path in &found {
            let carried = models_carried(path);
            // `>`, not `>=`: an equal copy earlier on the ladder stays,
            // which is what makes a user's own install win over the bundle.
            if best.is_none_or(|(most, _)| carried > most) {
                best = Some((carried, path));
            }
        }
        best.map(|(_, path)| path.clone())
    }

    pub(super) fn find(&self, binary: &str) -> Option<PathBuf> {
        let roots: Vec<PathBuf> = [self.tools.clone(), self.bundled.clone()]
            .into_iter()
            .flatten()
            .collect();
        tools::find_in_roots(binary, &roots)
    }
}

/// QEMU's data directory beside an emulator binary — `bin/../share/qemu`,
/// the layout both Espressif's package and rusty's use — when it is there.
/// The ROM images live in it, and a QEMU that cannot find it boots nothing.
pub(super) fn qemu_data_dir(emulator: &Path) -> Option<PathBuf> {
    let data = emulator.parent()?.parent()?.join("share").join("qemu");
    data.is_dir().then_some(data)
}

/// The gdb that can debug this project's chip, if it is installed.
///
/// Architecture decides: an Xtensa gdb cannot debug a RISC-V image, and the
/// error it produces names neither the chip nor the fix.
pub fn gdb_for(project: &EmbeddedProject) -> Option<PathBuf> {
    let xtensa = project
        .configured_target
        .as_deref()
        .is_some_and(|t| t.starts_with("xtensa"));
    find_gdb(xtensa, &Machine::here())
}

/// The debugger for one architecture, by the same ladder every other binary
/// is found with. The Xtensa build is named after the chip family it was
/// built for rather than the archive it came in.
pub(super) fn find_gdb(xtensa: bool, machine: &Machine) -> Option<PathBuf> {
    machine.find(if xtensa {
        "xtensa-esp32-elf-gdb"
    } else {
        "riscv32-esp-elf-gdb"
    })
}
