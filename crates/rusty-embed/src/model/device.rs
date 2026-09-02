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

/// How to reach the board.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Transport {
    /// Through the ROM serial bootloader. No extra hardware; Espressif only.
    Serial { port: String },
    /// Through a JTAG/SWD probe. Adds breakpoints and RTT, and is the only way
    /// onto a part with no serial bootloader.
    Probe { identifier: Option<String> },
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
