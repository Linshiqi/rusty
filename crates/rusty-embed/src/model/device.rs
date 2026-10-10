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
