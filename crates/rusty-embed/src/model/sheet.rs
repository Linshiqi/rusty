//! The simulated board as a schematic: placed symbols, and wires between
//! their pins.
//!
//! The second generation of `.rusty/sim.toml` — the first (`SimBoard`) had a
//! part *be* the GPIO it sat on, which answers "does GPIO2 go high" and
//! nothing an electronics person asks. Here a lamp is `Device:LED` with an
//! anode and a cathode, a resistor is a part, and which pin drives what is
//! read off the wires (`crate::nets`). `docs/schematic.md` is the design.
//!
//! Wire model, not file format: `simulate::board_file` owns the TOML.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::Symbol;

/// The devkit's reference: it is a part like any other, and the one every
/// sheet has.
pub const KIT_REFERENCE: &str = "U1";

/// A placed symbol.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Instance {
    /// `D1`, `R1`, `SW1` — unique on the sheet, and what a wire names.
    pub reference: String,
    /// `library:name` — `Device:LED`, `lcsc:C2286`, `rusty:Pot`.
    pub symbol: String,
    /// What is written beside the part: a colour for a lamp, `220` for a
    /// resistor, whatever the author wants to read there.
    #[serde(default)]
    pub value: String,
    /// The symbol's anchor on the sheet, in sheet units (pixels at zoom 1).
    pub x: f64,
    pub y: f64,
    /// Quarter turns clockwise on the screen: 0, 90, 180 or 270.
    #[serde(default, skip_serializing_if = "super::sim::is_upright")]
    pub rot: u16,
    /// Mirrored left-to-right — KiCad's X key.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mirror: bool,
    /// Behaviour-specific settings as text — an analog source's full scale
    /// (`max`) and where its slider starts (`start`). A part added tomorrow
    /// carries its knobs here without a model change.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub props: BTreeMap<String, String>,
}

impl Instance {
    /// A prop as a number, when it is one.
    pub fn prop<T: std::str::FromStr>(&self, key: &str) -> Option<T> {
        self.props.get(key)?.trim().parse().ok()
    }
}

/// One end of a wire: a part's pin, by the part's reference and the pin's
/// number or name (`D1.K`, `R1.2`, `U1.GPIO2`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinRef {
    pub part: String,
    pub pin: String,
}

impl PinRef {
    pub fn new(part: impl Into<String>, pin: impl Into<String>) -> Self {
        PinRef {
            part: part.into(),
            pin: pin.into(),
        }
    }

    /// `D1.K` → `D1`, `K`. The pin may itself carry a dot (`U1.3V3` does
    /// not, but a library could), so the split is at the first one.
    pub fn parse(text: &str) -> Option<Self> {
        let (part, pin) = text.trim().split_once('.')?;
        if part.is_empty() || pin.is_empty() {
            return None;
        }
        Some(PinRef::new(part, pin))
    }
}

impl std::fmt::Display for PinRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.part, self.pin)
    }
}

/// A wire between two pins, with the bends the author placed. Empty bends
/// means "route automatically".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Wire {
    pub from: PinRef,
    pub to: PinRef,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bends: Vec<(f64, f64)>,
}

/// The sheet: the devkit, the parts around it, and the wires.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sheet {
    /// The chip whose devkit is drawn — always the chip the project builds
    /// for, whatever the file says (the plan's notes carry the disagreement).
    pub chip: String,
    /// Where the devkit sits. Absent means the default place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kit_x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kit_y: Option<f64>,
    /// And how it lies, in the same two fields every other part has. The
    /// devkit is a part like any other and its geometry already turns
    /// through `orient`; these are here because a sheet whose parts sit
    /// below the board wants its header pointing down, and a turn that did
    /// not survive a save would be worse than no turn at all.
    #[serde(default, skip_serializing_if = "super::sim::is_upright")]
    pub kit_rot: u16,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub kit_mirror: bool,
    /// Everything but the devkit, which is `U1` and drawn by the chip.
    #[serde(default)]
    pub parts: Vec<Instance>,
    #[serde(default)]
    pub wires: Vec<Wire>,
    /// The symbols the parts use, resolved by the backend from the library
    /// so the frontend can draw without a second lookup. Never in the file.
    #[serde(default)]
    pub symbols: Vec<Symbol>,
    /// What loading wanted read: a migration from the first format, a part
    /// whose symbol no library has. Never in the file.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

impl Sheet {
    /// An empty sheet for `chip`.
    pub fn empty(chip: &str) -> Self {
        Sheet {
            chip: chip.to_string(),
            kit_x: None,
            kit_y: None,
            kit_rot: 0,
            kit_mirror: false,
            parts: Vec::new(),
            wires: Vec::new(),
            symbols: Vec::new(),
            notes: Vec::new(),
        }
    }

    pub fn part(&self, reference: &str) -> Option<&Instance> {
        self.parts.iter().find(|p| p.reference == reference)
    }

    /// The symbol a part is drawn with, from the resolved list.
    pub fn symbol_of(&self, reference: &str) -> Option<&Symbol> {
        let id = &self.part(reference)?.symbol;
        self.symbols.iter().find(|s| s.id() == *id)
    }

    /// The next free `prefix<n>`: `D3` when `D1` and `D2` are placed. A
    /// deleted `D2` is reused, as KiCad's annotation does.
    pub fn next_reference(&self, prefix: &str) -> String {
        (1..)
            .map(|n| format!("{prefix}{n}"))
            .find(|candidate| {
                candidate != KIT_REFERENCE && self.parts.iter().all(|p| p.reference != *candidate)
            })
            .expect("the integers do not run out")
    }

    /// Every wire touching a part, in order.
    pub fn wires_of<'a>(&'a self, reference: &'a str) -> impl Iterator<Item = &'a Wire> + 'a {
        self.wires
            .iter()
            .filter(move |w| w.from.part == reference || w.to.part == reference)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pin_reference_splits_at_the_first_dot_and_refuses_halves() {
        assert_eq!(PinRef::parse("D1.K"), Some(PinRef::new("D1", "K")));
        assert_eq!(
            PinRef::parse(" U1.GPIO2 "),
            Some(PinRef::new("U1", "GPIO2"))
        );
        assert_eq!(PinRef::parse("R1.").map(|p| p.pin), None);
        assert_eq!(PinRef::parse(".2"), None);
        assert_eq!(PinRef::parse("R1"), None);
        assert_eq!(PinRef::new("D1", "K").to_string(), "D1.K");
    }

    #[test]
    fn the_next_reference_fills_the_first_gap_and_never_takes_the_kits() {
        let mut sheet = Sheet::empty("esp32c3");
        assert_eq!(sheet.next_reference("D"), "D1");
        for reference in ["D1", "D3"] {
            sheet.parts.push(Instance {
                reference: reference.to_string(),
                symbol: "Device:LED".to_string(),
                value: String::new(),
                x: 0.0,
                y: 0.0,
                rot: 0,
                mirror: false,
                props: BTreeMap::new(),
            });
        }
        assert_eq!(sheet.next_reference("D"), "D2");
        assert_eq!(sheet.next_reference("U"), "U2", "U1 is the devkit");
    }
}
