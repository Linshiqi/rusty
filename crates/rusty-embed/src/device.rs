//! What is plugged in.
//!
//! Two ways onto a board, and they are not interchangeable:
//!
//! - A **serial port**, through the USB-to-UART bridge on the module or through
//!   the chip's own USB peripheral. Enough to flash and to read logs. Every
//!   Espressif part has a serial bootloader in ROM, so this needs no extra
//!   hardware.
//! - A **debug probe**, over JTAG/SWD. Adds breakpoints, memory inspection, and
//!   defmt over RTT. Required for STM32, which has no serial bootloader.
//!
//! Naming the bridge chip matters more than it looks: "CP210x" and "CH340" are
//! what a user sees printed on the board, and matching that against a COM
//! number is the difference between picking the right port and flashing their
//! Arduino by mistake.

use std::process::Command;

use crate::{
    catalog::Catalog,
    model::{Probe, SerialPort, UsbIdentity},
};

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
    let mut command = Command::new("probe-rs");
    command.arg("list");
    super::toolchain::no_console_window(&mut command);

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
}
