//! Getting the binary onto the board, and reading what it says back.
//!
//! Split deliberately in two:
//!
//! - [`plan`] decides *what to run* and is a pure function. It can be tested
//!   without a board, and its output is shown to the user before anything
//!   happens. Embedded developers live in a terminal; a tool that hides the
//!   command behind a button becomes something to work around.
//! - [`crate::process::spawn`] runs it and streams the output.
//!
//! rusty does not reimplement flashing. espflash and probe-rs are the tools the
//! ecosystem actually maintains, and wrapping them means a user's existing
//! knowledge — and their existing bug reports — still apply.

use crate::{
    catalog::Catalog,
    chip,
    error::{Error, Result},
    model::{CommandPlan, FlashAction, SerialPort, Transport},
};

/// Everything needed to decide on a command.
#[derive(Debug, Clone)]
pub struct FlashRequest {
    pub chip_id: String,
    /// probe-rs's name for the part as the project's runner gives it
    /// (`EmbeddedProject::probe_chip`), which wins over the catalogue's.
    pub probe_chip: Option<String>,
    pub transport: Transport,
    pub action: FlashAction,
    /// The linked ELF. espflash and probe-rs both take the ELF rather than a
    /// raw binary — they derive the flash layout from it.
    ///
    /// Optional because a serial monitor needs none: attaching to a board
    /// flashed last week, or by somebody else, is exactly the case for
    /// looking without building. Given, it turns defmt indices and panic
    /// addresses back into text; absent, everything that writes refuses.
    pub firmware: Option<std::path::PathBuf>,
    /// Decode defmt frames instead of showing raw bytes. Only meaningful when
    /// the firmware was built with defmt; the ELF carries the string table.
    pub defmt: bool,
    /// Serial only. Leave unset for the tool's default.
    pub baud: Option<u32>,
}

/// The chips a port's candidate boards carry — what is plausibly on the
/// other end of this wire.
///
/// Names rather than ids, because that is what device detection stored: the
/// user reads "ESP32-C3-DevKitM-1", not "esp32c3-devkitm-1".
pub fn chips_behind(catalog: &crate::catalog::Catalog, board_names: &[String]) -> Vec<String> {
    let mut out: Vec<String> = catalog
        .boards()
        .iter()
        .filter(|board| board_names.iter().any(|name| name == &board.name))
        .map(|board| board.chip.clone())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// "This cannot be the chip you are building for", when the evidence says so.
///
/// Silence is the default: an adapter rusty does not recognise is not
/// evidence of anything, and a plan blocked on a guess is exactly the
/// failure this workbench exists to prevent. But when the port names boards
/// and *none* of them carries the project's chip, saying nothing means
/// watching espflash fail on a chip magic mismatch — a message that names
/// neither the project nor the way out.
pub fn chip_mismatch(project_chip: &str, candidates: &[String]) -> Option<String> {
    if candidates.is_empty() || candidates.iter().any(|chip| chip == project_chip) {
        return None;
    }
    let seen = candidates.join(" or ");
    // Naming the way out matters more than naming the fault: the old wording
    // sent people off to recreate a project, which is not what this costs.
    // Switching the chip rewrites the target, the toolchain and the crate
    // features; only code that names a pin has to be looked at.
    Some(format!(
        "This project builds for {project_chip}, but the device on this port \
         looks like {seen}. Flashing will fail when the bootloader reports a \
         different chip. Switch this project's chip from the status bar — \
         click `chip {project_chip}` — or pick another port.",
    ))
}

/// [`chip_mismatch`] for the device a flash is about to go to, when the port
/// says so.
///
/// The device row already knows the port names boards; the plan knowing it
/// too is the difference between "espflash failed on a chip magic mismatch"
/// and a sentence naming both chips. A probe reports its own target — it is
/// not a bridge chip that could belong to several boards — so it warns of
/// nothing.
pub fn port_warning(
    chip_id: &str,
    transport: &Transport,
    ports: &[SerialPort],
    catalog: &Catalog,
) -> Option<String> {
    let candidates = match transport {
        Transport::Serial { port } => {
            let names = ports
                .iter()
                .find(|found| &found.name == port)
                .map(|found| found.boards.clone())
                .unwrap_or_default();
            chips_behind(catalog, &names)
        }
        Transport::Probe { .. } => Vec::new(),
    };
    chip_mismatch(chip_id, &candidates)
}

/// Decide what to run. Pure — no process is started.
pub fn plan(request: &FlashRequest) -> Result<CommandPlan> {
    let firmware = request
        .firmware
        .as_ref()
        .map(|path| path.display().to_string());
    // Everything but a serial monitor writes the image, or reads its symbols
    // to find the RTT buffer — so everything but that needs a build.
    let built = || {
        firmware.clone().ok_or_else(|| {
            Error::refused("Nothing has been built yet, so there is no image to use — build first.")
        })
    };
    let chip = chip::by_id(&request.chip_id);

    let (program, args, rationale) = match &request.transport {
        Transport::Serial { port } => {
            // espflash speaks only Espressif's ROM bootloader. Other parts
            // have bootloaders of their own — an STM32's UART one, WCH's ISP
            // — that it cannot talk to, so offering this path for anything
            // else sends the user somewhere that cannot work, with an error
            // that will not explain why.
            if let Some(chip) = &chip
                && !chip.flashers.contains(&crate::model::Flasher::Espflash)
            {
                return Err(Error::NoSerialBootloader {
                    chip: chip.name.clone(),
                });
            }

            let mut args = vec![
                "--chip".into(),
                request.chip_id.clone(),
                "--port".into(),
                port.clone(),
            ];
            if let Some(baud) = request.baud {
                args.push("--baud".into());
                args.push(baud.to_string());
            }
            if request.defmt {
                args.push("--log-format".into());
                args.push("defmt".into());
            }

            match request.action {
                FlashAction::Monitor => {
                    let mut monitor = vec!["monitor".to_string()];
                    monitor.extend(args);
                    // The ELF is what turns defmt indices back into strings,
                    // and what maps a panic address to a line — worth
                    // passing whenever there is one, and no reason not to
                    // attach when there is not.
                    if let Some(firmware) = &firmware {
                        monitor.push("--elf".into());
                        monitor.push(firmware.clone());
                    }
                    (
                        "espflash",
                        monitor,
                        "Attaching over serial without rewriting flash.",
                    )
                }
                action => {
                    let firmware = built()?;
                    let mut flash = vec!["flash".to_string()];
                    flash.extend(args);
                    if action == FlashAction::FlashAndMonitor {
                        flash.push("--monitor".into());
                    }
                    flash.push(firmware);
                    (
                        "espflash",
                        flash,
                        "Flashing over the ROM serial bootloader — no probe needed.",
                    )
                }
            }
        }

        Transport::Probe { identifier } if uses_wlink(chip.as_ref(), identifier.as_deref()) => {
            // wlink numbers probes itself; a probe listed by it carries the
            // number, and with none chosen it takes the first.
            let mut args = vec!["-v".to_string()];
            if let Some(index) = identifier
                .as_deref()
                .and_then(|id| id.strip_prefix("wlink:"))
            {
                args.push("-d".into());
                args.push(index.to_string());
            }
            // SDI print reaches the WCH-LinkE's own serial port, and
            // `--watch-serial` reads it there — the whole of ch32-hal's
            // `println!` path. A monitor resets rather than writes: wlink
            // turns SDI print on only across a reset.
            let rationale = match request.action {
                FlashAction::Flash => {
                    args.extend(["flash".to_string(), built()?]);
                    "Flashing through the WCH-LinkE over the one-wire debug pin, and stopping \
                     there."
                }
                FlashAction::FlashAndMonitor => {
                    args.extend([
                        "flash".to_string(),
                        "--enable-sdi-print".into(),
                        "--watch-serial".into(),
                        built()?,
                    ]);
                    "Flashing through the WCH-LinkE; SDI print arrives on its serial port."
                }
                FlashAction::Monitor => {
                    args.extend([
                        "reset".to_string(),
                        "--enable-sdi-print".into(),
                        "--watch-serial".into(),
                    ]);
                    "Restarting the part without rewriting its flash, with SDI print on — \
                     wlink turns it on only across a reset."
                }
            };
            ("wlink", args, rationale)
        }

        Transport::Probe { identifier } => {
            let target = request
                .probe_chip
                .clone()
                .or_else(|| chip.as_ref().and_then(|c| c.probe_rs_target.clone()))
                .ok_or_else(|| Error::UnknownProbeTarget {
                    chip: request.chip_id.clone(),
                })?;

            // `probe-rs run` flashes, attaches, and decodes RTT in one step,
            // which is the whole inner loop. `download` is the flash-only
            // form, and `attach` the look-only one: it was `run` too once,
            // which rewrote the flash of a board somebody had asked only to
            // watch — destroying the state they wanted to look at. All three
            // read the ELF, `attach` for where the RTT buffer is.
            let (subcommand, rationale) = match request.action {
                FlashAction::Flash => (
                    "download",
                    "Flashing through the debug probe, and stopping there.",
                ),
                FlashAction::FlashAndMonitor => (
                    "run",
                    "Flashing through the debug probe; defmt arrives over RTT.",
                ),
                FlashAction::Monitor => (
                    "attach",
                    "Attaching through the debug probe without rewriting flash; defmt \
                     arrives over RTT.",
                ),
            };
            let mut args = vec![subcommand.to_string(), "--chip".into(), target];
            if let Some(identifier) = identifier {
                args.push("--probe".into());
                args.push(identifier.clone());
            }
            args.push(built()?);

            ("probe-rs", args, rationale)
        }
    };

    let display = std::iter::once(program.to_string())
        .chain(args.iter().map(|a| quote_if_needed(a)))
        .collect::<Vec<_>>()
        .join(" ");

    Ok(CommandPlan {
        program: program.to_string(),
        args,
        display,
        rationale: rationale.to_string(),
        warning: None,
    })
}

/// Whether a probe flash goes through wlink: a probe wlink listed, or no
/// probe chosen on a part that prefers it. A probe probe-rs listed goes
/// through probe-rs, which knows the CH32V003 too.
fn uses_wlink(chip: Option<&crate::model::Chip>, identifier: Option<&str>) -> bool {
    match identifier {
        Some(id) => id.starts_with("wlink:"),
        None => chip.is_some_and(|c| c.flashers.first() == Some(&crate::model::Flasher::Wlink)),
    }
}

/// A line of a flash or monitor's output, without what the tool put in front
/// of the firmware's own words: `wlink --watch-serial` stamps every line with
/// the time it arrived (`2026-10-08 17:30:01.234 `), and a protocol line
/// behind a stamp is a line nothing reads as protocol.
pub fn firmware_line<'a>(program: &str, line: &'a str) -> &'a str {
    if !program.ends_with("wlink") && !program.ends_with("wlink.exe") {
        return line;
    }
    let bytes = line.as_bytes();
    // YYYY-MM-DD HH:MM:SS.mmm
    let stamped = bytes.len() >= 23
        && bytes[..23].iter().enumerate().all(|(i, b)| match i {
            4 | 7 => *b == b'-',
            10 => *b == b' ',
            13 | 16 => *b == b':',
            19 => *b == b'.',
            _ => b.is_ascii_digit(),
        });
    if !stamped {
        return line;
    }
    let rest = &line[23..];
    rest.strip_prefix(": ")
        .or_else(|| rest.strip_prefix(' '))
        .unwrap_or(rest)
}

/// Quote an argument that would not survive being pasted into a shell.
fn quote_if_needed(arg: &str) -> String {
    if arg.contains(' ') {
        format!("\"{arg}\"")
    } else {
        arg.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn port(name: &str, boards: &[&str]) -> SerialPort {
        SerialPort {
            name: name.to_string(),
            bridge: None,
            boards: boards.iter().map(|b| b.to_string()).collect(),
            likely_board: true,
            usb: None,
        }
    }

    fn serial_on(port: &str) -> Transport {
        Transport::Serial {
            port: port.to_string(),
        }
    }

    /// The warning names both chips when the port's boards all carry another
    /// part, and says nothing when the evidence is thinner than that.
    #[test]
    fn a_port_that_cannot_carry_the_projects_chip_is_named_before_the_flash() {
        let catalog = Catalog::builtin();
        let ports = vec![
            port("COM3", &["ESP32-C3-DevKitM-1"]),
            port("COM4", &["ESP32-C3-DevKitM-1", "ESP32-DevKitC V4"]),
            port("COM5", &[]),
        ];

        let warning = port_warning("esp32", &serial_on("COM3"), &ports, &catalog)
            .expect("a C3 board is not an esp32 project's board");
        assert!(warning.contains("esp32c3"), "{warning}");
        assert!(warning.contains("esp32"), "{warning}");

        assert_eq!(
            port_warning("esp32", &serial_on("COM4"), &ports, &catalog),
            None,
            "one of the candidates is the project's chip — no evidence of a mismatch",
        );
        assert_eq!(
            port_warning("esp32", &serial_on("COM5"), &ports, &catalog),
            None,
            "an adapter rusty does not recognise is not evidence of anything",
        );
        assert_eq!(
            port_warning("esp32", &serial_on("COM9"), &ports, &catalog),
            None,
            "a port that is not in the list is not in the list",
        );
        assert_eq!(
            port_warning(
                "esp32",
                &Transport::Probe { identifier: None },
                &ports,
                &catalog
            ),
            None,
            "a probe reports its own target; it is not a bridge chip",
        );
    }

    /// The mismatch names both chips and a way out — and stays quiet when
    /// the evidence does not support it, which is most of the time.
    #[test]
    fn a_mismatch_is_only_claimed_on_evidence() {
        let candidates = vec!["esp32c3".to_string(), "esp32c6".to_string()];

        let warning = chip_mismatch("esp32", &candidates).expect("a mismatch");
        assert!(
            warning.contains("esp32"),
            "names the project's chip: {warning}"
        );
        assert!(
            warning.contains("esp32c3"),
            "names what is plugged in: {warning}"
        );

        assert_eq!(
            chip_mismatch("esp32c3", &candidates),
            None,
            "one candidate matching is a match — boards share bridges",
        );
        assert_eq!(
            chip_mismatch("esp32", &[]),
            None,
            "an unrecognised adapter is not evidence of the wrong chip",
        );
    }
    use std::path::PathBuf;

    fn request(chip_id: &str, transport: Transport, action: FlashAction) -> FlashRequest {
        FlashRequest {
            chip_id: chip_id.to_string(),
            probe_chip: None,
            transport,
            action,
            firmware: Some(PathBuf::from("target/blinky")),
            defmt: false,
            baud: None,
        }
    }

    /// Attaching to a board needs no build: the one flashed last week is
    /// exactly the one somebody wants to watch. The ELF is passed when it
    /// exists, because it is what decodes defmt and panics.
    #[test]
    fn a_serial_monitor_attaches_without_a_build() {
        let mut req = request("esp32c3", serial(), FlashAction::Monitor);
        req.firmware = None;
        let plan = plan(&req).unwrap();
        assert_eq!(plan.args[0], "monitor");
        assert!(!plan.args.contains(&"--elf".to_string()));
        assert!(plan.display.contains("COM3"));
    }

    /// Everything that writes needs an image, and says so rather than
    /// handing espflash a path that is not there.
    #[test]
    fn writing_without_a_build_is_refused_by_name() {
        for action in [FlashAction::Flash, FlashAction::FlashAndMonitor] {
            let mut req = request("esp32c3", serial(), action);
            req.firmware = None;
            let err = plan(&req).unwrap_err().to_string();
            assert!(err.contains("build first"), "{err}");
        }
    }

    /// Watching through a probe must not rewrite the flash: that destroys
    /// the state of the board somebody attached to look at.
    #[test]
    fn a_probe_monitor_attaches_rather_than_flashing() {
        let plan = plan(&request(
            "esp32c3",
            Transport::Probe { identifier: None },
            FlashAction::Monitor,
        ))
        .unwrap();
        assert_eq!(plan.program, "probe-rs");
        assert_eq!(plan.args[0], "attach");
    }

    fn serial() -> Transport {
        Transport::Serial {
            port: "COM3".to_string(),
        }
    }

    #[test]
    fn serial_flash_uses_espflash_with_chip_and_port() {
        let plan = plan(&request("esp32c3", serial(), FlashAction::FlashAndMonitor)).unwrap();

        assert_eq!(plan.program, "espflash");
        assert_eq!(plan.args[0], "flash");
        assert!(plan.args.contains(&"--chip".to_string()));
        assert!(plan.args.contains(&"esp32c3".to_string()));
        assert!(plan.args.contains(&"COM3".to_string()));
        assert!(plan.args.contains(&"--monitor".to_string()));
        // The command shown to the user must be the command that runs.
        assert!(plan.display.starts_with("espflash flash "));
        assert!(plan.display.contains("COM3"));
    }

    #[test]
    fn flash_without_monitor_does_not_attach() {
        let plan = plan(&request("esp32c3", serial(), FlashAction::Flash)).unwrap();
        assert!(!plan.args.contains(&"--monitor".to_string()));
    }

    #[test]
    fn monitor_passes_the_elf_so_defmt_and_backtraces_resolve() {
        let mut req = request("esp32c3", serial(), FlashAction::Monitor);
        req.defmt = true;
        let plan = plan(&req).unwrap();

        assert_eq!(plan.args[0], "monitor");
        // Without --elf the decoder has no string table and shows indices.
        assert!(plan.args.contains(&"--elf".to_string()));
        assert!(plan.args.contains(&"defmt".to_string()));
    }

    #[test]
    fn baud_is_only_sent_when_asked_for() {
        let plain = plan(&request("esp32c3", serial(), FlashAction::Flash)).unwrap();
        assert!(!plain.args.contains(&"--baud".to_string()));

        let mut fast = request("esp32c3", serial(), FlashAction::Flash);
        fast.baud = Some(921_600);
        let fast = plan(&fast).unwrap();
        assert!(fast.args.contains(&"921600".to_string()));
    }

    /// espflash cannot talk to an STM32's bootloader. Producing an espflash
    /// command for it
    /// would fail with an error about the chip, sending the user to debug
    /// entirely the wrong thing.
    #[test]
    fn a_part_without_a_serial_bootloader_refuses_the_serial_path() {
        let err = plan(&request("stm32f411", serial(), FlashAction::Flash))
            .unwrap_err()
            .to_string();
        assert!(err.contains("STM32F411"), "{err}");
        assert!(
            err.contains("probe"),
            "the message has to point at the fix: {err}"
        );
    }

    #[test]
    fn probe_flash_uses_run_so_rtt_is_attached() {
        let plan = plan(&request(
            "esp32c3",
            Transport::Probe { identifier: None },
            FlashAction::FlashAndMonitor,
        ))
        .unwrap();

        assert_eq!(plan.program, "probe-rs");
        assert_eq!(plan.args[0], "run");
        assert!(plan.args.contains(&"esp32c3".to_string()));
    }

    #[test]
    fn probe_flash_only_uses_download() {
        let plan = plan(&request(
            "esp32c3",
            Transport::Probe { identifier: None },
            FlashAction::Flash,
        ))
        .unwrap();
        assert_eq!(plan.args[0], "download");
    }

    /// probe-rs target names for STM32 depend on package and flash size, which
    /// the die alone does not determine. Guessing one produces a plausible name
    /// with the wrong memory map — a far worse outcome than refusing.
    #[test]
    fn an_unknown_probe_target_refuses_rather_than_guessing() {
        let err = plan(&request(
            "stm32f411",
            Transport::Probe { identifier: None },
            FlashAction::Flash,
        ))
        .unwrap_err()
        .to_string();
        assert!(err.contains("probe-rs chip list"), "{err}");
    }

    /// A CH32V003 flashes through its own probe's tool unless a probe-rs
    /// probe was picked, and a probe wlink listed carries its number.
    #[test]
    fn a_ch32_flashes_through_wlink_and_monitors_its_sdi_print() {
        let probe = |identifier: Option<&str>| Transport::Probe {
            identifier: identifier.map(str::to_string),
        };
        let run = plan(&request(
            "ch32v003j4m6",
            probe(None),
            FlashAction::FlashAndMonitor,
        ))
        .unwrap();
        assert_eq!(run.program, "wlink");
        assert_eq!(
            run.args,
            [
                "-v",
                "flash",
                "--enable-sdi-print",
                "--watch-serial",
                "target/blinky"
            ]
        );
        let second = plan(&request(
            "ch32v003j4m6",
            probe(Some("wlink:1")),
            FlashAction::Flash,
        ))
        .unwrap();
        assert_eq!(second.args, ["-v", "-d", "1", "flash", "target/blinky"]);
        let watch = plan(&request("ch32v003j4m6", probe(None), FlashAction::Monitor)).unwrap();
        assert!(
            !watch.args.contains(&"flash".to_string()),
            "a monitor never writes"
        );
        let through_probe_rs = plan(&request(
            "ch32v003j4m6",
            probe(Some("WCH-Link -- 1a86:8010:abc (WchLink)")),
            FlashAction::Flash,
        ))
        .unwrap();
        assert_eq!(through_probe_rs.program, "probe-rs");
        assert!(through_probe_rs.args.contains(&"CH32V003J4M6".to_string()));

        // The X035 the same way: wlink first, probe-rs by its own name.
        let x035 = plan(&request("ch32x035f8u6", probe(None), FlashAction::Flash)).unwrap();
        assert_eq!(x035.program, "wlink");
        let x035 = plan(&request(
            "ch32x035f8u6",
            probe(Some("WCH-Link -- 1a86:8010:abc (WchLink)")),
            FlashAction::Flash,
        ))
        .unwrap();
        assert!(x035.args.contains(&"CH32X035F8U6".to_string()));
    }

    #[test]
    fn a_ch32_has_no_serial_bootloader_to_flash_through() {
        let serial = Transport::Serial {
            port: "COM5".to_string(),
        };
        assert!(plan(&request("ch32v003j4m6", serial, FlashAction::Flash)).is_err());
    }

    #[test]
    fn wlink_s_timestamp_comes_off_a_watched_line() {
        let line = "2026-10-08 17:30:01.234 [rusty:tel@10] y=1";
        assert_eq!(firmware_line("wlink", line), "[rusty:tel@10] y=1");
        assert_eq!(
            firmware_line("C:/x/wlink.exe", "2026-10-08 17:30:01.234: hi"),
            "hi"
        );
        assert_eq!(firmware_line("espflash", line), line);
        assert_eq!(firmware_line("wlink", "Flash done"), "Flash done");
    }
}
