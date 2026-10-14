//! What is plugged in, and how to reach it.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbIdentity {
    pub vendor_id: u16,
    pub product_id: u16,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub serial_number: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SerialPort {
    /// OS name: `COM3`, `/dev/ttyUSB0`, `/dev/cu.usbserial-0001`.
    pub name: String,
    /// The USB-to-UART bridge, named as it is printed on the board — `CP210x`,
    /// `CH340`. The fallback when no board in the catalogue matches.
    pub bridge: Option<String>,
    /// Boards whose USB identity matches this port.
    ///
    /// Usually zero or one. More than one means several boards share a bridge
    /// chip — very common, since a CP210x is a CP210x — and the UI has to let
    /// the user pick rather than guessing.
    pub boards: Vec<String>,
    /// True when this looks like a development board rather than a modem or a
    /// virtual port, which would only waste the user's time.
    pub likely_board: bool,
    pub usb: Option<UsbIdentity>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Probe {
    /// What `probe-rs --probe` expects.
    pub identifier: String,
    pub description: String,
}

/// A board waiting in its USB bootloader, which takes an image with no
/// probe and no serial bootloader: a Raspberry Pi RP2040 or RP2350 held in
/// BOOTSEL, which mounts as a drive, or an STM32 in its system bootloader,
/// which speaks USB DFU.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BootDevice {
    pub kind: BootKind,
    /// What the transport carries: the drive's root (`E:\`,
    /// `/media/me/RPI-RP2`) or the DFU device's `vid:pid`.
    pub id: String,
    /// The device's serial number, for telling two DFU boards apart.
    #[serde(default)]
    pub serial: Option<String>,
    /// How the device names itself: the UF2 drive's `Board-ID`
    /// (`RPI-RP2`, `RP2350`), or the DFU interface's name.
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BootKind {
    Uf2,
    Dfu,
}

/// How to reach the board.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Transport {
    /// Through the ROM serial bootloader. No extra hardware; Espressif only.
    Serial { port: String },
    /// Through a JTAG/SWD probe. Adds breakpoints and RTT, and is the only way
    /// onto a part with no serial bootloader.
    Probe { identifier: Option<String> },
    /// Onto a UF2 bootloader's drive: the image copied as a UF2 file.
    Uf2 { drive: String },
    /// Through USB DFU, `dfu-util` writing a raw binary.
    Dfu {
        device: String,
        #[serde(default)]
        serial: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FlashAction {
    /// Write the image and stop.
    Flash,
    /// Attach to a board already running, without rewriting flash.
    Monitor,
    /// Write, then stay attached for logs. The usual inner loop.
    FlashAndMonitor,
}

/// The device a verb should use when none is chosen, when that is not a
/// guess: the one board plugged in. `None` means ask.
///
/// `serial` is whether the project's chip has a ROM serial bootloader. When
/// it does, only ports that look like boards count — an ESP32-C3 with native
/// USB appears as a port *and* a probe, and counting both would ask every
/// time about one board. When it does not, the probes are all there is —
/// with `boot`, the boards waiting in a bootloader the part can be written
/// through: a Pico held in BOOTSEL with no probe plugged in is the one
/// board, as much as a probe would be.
pub fn only_candidate(
    ports: &[SerialPort],
    probes: &[Probe],
    boot: &[BootDevice],
    serial: bool,
) -> Option<Transport> {
    if serial {
        let mut boards = ports.iter().filter(|port| port.likely_board);
        let first = boards.next()?;
        return boards.next().is_none().then(|| Transport::Serial {
            port: first.name.clone(),
        });
    }
    let mut found = probes
        .iter()
        .map(|probe| Transport::Probe {
            identifier: Some(probe.identifier.clone()),
        })
        .chain(boot.iter().map(boot_transport));
    let first = found.next()?;
    found.next().is_none().then_some(first)
}

/// How a board waiting in its bootloader is written to.
pub fn boot_transport(device: &BootDevice) -> Transport {
    match device.kind {
        BootKind::Uf2 => Transport::Uf2 {
            drive: device.id.clone(),
        },
        BootKind::Dfu => Transport::Dfu {
            device: device.id.clone(),
            serial: device.serial.clone(),
        },
    }
}

/// Whether a part is flashed over serial. An unknown part is taken to be:
/// the serial path is the ordinary one, and the planner refuses it by name
/// for a part that has no bootloader.
pub fn serial_capable(chip: Option<&super::Chip>) -> bool {
    chip.is_none_or(|c| c.flashers.is_empty() || c.flashers.contains(&super::Flasher::Espflash))
}

/// The bootloaders a part can be written through, of those plugged in: a
/// UF2 drive for a part with a UF2 boot ROM, a DFU device for one with a
/// DFU bootloader. An unknown part has none.
pub fn usable_boot(chip: Option<&super::Chip>, boot: &[BootDevice]) -> Vec<BootDevice> {
    let flashers = chip.map(|c| c.flashers.as_slice()).unwrap_or_default();
    boot.iter()
        .filter(|device| match device.kind {
            BootKind::Uf2 => flashers.contains(&super::Flasher::Uf2),
            BootKind::Dfu => flashers.contains(&super::Flasher::Dfu),
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn port(name: &str, likely_board: bool) -> SerialPort {
        SerialPort {
            name: name.to_string(),
            bridge: Some("CP210x".to_string()),
            boards: Vec::new(),
            likely_board,
            usb: None,
        }
    }

    fn probe(id: &str) -> Probe {
        Probe {
            identifier: id.to_string(),
            description: "ESP JTAG".to_string(),
        }
    }

    /// One board plugged in is the answer; two is a question. A modem or a
    /// Bluetooth port is never a candidate, so it cannot make one board into
    /// a question either.
    #[test]
    fn the_only_board_is_chosen_and_anything_more_is_asked() {
        let one = [port("COM3", true), port("COM1", false)];
        assert!(matches!(
            only_candidate(&one, &[], &[], true),
            Some(Transport::Serial { port }) if port == "COM3"
        ));
        let two = [port("COM3", true), port("COM7", true)];
        assert!(only_candidate(&two, &[], &[], true).is_none());
        assert!(only_candidate(&[port("COM1", false)], &[], &[], true).is_none());
        assert!(only_candidate(&[], &[], &[], true).is_none());
    }

    /// A C3 on native USB is a port and a probe at once. For a chip with a
    /// serial bootloader the port is the answer and the probe is not a
    /// second candidate; for one without, the probe is all there is.
    #[test]
    fn a_serial_chip_counts_ports_and_a_probe_only_chip_counts_probes() {
        let ports = [port("COM5", true)];
        let probes = [probe("303a:1001:34:85:18:0A:47:F4")];
        assert!(matches!(
            only_candidate(&ports, &probes, &[], true),
            Some(Transport::Serial { .. })
        ));
        assert!(matches!(
            only_candidate(&ports, &probes, &[], false),
            Some(Transport::Probe { identifier: Some(id) }) if id.starts_with("303a")
        ));
        assert!(only_candidate(&ports, &[probe("a"), probe("b")], &[], false).is_none());
    }

    /// A Pico held in BOOTSEL with no probe plugged in is the one board; a
    /// probe as well is two, and asked about.
    #[test]
    fn a_board_in_its_bootloader_is_a_candidate_like_a_probe() {
        let drive = BootDevice {
            kind: BootKind::Uf2,
            id: "E:\\".to_string(),
            serial: None,
            label: "RPI-RP2".to_string(),
        };
        assert!(matches!(
            only_candidate(&[], &[], std::slice::from_ref(&drive), false),
            Some(Transport::Uf2 { drive }) if drive == "E:\\"
        ));
        assert!(only_candidate(&[], &[probe("a")], &[drive], false).is_none());
    }
}
