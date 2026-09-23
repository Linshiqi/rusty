//! Where a pin sits between the rails: dividers, potentiometers, and the
//! GPIO a pin reaches.

use std::collections::HashSet;

use super::graph::{Graph, Node};
use super::{Behaviour, Rail, Row, behaviour_of, ohms, power_rail};
use crate::model::{Instance, KIT_REFERENCE, PinRef, Sheet};
use crate::union_find::UnionFind;

/// Where a pin sits between the rails, as a fraction: 0.0 at ground, 1.0 at
/// the supply.
///
/// This is the one thing on the sheet a resistor's *value* decides, and the
/// reason it is worth reading at all: an ADC pin behind a divider reads a
/// number nothing else here can produce. `A<pin>=<counts>` has always
/// carried raw counts because rusty did not know anybody's divider — but
/// when the divider is *drawn*, with values on it, rusty does know, exactly,
/// and refusing then is refusing to read what the user wrote down.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Divider {
    /// 0.0 at ground, 1.0 at the supply.
    pub fraction: f64,
    /// The resistance to each rail in ohms — zero for a solid connection,
    /// infinite for no path at all.
    pub to_supply: f64,
    pub to_ground: f64,
}

/// Read [`Divider`] at one pin, or refuse.
///
/// **One resistor deep, and that is deliberate.** The shapes this answers
/// for are the ones people draw: a pin straight on a rail, a pull-up or
/// pull-down, and two resistors with the midpoint tapped. Parallel paths to
/// the same rail add as conductances, because that is exact. A chain of
/// three resistors, a network, anything with a value the sheet spells in a
/// way [`ohms`] cannot read — those get `None`, and `None` is the honest
/// answer: a solver that guessed at the rest would put a number under an
/// ADC reading that nobody could check.
pub fn divider_at(sheet: &Sheet, rows: &[Row], pin: &PinRef) -> Option<Divider> {
    let (graph, mut solid) = Graph::solid_of(sheet, rows, &HashSet::new());
    let here = solid.find(graph.pin_node(&pin.part, &pin.pin)?);

    // What a solid node sits on directly, with no resistance in the way.
    let rail_of = |solid: &mut UnionFind, root: Node| -> Option<Rail> {
        let mut found = None;
        for node in graph.members(solid, root) {
            let at = &graph.nodes[node];
            let rail = if at.part == KIT_REFERENCE {
                graph.row_of(node).and_then(|row| row.rail)
            } else {
                sheet.symbol_of(&at.part).and_then(power_rail)
            };
            if rail.is_some() {
                found = rail;
            }
        }
        found
    };

    if let Some(rail) = rail_of(&mut solid, here) {
        // On a rail itself: no divider, and the answer is the rail.
        return Some(match rail {
            Rail::Supply => Divider {
                fraction: 1.0,
                to_supply: 0.0,
                to_ground: f64::INFINITY,
            },
            Rail::Ground => Divider {
                fraction: 0.0,
                to_supply: f64::INFINITY,
                to_ground: 0.0,
            },
        });
    }

    // Every resistor with one leg here, and what its other leg sits on.
    let mut to_supply = 0.0f64; // conductance, summed
    let mut to_ground = 0.0f64;
    let mut unreadable = false;
    for part in &sheet.parts {
        let reference = part.reference.as_str();
        if graph.behaviours.get(reference) != Some(&Behaviour::Resistor) {
            continue;
        }
        let Some((a, b)) = graph.terminals(reference) else {
            continue;
        };
        let (a, b) = (solid.find(a), solid.find(b));
        let far = if a == here && b != here {
            b
        } else if b == here && a != here {
            a
        } else {
            continue;
        };
        let Some(rail) = rail_of(&mut solid, far) else {
            continue;
        };
        match ohms(&part.value) {
            // A zero-ohm link is a wire somebody drew as a resistor.
            Some(r) if r > 0.0 => match rail {
                Rail::Supply => to_supply += 1.0 / r,
                Rail::Ground => to_ground += 1.0 / r,
            },
            _ => unreadable = true,
        }
    }

    // A resistor that reaches a rail but says no value is a path that
    // exists and cannot be measured — which is not the same as no path, and
    // treating it as none is how the first version put an unvalued divider's
    // midpoint flat on ground. The sheet has not finished saying, so nor
    // does this.
    if unreadable {
        return None;
    }

    match (to_supply > 0.0, to_ground > 0.0) {
        // Both sides: the divider.
        (true, true) => Some(Divider {
            fraction: to_supply / (to_supply + to_ground),
            to_supply: 1.0 / to_supply,
            to_ground: 1.0 / to_ground,
        }),
        // One side only: no current flows, so the pin sits at that rail
        // whatever the resistor is — a pull-up's value does not change
        // where an unloaded pin rests.
        (true, false) => Some(Divider {
            fraction: 1.0,
            to_supply: 1.0 / to_supply,
            to_ground: f64::INFINITY,
        }),
        (false, true) => Some(Divider {
            fraction: 0.0,
            to_supply: f64::INFINITY,
            to_ground: 1.0 / to_ground,
        }),
        (false, false) => None,
    }
}

/// The converter's own resolution when the sheet does not say.
///
/// Twelve bits, which is the ESP32 family's SAR converter. It has a default
/// where the full-scale voltage does not, and the difference is the point:
/// the resolution is a fact about the chip rusty already knows, and the
/// voltage is a fact about how the firmware configured it, which only the
/// firmware knows.
pub const ADC_MAX: u16 = 4095;

/// Where a potentiometer's knob rests when the sheet does not say: the
/// middle of rusty's eight-bit turn.
pub const POT_REST: u8 = 128;

// What an analog part's props say, at their defaults. The backend reads them
// to send a run's first counts and the panel to place its sliders, and the
// two have to agree before anybody drags anything.

/// The counts at full scale, from `max`.
pub fn adc_max(part: &Instance) -> u16 {
    part.prop("max").unwrap_or(ADC_MAX)
}

/// Where a knob starts, from `start`.
pub fn pot_start(part: &Instance) -> u8 {
    part.prop("start").unwrap_or(POT_REST)
}

/// Where an analog source's counts start, from `start`.
pub fn analog_start(part: &Instance) -> u16 {
    part.prop("start").unwrap_or(0)
}

/// A potentiometer as the converter sees it: the GPIO its wiper reaches,
/// and where each end of its track sits between the rails.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PotSpan {
    pub gpio: u8,
    /// The fraction at the wiper with the knob hard one way and hard the
    /// other: pin `1`'s end at zero, pin `3`'s at full.
    pub at_zero: f64,
    pub at_full: f64,
}

/// Read a pot's span, or refuse.
///
/// This closes the hole the potentiometer has had since it was drawn.
/// `P<pin>=<0..255>` reaches firmware that reads rusty's own text protocol
/// and nothing else, because what a wiper converts to depends on what its
/// two ends are wired to, and turning 128 into counts would have asserted a
/// rail-to-rail divider nobody stated. When the sheet *does* state it —
/// both ends on rails — there is nothing left to assume and the wiper's
/// position is ADC counts through `adc.read_oneshot()` like any other pin.
///
/// An end behind a resistor is refused rather than read: it forms a divider
/// with the pot's own track, and the track's resistance is not on the
/// sheet. The knob's zero is pin `1`'s end; two wires swapped reverses it,
/// which is the same fix as on the bench.
pub fn pot_span(sheet: &Sheet, rows: &[Row], part: &str) -> Option<PotSpan> {
    if behaviour_of(sheet.symbol_of(part)?) != Behaviour::Pot {
        return None;
    }
    let end = |pin: &str| -> Option<f64> {
        let at = divider_at(sheet, rows, &PinRef::new(part, pin))?;
        (at.to_supply == 0.0 || at.to_ground == 0.0).then_some(at.fraction)
    };
    Some(PotSpan {
        gpio: gpio_of(sheet, rows, part, "W")?,
        at_zero: end("1")?,
        at_full: end("3")?,
    })
}

impl PotSpan {
    /// The counts a wiper at `turn` (0..=255) puts on the pin, for a
    /// converter whose full scale is `max`.
    pub fn counts(&self, turn: u8, max: u16) -> u16 {
        let t = f64::from(turn) / 255.0;
        let fraction = self.at_zero + t * (self.at_full - self.at_zero);
        (fraction.clamp(0.0, 1.0) * f64::from(max)).round() as u16
    }
}

/// The GPIO a part's pin reaches through the wires and the resistors — what
/// a pot's wiper or a motor's duty pin is *on*, in the firmware's terms.
pub fn gpio_of(sheet: &Sheet, rows: &[Row], part: &str, pin: &str) -> Option<u8> {
    let (graph, mut dc) = Graph::conducting_of(sheet, rows);
    let root = dc.find(graph.pin_node(part, pin)?);
    graph.gpio_in(&mut dc, root)
}
