//! What the rules want said about a sheet.

use serde::{Deserialize, Serialize};

/// Something the rules want said about the sheet. Not a refusal: the board
/// runs, and this is what somebody at the desk would point at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Warning {
    /// A lamp with a GPIO wired straight to it and no resistor between.
    /// It lights here; on the desk it would not for long.
    LedWithoutResistor { part: String },
    /// A wire names a part or a pin the sheet does not have.
    DanglingWire { from: String, to: String },
    /// Ground and a supply on one net.
    Short { pins: Vec<String> },
    /// Two GPIOs the firmware drives to different levels, wired together.
    Conflict { pins: Vec<String> },
    /// A switch with fewer than two wired sides, or with no GPIO on one side
    /// and no rail on the other: pressing it changes nothing.
    SwitchDrivesNothing { part: String },
    /// A part carrying an `addr` that is not a 7-bit hex address.
    BusAddressUnreadable { part: String, value: String },
    /// A part carrying a `regs` that is not `<reg>=<hex>` pairs.
    BusRegistersUnreadable { part: String, value: String },
    /// A part with an address whose `SDA` or `SCL` reaches no GPIO. The
    /// emulator would answer it anyway; the desk would not.
    BusNotWired { part: String },
    /// A part carrying a `model` that is not a sensor rusty can answer for.
    /// It still goes on the bus with whatever `regs` says; it just reads
    /// nothing a slider could move.
    SensorModelUnknown { part: String, value: String },
    /// A part carrying a `cs` that is not a chip select this part has, or a
    /// `miso` that is not hex bytes.
    WireSelectUnreadable { part: String, value: String },
    /// A part on a chip select whose `SCK` or `MOSI` reaches no GPIO.
    WireNotWired { part: String },
    /// A pin with no wire on it, on a part that is otherwise wired, and
    /// which the author has not marked as deliberately unconnected.
    ///
    /// The two conditions are what keep it usable. **Otherwise wired**,
    /// because a part just dropped on the sheet has every pin loose and
    /// nobody wants six findings for it; a part with three of four pins
    /// joined is the one where the fourth is a mistake. **Not marked**,
    /// because that is what the no-connect flag is for — a finding with no
    /// way to be answered is one people learn to scroll past.
    PinReachesNothing { part: String, pin: String },
    /// Two pins that both drive, on one net. Not the same as `Conflict`,
    /// which is two *GPIOs* the firmware has driven apart at run time: this
    /// is true of the drawing whether or not anything is running, and it is
    /// true before the board is built.
    OutputsFighting { pins: Vec<String> },
}

impl Warning {
    /// The part this finding is about, when it is about one.
    ///
    /// Used to keep the generic finding out of the way of a specific one:
    /// a switch with one side loose already has `SwitchDrivesNothing` said
    /// about it, and adding "and a pin reaches nothing" is the same problem
    /// twice. Two findings for one fault is how a list stops being read.
    pub fn about_part(&self) -> Option<&str> {
        match self {
            Warning::LedWithoutResistor { part }
            | Warning::SwitchDrivesNothing { part }
            | Warning::BusAddressUnreadable { part, .. }
            | Warning::BusRegistersUnreadable { part, .. }
            | Warning::BusNotWired { part }
            | Warning::SensorModelUnknown { part, .. }
            | Warning::WireSelectUnreadable { part, .. }
            | Warning::WireNotWired { part }
            | Warning::PinReachesNothing { part, .. } => Some(part),
            Warning::DanglingWire { .. }
            | Warning::Short { .. }
            | Warning::Conflict { .. }
            | Warning::OutputsFighting { .. } => None,
        }
    }
}

impl std::fmt::Display for Warning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Warning::LedWithoutResistor { part } => write!(
                f,
                "{part} has a GPIO wired straight to it: it lights here, and on the desk it would not for long without a series resistor"
            ),
            Warning::DanglingWire { from, to } => {
                write!(
                    f,
                    "a wire from {from} to {to} names a pin the sheet does not have"
                )
            }
            Warning::Short { pins } => {
                write!(f, "ground and a supply share a net: {}", pins.join(", "))
            }
            Warning::Conflict { pins } => write!(
                f,
                "two GPIOs driven to different levels are wired together: {}",
                pins.join(", ")
            ),
            Warning::SwitchDrivesNothing { part } => write!(
                f,
                "{part} reaches no GPIO on one side and no rail on the other, so pressing it changes nothing"
            ),
            Warning::BusAddressUnreadable { part, value } => write!(
                f,
                "{part}'s address {value:?} is not a 7-bit I2C address in hex, so it is not put on the bus"
            ),
            Warning::BusRegistersUnreadable { part, value } => write!(
                f,
                "{part}'s registers {value:?} are not <reg>=<hex bytes> pairs, so it answers with zeros"
            ),
            Warning::BusNotWired { part } => write!(
                f,
                "{part} has an address but its SDA or SCL reaches no GPIO: the emulator would answer it and the board on your desk would not"
            ),
            Warning::SensorModelUnknown { part, value } => write!(
                f,
                "{part}'s model {value:?} is not a sensor rusty answers for (mpu6050, bmp280, bme280), so it reads only what its registers say"
            ),
            Warning::WireSelectUnreadable { part, value } => write!(
                f,
                "{part}'s chip select {value:?} is not a line this part has, or its answer is not hex bytes"
            ),
            Warning::WireNotWired { part } => write!(
                f,
                "{part} is on a chip select but its SCK or MOSI reaches no GPIO: the emulator would talk to it and the board on your desk would not"
            ),
            Warning::PinReachesNothing { part, pin } => write!(
                f,
                "{part}.{pin} has no wire on it while the rest of {part} is wired — join it, or mark it as not connected"
            ),
            Warning::OutputsFighting { pins } => write!(
                f,
                "two pins that both drive are wired together: {}",
                pins.join(", ")
            ),
        }
    }
}
