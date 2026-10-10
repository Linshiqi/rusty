//! The build systems a project can be in beside Cargo — PlatformIO and
//! CMake: how one is recognised, where it writes its chip down, the commands
//! that build and flash it, and where its images land.
//!
//! Cargo keeps its own module (`project`); this one answers the same
//! questions for the other two, and `project::detect` hands a directory to
//! whichever file is at its root — `Cargo.toml` first, then
//! `platformio.ini`, then `CMakeLists.txt`.
//!
//! **The chip is read where the project's own tool reads it, or not at
//! all.** A PlatformIO environment names a board, and a board names its MCU;
//! the Pico SDK reads `PICO_BOARD` and `PICO_PLATFORM`; ESP-IDF reads
//! `CONFIG_IDF_TARGET` out of `sdkconfig`; STM32CubeMX writes the ordering
//! code into the `.ioc` it generated the CMake from. A project that says
//! none of these has no chip, and says so, rather than one guessed off a
//! compiler flag.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::catalog::Catalog;
use crate::error::{Error, Result};
use crate::model::{
    BuildSetup, BuildSystem, CmakeSdk, CommandPlan, EmbeddedProject, Firmware, FlashAction,
    Problem, Severity, Transport,
};

/// Which build system's file is at `root`, Cargo's first.
pub fn system_at(root: &Path) -> Option<BuildSystem> {
    if root.join("Cargo.toml").is_file() {
        Some(BuildSystem::Cargo)
    } else if root.join("platformio.ini").is_file() {
        Some(BuildSystem::PlatformIo)
    } else if root.join("CMakeLists.txt").is_file() {
        Some(BuildSystem::Cmake)
    } else {
        None
    }
}

/// What can be told about a PlatformIO or CMake project at `root`.
pub(crate) fn detect(root: &Path, system: BuildSystem) -> Result<EmbeddedProject> {
    let catalog = Catalog::load(Some(root));
    match system {
        BuildSystem::PlatformIo => detect_platformio(root, &catalog),
        BuildSystem::Cmake => detect_cmake(root, &catalog),
        BuildSystem::Cargo => Err(Error::refused(
            "a Cargo project is detected by `project::detect`",
        )),
    }
}

fn empty(root: &Path, build: BuildSetup, evidence: Vec<String>) -> EmbeddedProject {
    EmbeddedProject {
        root: root.display().to_string(),
        chip: None,
        chip_source: None,
        firmware_dir: None,
        playground: None,
        runtime: None,
        configured_target: None,
        configured_toolchain: None,
        probe_chip: None,
        frameworks: Vec::new(),
        uses_defmt: false,
        uses_embassy: false,
        c_interop: Default::default(),
        evidence,
        problems: Vec::new(),
        build,
    }
}

fn read(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).map_err(|source| Error::Read {
        path: path.display().to_string(),
        source,
    })
}

// ─── PlatformIO ──────────────────────────────────────────────────────────────

/// `platformio.ini` as sections of keys, in the file's order. PlatformIO's
/// own reader is Python's `configparser`: `;` and `#` comment lines, an
/// indented line continuing the value above it, and an inline `;` comment
/// after whitespace.
fn parse_ini(text: &str) -> Vec<(String, BTreeMap<String, String>)> {
    let mut sections: Vec<(String, BTreeMap<String, String>)> = Vec::new();
    let mut open: Option<String> = None;
    for raw in text.lines() {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with(';') || trimmed.starts_with('#') {
            continue;
        }
        let line = match trimmed.find(" ;") {
            Some(at) => trimmed[..at].trim_end(),
            None => trimmed,
        };
        if line.starts_with('[') && line.ends_with(']') {
            sections.push((line[1..line.len() - 1].trim().to_string(), BTreeMap::new()));
            open = None;
            continue;
        }
        let Some((_, keys)) = sections.last_mut() else {
            continue;
        };
        let continues = raw.starts_with(char::is_whitespace);
        match (continues, &open, line.split_once('=')) {
            (true, Some(key), _) => {
                let value = keys.entry(key.clone()).or_default();
                if !value.is_empty() {
                    value.push('\n');
                }
                value.push_str(line);
            }
            (_, _, Some((key, value))) => {
                let key = key.trim().to_string();
                keys.insert(key.clone(), value.trim().to_string());
                open = Some(key);
            }
            _ => {}
        }
    }
    sections
}

/// One environment's keys: `[env]`'s, then what `extends` names, then its
/// own — PlatformIO's order, the later winning.
fn environment(
    sections: &[(String, BTreeMap<String, String>)],
    name: &str,
) -> BTreeMap<String, String> {
    fn section<'a>(
        sections: &'a [(String, BTreeMap<String, String>)],
        name: &str,
    ) -> Option<&'a BTreeMap<String, String>> {
        sections.iter().find(|(n, _)| n == name).map(|(_, k)| k)
    }
    let mut keys = section(sections, "env").cloned().unwrap_or_default();
    let own = section(sections, &format!("env:{name}"));
    let mut chain = Vec::new();
    let mut next = own.and_then(|k| k.get("extends")).cloned();
    while let Some(base) = next.take() {
        // Bounded: a file whose environments extend each other in a circle
        // is read once round, not for ever.
        if chain.len() > 8 || chain.contains(&base) {
            break;
        }
        if let Some(keys) = section(sections, base.trim()) {
            next = keys.get("extends").cloned();
        }
        chain.push(base);
    }
    for base in chain.iter().rev() {
        if let Some(base) = section(sections, base.trim()) {
            keys.extend(base.clone());
        }
    }
    if let Some(own) = own {
        keys.extend(own.clone());
    }
    keys
}

fn list(value: &str) -> Vec<String> {
    value
        .split([',', '\n'])
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

fn detect_platformio(root: &Path, catalog: &Catalog) -> Result<EmbeddedProject> {
    let text = read(&root.join("platformio.ini"))?;
    let sections = parse_ini(&text);
    let environments: Vec<String> = sections
        .iter()
        .filter_map(|(name, _)| name.strip_prefix("env:"))
        .map(|name| name.trim().to_string())
        .collect();
    let chosen = sections
        .iter()
        .find(|(name, _)| name == "platformio")
        .and_then(|(_, keys)| keys.get("default_envs"))
        .and_then(|envs| list(envs).into_iter().next())
        .or_else(|| environments.first().cloned());

    let mut project = empty(
        root,
        BuildSetup {
            system: BuildSystem::PlatformIo,
            environment: chosen.clone(),
            environments: environments.clone(),
            ..Default::default()
        },
        vec!["platformio.ini".to_string()],
    );

    let Some(env) = chosen else {
        project.problems.push(Problem::new(
            Severity::Blocking,
            "pio-env-none",
            "No environment in platformio.ini",
            "PlatformIO builds an `[env:…]` section, and this file declares none.",
        ));
        return Ok(project);
    };
    let keys = environment(&sections, &env);
    project.frameworks = keys
        .get("framework")
        .map(|f| list(f))
        .unwrap_or_default()
        .into_iter()
        .map(|framework| format!("{framework} — PlatformIO framework"))
        .collect();
    if let Some(platform) = keys.get("platform") {
        project
            .frameworks
            .insert(0, format!("{platform} — PlatformIO platform"));
    }

    let board = keys.get("board").cloned();
    let found = keys
        .get("board_build.mcu")
        .and_then(|mcu| {
            catalog.chip_named(mcu).map(|chip| {
                (
                    chip.id.clone(),
                    format!("platformio.ini `board_build.mcu = {mcu}` in [env:{env}]"),
                )
            })
        })
        .or_else(|| {
            let board = board.as_deref()?;
            catalog
                .board_for_platformio(board)
                .map(|b| {
                    (
                        b.chip.clone(),
                        format!("PlatformIO board `{board}` ({})", b.name),
                    )
                })
                .or_else(|| {
                    let mcu = installed_board_mcu(board)?;
                    let chip = catalog.chip_named(&mcu)?;
                    Some((
                        chip.id.clone(),
                        format!("PlatformIO's description of board `{board}` (mcu `{mcu}`)"),
                    ))
                })
        });
    match found {
        Some((chip, source)) => {
            project.chip = Some(chip);
            project.chip_source = Some(source);
        }
        None => project.problems.push(
            Problem::new(
                Severity::Warning,
                "pio-chip-unknown",
                "The chip could not be read from platformio.ini",
                format!(
                    "Environment `{env}` names board `{}`, which neither rusty's catalogue \
                     nor PlatformIO's installed boards describe as a part rusty knows. \
                     PlatformIO still builds and uploads it; rusty cannot size it against \
                     the part, flash it another way or simulate it. `board_build.mcu` in \
                     the environment names the part outright.",
                    board.as_deref().unwrap_or("(none)")
                ),
            )
            .arg("env", env.clone())
            .arg("board", board.unwrap_or_default()),
        ),
    }
    Ok(project)
}

/// The MCU an installed PlatformIO platform describes `board` as having,
/// from the board's own JSON — what PlatformIO itself reads.
fn installed_board_mcu(board: &str) -> Option<String> {
    let core = std::env::var_os("PLATFORMIO_CORE_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::home_dir().map(|home| home.join(".platformio")))?;
    let platforms = std::fs::read_dir(core.join("platforms")).ok()?;
    for platform in platforms.flatten() {
        let file = platform.path().join("boards").join(format!("{board}.json"));
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        if let Some(mcu) = json.pointer("/build/mcu").and_then(|m| m.as_str()) {
            return Some(mcu.to_string());
        }
    }
    None
}

// ─── CMake ───────────────────────────────────────────────────────────────────

/// The value of `set(<name> <value>)` in a CMakeLists, unquoted.
fn cmake_set(text: &str, name: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.trim();
        let rest = line
            .strip_prefix("set(")
            .or_else(|| line.strip_prefix("set ("))?;
        let rest = rest.trim_start().strip_prefix(name)?;
        if !rest.starts_with(char::is_whitespace) {
            return None;
        }
        let value = rest.trim().trim_end_matches(')').trim();
        let value = value.split_whitespace().next()?.trim_matches('"');
        (!value.is_empty()).then(|| value.to_string())
    })
}

/// The Pico SDK's boards, by the chip each carries. A board the SDK knows
/// and this list does not is not guessed at.
const PICO_BOARDS: [(&str, &str); 4] = [
    ("pico", "rp2040"),
    ("pico_w", "rp2040"),
    ("pico2", "rp235xa"),
    ("pico2_w", "rp235xa"),
];

fn detect_cmake(root: &Path, catalog: &Catalog) -> Result<EmbeddedProject> {
    let text = read(&root.join("CMakeLists.txt"))?;
    let lower = text.to_ascii_lowercase();
    let mut evidence = vec!["CMakeLists.txt".to_string()];

    let ioc = std::fs::read_dir(root).ok().and_then(|entries| {
        entries
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|x| x == "ioc"))
    });
    let sdk = if lower.contains("tools/cmake/project.cmake") {
        Some(CmakeSdk::EspIdf)
    } else if lower.contains("pico_sdk_init") || lower.contains("pico_sdk_import") {
        Some(CmakeSdk::PicoSdk)
    } else if ioc.is_some() {
        Some(CmakeSdk::Stm32Cube)
    } else {
        None
    };

    let (preset, preset_dir) = match configure_preset(root) {
        Some((name, dir)) => {
            evidence.push("CMakePresets.json".to_string());
            (Some(name), dir)
        }
        None => (None, None),
    };
    let build_dir = preset_dir.unwrap_or_else(|| "build".to_string());

    let mut project = empty(
        root,
        BuildSetup {
            system: BuildSystem::Cmake,
            sdk,
            preset,
            build_dir: Some(build_dir),
            ..Default::default()
        },
        evidence,
    );
    if let Some(sdk) = sdk {
        project
            .frameworks
            .push(format!("{} — CMake SDK", sdk.label()));
    }

    let found: Option<(String, String)> = match sdk {
        Some(CmakeSdk::PicoSdk) => {
            let board = cmake_set(&text, "PICO_BOARD");
            let platform = cmake_set(&text, "PICO_PLATFORM");
            match (&board, &platform) {
                (Some(board), _) => {
                    PICO_BOARDS
                        .iter()
                        .find(|(name, _)| name == board)
                        .map(|(_, chip)| {
                            (
                                chip.to_string(),
                                format!("CMakeLists.txt `PICO_BOARD {board}`"),
                            )
                        })
                }
                (None, Some(platform)) if platform.starts_with("rp2350") => Some((
                    "rp235xa".to_string(),
                    format!(
                        "CMakeLists.txt `PICO_PLATFORM {platform}`, whose default board is the \
                         Pico 2"
                    ),
                )),
                (None, Some(platform)) if platform == "rp2040" => Some((
                    "rp2040".to_string(),
                    "CMakeLists.txt `PICO_PLATFORM rp2040`".to_string(),
                )),
                (None, Some(_)) => None,
                (None, None) => Some((
                    "rp2040".to_string(),
                    "the Pico SDK's default board, the Pico (no PICO_BOARD set)".to_string(),
                )),
            }
        }
        Some(CmakeSdk::EspIdf) => idf_target(root)
            .map(|(target, file)| {
                (
                    target.clone(),
                    format!("{file} `CONFIG_IDF_TARGET=\"{target}\"`"),
                )
            })
            .or_else(|| {
                Some((
                    "esp32".to_string(),
                    "ESP-IDF's default target, the ESP32 (no sdkconfig names another)".to_string(),
                ))
            }),
        Some(CmakeSdk::Stm32Cube) => ioc.as_deref().and_then(|ioc| {
            let text = std::fs::read_to_string(ioc).ok()?;
            let name = ioc.file_name()?.to_string_lossy().into_owned();
            project.evidence.push(name.clone());
            ["Mcu.CPN=", "Mcu.UserName=", "Mcu.Name="]
                .iter()
                .find_map(|key| {
                    text.lines()
                        .find_map(|line| line.trim().strip_prefix(key))
                        .map(str::trim)
                })
                .map(|part| (part.to_string(), name))
        }),
        None => None,
    }
    .and_then(|(name, source)| {
        // Every answer goes through the catalogue: a part it does not know
        // is no chip, whatever the file called it.
        let chip = catalog.chip_named(&name)?;
        Some((chip.id.clone(), source))
    });

    match found {
        Some((chip, source)) => {
            project.chip = Some(chip);
            project.chip_source = Some(source);
        }
        None => project.problems.push(
            Problem::new(
                Severity::Warning,
                "cmake-chip-unknown",
                "The chip could not be read from the CMake project",
                format!(
                    "rusty reads the chip where the project's SDK writes it — the Pico SDK's \
                     PICO_BOARD, ESP-IDF's sdkconfig, STM32CubeMX's .ioc — and found none it \
                     knows here{}. CMake still builds it; rusty cannot size it against the \
                     part, flash it or simulate it until it can tell which part it is.",
                    sdk.map(|s| format!(" (a {} project)", s.label()))
                        .unwrap_or_default()
                ),
            )
            .arg("sdk", sdk.map(|s| s.label()).unwrap_or("CMake")),
        ),
    }
    Ok(project)
}

/// `CONFIG_IDF_TARGET` from `sdkconfig`, or from `sdkconfig.defaults` when
/// the project has not been configured yet, with the file it came from.
fn idf_target(root: &Path) -> Option<(String, &'static str)> {
    for file in ["sdkconfig", "sdkconfig.defaults"] {
        let Ok(text) = std::fs::read_to_string(root.join(file)) else {
            continue;
        };
        if let Some(target) = text.lines().find_map(|line| {
            line.trim()
                .strip_prefix("CONFIG_IDF_TARGET=")
                .map(|v| v.trim().trim_matches('"').to_string())
        }) {
            return Some((target, file));
        }
    }
    None
}

/// The first configure preset a person could choose in `CMakePresets.json`
/// (not `hidden`), with its binary directory relative to the root when the
/// preset or one it inherits says where.
fn configure_preset(root: &Path) -> Option<(String, Option<String>)> {
    let text = std::fs::read_to_string(root.join("CMakePresets.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    let presets = json.get("configurePresets")?.as_array()?;
    let by_name = |name: &str| {
        presets
            .iter()
            .find(|p| p.get("name").and_then(|n| n.as_str()) == Some(name))
    };
    let chosen = presets
        .iter()
        .find(|p| !p.get("hidden").and_then(|h| h.as_bool()).unwrap_or(false))?;
    let name = chosen.get("name")?.as_str()?.to_string();

    // Walk `inherits` (a name or a list of names) until a binaryDir turns up.
    let mut queue = vec![chosen];
    let mut binary_dir = None;
    let mut seen = 0;
    while let Some(preset) = queue.pop() {
        seen += 1;
        if seen > 16 {
            break;
        }
        if let Some(dir) = preset.get("binaryDir").and_then(|d| d.as_str()) {
            binary_dir = Some(dir.to_string());
            break;
        }
        match preset.get("inherits") {
            Some(serde_json::Value::String(base)) => queue.extend(by_name(base)),
            Some(serde_json::Value::Array(bases)) => {
                for base in bases.iter().rev().filter_map(|b| b.as_str()) {
                    queue.extend(by_name(base));
                }
            }
            _ => {}
        }
    }
    let dir = binary_dir.map(|dir| {
        dir.replace("${presetName}", &name)
            .replace("${sourceDir}/", "")
            .replace("${sourceDir}", ".")
    });
    Some((name, dir))
}

// ─── building ────────────────────────────────────────────────────────────────

/// The commands that build `project`, in order — one for Cargo and
/// PlatformIO, and for CMake a configure step first while the build
/// directory has no cache yet. Built for release, as Cargo's is, and with
/// `compile_commands.json` written, which is what an editor reads C by.
pub fn build_plans(project: &EmbeddedProject, root: &Path) -> Result<Vec<CommandPlan>> {
    let build = &project.build;
    match build.system {
        BuildSystem::Cargo => Ok(vec![CommandPlan::new(
            "cargo",
            vec!["build".into(), "--release".into()],
            "the project's own toolchain builds the exact firmware a device would get",
        )]),
        BuildSystem::PlatformIo => {
            let env = build.environment.clone().ok_or_else(|| {
                Error::refused(
                    "platformio.ini declares no [env:…] section, so there is nothing to build",
                )
            })?;
            let mut plans = vec![CommandPlan::new(
                "pio",
                vec!["run".into(), "-e".into(), env.clone()],
                "PlatformIO builds the environment with the toolchain its platform installs",
            )];
            // Once, so clangd has every file's flags: PlatformIO writes no
            // compile database unless asked.
            if !root.join("compile_commands.json").is_file() {
                plans.push(CommandPlan::new(
                    "pio",
                    vec![
                        "run".into(),
                        "-e".into(),
                        env,
                        "-t".into(),
                        "compiledb".into(),
                    ],
                    "writes compile_commands.json, which the editor reads C and C++ by",
                ));
            }
            Ok(plans)
        }
        BuildSystem::Cmake => {
            if build.sdk == Some(CmakeSdk::EspIdf) {
                return Ok(vec![CommandPlan::new(
                    "idf.py",
                    vec!["build".into()],
                    "ESP-IDF's own driver configures and builds the project",
                )]);
            }
            let dir = build
                .build_dir
                .clone()
                .unwrap_or_else(|| "build".to_string());
            let mut plans = Vec::new();
            if !root.join(&dir).join("CMakeCache.txt").is_file() {
                let mut args = match &build.preset {
                    Some(preset) => vec!["--preset".to_string(), preset.clone()],
                    None => vec![
                        "-S".into(),
                        ".".into(),
                        "-B".into(),
                        dir.clone(),
                        // Ninja, because the generator a host defaults to —
                        // Visual Studio's on Windows — cannot cross-compile.
                        "-G".into(),
                        "Ninja".into(),
                        "-DCMAKE_BUILD_TYPE=Release".into(),
                    ],
                };
                args.push("-DCMAKE_EXPORT_COMPILE_COMMANDS=ON".into());
                plans.push(CommandPlan::new(
                    "cmake",
                    args,
                    "configures the build directory once; later builds reuse it",
                ));
            }
            plans.push(CommandPlan::new(
                "cmake",
                vec!["--build".into(), dir],
                "builds what the configured directory describes",
            ));
            Ok(plans)
        }
    }
}

/// Flashing and monitoring a PlatformIO project: PlatformIO's own upload,
/// which reads the upload protocol and the probe from the environment. A
/// serial port chosen in the title bar is handed to it; a probe is not —
/// PlatformIO picks its debugger from `upload_protocol`.
pub fn platformio_flash(
    env: &str,
    transport: &Transport,
    action: FlashAction,
    baud: Option<u32>,
) -> CommandPlan {
    let port = match transport {
        Transport::Serial { port } => Some(port.clone()),
        Transport::Probe { .. } => None,
    };
    let mut args: Vec<String> = Vec::new();
    let rationale = match action {
        FlashAction::Monitor => {
            args.extend(["device".into(), "monitor".into(), "-e".into(), env.into()]);
            if let Some(port) = &port {
                args.extend(["-p".into(), port.clone()]);
            }
            if let Some(baud) = baud {
                args.extend(["-b".into(), baud.to_string()]);
            }
            "PlatformIO's monitor, at the environment's monitor_speed"
        }
        FlashAction::Flash | FlashAction::FlashAndMonitor => {
            args.extend([
                "run".into(),
                "-e".into(),
                env.into(),
                "-t".into(),
                "upload".into(),
            ]);
            if action == FlashAction::FlashAndMonitor {
                args.extend(["-t".into(), "monitor".into()]);
            }
            if let Some(port) = &port {
                args.extend(["--upload-port".into(), port.clone()]);
                if action == FlashAction::FlashAndMonitor {
                    args.extend(["--monitor-port".into(), port.clone()]);
                }
            }
            "PlatformIO's upload, by the protocol the environment names"
        }
    };
    CommandPlan::new("pio", args, rationale)
}

// ─── images ──────────────────────────────────────────────────────────────────

/// Whether `path` is a linked image for a microcontroller: an ELF
/// executable for Arm, RISC-V or Xtensa. A CMake build directory is full of
/// ELF object files and, on Linux, of host tools the Pico SDK builds for
/// itself (`pioasm`, `picotool`) — all ELF, none of them firmware.
pub(crate) fn embedded_elf(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut head = [0u8; 20];
    if file.read_exact(&mut head).is_err() || head[..4] != *b"\x7fELF" {
        return false;
    }
    // Little-endian is every part here; the header's own byte says so.
    let little = head[5] == 1;
    let half = |at: usize| {
        let bytes = [head[at], head[at + 1]];
        if little {
            u16::from_le_bytes(bytes)
        } else {
            u16::from_be_bytes(bytes)
        }
    };
    const EXECUTABLE: u16 = 2;
    const ARM: u16 = 40;
    const XTENSA: u16 = 94;
    const RISCV: u16 = 243;
    half(16) == EXECUTABLE && matches!(half(18), ARM | XTENSA | RISCV)
}

/// Directories of a CMake build that hold no image of the project's own:
/// CMake's bookkeeping, ESP-IDF's bootloader and partition table, fetched
/// dependencies.
const NOT_THE_APP: [&str; 5] = [
    "CMakeFiles",
    "bootloader",
    "partition_table",
    "_deps",
    "esp-idf",
];

/// The images a PlatformIO or CMake project has built, newest first.
pub(crate) fn images(root: &Path, project: &EmbeddedProject) -> Vec<Firmware> {
    let build = &project.build;
    let mut found = Vec::new();
    match build.system {
        BuildSystem::Cargo => {}
        BuildSystem::PlatformIo => {
            for env in &build.environments {
                let path = root
                    .join(".pio")
                    .join("build")
                    .join(env)
                    .join("firmware.elf");
                if path.is_file() {
                    found.push(image(
                        &path,
                        "release",
                        env,
                        build.environment.as_deref() == Some(env),
                    ));
                }
            }
        }
        BuildSystem::Cmake => {
            let dir = build
                .build_dir
                .clone()
                .unwrap_or_else(|| "build".to_string());
            let mut queue = vec![(root.join(&dir), 0)];
            while let Some((at, depth)) = queue.pop() {
                let Ok(entries) = std::fs::read_dir(&at) else {
                    continue;
                };
                for entry in entries.flatten() {
                    let path = entry.path();
                    let Ok(kind) = entry.file_type() else {
                        continue;
                    };
                    if kind.is_dir() {
                        let skip = NOT_THE_APP
                            .iter()
                            .any(|name| entry.file_name() == std::ffi::OsStr::new(name));
                        if depth < 2 && !skip {
                            queue.push((path, depth + 1));
                        }
                    } else if kind.is_file() && embedded_elf(&path) {
                        found.push(image(&path, &dir, "cmake", true));
                    }
                }
            }
        }
    }
    found.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| a.name.cmp(&b.name))
    });
    found
}

fn image(path: &Path, profile: &str, target: &str, chosen: bool) -> Firmware {
    let metadata = std::fs::metadata(path).ok();
    Firmware {
        path: path.display().to_string(),
        name: path
            .file_stem()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        profile: profile.to_string(),
        target: target.to_string(),
        bytes: metadata.as_ref().map_or(0, |m| m.len()),
        modified: metadata
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs()),
        matches_configured_target: chosen,
    }
}

// ─── what an editor reads ────────────────────────────────────────────────────

/// Where `compile_commands.json` is for `project` — the file clangd reads
/// every file's flags from: a CMake project's build directory, ESP-IDF's
/// `build/`, a PlatformIO project's root (`pio run -t compiledb` writes it
/// there). `None` for Cargo, which has no such file.
pub fn compile_commands_dir(root: &Path, project: &EmbeddedProject) -> Option<PathBuf> {
    match project.build.system {
        BuildSystem::Cargo => None,
        BuildSystem::PlatformIo => Some(root.to_path_buf()),
        BuildSystem::Cmake => Some(
            root.join(
                project
                    .build
                    .build_dir
                    .clone()
                    .unwrap_or_else(|| "build".to_string()),
            ),
        ),
    }
}

/// The cross compilers clangd may run to learn their system headers
/// (`--query-driver`), as globs: every C compiler the catalogue names for a
/// part — `arm-none-eabi-gcc` becomes `**/arm-none-eabi-*` — and ESP-IDF's
/// per-chip Xtensa names. Named rather than `**/*gcc*`: clangd *runs* what
/// matches, and a glob that matches anything runs anything a compile
/// database names.
pub fn query_driver_globs(catalog: &Catalog) -> Vec<String> {
    let mut globs: Vec<String> = catalog
        .chips()
        .iter()
        .filter_map(|chip| chip.c_compiler.as_ref())
        .filter_map(|compiler| {
            let prefix = compiler
                .binary
                .strip_suffix("gcc")
                .or_else(|| compiler.binary.strip_suffix("cc"))?;
            (!prefix.is_empty()).then(|| format!("**/{prefix}*"))
        })
        .collect();
    globs.push("**/xtensa-esp*-elf-*".to_string());
    globs.sort();
    globs.dedup();
    globs
}

// ─── C inside a Cargo project ────────────────────────────────────────────────

/// The C and C++ sources a Cargo project carries — what a `cc` build script
/// compiles — as paths relative to `root` with `/`, in a stable order.
///
/// The walk is the tree's: dot entries, `target/` and any directory cargo
/// marked as a build directory (`CACHEDIR.TAG`) are left out, and it stops
/// at a few thousand entries, since a vendored SDK under the root is no
/// reason to read the whole of it before the editor can start.
pub fn c_sources(root: &Path) -> Vec<String> {
    const LIMIT: usize = 4000;
    let mut found = Vec::new();
    let mut seen = 0usize;
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > LIMIT {
                break;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                if name != "target" && !path.join("CACHEDIR.TAG").is_file() {
                    pending.push(path);
                }
                continue;
            }
            if c_language(&name).is_some()
                && let Ok(relative) = path.strip_prefix(root)
            {
                found.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    found.sort();
    found
}

/// Whether a file is a C or C++ source or header, by its extension:
/// `Some(false)` for C, `Some(true)` for C++. `.h` is C — what a firmware
/// header nearly always is.
fn c_language(name: &str) -> Option<bool> {
    match name.rsplit_once('.')?.1 {
        "c" | "h" => Some(false),
        "cc" | "cpp" | "cxx" | "hh" | "hpp" | "hxx" => Some(true),
        _ => None,
    }
}

/// `relative` (with `/`) under `root`, joined a segment at a time so the
/// path is the host's own: `E:\proj\csrc\vendor.c`, not `E:\proj\csrc/vendor.c`,
/// which clangd would hold as a second spelling of the file the editor
/// opens.
fn native(root: &Path, relative: &str) -> PathBuf {
    relative
        .split('/')
        .fold(root.to_path_buf(), |path, segment| path.join(segment))
}

fn is_header(path: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, extension)| matches!(extension, "h" | "hh" | "hpp" | "hxx"))
}

/// The compile database clangd reads for the C in a Cargo project, as JSON:
/// one entry per source in `sources` (relative to `root`, as `c_sources`
/// names them), compiled by the part's cross compiler.
///
/// `cc` records no command line, so this is what it *would* run, said
/// plainly: the part's compiler by name — which `--query-driver` then asks
/// for its system headers and its target, so `<string.h>` and `<stdint.h>`
/// are newlib's and not the host's — freestanding as firmware C is, every
/// directory holding a header on the include path, and for C++ the three
/// switches the scaffold's build script compiles with. The flags a project's
/// own `build.rs` adds are not in it; a define it passes is one clangd does
/// not know.
pub fn cargo_compile_commands(
    root: &Path,
    sources: &[String],
    compiler: Option<&str>,
) -> serde_json::Value {
    let directory = root.display().to_string();
    let mut includes: Vec<String> = sources
        .iter()
        .filter(|path| is_header(path))
        .map(|path| match path.rsplit_once('/') {
            Some((dir, _)) => native(root, dir).display().to_string(),
            None => directory.clone(),
        })
        .collect();
    includes.sort();
    includes.dedup();

    let entries: Vec<serde_json::Value> = sources
        .iter()
        .filter_map(|path| {
            let name = path.rsplit('/').next().unwrap_or(path);
            let cpp = c_language(name)?;
            // Headers are not compiled; clangd takes their flags from a
            // source beside them.
            if is_header(name) {
                return None;
            }
            let program = match (compiler, cpp) {
                (Some(gcc), true) => gcc
                    .strip_suffix("gcc")
                    .map_or(gcc.to_string(), |stem| format!("{stem}g++")),
                (Some(gcc), false) => gcc.to_string(),
                (None, true) => "c++".to_string(),
                (None, false) => "cc".to_string(),
            };
            let mut arguments = vec![program];
            if cpp {
                arguments.extend(
                    [
                        "-std=gnu++17",
                        "-fno-exceptions",
                        "-fno-rtti",
                        "-fno-threadsafe-statics",
                    ]
                    .map(String::from),
                );
            } else {
                arguments.push("-std=gnu11".to_string());
            }
            if compiler.is_some() {
                arguments.push("-ffreestanding".to_string());
            }
            arguments.extend(includes.iter().map(|dir| format!("-I{dir}")));
            let file = native(root, path).display().to_string();
            arguments.extend(["-c".to_string(), file.clone()]);
            Some(serde_json::json!({
                "directory": directory,
                "file": file,
                "arguments": arguments,
            }))
        })
        .collect();
    serde_json::Value::Array(entries)
}

/// Write the compile database for a Cargo project's C, and answer with the
/// directory it is in — `None` when the project has no C or C++ at all, so
/// no clangd is started for it.
///
/// It goes under the data directory (`clangd/<hash of the root>/`), never
/// into the project: a file rusty made up has no place in somebody's
/// repository, and `target/` may not be where the build directory is.
pub fn write_cargo_compile_commands(root: &Path, compiler: Option<&str>) -> Option<PathBuf> {
    let sources = c_sources(root);
    if sources.iter().all(|path| is_header(path)) {
        return None;
    }
    let database = cargo_compile_commands(root, &sources, compiler);
    let key = root
        .display()
        .to_string()
        .bytes()
        .fold(0xcbf29ce484222325u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
        });
    let dir = crate::config::data_dir()?
        .join("clangd")
        .join(format!("{key:016x}"));
    std::fs::create_dir_all(&dir).ok()?;
    let text = serde_json::to_string_pretty(&database).ok()?;
    std::fs::write(dir.join("compile_commands.json"), text).ok()?;
    Some(dir)
}

// ─── the tools each needs ────────────────────────────────────────────────────

/// The programs a project's build system runs, beside what each is for and
/// how to get it: `pio`; `cmake` and Ninja; ESP-IDF's `idf.py`. The cross C
/// compiler is the chip's (`Chip::c_compiler`), reported with the others.
pub fn tools_for(build: &BuildSetup) -> Vec<(&'static str, &'static str, &'static str)> {
    match build.system {
        BuildSystem::Cargo => Vec::new(),
        BuildSystem::PlatformIo => vec![(
            "pio",
            "PlatformIO's command line — builds, uploads and monitors this project",
            "python -m pip install -U platformio",
        )],
        BuildSystem::Cmake if build.sdk == Some(CmakeSdk::EspIdf) => vec![(
            "idf.py",
            "ESP-IDF's build driver — run ESP-IDF's export script so it is on PATH",
            "install ESP-IDF (https://docs.espressif.com/projects/esp-idf/en/stable/esp32/get-started/) and run its export script",
        )],
        BuildSystem::Cmake => {
            let mut tools = vec![(
                "cmake",
                "Configures and builds this project",
                "install CMake from https://cmake.org/download/ and put it on PATH",
            )];
            if build.preset.is_none() {
                tools.push((
                    "ninja",
                    "The generator rusty configures a cross build with",
                    "install Ninja from https://github.com/ninja-build/ninja/releases and put it on PATH",
                ));
            }
            tools
        }
    }
}

#[cfg(test)]
mod tests;
