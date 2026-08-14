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
    chip,
    error::{Error, Result},
    model::{CommandPlan, FlashAction, Transport},
};

/// Everything needed to decide on a command.
#[derive(Debug, Clone)]
pub struct FlashRequest {
    pub chip_id: String,
    pub transport: Transport,
    pub action: FlashAction,
    /// The linked ELF. espflash and probe-rs both take the ELF rather than a
    /// raw binary — they derive the flash layout from it.
    pub firmware: std::path::PathBuf,
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

/// Decide what to run. Pure — no process is started.
pub fn plan(request: &FlashRequest) -> Result<CommandPlan> {
    let firmware = request.firmware.display().to_string();
    let chip = chip::by_id(&request.chip_id);

    let (program, args, rationale) = match &request.transport {
        Transport::Serial { port } => {
            // Only Espressif parts have a serial bootloader in ROM. Offering
            // this path for anything else sends the user somewhere that cannot
            // work, with an error that will not explain why.
            if let Some(chip) = &chip
                && !chip.flashers.contains(&crate::model::Flasher::Espflash)
            {
                return Err(Error::NoSerialBootloader {
                    chip: chip.name.clone(),
                });
            }

            let mut args = vec!["--chip".into(), request.chip_id.clone(), "--port".into(), port.clone()];
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
                    // and what maps a panic address to a line.
                    monitor.push("--elf".into());
                    monitor.push(firmware.clone());
                    (
                        "espflash",
                        monitor,
                        "Attaching over serial without rewriting flash.",
                    )
                }
                action => {
                    let mut flash = vec!["flash".to_string()];
                    flash.extend(args);
                    if action == FlashAction::FlashAndMonitor {
                        flash.push("--monitor".into());
                    }
                    flash.push(firmware.clone());
                    (
                        "espflash",
                        flash,
                        "Flashing over the ROM serial bootloader — no probe needed.",
                    )
                }
            }
        }

        Transport::Probe { identifier } => {
            let target = chip
                .as_ref()
                .and_then(|c| c.probe_rs_target.clone())
                .ok_or_else(|| Error::UnknownProbeTarget {
                    chip: request.chip_id.clone(),
                })?;

            // `probe-rs run` flashes, attaches, and decodes RTT in one step,
            // which is the whole inner loop. `download` is the flash-only form.
            let subcommand = match request.action {
                FlashAction::Flash => "download",
                _ => "run",
            };
            let mut args = vec![subcommand.to_string(), "--chip".into(), target];
            if let Some(identifier) = identifier {
                args.push("--probe".into());
                args.push(identifier.clone());
            }
            args.push(firmware.clone());

            (
                "probe-rs",
                args,
                "Flashing through the debug probe; defmt arrives over RTT.",
            )
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

    /// The mismatch names both chips and a way out — and stays quiet when
    /// the evidence does not support it, which is most of the time.
    #[test]
    fn a_mismatch_is_only_claimed_on_evidence() {
        let candidates = vec!["esp32c3".to_string(), "esp32c6".to_string()];

        let warning = chip_mismatch("esp32", &candidates).expect("a mismatch");
        assert!(warning.contains("esp32"), "names the project's chip: {warning}");
        assert!(warning.contains("esp32c3"), "names what is plugged in: {warning}");

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
            transport,
            action,
            firmware: PathBuf::from("target/blinky"),
            defmt: false,
            baud: None,
        }
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

    /// STM32 has no serial bootloader. Producing an espflash command for it
    /// would fail with an error about the chip, sending the user to debug
    /// entirely the wrong thing.
    #[test]
    fn a_part_without_a_serial_bootloader_refuses_the_serial_path() {
        let err = plan(&request("stm32f411", serial(), FlashAction::Flash))
            .unwrap_err()
            .to_string();
        assert!(err.contains("STM32F411"), "{err}");
        assert!(err.contains("probe"), "the message has to point at the fix: {err}");
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
}
