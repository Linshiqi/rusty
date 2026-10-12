//! What is plugged in.
//!
//! Two ways onto a board, and they are not interchangeable:
//!
//! - A **serial port**, through the USB-to-UART bridge on the module or through
//!   the chip's own USB peripheral. Enough to flash and to read logs. Every
//!   Espressif part has a serial bootloader in ROM, so this needs no extra
//!   hardware.
//! - A **debug probe**, over JTAG/SWD. Adds breakpoints, memory inspection, and
//!   defmt over RTT. Required for STM32 and WCH's parts, whose bootloaders
//!   espflash does not speak.
//! - A **board waiting in its USB bootloader** ([`list_boot_devices`]): an RP2040
//!   or RP2350 held in BOOTSEL mounts as a drive with an `INFO_UF2.TXT` on it,
//!   and an STM32 started with BOOT0 high answers USB DFU. Neither needs a
//!   probe, which is the board most people buy.
//!
//! Naming the bridge chip matters more than it looks: "CP210x" and "CH340" are
//! what a user sees printed on the board, and matching that against a COM
//! number is the difference between picking the right port and flashing their
//! Arduino by mistake.

use std::path::{Path, PathBuf};

use crate::{
    catalog::Catalog,
    model::{BootDevice, BootKind, Probe, SerialPort, UsbIdentity},
    process, tools,
};

/// Boards waiting in a USB bootloader: UF2 drives, then DFU devices.
pub fn list_boot_devices() -> Vec<BootDevice> {
    let mut out: Vec<BootDevice> = drive_roots()
        .iter()
        .filter_map(|root| uf2_drive_at(root))
        .collect();
    out.extend(dfu_devices());
    out
}

/// Where a removable drive mounts on this machine: every drive letter on
/// Windows (but the two floppy letters, which can stall a read), the volumes
/// on macOS, and the per-user media directories on Linux.
fn drive_roots() -> Vec<PathBuf> {
    if cfg!(windows) {
        return (b'C'..=b'Z')
            .map(|letter| PathBuf::from(format!("{}:\\", letter as char)))
            .collect();
    }
    let mut bases = vec![PathBuf::from("/Volumes"), PathBuf::from("/media")];
    if let Ok(user) = std::env::var("USER") {
        bases.push(PathBuf::from("/media").join(&user));
        bases.push(PathBuf::from("/run/media").join(&user));
    }
    bases
        .iter()
        .filter_map(|base| std::fs::read_dir(base).ok())
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .collect()
}

/// The UF2 bootloader drive mounted at `root`, if that is what it is: one
/// with an `INFO_UF2.TXT`, named by the `Board-ID` it gives.
pub fn uf2_drive_at(root: &Path) -> Option<BootDevice> {
    let info = std::fs::read_to_string(root.join("INFO_UF2.TXT")).ok()?;
    Some(BootDevice {
        kind: BootKind::Uf2,
        id: root.display().to_string(),
        serial: None,
        label: uf2_board_id(&info)?,
    })
}

/// The `Board-ID:` line of an `INFO_UF2.TXT` — `RPI-RP2` on an RP2040,
/// `RP2350` on an RP2350.
pub fn uf2_board_id(info: &str) -> Option<String> {
    info.lines()
        .find_map(|line| line.strip_prefix("Board-ID:"))
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
}

/// Devices `dfu-util -l` finds, when dfu-util is installed. Absent means
/// none, and costs no spawn.
fn dfu_devices() -> Vec<BootDevice> {
    let Some(dfu_util) = tools::find("dfu-util") else {
        return Vec::new();
    };
    let mut command = process::command(dfu_util);
    command.arg("-l");
    match command.output() {
        Ok(output) => parse_dfu_list(&String::from_utf8_lossy(&output.stdout)),
        Err(_) => Vec::new(),
    }
}

/// `dfu-util -l`'s `Found DFU:` lines, one device each: its `vid:pid`, its
/// serial, and the name of its first alternate setting (an STM32's
/// `@Internal Flash  /0x08000000/…`). A device in runtime mode (`Found
/// Runtime:`) is not in its bootloader yet and is left out.
pub fn parse_dfu_list(text: &str) -> Vec<BootDevice> {
    let field = |line: &str, key: &str| -> Option<String> {
        let at = line.find(&format!("{key}="))? + key.len() + 1;
        let rest = &line[at..];
        Some(match rest.strip_prefix('"') {
            Some(quoted) => quoted.split('"').next()?.to_string(),
            None => rest.split([',', ' ']).next()?.to_string(),
        })
    };
    let mut out: Vec<BootDevice> = Vec::new();
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("Found DFU: [") else {
            continue;
        };
        let Some((id, _)) = rest.split_once(']') else {
            continue;
        };
        let serial = field(line, "serial").filter(|s| !s.is_empty() && s != "UNKNOWN");
        if out
            .iter()
            .any(|device| device.id == id && device.serial == serial)
        {
            continue;
        }
        let name = field(line, "name").unwrap_or_default();
        let label = name
            .trim_start_matches('@')
            .split('/')
            .next()
            .unwrap_or_default()
            .trim()
            .to_string();
        out.push(BootDevice {
            kind: BootKind::Dfu,
            id: id.to_string(),
            serial,
            label: if label.is_empty() {
                id.to_string()
            } else {
                label
            },
        });
    }
    out
}

/// USB vendor/product pairs seen on Espressif development boards.
///
/// The last entry is the interesting one: Espressif's own vendor id means the
/// chip is presenting USB directly, with no bridge chip on the board — which
/// also means the port disappears when the firmware reconfigures USB, and that
/// is a real support question rather than a fault.
const KNOWN_BRIDGES: &[(u16, u16, &str)] = &[
    (0x10C4, 0xEA60, "Silicon Labs CP210x"),
    (0x10C4, 0xEA70, "Silicon Labs CP2105"),
    (0x1A86, 0x7523, "WCH CH340"),
    (0x1A86, 0x55D4, "WCH CH9102"),
    (0x0403, 0x6001, "FTDI FT232"),
    (0x0403, 0x6010, "FTDI FT2232"),
    (0x303A, 0x1001, "Espressif native USB (USB Serial/JTAG)"),
    (0x303A, 0x0002, "Espressif native USB (CDC)"),
];

/// Vendor ids belonging to debug probes rather than serial bridges.
const PROBE_VENDORS: &[(u16, &str)] = &[
    (0x0483, "ST-LINK"),
    (0x1366, "SEGGER J-Link"),
    (0x2E8A, "Raspberry Pi Debug Probe"),
    (0x1209, "CMSIS-DAP (community)"),
    (0x0D28, "CMSIS-DAP (Arm)"),
];

fn describe(vid: u16, pid: u16) -> Option<&'static str> {
    KNOWN_BRIDGES
        .iter()
        .find(|(v, p, _)| *v == vid && *p == pid)
        .map(|(_, _, name)| *name)
        .or_else(|| {
            PROBE_VENDORS
                .iter()
                .find(|(v, _)| *v == vid)
                .map(|(_, name)| *name)
        })
}

/// Serial ports currently present, named against the board catalogue.
///
/// Returns an empty list rather than an error when enumeration fails: on a
/// machine with no ports at all that is the truthful answer, and an error would
/// make the panel look broken when nothing is wrong.
pub fn list_serial_ports(catalog: &Catalog) -> Vec<SerialPort> {
    let Ok(ports) = serialport::available_ports() else {
        return Vec::new();
    };

    let mut out: Vec<SerialPort> = ports
        .into_iter()
        .map(|port| {
            let usb = match &port.port_type {
                serialport::SerialPortType::UsbPort(info) => Some(UsbIdentity {
                    vendor_id: info.vid,
                    product_id: info.pid,
                    manufacturer: info.manufacturer.clone(),
                    product: info.product.clone(),
                    serial_number: info.serial_number.clone(),
                }),
                _ => None,
            };

            // A named board beats a named bridge: "ESP32-C3-DevKitM-1" is what
            // the user has on the desk, "CP210x" is a chip on it.
            let boards: Vec<String> = usb
                .as_ref()
                .map(|u| {
                    catalog
                        .boards_for_usb(u.vendor_id, u.product_id)
                        .into_iter()
                        .map(|b| b.name.clone())
                        .collect()
                })
                .unwrap_or_default();

            let bridge = usb
                .as_ref()
                .and_then(|u| describe(u.vendor_id, u.product_id))
                .map(str::to_string);

            SerialPort {
                name: port.port_name,
                // Either a known board or a known bridge means this is almost
                // certainly it; everything else is modems, Bluetooth stacks,
                // and virtual ports that would only waste the user's time.
                likely_board: !boards.is_empty() || bridge.is_some(),
                boards,
                bridge,
                usb,
            }
        })
        .collect();

    // Likely boards first, then stable by name so the list does not reshuffle
    // between refreshes.
    out.sort_by(|a, b| {
        b.likely_board
            .cmp(&a.likely_board)
            .then_with(|| a.name.cmp(&b.name))
    });
    out
}

/// Debug probes, as reported by `probe-rs list`.
///
/// Shelling out rather than linking probe-rs: the CLI is the supported
/// interface, it is what the user will run by hand anyway, and linking the
/// library would pull a USB stack into a desktop app that mostly does not need
/// one.
pub fn list_probes() -> Vec<Probe> {
    let mut probes = probe_rs_probes();
    probes.extend(wlink_probes());
    probes
}

/// WCH-Link probes, as `wlink list` numbers them — the number is what its
/// `-d` takes, so it travels as the identifier (`wlink:0`). Asked of wlink
/// rather than read off USB because a WCH-LinkE in DAP mode answers to
/// another product id, and wlink says which mode it is in.
fn wlink_probes() -> Vec<Probe> {
    let Some(wlink) = tools::find("wlink") else {
        return Vec::new();
    };
    let mut command = process::command(wlink);
    command.arg("list");
    let Ok(output) = command.output() else {
        return Vec::new();
    };
    parse_wlink_list(&String::from_utf8_lossy(&output.stdout))
}

/// `<WCH-Link#0 nusb device> ID 1a86:8010 Serial … (RV mode)`, one per probe.
fn parse_wlink_list(text: &str) -> Vec<Probe> {
    text.lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix("<WCH-Link#")?;
            let index: String = rest.chars().take_while(char::is_ascii_digit).collect();
            (!index.is_empty()).then(|| Probe {
                identifier: format!("wlink:{index}"),
                description: line.trim().to_string(),
            })
        })
        .collect()
}

fn probe_rs_probes() -> Vec<Probe> {
    // Found by the same ladder the toolchain panel reports it with, so a
    // probe-rs in `~/.cargo/bin` that is not on this window's PATH is listed
    // as installed *and* asked. Absent means no probes, and costs no spawn.
    let Some(probe_rs) = tools::find("probe-rs") else {
        return Vec::new();
    };
    let mut command = process::command(probe_rs);
    command.arg("list");

    let Ok(output) = command.output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        // `probe-rs list` prints a header line when it finds nothing, and an
        // enumerated list otherwise. Only the enumerated entries matter.
        .filter(|line| line.starts_with(|c: char| c.is_ascii_digit()))
        .map(|line| {
            let description = line
                .split_once(':')
                .map(|(_, rest)| rest.trim())
                .unwrap_or(line)
                .to_string();
            Probe {
                identifier: description.clone(),
                description,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_bridges_are_named_by_what_is_printed_on_the_board() {
        assert_eq!(describe(0x10C4, 0xEA60), Some("Silicon Labs CP210x"));
        assert_eq!(describe(0x1A86, 0x7523), Some("WCH CH340"));
        // Espressif's own vendor id: the chip is the USB device, there is no
        // bridge chip at all.
        assert!(describe(0x303A, 0x1001).unwrap().contains("native USB"));
        // Probes are matched on vendor alone, because the product id varies
        // across every clone board in existence.
        assert_eq!(describe(0x0483, 0x3748), Some("ST-LINK"));
        assert_eq!(describe(0x1366, 0x9999), Some("SEGGER J-Link"));
        assert_eq!(describe(0xDEAD, 0xBEEF), None);
    }

    #[test]
    fn a_wlink_probe_is_identified_by_the_index_its_d_flag_takes() {
        let listed = "<WCH-Link#0 nusb device> ID 1a86:8010 Serial 1234 (Full) (RV mode)\n\
                      <WCH-Link#1 WCHLinkDLL device> CH375Driver Device 1a86:8010\n\
                      some log line\n";
        let probes = parse_wlink_list(listed);
        let ids: Vec<&str> = probes.iter().map(|p| p.identifier.as_str()).collect();
        assert_eq!(ids, ["wlink:0", "wlink:1"]);
        assert!(probes[0].description.contains("RV mode"));
    }

    #[test]
    fn enumeration_never_fails_the_caller() {
        // Whatever this machine has, listing must not panic or error — a
        // developer machine with no board attached is the normal case.
        let _ = list_serial_ports(&Catalog::builtin());
        let _ = list_probes();
    }

    #[test]
    fn a_catalogued_board_is_matched_by_its_usb_identity() {
        let catalog = Catalog::builtin();

        // The XIAO and the C3 devkit both enumerate as Espressif native USB,
        // so this must return every candidate rather than picking one.
        let matches = catalog.boards_for_usb(0x303A, 0x1001);
        assert!(matches.len() > 1, "expected several, got {matches:?}");
        assert!(matches.iter().all(|b| b.chip.starts_with("esp32")));

        // A CH340 board is a different device entirely.
        let ch340 = catalog.boards_for_usb(0x1A86, 0x7523);
        assert!(ch340.iter().any(|b| b.name.contains("M5Stamp")));

        assert!(catalog.boards_for_usb(0xDEAD, 0xBEEF).is_empty());
    }

    /// `dfu-util -l` as it prints an STM32 in its system bootloader: one
    /// device, however many alternate settings it lists, named by its first;
    /// a device still in runtime mode is not one to write.
    #[test]
    fn dfu_util_lists_one_device_per_board_in_its_bootloader() {
        let listed = "dfu-util 0.11\n\nCopyright 2005-2009 Weston Schmidt, Harald Welte and OpenMoko Inc.\n\
Found Runtime: [0483:5740] ver=0200, devnum=7, cfg=1, intf=0, path=\"1-2\", alt=0, name=\"UNKNOWN\", serial=\"UNKNOWN\"\n\
Found DFU: [0483:df11] ver=2200, devnum=8, cfg=1, intf=0, path=\"1-1\", alt=3, name=\"@Device Feature/0xFFFF0000/01*004 e\", serial=\"3276395A3438\"\n\
Found DFU: [0483:df11] ver=2200, devnum=8, cfg=1, intf=0, path=\"1-1\", alt=0, name=\"@Internal Flash  /0x08000000/04*016Kg,01*064Kg,03*128Kg\", serial=\"3276395A3438\"\n";
        let devices = parse_dfu_list(listed);
        assert_eq!(devices.len(), 1, "{devices:?}");
        assert_eq!(devices[0].id, "0483:df11");
        assert_eq!(devices[0].serial.as_deref(), Some("3276395A3438"));
        assert_eq!(devices[0].kind, BootKind::Dfu);
    }

    /// A UF2 drive is told by its INFO_UF2.TXT and named by its Board-ID.
    #[test]
    fn a_uf2_drive_is_read_off_its_info_file() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            uf2_drive_at(dir.path()).is_none(),
            "a drive without the file is none"
        );
        std::fs::write(
            dir.path().join("INFO_UF2.TXT"),
            "UF2 Bootloader v3.0\r\nModel: Raspberry Pi RP2\r\nBoard-ID: RPI-RP2\r\n",
        )
        .unwrap();
        let drive = uf2_drive_at(dir.path()).unwrap();
        assert_eq!(drive.label, "RPI-RP2");
        assert_eq!(drive.kind, BootKind::Uf2);
    }
}
