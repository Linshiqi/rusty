//! Wokwi-style simulation, on Espressif's QEMU.
//!
//! Local-first deliberately: Espressif ships QEMU builds with ESP machine
//! models (`-M esp32c3` and friends), which boot the very image `espflash`
//! would put on a real board — ROM, second-stage bootloader, partition
//! table, app — and speak UART on stdio. No account, no cloud, no token.
//!
//! The loop is three commands, each inspectable in the panel before it runs:
//!
//! 1. `cargo build --release` — the project's own toolchain does the work.
//! 2. `espflash save-image --merge` — a bootable 4MB flash image, the same
//!    bytes a device would hold.
//! 3. `qemu-system-<arch> -M <chip> -nographic` — serial streams back into
//!    the dock until stopped.
//!
//! Refusals name what is missing and how to get it. A chip QEMU has no
//! machine model for is refused with the list of ones it has — a plausible
//! "it might work" would cost someone an afternoon.

use std::path::{Path, PathBuf};

use crate::config;
use crate::install::GDB_RELEASE;
use crate::model::{
    CommandPlan, EmbeddedProject, PartDef, SimBoard, SimButton, SimDebug, SimDisplay, SimLed,
    SimPlan, SimPot, SimRgb, SimSeven, SimTool, UNWIRED_PIN,
};
use crate::tools::{exe, home_dir, on_path};

/// Chips Espressif's QEMU actually models, with the system emulator each
/// needs. Kept small and honest — c6/h2/p4 have no machine model yet.
const MACHINES: &[(&str, &str)] = &[
    ("esp32c3", "qemu-system-riscv32"),
    ("esp32", "qemu-system-xtensa"),
    ("esp32s3", "qemu-system-xtensa"),
];

/// Everything needed to simulate `project`, or exactly why not.
///
/// `debug` changes the build, not just the QEMU flags: a release build has
/// no code on many lines, so gdb moves a breakpoint to the next line that
/// does, and the margin ends up marking a line execution never reaches.
/// Debug runs build unoptimised — measured at 284 KB against release's
/// 85 KB on the demo project, which is 7% of a 4 MB flash and the right
/// trade when the point is to stop where you clicked.
pub fn plan(project: &EmbeddedProject, debug: bool) -> SimPlan {
    let Some(chip) = project.chip.as_deref() else {
        return SimPlan {
            supported: false,
            reason: Some(
                "no chip could be detected for this project, and a simulator needs to know \
                 which machine to model — set the target in .cargo/config.toml"
                    .to_string(),
            ),
            missing: Vec::new(),
            steps: Vec::new(),
            board: None,
            parts: Vec::new(),
            debug: None,
            debug_tool: None,
        };
    };

    let Some((_, emulator)) = MACHINES.iter().find(|(name, _)| *name == chip) else {
        let known: Vec<&str> = MACHINES.iter().map(|(name, _)| *name).collect();
        return SimPlan {
            supported: false,
            reason: Some(format!(
                "QEMU has no machine model for {chip}; it can model {}",
                known.join(", "),
            )),
            missing: Vec::new(),
            steps: Vec::new(),
            board: None,
            parts: Vec::new(),
            debug: None,
            debug_tool: None,
        };
    };

    let mut missing = Vec::new();
    let espflash = match find_espflash() {
        Some(path) => path,
        None => {
            missing.push(SimTool {
                name: "espflash".to_string(),
                install: "cargo install espflash".to_string(),
            });
            PathBuf::from("espflash")
        }
    };
    let qemu = match find_qemu(emulator) {
        Some(path) => path,
        None => {
            missing.push(SimTool {
                name: emulator.to_string(),
                install: format!(
                    "download the {emulator} build from \
                     https://github.com/espressif/qemu/releases and unpack it into the data \
                     directory's tools/qemu/"
                ),
            });
            PathBuf::from(emulator)
        }
    };

    let Some(target) = project.configured_target.as_deref() else {
        return SimPlan {
            supported: false,
            reason: Some(
                "no build target in .cargo/config.toml — the simulator cannot guess where \
                 the ELF will land"
                    .to_string(),
            ),
            missing,
            steps: Vec::new(),
            board: None,
            parts: Vec::new(),
            debug: None,
            debug_tool: None,
        };
    };
    let binary = package_name(Path::new(&project.root)).unwrap_or_else(|| "app".to_string());
    // Cargo's own directory names, and the profile the build below asks
    // for — one decision, spelled once.
    let profile = if debug { "debug" } else { "release" };
    let elf = format!("target/{target}/{profile}/{binary}");
    let image = "target/rusty-sim/flash.bin".to_string();

    // `--config` rather than an edit: the project's own `[profile.dev]`
    // usually sets `opt-level = "s"` — esp-generate's template does, saying
    // the default debug profile is too slow for the hardware — so dropping
    // `--release` alone still optimises, and breakpoints still move.
    // Overriding on the command line leaves their manifest alone and shows
    // in the dock exactly what ran.
    let build = if debug {
        CommandPlan {
            program: "cargo".to_string(),
            args: vec![
                "build".to_string(),
                "--config".to_string(),
                "profile.dev.opt-level=0".to_string(),
            ],
            display: "cargo build --config profile.dev.opt-level=0".to_string(),
            rationale: "unoptimised, so a breakpoint stops on the line you set it on rather \
                        than the next one the optimiser left standing"
                .to_string(),
            warning: None,
        }
    } else {
        CommandPlan {
            program: "cargo".to_string(),
            args: vec!["build".to_string(), "--release".to_string()],
            display: "cargo build --release".to_string(),
            rationale: "the project's own toolchain builds the exact firmware a device would get"
                .to_string(),
            warning: None,
        }
    };
    let mut image_args = vec![
        "save-image".to_string(),
        "--chip".to_string(),
        chip.to_string(),
        "--merge".to_string(),
        elf.clone(),
        image.clone(),
    ];
    let image_step = CommandPlan {
        display: format!("espflash {}", image_args.join(" ")),
        program: espflash.to_string_lossy().into_owned(),
        args: std::mem::take(&mut image_args),
        rationale: "merges bootloader, partition table and app into the bootable flash image \
                    QEMU maps as the SPI flash"
            .to_string(),
        warning: None,
    };
    let qemu_args = vec![
        "-M".to_string(),
        chip.to_string(),
        "-nographic".to_string(),
        "-drive".to_string(),
        format!("file={image},if=mtd,format=raw"),
        "-serial".to_string(),
        "mon:stdio".to_string(),
    ];
    let run = CommandPlan {
        display: format!("{emulator} {}", qemu_args.join(" ")),
        program: qemu.to_string_lossy().into_owned(),
        args: qemu_args,
        rationale: "boots the image in Espressif's QEMU; the serial console streams here \
                    until stopped"
            .to_string(),
        warning: None,
    };

    // Debugging is optional on top of the same boot: present when the
    // matching gdb exists, an installable card when it does not.
    let xtensa = *emulator == "qemu-system-xtensa";
    let (debug, debug_tool) = match find_gdb(xtensa) {
        Some(gdb) => (
            Some(SimDebug {
                gdb_command: format!(
                    "\"{}\" \"{elf}\" -ex \"target remote :1234\"",
                    gdb.display(),
                ),
                elf: elf.clone(),
                port: 1234,
            }),
            None,
        ),
        None => {
            let family = if xtensa {
                "xtensa-esp-elf-gdb"
            } else {
                "riscv32-esp-elf-gdb"
            };
            (
                None,
                Some(SimTool {
                    name: family.to_string(),
                    install: format!(
                        "download {family} from https://github.com/espressif/binutils-gdb/releases/tag/{GDB_RELEASE} into the data directory's tools/"
                    ),
                }),
            )
        }
    };

    SimPlan {
        supported: true,
        reason: None,
        missing,
        steps: vec![build, image_step, run],
        board: board_view(Path::new(&project.root)),
        parts: user_parts(Path::new(&project.root)),
        debug,
        debug_tool,
    }
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
    find_gdb(xtensa)
}

fn find_gdb(xtensa: bool) -> Option<PathBuf> {
    let family = if xtensa {
        "xtensa-esp-elf-gdb"
    } else {
        "riscv32-esp-elf-gdb"
    };
    let binary = if xtensa {
        "xtensa-esp32-elf-gdb"
    } else {
        "riscv32-esp-elf-gdb"
    };
    if let Some(tools) = config::data_dir().map(|d| d.join("tools")) {
        let bundled = tools.join(family).join("bin").join(exe(binary));
        if bundled.is_file() {
            return Some(bundled);
        }
    }
    on_path(binary)
}

/// `.rusty/sim.toml`, as it is written on disk.
///
/// Its own types, converted to and from the wire model — the file/wire split
/// every user-authored TOML here gets, so a panel refactor cannot silently
/// break the files people wrote (rule 2).
///
/// **One definition per part, used in both directions.** Reading and writing
/// were two parallel sets of structs, and parallel sets drift: `flip` was
/// added to the wire model and to *neither* of them, so mirroring a part was
/// dropped on save and read back as `false` — the mirror survived until the
/// project was reopened. Sharing the definition makes that omission a
/// compile error instead of a silent loss.
mod file {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Default, Deserialize, Serialize)]
    pub struct Board {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub chip: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub x: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub y: Option<f64>,
    }

    /// Where a part sits, in the file's spelling.
    ///
    /// Flattened, so the keys stay where a hand-written file puts them —
    /// `x`, `y`, `rot`, `flip` directly inside `[[led]]`, not under a
    /// sub-table nobody asked for. The wire model nests instead, for a reason
    /// that does not apply here: TOML is self-describing and its own
    /// deserializer types integers, which is what flatten needs.
    #[derive(Debug, Default, Deserialize, Serialize)]
    pub struct Place {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub x: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub y: Option<f64>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub routes: Vec<Vec<(f64, f64)>>,
        #[serde(default, skip_serializing_if = "crate::model::is_upright")]
        pub rot: u16,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        pub flip: bool,
    }

    #[derive(Debug, Default, Deserialize, Serialize)]
    pub struct Led {
        pub pin: u8,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub color: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub label: Option<String>,
        #[serde(flatten)]
        pub place: Place,
    }

    #[derive(Debug, Default, Deserialize, Serialize)]
    pub struct Button {
        pub pin: u8,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub label: Option<String>,
        #[serde(flatten)]
        pub place: Place,
    }

    #[derive(Debug, Default, Deserialize, Serialize)]
    pub struct Rgb {
        pub r: u8,
        pub g: u8,
        pub b: u8,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub label: Option<String>,
        #[serde(flatten)]
        pub place: Place,
    }

    #[derive(Debug, Default, Deserialize, Serialize)]
    pub struct Seven {
        pub pins: [u8; 7],
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub label: Option<String>,
        #[serde(flatten)]
        pub place: Place,
    }

    #[derive(Debug, Default, Deserialize, Serialize)]
    pub struct Display {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub label: Option<String>,
        /// Absent means "not wired yet" — old board files carry no pins at
        /// all, and an unwired screen still shows text.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub sda: Option<u8>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub scl: Option<u8>,
        #[serde(flatten)]
        pub place: Place,
    }

    #[derive(Debug, Default, Deserialize, Serialize)]
    pub struct Pot {
        pub pin: u8,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub label: Option<String>,
        #[serde(flatten)]
        pub place: Place,
    }

    /// Values before tables: TOML puts `[board]` after nothing, and every
    /// array-of-tables after that. Reordering these fields reorders the file.
    #[derive(Debug, Default, Deserialize, Serialize)]
    pub struct Sheet {
        #[serde(default)]
        pub board: Board,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub led: Vec<Led>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub button: Vec<Button>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub rgb: Vec<Rgb>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub seven: Vec<Seven>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub display: Vec<Display>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub pot: Vec<Pot>,
    }

    impl Place {
        /// `into_`, not `to_`: this consumes the file record so the route
        /// vectors move rather than being cloned on every board load.
        pub fn into_model(self) -> crate::model::Placement {
            crate::model::Placement {
                x: self.x,
                y: self.y,
                routes: self.routes,
                rot: self.rot,
                flip: self.flip,
            }
        }

        /// Positions are rounded on the way out: the canvas works in
        /// fractional pixels and a file full of `128.00000000000003` is a
        /// file nobody wants to read or diff.
        pub fn from_model(place: &crate::model::Placement) -> Self {
            Place {
                x: place.x.map(f64::round),
                y: place.y.map(f64::round),
                routes: place.routes.clone(),
                rot: place.rot,
                flip: place.flip,
            }
        }
    }
}

/// The board `.rusty/sim.toml` describes, if the project carries one.
fn board_view(root: &Path) -> Option<SimBoard> {
    let text = std::fs::read_to_string(root.join(".rusty/sim.toml")).ok()?;
    let parsed: file::Sheet = toml::from_str(&text).ok()?;
    if parsed.led.is_empty()
        && parsed.button.is_empty()
        && parsed.rgb.is_empty()
        && parsed.seven.is_empty()
        && parsed.display.is_empty()
        && parsed.pot.is_empty()
    {
        return None;
    }
    Some(SimBoard {
        chip: parsed.board.chip.unwrap_or_else(|| "esp32".to_string()),
        kit_x: parsed.board.x,
        kit_y: parsed.board.y,
        leds: parsed
            .led
            .into_iter()
            .map(|led| SimLed {
                label: led.label.unwrap_or_else(|| format!("GPIO{}", led.pin)),
                color: led.color.unwrap_or_else(|| "green".to_string()),
                pin: led.pin,
                place: led.place.into_model(),
            })
            .collect(),
        buttons: parsed
            .button
            .into_iter()
            .map(|b| SimButton {
                label: b.label.unwrap_or_else(|| format!("BTN{}", b.pin)),
                pin: b.pin,
                place: b.place.into_model(),
            })
            .collect(),
        rgbs: parsed
            .rgb
            .into_iter()
            .map(|rgb| SimRgb {
                label: rgb.label.unwrap_or_else(|| "RGB".to_string()),
                r: rgb.r,
                g: rgb.g,
                b: rgb.b,
                place: rgb.place.into_model(),
            })
            .collect(),
        sevens: parsed
            .seven
            .into_iter()
            .map(|seven| SimSeven {
                label: seven.label.unwrap_or_else(|| "7SEG".to_string()),
                pins: seven.pins,
                place: seven.place.into_model(),
            })
            .collect(),
        displays: parsed
            .display
            .into_iter()
            .map(|display| SimDisplay {
                label: display.label.unwrap_or_else(|| "DISPLAY".to_string()),
                sda: display.sda.unwrap_or(UNWIRED_PIN),
                scl: display.scl.unwrap_or(UNWIRED_PIN),
                place: display.place.into_model(),
            })
            .collect(),
        pots: parsed
            .pot
            .into_iter()
            .map(|pot| SimPot {
                label: pot.label.unwrap_or_else(|| format!("POT{}", pot.pin)),
                pin: pot.pin,
                place: pot.place.into_model(),
            })
            .collect(),
    })
}

/// The user's own part definitions, from `.rusty/parts/*.toml`.
///
/// A file that does not parse is skipped rather than sinking the whole
/// library; the panel offers what could be read.
pub fn user_parts(root: &Path) -> Vec<PartDef> {
    #[derive(serde::Deserialize)]
    struct File {
        name: String,
        color: Option<String>,
    }

    let Ok(entries) = std::fs::read_dir(root.join(".rusty/parts")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(parsed) = toml::from_str::<File>(&text) else {
            continue;
        };
        out.push(PartDef {
            name: parsed.name,
            color: parsed.color.unwrap_or_else(|| "green".to_string()),
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

// Version pins, the installer and the download ladder live in `install`;
// proxy policy lives in `net`; finding a binary lives in `tools`. This module
// is the simulator, and nothing else.

/// Create the directory the image step writes into. espflash does not make
/// parent directories, and "os error 3" from a missing folder reads like a
/// broken tool rather than a missing mkdir.
pub fn prepare(root: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(root.join("target/rusty-sim"))
}

/// espflash: PATH first, then the copy the workbench keeps in its data
/// directory, then cargo's bin dir.
fn find_espflash() -> Option<PathBuf> {
    if let Some(path) = on_path("espflash") {
        return Some(path);
    }
    if let Some(tools) = config::data_dir().map(|d| d.join("tools/espflash").join(exe("espflash")))
        && tools.is_file()
    {
        return Some(tools);
    }
    let cargo = home_dir()?.join(".cargo/bin").join(exe("espflash"));
    cargo.is_file().then_some(cargo)
}

/// QEMU: PATH first, then the data directory's tools/qemu/bin.
fn find_qemu(emulator: &str) -> Option<PathBuf> {
    if let Some(path) = on_path(emulator) {
        return Some(path);
    }
    let tools = config::data_dir()?
        .join("tools/qemu/bin")
        .join(exe(emulator));
    tools.is_file().then_some(tools)
}

/// The line only rusty's GPIO model emits, and the thing to look for in a
/// binary to know whether it has one.
///
/// A marker in the emulator's own output rather than a version file beside
/// it: the question is "does *this* binary keep pin state", and a user who
/// dropped Espressif's build into the same directory must get the right
/// answer. The CI gate greps the same literal for the same reason.
const GPIO_MODEL_MARKER: &[u8] = b"[rusty:gpio@";

/// Does this emulator model GPIO, or is it the stock one whose write handler
/// is an empty function?
///
/// Everything downstream branches on this: with the model, the board shows
/// what a pin *is* and a button drives the register the firmware reads;
/// without it, the firmware has to narrate its own pins over the serial line
/// and a button arrives as a `B14=1` message. Both work — but claiming the
/// first while running the second would show a dark LED for correct firmware,
/// which is the failure this whole path exists to remove.
///
/// Cached on path, length and mtime, because it is asked once per run and the
/// answer costs a scan of a hundred-megabyte file.
pub fn has_gpio_model(qemu: &Path) -> bool {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    type Stamp = (std::path::PathBuf, u64, Option<std::time::SystemTime>);
    static SEEN: OnceLock<Mutex<HashMap<Stamp, bool>>> = OnceLock::new();

    let Ok(meta) = std::fs::metadata(qemu) else {
        return false;
    };
    let stamp: Stamp = (qemu.to_path_buf(), meta.len(), meta.modified().ok());

    let cache = SEEN.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(seen) = cache.lock()
        && let Some(known) = seen.get(&stamp)
    {
        return *known;
    }

    let found = scan_for(qemu, GPIO_MODEL_MARKER);
    if let Ok(mut seen) = cache.lock() {
        seen.insert(stamp, found);
    }
    found
}

/// Is `needle` anywhere in this file?
///
/// Chunked with an overlap of `needle.len() - 1`, so a marker straddling a
/// chunk boundary is still found — reading a hundred megabytes into memory to
/// avoid thinking about that is the version of this that makes the toolchain
/// panel stutter.
fn scan_for(path: &Path, needle: &[u8]) -> bool {
    use std::io::Read;

    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let overlap = needle.len() - 1;
    let mut buffer = vec![0u8; 1 << 20];
    let mut filled = 0usize;
    loop {
        match file.read(&mut buffer[filled..]) {
            Ok(0) => return false,
            Ok(read) => {
                filled += read;
                if buffer[..filled].windows(needle.len()).any(|w| w == needle) {
                    return true;
                }
                if filled + overlap >= buffer.len() {
                    buffer.copy_within(filled - overlap..filled, 0);
                    filled = overlap;
                }
            }
            Err(_) => return false,
        }
    }
}

/// Where pin changes leave the emulator and host-driven levels go back in.
///
/// Its own chardev, not the serial line: the UART belongs to the firmware,
/// and interleaving the two would make each unreadable to whoever wanted the
/// other. `-global` rather than `-device` because the machine creates the
/// GPIO device itself — there is no `-device` line to hang a chardev off.
///
/// QEMU listens and rusty connects, which is the arrangement the CI gate
/// proves; having rusty listen would be a second arrangement nothing has
/// booted.
pub fn pins_args(port: u16) -> Vec<String> {
    vec![
        "-chardev".to_string(),
        format!("socket,id=pins,host=127.0.0.1,port={port},server=on,wait=off"),
        "-global".to_string(),
        "driver=esp32.gpio,property=pins,value=pins".to_string(),
    ]
}

/// The proxy the rest of this machine uses, if any.
///
/// Write the board back to `.rusty/sim.toml`, the file the editor edits.
///
/// Serialised through the [`file`] structs, not the wire ones — the file
/// format is a contract with people who write it by hand, and it stays stable
/// when the wire model grows.
pub fn save_board(root: &Path, board: &SimBoard) -> std::result::Result<(), String> {
    let unwired = |pin: u8| (pin != UNWIRED_PIN).then_some(pin);

    let sheet = file::Sheet {
        board: file::Board {
            chip: Some(board.chip.clone()),
            x: board.kit_x.map(f64::round),
            y: board.kit_y.map(f64::round),
        },
        led: board
            .leds
            .iter()
            .map(|led| file::Led {
                pin: led.pin,
                color: Some(led.color.clone()),
                label: Some(led.label.clone()),
                place: file::Place::from_model(&led.place),
            })
            .collect(),
        button: board
            .buttons
            .iter()
            .map(|b| file::Button {
                pin: b.pin,
                label: Some(b.label.clone()),
                place: file::Place::from_model(&b.place),
            })
            .collect(),
        rgb: board
            .rgbs
            .iter()
            .map(|rgb| file::Rgb {
                r: rgb.r,
                g: rgb.g,
                b: rgb.b,
                label: Some(rgb.label.clone()),
                place: file::Place::from_model(&rgb.place),
            })
            .collect(),
        seven: board
            .sevens
            .iter()
            .map(|seven| file::Seven {
                pins: seven.pins,
                label: Some(seven.label.clone()),
                place: file::Place::from_model(&seven.place),
            })
            .collect(),
        display: board
            .displays
            .iter()
            .map(|display| file::Display {
                label: Some(display.label.clone()),
                sda: unwired(display.sda),
                scl: unwired(display.scl),
                place: file::Place::from_model(&display.place),
            })
            .collect(),
        pot: board
            .pots
            .iter()
            .map(|pot| file::Pot {
                pin: pot.pin,
                label: Some(pot.label.clone()),
                place: file::Place::from_model(&pot.place),
            })
            .collect(),
    };
    let text = toml::to_string_pretty(&sheet).map_err(|e| format!("could not encode: {e}"))?;
    let dir = root.join(".rusty");
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create .rusty: {e}"))?;
    std::fs::write(dir.join("sim.toml"), text).map_err(|e| format!("could not write: {e}"))
}

/// The `[package] name` of the project's manifest — where cargo will put the
/// ELF. A scan, like the edition scan in rusty-edit, for the same reason.
fn package_name(root: &Path) -> Option<String> {
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).ok()?;
    let mut in_package = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if in_package && let Some(rest) = line.strip_prefix("name") {
            let rest = rest.trim_start();
            if let Some(value) = rest.strip_prefix('=') {
                return Some(value.trim().trim_matches('"').to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Placement;

    /// A debug run is a different build, not just different QEMU flags.
    ///
    /// Dropping `--release` would not be enough: esp-generate's template
    /// sets `[profile.dev] opt-level = "s"`, so the dev profile optimises
    /// too and breakpoints still move off the line they were set on.
    #[test]
    fn a_debug_run_builds_unoptimised_and_takes_that_elf() {
        let target = Some("riscv32imc-unknown-none-elf");
        let release = plan(&project(Some("esp32c3"), target), false);
        let debug = plan(&project(Some("esp32c3"), target), true);

        assert!(
            release.steps[0].display.contains("--release"),
            "the ordinary run builds what a device would get: {}",
            release.steps[0].display,
        );
        assert!(
            debug.steps[0].display.contains("profile.dev.opt-level=0"),
            "the debug run overrides the profile on the command line rather than editing \
             anybody's manifest: {}",
            debug.steps[0].display,
        );
        assert!(
            debug.steps[1].display.contains("/debug/"),
            "and images the ELF that build produced: {}",
            debug.steps[1].display,
        );
        assert!(
            release.steps[1].display.contains("/release/"),
            "while a release run images its own: {}",
            release.steps[1].display,
        );
        // The two plans name *different* binaries. That is the whole reason
        // only the run that armed the target may say which one gdb reads:
        // pointing the debugger at the release ELF while the unoptimised
        // image ran reported the breakpoint six lines down and never hit it,
        // with nothing anywhere saying the two were different builds.
        //
        // `is_some_and` rather than `expect` because a machine with no esp
        // gdb installed has no debug target to name, and CI is one.
        assert!(
            debug.debug.is_some_and(|d| d.elf.contains("/debug/")),
            "as does the debugger",
        );
        assert!(
            release.debug.is_some_and(|d| d.elf.contains("/release/")),
            "each reading the build it belongs to",
        );
    }

    fn project(chip: Option<&str>, target: Option<&str>) -> EmbeddedProject {
        EmbeddedProject {
            root: ".".to_string(),
            chip: chip.map(str::to_string),
            chip_source: None,
            runtime: None,
            configured_target: target.map(str::to_string),
            configured_toolchain: None,
            frameworks: Vec::new(),
            uses_defmt: false,
            uses_embassy: false,
            evidence: Vec::new(),
            problems: Vec::new(),
            c_interop: Default::default(),
        }
    }

    /// A round trip proves nothing about a field left at its default.
    ///
    /// This test existed while `flip` was being dropped on save, and passed:
    /// the fixture set `flip: false` on every part, so a writer that never
    /// wrote it and a reader that hard-coded `false` agreed perfectly. The
    /// rule the fixture now follows is that **every optional field differs
    /// from its default** — that is what makes the comparison mean something.
    #[test]
    fn the_board_round_trips_through_save_and_load() {
        let dir = tempfile::Builder::new()
            .prefix("rusty-sim-rt")
            .tempdir()
            .expect("tempdir");
        let place = |x: f64, y: f64, rot: u16, flip: bool| Placement {
            x: Some(x),
            y: Some(y),
            routes: vec![vec![(120.0, 60.0), (200.0, 90.0)]],
            rot,
            flip,
        };
        let board = SimBoard {
            chip: "esp32".to_string(),
            kit_x: Some(420.0),
            kit_y: Some(30.0),
            leds: vec![SimLed {
                pin: 26,
                color: "green".to_string(),
                label: "G".to_string(),
                place: place(40.0, 60.0, 90, true),
            }],
            buttons: vec![SimButton {
                pin: 14,
                label: "BTN14".to_string(),
                place: place(30.0, 120.0, 180, true),
            }],
            rgbs: vec![SimRgb {
                r: 21,
                g: 22,
                b: 23,
                label: "RGB".to_string(),
                place: place(80.0, 160.0, 270, true),
            }],
            sevens: vec![SimSeven {
                pins: [1, 2, 3, 4, 5, 6, 7],
                label: "7SEG".to_string(),
                place: place(200.0, 40.0, 270, true),
            }],
            displays: vec![
                SimDisplay {
                    sda: 21,
                    scl: 22,
                    label: "DISPLAY".to_string(),
                    place: place(300.0, 200.0, 90, true),
                },
                // The sentinel has to survive too: a screen nobody has wired
                // writes no pins at all, and must read back unwired rather
                // than as GPIO0.
                SimDisplay {
                    sda: UNWIRED_PIN,
                    scl: UNWIRED_PIN,
                    label: "LOOSE".to_string(),
                    place: Placement::default(),
                },
            ],
            pots: vec![SimPot {
                pin: 34,
                label: "POT34".to_string(),
                place: place(20.0, 200.0, 90, true),
            }],
        };
        save_board(dir.path(), &board).expect("save");
        let loaded = board_view(dir.path()).expect("load");
        assert_eq!(loaded, board);

        // Named explicitly as well as compared: `assert_eq` on the whole
        // board says "something differs", and the field that differs is the
        // one worth naming.
        assert!(loaded.leds[0].place.flip, "a mirrored part stays mirrored");
        assert_eq!(
            loaded.sevens[0].place.rot, 270,
            "and a turned one stays turned"
        );
        assert_eq!(loaded.displays[1].sda, UNWIRED_PIN);
    }

    #[test]
    fn user_parts_load_and_bad_files_are_skipped() {
        let dir = tempfile::Builder::new()
            .prefix("rusty-parts")
            .tempdir()
            .expect("tempdir");
        let parts = dir.path().join(".rusty/parts");
        std::fs::create_dir_all(&parts).expect("dirs");
        std::fs::write(
            parts.join("relay.toml"),
            "name = \"relay\"\ncolor = \"red\"\n",
        )
        .expect("write");
        std::fs::write(parts.join("buzzer.toml"), "name = \"buzzer\"\n").expect("write");
        std::fs::write(parts.join("broken.toml"), "not = = toml").expect("write");

        let defs = user_parts(dir.path());
        assert_eq!(defs.len(), 2, "{defs:?}");
        assert_eq!(defs[0].name, "buzzer");
        assert_eq!(defs[0].color, "green");
        assert_eq!(defs[1].name, "relay");
        assert_eq!(defs[1].color, "red");
        assert!(user_parts(Path::new("nowhere")).is_empty());
    }

    #[test]
    fn the_board_file_converts_to_the_wire_model() {
        let dir = tempfile::Builder::new()
            .prefix("rusty-sim-board")
            .tempdir()
            .expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".rusty")).expect("dirs");
        std::fs::write(
            dir.path().join(".rusty/sim.toml"),
            "[board]\nchip = \"esp32\"\n[[led]]\npin = 26\ncolor = \"green\"\n[[led]]\npin = 27\ncolor = \"blue\"\nlabel = \"BLUE\"\n",
        )
        .expect("write");
        let board = board_view(dir.path()).expect("board");
        assert_eq!(board.chip, "esp32");
        assert_eq!(board.leds.len(), 2);
        assert_eq!(board.leds[0].label, "GPIO26");
        assert_eq!(board.leds[1].label, "BLUE");
        assert!(board_view(Path::new("nowhere-at-all")).is_none());
    }

    /// The marker has to be found wherever it lands, including across a read
    /// boundary — which is the case a chunked scan gets wrong, and it gets it
    /// wrong silently: the answer is "this is the stock emulator", and the
    /// board quietly goes back to trusting the firmware's narration.
    #[test]
    fn the_model_marker_is_found_even_when_it_straddles_a_chunk() {
        let dir = std::env::temp_dir().join("rusty-scan-test");
        std::fs::create_dir_all(&dir).expect("temp dir");

        let chunk = 1 << 20;
        for offset in [0usize, 4096, chunk - 6, chunk, chunk + 1, chunk * 2 - 3] {
            let path = dir.join(format!("marked-{offset}.bin"));
            let mut bytes = vec![b'.'; offset + GPIO_MODEL_MARKER.len() + 4096];
            bytes[offset..offset + GPIO_MODEL_MARKER.len()].copy_from_slice(GPIO_MODEL_MARKER);
            std::fs::write(&path, &bytes).expect("write");
            assert!(
                scan_for(&path, GPIO_MODEL_MARKER),
                "marker at byte {offset} was missed",
            );
            let _ = std::fs::remove_file(&path);
        }

        // And a binary without it must not be mistaken for one with it: the
        // stock emulator answering "yes" is a board claiming pin state it
        // does not have.
        let plain = dir.join("stock.bin");
        std::fs::write(&plain, vec![b'.'; chunk * 2]).expect("write");
        assert!(!scan_for(&plain, GPIO_MODEL_MARKER));
        assert!(!has_gpio_model(&plain));
        let _ = std::fs::remove_file(&plain);

        // A path that is not there is not a model, and must not panic.
        assert!(!has_gpio_model(&dir.join("absent")));
    }

    #[test]
    fn the_pin_channel_is_a_chardev_of_its_own() {
        let args = pins_args(4444);
        let joined = args.join(" ");
        assert!(joined.contains("socket,id=pins"), "{joined}");
        assert!(joined.contains("port=4444"), "{joined}");
        // QEMU listens, rusty connects — the arrangement CI boots. `wait=off`
        // so a run still starts when nothing ever connects.
        assert!(joined.contains("server=on"), "{joined}");
        assert!(joined.contains("wait=off"), "{joined}");
        // -global, because the machine creates the device; there is no
        // -device line to attach the chardev to.
        assert!(
            joined.contains("-global driver=esp32.gpio,property=pins,value=pins"),
            "{joined}",
        );
        // Never the serial line: that one belongs to the firmware.
        assert!(!joined.contains("-serial"), "{joined}");
    }

    #[test]
    fn an_unmodelled_chip_is_refused_with_the_supported_list() {
        let plan = plan(
            &project(Some("esp32c6"), Some("riscv32imac-unknown-none-elf")),
            false,
        );
        assert!(!plan.supported);
        let reason = plan.reason.expect("names the problem");
        assert!(reason.contains("esp32c6"), "{reason}");
        assert!(
            reason.contains("esp32c3"),
            "the alternatives are listed: {reason}"
        );
    }

    #[test]
    fn a_supported_chip_plans_three_inspectable_steps() {
        let sim = plan(
            &project(Some("esp32c3"), Some("riscv32imc-unknown-none-elf")),
            false,
        );
        assert!(sim.supported, "{:?}", sim.reason);
        assert_eq!(sim.steps.len(), 3);
        assert_eq!(sim.steps[0].display, "cargo build --release");
        assert!(
            sim.steps[1].display.contains("save-image"),
            "{}",
            sim.steps[1].display
        );
        assert!(sim.steps[1].display.contains("--merge"));
        assert!(
            sim.steps[2].display.contains("-M esp32c3"),
            "{}",
            sim.steps[2].display
        );
        assert!(sim.steps[2].display.contains("if=mtd"));
    }

    #[test]
    fn no_chip_and_no_target_refuse_rather_than_guess() {
        assert!(!plan(&project(None, None), false).supported);
        let sim = plan(&project(Some("esp32c3"), None), false);
        assert!(!sim.supported);
        assert!(sim.reason.expect("says why").contains(".cargo/config.toml"));
    }
}
