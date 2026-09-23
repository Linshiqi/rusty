//! What the wires mean: the nets of a sheet, and the DC rules that turn a
//! GPIO level into a lit lamp or a pressed button into a pin level.
//!
//! Pure and compiled unconditionally, because both sides read it: the
//! frontend lights the LEDs from the levels the firmware reports, and the
//! backend learns which GPIO a button reaches and what level a press puts
//! there. Two computations of "what does this wire connect" would be two
//! opinions, and a lamp that lit on screen for a pin the emulator was never
//! driven to is the bug that split would produce.
//!
//! The rules are the ones `docs/schematic.md` names. A net's level is the
//! level of the rail or GPIO driving it; a resistor conducts and a
//! capacitor does not; a switch conducts while pressed; an LED lights when
//! its anode is high and its cathode low, and nothing else lights it. A
//! finding that is not a refusal — a lamp on a GPIO with no resistor in
//! the path, a rail shorted to another — is a [`Warning`], said in words
//! rather than drawn wrong.

mod analog;
mod behaviour;
mod bus;
mod evaluate;
mod graph;
mod kit;
mod switch;
mod value;
mod warning;

pub use analog::{Divider, PotSpan, divider_at, gpio_of, pot_span};
pub use behaviour::{Behaviour, Rail, behaviour_of, power_rail};
pub use bus::{BusDevice, WireDevice, bus_devices, hex_address, sensor_model, wire_devices};
pub use evaluate::{Evaluation, Inputs, evaluate};
pub use graph::solid_nets;
pub use kit::{Row, bind_to_kit, gpio_named, kit_pin, kit_rows};
pub use switch::{button_drives, keypad_tie, switch_tie};
pub use value::{farads, ohms, volts};
pub use warning::Warning;

#[cfg(test)]
mod tests;
