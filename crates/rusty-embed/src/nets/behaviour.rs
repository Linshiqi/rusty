//! What a placed symbol does on the sheet, and the rail a power symbol puts
//! on its net.

use serde::{Deserialize, Serialize};

use crate::model::Symbol;

/// What a placed symbol *does* on the sheet. Read off the symbol's library
/// and name, then its reference prefix and pin names — so a part imported
/// from LCSC with the prefix `LED` and pins `A`/`K` lights like the
/// built-in one, and an `R` from anywhere conducts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Behaviour {
    Led,
    Resistor,
    Capacitor,
    Switch,
    /// Three lamps with a common pin (`rusty:RGB_LED`).
    Rgb,
    /// Sixteen keys on four rows and four columns (`rusty:Keypad`). A key
    /// *joins* a row to a column rather than driving either, which is what
    /// makes a matrix a matrix and what `switch_tie` reads for a switch.
    Keypad,
    /// Addressable LEDs on one wire (`rusty:Strip`): a WS2812 chain, whose
    /// colours arrive as the bytes RMT clocked out rather than as levels on
    /// pins. Its pins carry no level of their own — the data wire is a
    /// stream and the emulator reads it as one.
    Strip,
    /// Seven lamps with a common pin (`rusty:7SEG`).
    Seven,
    /// Shows the `[rusty:disp]` channel; its pins carry no level.
    Display,
    /// A knob whose wiper the panel sends as `P<gpio>=<0..255>`.
    Pot,
    /// A voltage the panel sends as ADC counts, `A<gpio>=<count>`.
    Analog,
    /// Duty on `PWM`, direction on `IN1`/`IN2`.
    Motor,
    /// A rail on the sheet: `rusty:GND` or `rusty:Supply`, which puts its
    /// net at a level without a wire running all the way to the devkit.
    Power,
    /// A name for a net (`rusty:Label`): every label carrying the same
    /// value is one net, however far apart they are drawn. What a
    /// schematic uses instead of a wire across the whole sheet.
    Label,
    /// A sounder on one pin: it is on while its net is driven the way its
    /// polarity says, and the panel says so rather than making a noise.
    Buzzer,
    /// A hobby servo: the angle follows the duty on its signal pin.
    Servo,
    /// A sensor module. Its value names the channel the firmware declared
    /// with `[rusty:sensor]`, and the panel feeds that channel — never one
    /// the firmware never asked for, which is the tunables' rule pointed
    /// at the other half of the loop.
    Sensor,
    /// Drawn and wired, and nothing more is known about it.
    Other,
}

pub fn behaviour_of(symbol: &Symbol) -> Behaviour {
    match (symbol.library.as_str(), symbol.name.as_str()) {
        ("rusty", "Pot") => return Behaviour::Pot,
        ("rusty", "Analog") => return Behaviour::Analog,
        ("rusty", "Display") => return Behaviour::Display,
        ("rusty", "RGB_LED") => return Behaviour::Rgb,
        ("rusty", "Strip") => return Behaviour::Strip,
        ("rusty", "Keypad") => return Behaviour::Keypad,
        ("rusty", "7SEG") => return Behaviour::Seven,
        ("rusty", "Motor") => return Behaviour::Motor,
        ("rusty", "GND" | "Supply") => return Behaviour::Power,
        ("rusty", "Label") => return Behaviour::Label,
        ("rusty", "Buzzer") => return Behaviour::Buzzer,
        ("rusty", "Servo") => return Behaviour::Servo,
        ("rusty", "Sensor") => return Behaviour::Sensor,
        _ => {}
    }
    let prefix = symbol.reference.trim_end_matches(['?', '_']);
    let has = |name: &str| symbol.pins.iter().any(|p| p.name == name);
    match prefix {
        "R" => Behaviour::Resistor,
        "C" => Behaviour::Capacitor,
        "SW" | "S" => Behaviour::Switch,
        "LED" => Behaviour::Led,
        // KiCad's diodes share the prefix; the LED is the one with an
        // anode and a cathode named as such, or the one called an LED.
        "D" if has("A") && has("K") || symbol.name.to_uppercase().contains("LED") => Behaviour::Led,
        _ => Behaviour::Other,
    }
}

/// The rail a power symbol puts on its net, or `None` for a symbol that
/// is not one. Ground is ground; everything else is a supply, whatever
/// voltage its value names — the rules are DC on and off, and a symbol
/// claiming to know 3.3 V from 5 V would be claiming more than they read.
pub fn power_rail(symbol: &Symbol) -> Option<Rail> {
    if behaviour_of(symbol) != Behaviour::Power {
        return None;
    }
    Some(if symbol.name == "GND" {
        Rail::Ground
    } else {
        Rail::Supply
    })
}

/// What a rail is at: the two levels a supply pin can have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rail {
    Ground,
    Supply,
}
