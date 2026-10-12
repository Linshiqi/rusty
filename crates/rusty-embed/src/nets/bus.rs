//! The parts a sheet puts on the I2C bus and on SPI chip selects, read off
//! their properties and checked against their wiring.

use serde::{Deserialize, Serialize};

use super::{Row, Warning, gpio_of};
use crate::model::{Instance, Sheet};
use crate::protocol::hex_bytes;

/// A part the sheet puts on a chip select, and the bytes it answers with.
///
/// The SPI half of [`BusDevice`], and deliberately simpler because SPI is:
/// there is no addressing to key an answer on, so what a device says is one
/// buffer read from its start on every transfer. A display declares nothing
/// and is written to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireDevice {
    pub part: String,
    pub select: u8,
    pub miso: Vec<u8>,
}

/// How many chip selects the peripheral has. Beyond this a `cs` is a typo,
/// not a line.
const WIRE_SELECTS: u8 = 6;

/// Every device the sheet puts on a chip select, and what it wants said
/// about the ones it could not.
///
/// The same two rules as the I2C half: a `cs` prop is what puts a part on
/// the wire, and its `SCK` and `MOSI` have to reach GPIOs or it is named and
/// left off — the emulator does not route through the GPIO matrix, so an
/// unwired device would work there and be dead on the desk.
pub fn wire_devices(sheet: &Sheet, rows: &[Row]) -> (Vec<WireDevice>, Vec<Warning>) {
    let mut devices = Vec::new();
    let mut warnings = Vec::new();

    for part in &sheet.parts {
        let Some(select) = part.props.get("cs") else {
            continue;
        };
        let select = select.trim();
        if select.is_empty() {
            continue;
        }
        let miso = part.props.get("miso").map(String::as_str).unwrap_or("");
        let bytes = hex_bytes(miso.trim());
        let Some((select, bytes)) = select
            .parse::<u8>()
            .ok()
            .filter(|n| *n < WIRE_SELECTS)
            .zip(bytes)
        else {
            warnings.push(Warning::WireSelectUnreadable {
                part: part.reference.clone(),
                value: format!("{select} / {miso}"),
            });
            continue;
        };
        if gpio_of(sheet, rows, &part.reference, "SCK").is_none()
            || gpio_of(sheet, rows, &part.reference, "MOSI").is_none()
        {
            warnings.push(Warning::WireNotWired {
                part: part.reference.clone(),
            });
            continue;
        }
        devices.push(WireDevice {
            part: part.reference.clone(),
            select,
            miso: bytes,
        });
    }
    (devices, warnings)
}

/// A part the sheet puts on the I2C bus: an address and what it answers.
///
/// Declared by the part's own properties rather than by its kind, so a
/// sensor, a display and a breakout imported from LCSC all reach the bus
/// the same way. **No address, no device** — the absence refuses rather
/// than guessing one, for the reason the tunables and the sensors do: an
/// address rusty invented is a bus scan finding a part nobody fitted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BusDevice {
    pub part: String,
    pub address: u8,
    /// Where each run of bytes starts, and the bytes. Empty is a device
    /// that acknowledges and answers zeros — a display, which is read from
    /// by nobody.
    pub regs: Vec<(u8, Vec<u8>)>,
}

/// `75=68,3b=010203040506` — where each run starts and what is in it.
///
/// Hex throughout, because a datasheet's register map is written in hex and
/// retyping it in decimal is where the transcription errors come from. An
/// odd number of digits is refused rather than rounded: a truncated hex
/// string is a valid, wrong one.
fn parse_regs(text: &str) -> Option<Vec<(u8, Vec<u8>)>> {
    let mut runs = Vec::new();
    for run in text.split(',').map(str::trim).filter(|r| !r.is_empty()) {
        let (at, bytes) = run.split_once('=')?;
        let at = u8::from_str_radix(at.trim(), 16).ok()?;
        runs.push((at, hex_bytes(bytes.trim())?));
    }
    Some(runs)
}

/// The part a sheet says it is, from its `model` prop, among the parts the
/// library carries: `Ok(None)` when it names none, and the text it named
/// when that is no part anybody has declared.
pub fn sensor_model<'a>(
    specs: &'a [crate::sensor::Spec],
    part: &Instance,
) -> Result<Option<&'a crate::sensor::Spec>, String> {
    match part.props.get("model").map(|text| text.trim()) {
        None | Some("") => Ok(None),
        Some(text) => crate::sensor::Spec::find(specs, text)
            .map(Some)
            .ok_or_else(|| text.to_string()),
    }
}

/// A part's `addr` prop as a number: hex, with or without its `0x`. Whether
/// the number is an address the bus can carry is the caller's to judge.
pub fn hex_address(text: &str) -> Option<u8> {
    u8::from_str_radix(text.trim().trim_start_matches("0x"), 16).ok()
}

/// Every device the sheet puts on the bus, and what it wants said about the
/// ones it could not.
///
/// The wiring is checked, not assumed. The emulator's bus does not route
/// through the GPIO matrix, so a device with no wires at all would answer
/// there and be silent on the desk — the confident wrong answer this
/// workbench exists to avoid. A part with an address whose `SDA` or `SCL`
/// reaches no GPIO is named and left off.
pub fn bus_devices(
    sheet: &Sheet,
    rows: &[Row],
    specs: &[crate::sensor::Spec],
) -> (Vec<BusDevice>, Vec<Warning>) {
    let mut devices = Vec::new();
    let mut warnings = Vec::new();

    for part in &sheet.parts {
        let Some(address) = part.props.get("addr") else {
            continue;
        };
        let address = address.trim();
        if address.is_empty() {
            continue;
        }
        let Some(parsed) = hex_address(address).filter(|a| *a <= 0x7f) else {
            warnings.push(Warning::BusAddressUnreadable {
                part: part.reference.clone(),
                value: address.to_string(),
            });
            continue;
        };
        if gpio_of(sheet, rows, &part.reference, "SDA").is_none()
            || gpio_of(sheet, rows, &part.reference, "SCL").is_none()
        {
            warnings.push(Warning::BusNotWired {
                part: part.reference.clone(),
            });
            continue;
        }
        // A sensor rusty answers for starts with its own registers — who it
        // is, how it is calibrated, what it reads — and anything the sheet
        // spells out in `regs` lands over them, so a hand-written register
        // still means what it says.
        let mut regs = match sensor_model(specs, part) {
            Ok(Some(spec)) => crate::sensor::Device::new(spec.clone(), &part.props).registers(),
            Ok(None) => Vec::new(),
            Err(value) => {
                warnings.push(Warning::SensorModelUnknown {
                    part: part.reference.clone(),
                    value,
                });
                Vec::new()
            }
        };
        match part.props.get("regs").map(String::as_str) {
            None => {}
            Some(text) if text.trim().is_empty() => {}
            Some(text) => match parse_regs(text) {
                Some(explicit) => regs.extend(explicit),
                None => warnings.push(Warning::BusRegistersUnreadable {
                    part: part.reference.clone(),
                    value: text.to_string(),
                }),
            },
        }
        devices.push(BusDevice {
            part: part.reference.clone(),
            address: parsed,
            regs,
        });
    }
    (devices, warnings)
}
