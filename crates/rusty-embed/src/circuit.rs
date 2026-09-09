//! The sheet as a circuit the solver can answer about.
//!
//! Step 5 of `docs/kicad.md`'s stage 4, and done before the transient one
//! deliberately: until something builds this, `solve` is a library nothing
//! calls, and the modelling gaps are easier to find here than under another
//! layer. `nets` already answers which pins are one node; modified nodal
//! analysis wants exactly that, plus what sits between the nodes.
//!
//! **The partition is the solid one**, not the conducting one. A resistor's
//! two ends are two nodes with an element between them — merging them, which
//! is right for "what reaches what", would erase the only thing that makes a
//! voltage computable.
//!
//! **What it refuses is the interesting half.** A sheet says what is joined
//! to what; it does not always say enough to put a number on it. A resistor
//! with no readable value, a rail nobody gave a voltage, a lamp with no
//! forward drop: each is named, with the property that would answer it, and
//! nothing is invented. That is the same rule the whole simulator runs on —
//! `A<pin>=<counts>` carries counts and not volts because rusty does not
//! know anybody's divider — pointed at the one place where the sheet *can*
//! say enough.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use crate::model::{PinRef, Sheet};
use crate::nets::{self, Behaviour, Rail, Row, behaviour_of, farads, ohms, power_rail, volts};
use crate::solve::{Circuit, Element, Solution, Trouble};

/// The circuit, and how to read an answer back onto the sheet.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Bridged {
    pub circuit: Circuit,
    /// Which node each pin sits on. Ground is node 0.
    pub node_of: BTreeMap<PinRef, usize>,
    /// Which element each part became, so its current can be asked for by
    /// reference rather than by position.
    pub element_of: BTreeMap<String, usize>,
}

/// What the sheet did not say, and the property that would say it.
///
/// Every one of these is a refusal rather than a default, and each names
/// the part so the message can point at it on the canvas.
#[derive(Debug, Clone, PartialEq)]
pub enum Unstated {
    /// A resistor whose value is not a resistance `ohms` can read.
    Resistance { part: String, value: String },
    /// A rail with no voltage in its name: `VCC` says which net, not what
    /// it is at.
    Supply { part: String, value: String },
    /// A lamp with no `vf` property. The forward drop is a fact about the
    /// part rather than about the drawing, and a red one is not a blue one
    /// — guessing it would put the current out by a factor of two.
    ForwardVoltage { part: String },
    /// A capacitor whose value is not a capacitance.
    Capacitance { part: String, value: String },
    /// Nothing on the sheet is ground, so no voltage has anything to be
    /// relative to.
    NoGround,
}

impl std::fmt::Display for Unstated {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Unstated::Resistance { part, value } => write!(
                f,
                "{part}'s value {value:?} is not a resistance, so no current through it can be worked out — write it as 220, 4k7 or 10k"
            ),
            Unstated::Supply { part, value } => write!(
                f,
                "{part} is a rail called {value:?}, which names the net without saying what it is at — write it as 3V3 or 5V"
            ),
            Unstated::ForwardVoltage { part } => write!(
                f,
                "{part} has no forward voltage, and a lamp's is a fact about the part rather than the drawing — give it a `vf` of 2.0 for a red one, 3.2 for a blue"
            ),
            Unstated::Capacitance { part, value } => write!(
                f,
                "{part}'s value {value:?} is not a capacitance, so nothing can be said about how long it takes to charge — write it as 100n, 4u7 or 10uF"
            ),
            Unstated::NoGround => f.write_str(
                "nothing on this sheet is ground, so there is nothing for a voltage to be relative to",
            ),
        }
    }
}

impl std::error::Error for Unstated {}

/// Why a sheet has no numbers on it.
///
/// Two halves, and they are different kinds of answer. `Unstated` is about
/// the *drawing* and names a property that would fix it; `Trouble` is about
/// the *circuit* and is a finding — a node that reaches ground through
/// nothing is a mistake somebody made, not a field they forgot.
#[derive(Debug, Clone, PartialEq)]
pub enum Unsolved {
    Unstated(Unstated),
    /// A node that reaches ground through nothing, named by the pins
    /// sitting on it.
    ///
    /// [`Trouble::Floating`] carries the bridge's own node number, which is
    /// an index into an array this module built and renumbered — nothing
    /// anybody can look at on a sheet. The pins are what they drew, so the
    /// one refusal a person is actually likely to hit (a part with an end
    /// left loose) says which end.
    Floating {
        pins: Vec<PinRef>,
    },
    Trouble(Trouble),
}

impl std::fmt::Display for Unsolved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Unsolved::Unstated(why) => why.fmt(f),
            Unsolved::Floating { pins } => {
                let named: Vec<String> = pins.iter().map(PinRef::to_string).collect();
                write!(
                    f,
                    "{} reaches ground through nothing, so there is no voltage to report there",
                    named.join(", ")
                )
            }
            Unsolved::Trouble(why) => why.fmt(f),
        }
    }
}

impl std::error::Error for Unsolved {}

impl Unsolved {
    /// Which part the refusal is about, when it is about one.
    ///
    /// A panel puts the reason beside that part and nowhere else: a sheet
    /// has one reason at a time and thirty parts, and showing it on all of
    /// them would say "something is wrong here" twenty-nine times over. The
    /// ones that answer `None` are about the drawing as a whole — no
    /// ground, or a circuit that cannot be solved — and belong wherever
    /// somebody asked for a number rather than beside a part.
    pub fn part(&self) -> Option<&str> {
        match self {
            Unsolved::Unstated(
                Unstated::Resistance { part, .. }
                | Unstated::Supply { part, .. }
                | Unstated::ForwardVoltage { part }
                | Unstated::Capacitance { part, .. },
            ) => Some(part),
            Unsolved::Unstated(Unstated::NoGround)
            | Unsolved::Floating { .. }
            | Unsolved::Trouble(_) => None,
        }
    }
}

/// So `of`'s refusal can travel through a `?`. There is deliberately no
/// matching one for [`Trouble`]: `operating_point` has to *look* at that
/// one, because `Floating` becomes the pins it is about rather than being
/// carried straight through, and a `?` there would silently skip that.
impl From<Unstated> for Unsolved {
    fn from(why: Unstated) -> Self {
        Unsolved::Unstated(why)
    }
}

/// A sheet with a number on every node of it.
///
/// The operating point: where the circuit sits once everything has settled,
/// which is what a probe on a schematic is asking. It is deliberately not a
/// [`crate::live::Live`] — that one is walking a firmware's clock and
/// carries charge from one instant to the next, and a sheet nobody is
/// running has no instant to be at.
#[derive(Debug, Clone, PartialEq)]
pub struct Solved {
    pub bridged: Bridged,
    pub answer: Solution,
}

/// Solve the sheet where it settles.
pub fn operating_point(
    sheet: &Sheet,
    rows: &[Row],
    pressed: &HashSet<String>,
    levels: &BTreeMap<u8, bool>,
) -> Result<Solved, Unsolved> {
    let bridged = of(sheet, rows, pressed, levels)?;
    match crate::solve::dc(&bridged.circuit) {
        Ok(answer) => Ok(Solved { bridged, answer }),
        // The one refusal worth restating: the solver knows the node and
        // the bridge knows the pins, and only here are both in hand.
        Err(Trouble::Floating { node }) => Err(Unsolved::Floating {
            pins: bridged
                .node_of
                .iter()
                .filter(|(_, on)| **on == node)
                .map(|(pin, _)| pin.clone())
                .collect(),
        }),
        Err(why) => Err(Unsolved::Trouble(why)),
    }
}

impl Solved {
    /// What a pin is at, or nothing when the sheet gives it no node — a pin
    /// nothing is wired to has no voltage rather than zero.
    pub fn volts_at(&self, pin: &PinRef) -> Option<f64> {
        Some(self.answer.volts_at(*self.bridged.node_of.get(pin)?))
    }

    /// What is across a part and what is going through it, or nothing when
    /// the part is not an element of the circuit.
    ///
    /// Most of the sheet is not: a display, a sensor and a label have no
    /// electrical model here, and neither does a rail, whose voltage is the
    /// answer rather than something to be read off it.
    pub fn reading(&self, reference: &str) -> Option<Reading> {
        let index = *self.bridged.element_of.get(reference)?;
        Some(Reading {
            across: self.answer.across(&self.bridged.circuit, index)?,
            through: self.answer.amps_through(&self.bridged.circuit, index)?,
        })
    }
}

/// What a part is doing, in the passive convention: both measured from the
/// element's first terminal to its second, so their product is the power it
/// is dissipating and a negative one is a part that is supplying.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reading {
    pub across: f64,
    pub through: f64,
}

impl Reading {
    /// What it is dissipating, in watts.
    pub fn watts(&self) -> f64 {
        self.across * self.through
    }
}

/// The current a lamp's `vf` is quoted at.
///
/// A datasheet gives one forward drop at one current, and this is the one
/// assumed when no other is stated — ten milliamps, where indicator LEDs
/// are specified. It only shapes the curve *away* from that point; at the
/// few milliamps a sheet actually runs, the answer is within a few percent
/// of the arithmetic a person does by hand, which is the number they will
/// check it against.
const LAMP_AT: f64 = 0.010;

/// A lamp's ideality. Two is the usual figure for the wide-bandgap
/// junctions indicator LEDs are made of, and it is stated here rather than
/// buried so that a part which needs another can be given one later.
const LAMP_IDEALITY: f64 = 2.0;

/// Build the circuit.
///
/// `levels` is what the firmware has reported per GPIO, so a pin it is
/// driving becomes a source; one it has said nothing about is left as an
/// ordinary node, which is the honest reading of an input.
pub fn of(
    sheet: &Sheet,
    rows: &[Row],
    pressed: &HashSet<String>,
    levels: &BTreeMap<u8, bool>,
) -> Result<Bridged, Unstated> {
    let solid = nets::solid_nets(sheet, rows, pressed);

    // ── which solid net is ground ───────────────────────────────────────
    let mut ground: Option<usize> = None;
    let mut rail_at: BTreeMap<usize, f64> = BTreeMap::new();
    for part in &sheet.parts {
        let Some(symbol) = sheet.symbol_of(&part.reference) else {
            continue;
        };
        let Some(rail) = power_rail(symbol) else {
            continue;
        };
        let Some(first) = symbol.pins.first() else {
            continue;
        };
        let Some(net) = solid.get(&PinRef::new(&part.reference, &first.number)) else {
            continue;
        };
        match rail {
            Rail::Ground => ground = Some(*net),
            Rail::Supply => {
                let value = if part.value.trim().is_empty() {
                    symbol.name.clone()
                } else {
                    part.value.clone()
                };
                let Some(v) = volts(&value) else {
                    return Err(Unstated::Supply {
                        part: part.reference.clone(),
                        value,
                    });
                };
                rail_at.insert(*net, v);
            }
        }
    }
    // The devkit's own header is a rail too, and its rows carry the
    // voltage in their names — `3V3` is not a label somebody invented, it
    // is what is silkscreened on the board.
    let mut supply_volts: Option<f64> = None;
    for (row, spec) in rows.iter().enumerate() {
        let Some(rail) = spec.rail else { continue };
        let Some(net) = solid
            .get(&PinRef::new(crate::model::KIT_REFERENCE, &spec.name))
            .or_else(|| {
                solid.get(&PinRef::new(
                    crate::model::KIT_REFERENCE,
                    (row + 1).to_string(),
                ))
            })
        else {
            continue;
        };
        match rail {
            Rail::Ground => ground = ground.or(Some(*net)),
            Rail::Supply => {
                if let Some(v) = volts(&spec.name) {
                    rail_at.insert(*net, v);
                    supply_volts = Some(supply_volts.map_or(v, |had: f64| had.max(v)));
                }
            }
        }
    }
    for v in rail_at.values() {
        supply_volts = Some(supply_volts.map_or(*v, |had: f64| had.max(*v)));
    }
    let Some(ground) = ground else {
        return Err(Unstated::NoGround);
    };

    // ── solid nets become nodes, ground first ───────────────────────────
    let mut number: BTreeMap<usize, usize> = BTreeMap::new();
    number.insert(ground, 0);
    for net in solid.values() {
        let next = number.len();
        number.entry(*net).or_insert(next);
    }
    let node_of: BTreeMap<PinRef, usize> = solid
        .iter()
        .map(|(pin, net)| (pin.clone(), number[net]))
        .collect();
    let node = |pin: &PinRef| node_of.get(pin).copied();

    let mut elements: Vec<Element> = Vec::new();
    let mut element_of: BTreeMap<String, usize> = BTreeMap::new();

    for (net, at) in &rail_at {
        elements.push(Element::Source {
            plus: number[net],
            minus: 0,
            volts: *at,
        });
    }

    // ── and the parts between them ──────────────────────────────────────
    for part in &sheet.parts {
        let Some(symbol) = sheet.symbol_of(&part.reference) else {
            continue;
        };
        let ends = |a: &str, b: &str| -> Option<(usize, usize)> {
            Some((
                node(&PinRef::new(&part.reference, a))?,
                node(&PinRef::new(&part.reference, b))?,
            ))
        };
        let two = || -> Option<(usize, usize)> {
            let mut pins = symbol.pins.iter().filter(|p| !p.hidden);
            let (a, b) = (pins.next()?, pins.next()?);
            ends(&a.number, &b.number)
        };
        let placed = match behaviour_of(symbol) {
            Behaviour::Resistor => {
                let Some((a, b)) = two() else { continue };
                if a == b {
                    continue;
                }
                let Some(ohms) = ohms(&part.value) else {
                    return Err(Unstated::Resistance {
                        part: part.reference.clone(),
                        value: part.value.clone(),
                    });
                };
                Element::Resistor { a, b, ohms }
            }
            Behaviour::Led => {
                // Anode and cathode by name, because which way round it is
                // is the whole of what a lamp contributes.
                let Some((anode, cathode)) = ends("A", "K").or_else(two) else {
                    continue;
                };
                if anode == cathode {
                    continue;
                }
                let Some(forward) = part.prop::<f64>("vf").filter(|v| *v > 0.0) else {
                    return Err(Unstated::ForwardVoltage {
                        part: part.reference.clone(),
                    });
                };
                Element::Diode {
                    anode,
                    cathode,
                    saturation: lamp_saturation(forward),
                    ideality: LAMP_IDEALITY,
                }
            }
            Behaviour::Capacitor => {
                // In the circuit even though it does nothing at DC: the
                // transient is what it is for, and a capacitor left out is
                // a node that has no voltage while it is charging.
                let Some((a, b)) = two() else { continue };
                if a == b {
                    continue;
                }
                let Some(farads) = farads(&part.value).filter(|f| *f > 0.0) else {
                    return Err(Unstated::Capacitance {
                        part: part.reference.clone(),
                        value: part.value.clone(),
                    });
                };
                Element::Capacitor { a, b, farads }
            }
            Behaviour::Switch if pressed.contains(part.reference.as_str()) => {
                // A closed switch is already one solid node, so there is
                // nothing left to add; `solid_nets` joined it.
                continue;
            }
            // A capacitor is an open circuit at DC, and that is not a gap
            // in the model — it is the answer.
            _ => continue,
        };
        element_of.insert(part.reference.clone(), elements.len());
        elements.push(placed);
    }

    // ── what the firmware is driving ────────────────────────────────────
    for (row, spec) in rows.iter().enumerate() {
        let Some(gpio) = spec.gpio else { continue };
        let Some(level) = levels.get(&gpio) else {
            continue;
        };
        let Some(at) = solid
            .get(&PinRef::new(crate::model::KIT_REFERENCE, &spec.name))
            .or_else(|| {
                solid.get(&PinRef::new(
                    crate::model::KIT_REFERENCE,
                    (row + 1).to_string(),
                ))
            })
        else {
            continue;
        };
        let plus = number[at];
        if plus == 0 {
            continue;
        }
        let driven = if *level {
            let Some(rail) = supply_volts else {
                return Err(Unstated::Supply {
                    part: crate::model::KIT_REFERENCE.to_string(),
                    value: spec.name.clone(),
                });
            };
            rail
        } else {
            0.0
        };
        element_of.insert(spec.name.clone(), elements.len());
        elements.push(Element::Source {
            plus,
            minus: 0,
            volts: driven,
        });
    }

    // ── and then only the part of the sheet that is a circuit ──────────
    //
    // Every devkit row is a solid net whether or not anything is wired to
    // it, so a sheet with one lamp on it would otherwise hand the solver
    // twenty untouched nodes and be told the first of them floats. A node
    // nothing is attached to is not *in* the circuit, which is a different
    // thing from a node that is attached and cannot reach ground — the
    // second is a finding and the first is a header pin nobody used.
    let mut used: BTreeSet<usize> = elements.iter().flat_map(touches).collect();
    used.insert(0);
    let renumber: BTreeMap<usize, usize> = std::iter::once((0usize, 0usize))
        .chain(
            used.iter()
                .filter(|node| **node != 0)
                .enumerate()
                .map(|(at, node)| (*node, at + 1)),
        )
        .collect();
    let elements: Vec<Element> = elements
        .into_iter()
        .map(|element| moved(element, &renumber))
        .collect();

    Ok(Bridged {
        circuit: Circuit {
            nodes: renumber.len(),
            elements,
        },
        node_of: node_of
            .into_iter()
            .filter_map(|(pin, node)| Some((pin, renumber.get(&node).copied()?)))
            .collect(),
        element_of,
    })
}

/// The nodes an element is attached to.
fn touches(element: &Element) -> [usize; 2] {
    match *element {
        Element::Resistor { a, b, .. } | Element::Short { a, b } => [a, b],
        Element::Source { plus, minus, .. } => [plus, minus],
        Element::Current { from, into, .. } => [from, into],
        Element::Diode { anode, cathode, .. } => [anode, cathode],
        Element::Capacitor { a, b, .. } | Element::Inductor { a, b, .. } => [a, b],
    }
}

/// The same element with its nodes renumbered.
fn moved(element: Element, to: &BTreeMap<usize, usize>) -> Element {
    let at = |node: usize| to.get(&node).copied().unwrap_or(0);
    match element {
        Element::Resistor { a, b, ohms } => Element::Resistor {
            a: at(a),
            b: at(b),
            ohms,
        },
        Element::Short { a, b } => Element::Short { a: at(a), b: at(b) },
        Element::Source { plus, minus, volts } => Element::Source {
            plus: at(plus),
            minus: at(minus),
            volts,
        },
        Element::Current { from, into, amps } => Element::Current {
            from: at(from),
            into: at(into),
            amps,
        },
        Element::Diode {
            anode,
            cathode,
            saturation,
            ideality,
        } => Element::Diode {
            anode: at(anode),
            cathode: at(cathode),
            saturation,
            ideality,
        },
        Element::Capacitor { a, b, farads } => Element::Capacitor {
            a: at(a),
            b: at(b),
            farads,
        },
        Element::Inductor { a, b, henries } => Element::Inductor {
            a: at(a),
            b: at(b),
            henries,
        },
    }
}

/// The saturation current that makes a lamp drop `forward` at [`LAMP_AT`].
fn lamp_saturation(forward: f64) -> f64 {
    let thermal = LAMP_IDEALITY * 0.025_865;
    (LAMP_AT / (forward / thermal).exp()).max(1e-300)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Fill, Graphic, Instance, Pin, PinKind, Symbol};
    use crate::solve::dc;

    fn pin(number: &str, name: &str, x: f64) -> Pin {
        Pin {
            number: number.into(),
            name: name.into(),
            kind: PinKind::Passive,
            at: (x, 0.0),
            length: 2.54,
            angle: if x < 0.0 { 0 } else { 180 },
            hidden: false,
        }
    }

    fn symbol(library: &str, name: &str, reference: &str, pins: &[(&str, &str)]) -> Symbol {
        Symbol {
            library: library.into(),
            name: name.into(),
            reference: reference.into(),
            value: name.into(),
            description: None,
            pins: pins
                .iter()
                .enumerate()
                .map(|(i, (number, name))| pin(number, name, if i == 0 { -2.54 } else { 2.54 }))
                .collect(),
            graphics: vec![Graphic::Rectangle {
                start: (-1.0, 1.0),
                end: (1.0, -1.0),
                width: 0.0,
                fill: Fill::None,
            }],
        }
    }

    fn sheet() -> Sheet {
        let mut sheet = Sheet::empty("esp32c3");
        sheet.symbols = vec![
            symbol("Device", "R", "R", &[("1", "~"), ("2", "~")]),
            symbol("Device", "C", "C", &[("1", "~"), ("2", "~")]),
            symbol("Device", "LED", "D", &[("1", "K"), ("2", "A")]),
            symbol("rusty", "GND", "#PWR", &[("1", "GND")]),
            symbol("rusty", "Supply", "#PWR", &[("1", "VCC")]),
        ];
        sheet
    }

    fn place(sheet: &mut Sheet, reference: &str, id: &str, value: &str) {
        sheet.parts.push(Instance {
            reference: reference.into(),
            symbol: id.into(),
            value: value.into(),
            x: 0.0,
            y: 0.0,
            rot: 0,
            mirror: false,
            props: Default::default(),
        });
    }

    fn wire(sheet: &mut Sheet, from: &str, to: &str) {
        sheet.wires.push(crate::model::Wire {
            from: PinRef::parse(from).unwrap(),
            to: PinRef::parse(to).unwrap(),
            bends: Vec::new(),
        });
    }

    fn rows() -> Vec<Row> {
        nets::kit_rows("esp32c3", &[0, 1, 2, 3, 4, 5])
    }

    fn build(sheet: &Sheet) -> Result<Bridged, Unstated> {
        of(sheet, &rows(), &HashSet::new(), &BTreeMap::new())
    }

    /// The divider again, this time drawn on a sheet rather than written
    /// as a netlist: the same ratio has to come out, which is what says the
    /// bridge and the solver agree about what a drawing means.
    #[test]
    fn a_divider_drawn_on_the_sheet_solves_to_the_ratio() {
        let mut s = sheet();
        place(&mut s, "R1", "Device:R", "20k");
        place(&mut s, "R2", "Device:R", "10k");
        place(&mut s, "PWR1", "rusty:Supply", "3V3");
        place(&mut s, "GND1", "rusty:GND", "GND");
        wire(&mut s, "PWR1.VCC", "R1.1");
        wire(&mut s, "R1.2", "R2.1");
        wire(&mut s, "R2.2", "GND1.GND");

        let bridged = build(&s).expect("built");
        let found = dc(&bridged.circuit).expect("solved");
        let middle = bridged.node_of[&PinRef::new("R1", "2")];
        assert!(
            (found.volts_at(middle) - 3.3 / 3.0).abs() < 1e-9,
            "10k of 30k across 3V3: {}",
            found.volts_at(middle)
        );
    }

    /// A capacitor is in the circuit — the transient is what it is for —
    /// and carries nothing at DC, which is not a hole in the model but the
    /// answer. Where that leaves a node with no DC voltage at all, it is
    /// reported rather than called zero: the two are different claims, and
    /// only one of them is true.
    #[test]
    fn a_capacitor_is_in_the_circuit_and_open_at_dc() {
        // Across the rail, where a decoupling capacitor is.
        let mut s = sheet();
        place(&mut s, "C1", "Device:C", "100n");
        place(&mut s, "R1", "Device:R", "1k");
        place(&mut s, "PWR1", "rusty:Supply", "3V3");
        place(&mut s, "GND1", "rusty:GND", "GND");
        wire(&mut s, "PWR1.VCC", "C1.1");
        wire(&mut s, "C1.2", "GND1.GND");
        wire(&mut s, "PWR1.VCC", "R1.1");
        wire(&mut s, "R1.2", "GND1.GND");

        let bridged = build(&s).expect("built");
        assert!(
            bridged
                .circuit
                .elements
                .iter()
                .any(|e| matches!(e, Element::Capacitor { .. })),
            "it is in the circuit for the transient to use"
        );
        let found = dc(&bridged.circuit).expect("and the DC answer is unaffected");
        assert!((found.volts_at(bridged.node_of[&PinRef::new("R1", "1")]) - 3.3).abs() < 1e-9);

        // And one with nothing on its far side: that node has no DC
        // voltage, which is said rather than answered with a zero.
        let mut s = sheet();
        place(&mut s, "C1", "Device:C", "100n");
        place(&mut s, "R1", "Device:R", "1k");
        place(&mut s, "PWR1", "rusty:Supply", "3V3");
        place(&mut s, "GND1", "rusty:GND", "GND");
        wire(&mut s, "PWR1.VCC", "C1.1");
        wire(&mut s, "PWR1.VCC", "R1.1");
        wire(&mut s, "R1.2", "GND1.GND");
        let bridged = build(&s).expect("built");
        assert!(matches!(
            dc(&bridged.circuit),
            Err(crate::solve::Trouble::Floating { .. })
        ));
    }

    #[test]
    fn a_capacitance_is_read_the_way_it_is_written() {
        use crate::nets::farads;
        // Relative, because `100 * 1e-9` and `1e-7` are the same number and
        // not the same bits — the multiplier is applied, not looked up.
        let is = |text: &str, want: f64| {
            let got = farads(text).unwrap_or_else(|| panic!("{text} read as nothing"));
            assert!(
                (got - want).abs() <= 1e-12 * want,
                "{text}: {got} against {want}"
            );
        };
        is("100n", 1e-7);
        is("100nF", 1e-7);
        is("4n7", 4.7e-9);
        is("10u", 1e-5);
        is("10µF", 1e-5);
        is("1p", 1e-12);
        is("2.2u", 2.2e-6);
        assert_eq!(farads("C0805"), None, "a package is not a capacitance");
        assert_eq!(farads(""), None);
        assert_eq!(farads("red"), None);
    }

    /// The three refusals, each naming the part and the property that would
    /// answer it. This is the half of the bridge that matters most: a sheet
    /// says what is joined to what, and a number needs more than that.
    #[test]
    fn what_the_sheet_did_not_say_is_refused_by_name() {
        let mut s = sheet();
        place(&mut s, "R1", "Device:R", "brown");
        place(&mut s, "GND1", "rusty:GND", "GND");
        wire(&mut s, "R1.2", "GND1.GND");
        assert_eq!(
            build(&s),
            Err(Unstated::Resistance {
                part: "R1".into(),
                value: "brown".into()
            })
        );

        let mut s = sheet();
        place(&mut s, "PWR1", "rusty:Supply", "VCC");
        place(&mut s, "GND1", "rusty:GND", "GND");
        wire(&mut s, "PWR1.VCC", "GND1.GND");
        assert_eq!(
            build(&s),
            Err(Unstated::Supply {
                part: "PWR1".into(),
                value: "VCC".into()
            })
        );

        let mut s = sheet();
        place(&mut s, "D1", "Device:LED", "red");
        place(&mut s, "GND1", "rusty:GND", "GND");
        wire(&mut s, "D1.K", "GND1.GND");
        assert_eq!(
            build(&s),
            Err(Unstated::ForwardVoltage { part: "D1".into() })
        );

        // The devkit always brings a ground row with it, so this only
        // happens on a sheet that has no devkit either — which is what an
        // imported schematic drawn for another board looks like before
        // anything is joined to U1.
        let mut s = sheet();
        place(&mut s, "R1", "Device:R", "1k");
        place(&mut s, "PWR1", "rusty:Supply", "3V3");
        wire(&mut s, "PWR1.VCC", "R1.1");
        assert_eq!(
            of(&s, &[], &HashSet::new(), &BTreeMap::new()),
            Err(Unstated::NoGround)
        );
    }

    /// A lamp on a rail through its resistor, drawn as a sheet — the
    /// circuit every board here has, and the number a person would work
    /// out by hand.
    #[test]
    fn a_lamp_and_its_resistor_draw_what_the_arithmetic_says() {
        let mut s = sheet();
        place(&mut s, "R1", "Device:R", "330");
        place(&mut s, "D1", "Device:LED", "red");
        place(&mut s, "PWR1", "rusty:Supply", "3V3");
        place(&mut s, "GND1", "rusty:GND", "GND");
        if let Some(lamp) = s.parts.iter_mut().find(|p| p.reference == "D1") {
            lamp.props.insert("vf".into(), "2.0".into());
        }
        wire(&mut s, "PWR1.VCC", "R1.1");
        wire(&mut s, "R1.2", "D1.A");
        wire(&mut s, "D1.K", "GND1.GND");

        let bridged = build(&s).expect("built");
        let found = dc(&bridged.circuit).expect("solved");
        let across = found.volts_at(bridged.node_of[&PinRef::new("D1", "A")]);
        let current = (3.3 - across) / 330.0;
        // By hand: (3.3 − 2.0) / 330 ≈ 3.9 mA. The lamp's own curve moves
        // the drop a little from the quoted 2.0 V at ten milliamps, which
        // is why this is a band and not an equality.
        assert!(
            (3.0e-3..4.5e-3).contains(&current),
            "about four milliamps: {current} A at {across} V"
        );
    }

    /// A pin the firmware is driving is a source, and the level decides
    /// which way the lamp sees it.
    #[test]
    fn a_driven_pin_becomes_a_source_at_the_boards_own_rail() {
        let mut s = sheet();
        place(&mut s, "R1", "Device:R", "1k");
        place(&mut s, "GND1", "rusty:GND", "GND");
        wire(&mut s, "U1.GPIO2", "R1.1");
        wire(&mut s, "R1.2", "GND1.GND");

        let high = of(
            &s,
            &rows(),
            &HashSet::new(),
            &[(2u8, true)].into_iter().collect(),
        )
        .expect("built");
        let found = dc(&high.circuit).expect("solved");
        let driven = high.node_of[&PinRef::new("R1", "1")];
        assert!(
            (found.volts_at(driven) - 3.3).abs() < 1e-9,
            "the devkit's own 3V3 row names its voltage: {}",
            found.volts_at(driven)
        );

        let low = of(
            &s,
            &rows(),
            &HashSet::new(),
            &[(2u8, false)].into_iter().collect(),
        )
        .expect("built");
        let found = dc(&low.circuit).expect("solved");
        assert!(found.volts_at(low.node_of[&PinRef::new("R1", "1")]).abs() < 1e-9);
    }

    /// What the panel puts beside a part, against the arithmetic anybody
    /// does on a divider: two resistors in series carry **one** current,
    /// `3.3 / 30k`, and each drops its own share of it.
    #[test]
    fn a_part_reads_the_current_and_the_drop_the_series_arithmetic_gives() {
        let mut s = sheet();
        place(&mut s, "R1", "Device:R", "20k");
        place(&mut s, "R2", "Device:R", "10k");
        place(&mut s, "PWR1", "rusty:Supply", "3V3");
        place(&mut s, "GND1", "rusty:GND", "GND");
        wire(&mut s, "PWR1.VCC", "R1.1");
        wire(&mut s, "R1.2", "R2.1");
        wire(&mut s, "R2.2", "GND1.GND");

        let solved = operating_point(&s, &rows(), &HashSet::new(), &BTreeMap::new())
            .expect("a divider is solvable");
        let one = solved.reading("R1").expect("R1 is an element");
        let two = solved.reading("R2").expect("R2 is an element");
        let amps = 3.3 / 30_000.0;
        assert!((one.through - amps).abs() < 1e-12, "{one:?}");
        assert!(
            (one.through - two.through).abs() < 1e-15,
            "in series, so one current: {one:?} {two:?}"
        );
        assert!((one.across - 3.3 * 2.0 / 3.0).abs() < 1e-9, "{one:?}");
        assert!((two.across - 3.3 / 3.0).abs() < 1e-9, "{two:?}");
        assert!(
            (one.watts() - amps * amps * 20_000.0).abs() < 1e-12,
            "I²R: {}",
            one.watts()
        );
        // Nothing electrical to say about a rail, and that is not a
        // failure — its voltage is the answer, not something read off it.
        assert_eq!(solved.reading("PWR1"), None);
        assert_eq!(
            solved
                .volts_at(&PinRef::new("R1", "2"))
                .map(|v| (v * 1e6).round()),
            Some((3.3f64 / 3.0 * 1e6).round())
        );
    }

    /// **The sign is which way the part is drawn, and only a test can hold
    /// it.** `through` is the passive convention — into the first terminal
    /// — so the same resistor wired the other way round reads the negative
    /// of itself. The doc comment on `Solution::through` claimed the
    /// opposite of its own test for a while, which is exactly how a panel
    /// ends up reporting a current backwards.
    #[test]
    fn turning_a_part_round_turns_its_reading_round() {
        let build = |from_pin: &str, to_pin: &str| {
            let mut s = sheet();
            place(&mut s, "R1", "Device:R", "1k");
            place(&mut s, "PWR1", "rusty:Supply", "3V3");
            place(&mut s, "GND1", "rusty:GND", "GND");
            wire(&mut s, "PWR1.VCC", &format!("R1.{from_pin}"));
            wire(&mut s, &format!("R1.{to_pin}"), "GND1.GND");
            operating_point(&s, &rows(), &HashSet::new(), &BTreeMap::new())
                .expect("solvable")
                .reading("R1")
                .expect("an element")
        };
        let forwards = build("1", "2");
        let backwards = build("2", "1");
        assert!((forwards.through - 3.3e-3).abs() < 1e-12, "{forwards:?}");
        assert!((backwards.through + 3.3e-3).abs() < 1e-12, "{backwards:?}");
        assert!((forwards.across + backwards.across).abs() < 1e-12);
        // Dissipation has no direction, and a reading that got the sign
        // wrong on one of the two would show it here.
        assert!((forwards.watts() - backwards.watts()).abs() < 1e-15);
        assert!(forwards.watts() > 0.0, "a resistor dissipates");
    }

    /// The gate that a linear one cannot be: at a junction the *reading*
    /// has to satisfy Kirchhoff, with the lamp's own curve evaluated at the
    /// answer rather than at the figure its `vf` was quoted at. Two
    /// elements in series, one of them exponential, and one current.
    #[test]
    fn a_lamp_reads_the_same_current_as_the_resistor_feeding_it() {
        let mut s = sheet();
        place(&mut s, "R1", "Device:R", "330");
        place(&mut s, "D1", "Device:LED", "red");
        place(&mut s, "PWR1", "rusty:Supply", "3V3");
        place(&mut s, "GND1", "rusty:GND", "GND");
        if let Some(lamp) = s.parts.iter_mut().find(|p| p.reference == "D1") {
            lamp.props.insert("vf".into(), "2.0".into());
        }
        wire(&mut s, "PWR1.VCC", "R1.1");
        wire(&mut s, "R1.2", "D1.A");
        wire(&mut s, "D1.K", "GND1.GND");

        let solved =
            operating_point(&s, &rows(), &HashSet::new(), &BTreeMap::new()).expect("solvable");
        let resistor = solved.reading("R1").expect("R1");
        let lamp = solved.reading("D1").expect("D1");
        // Both run the way the loop does — the resistor from its pin 1,
        // which is on the supply, and the junction from its anode, which
        // a diode's element always does — so in series they read the same
        // number rather than opposite ones. Which sign a part reads is a
        // fact about how it was drawn, and that is the neighbouring test.
        assert!(
            (resistor.through - lamp.through).abs() < 1e-12,
            "one current through the pair: {resistor:?} {lamp:?}"
        );
        assert!(
            (resistor.across + lamp.across - 3.3).abs() < 1e-9,
            "and the two drops add up to the rail: {resistor:?} {lamp:?}"
        );
        assert!(
            (3.0e-3..4.5e-3).contains(&resistor.through),
            "about four milliamps by hand: {resistor:?}"
        );
    }

    /// A capacitor carries nothing at DC, and `0.0` is an answer rather
    /// than a refusal — a panel showing nothing there would read as a part
    /// the solver could not reach.
    #[test]
    fn a_capacitor_reads_the_rail_and_no_current() {
        let mut s = sheet();
        place(&mut s, "C1", "Device:C", "100n");
        place(&mut s, "PWR1", "rusty:Supply", "3V3");
        place(&mut s, "GND1", "rusty:GND", "GND");
        wire(&mut s, "PWR1.VCC", "C1.1");
        wire(&mut s, "C1.2", "GND1.GND");

        let solved =
            operating_point(&s, &rows(), &HashSet::new(), &BTreeMap::new()).expect("solvable");
        let reading = solved.reading("C1").expect("C1 is an element");
        assert!((reading.across - 3.3).abs() < 1e-9, "{reading:?}");
        assert_eq!(reading.through, 0.0, "open at DC");
    }

    /// The two halves of a refusal stay apart. A drawing that has not said
    /// enough names the property; a drawing that says enough and is *wrong*
    /// is the solver's finding, and a panel that showed one as the other
    /// would send somebody to fill in a field that is already there.
    #[test]
    fn what_cannot_be_solved_says_which_kind_of_wrong_it_is() {
        let mut s = sheet();
        place(&mut s, "R1", "Device:R", "wrong");
        place(&mut s, "PWR1", "rusty:Supply", "3V3");
        place(&mut s, "GND1", "rusty:GND", "GND");
        wire(&mut s, "PWR1.VCC", "R1.1");
        wire(&mut s, "R1.2", "GND1.GND");
        let why = operating_point(&s, &rows(), &HashSet::new(), &BTreeMap::new())
            .expect_err("a value that is not a resistance");
        assert!(
            matches!(why, Unsolved::Unstated(Unstated::Resistance { .. })),
            "{why:?}"
        );
        assert!(why.to_string().contains("4k7"), "it says how to write one");

        // And two resistors wired to each other and to nothing else: a
        // drawing that says everything and still has no answer, because
        // those two nodes reach ground through nothing. Somebody drew this
        // by moving a part off its rail.
        let mut s = sheet();
        place(&mut s, "R1", "Device:R", "1k");
        place(&mut s, "R2", "Device:R", "1k");
        place(&mut s, "GND1", "rusty:GND", "GND");
        wire(&mut s, "R1.1", "R2.1");
        wire(&mut s, "R1.2", "R2.2");
        let why = operating_point(&s, &rows(), &HashSet::new(), &BTreeMap::new())
            .expect_err("a loop that never reaches ground");
        let Unsolved::Floating { pins } = &why else {
            panic!("a node with no path to ground: {why:?}");
        };
        // And it says which pins, in the sheet's own words. The solver
        // knows this as "node 1", which is an index into an array this
        // module built and renumbered — a number nobody can point at on a
        // canvas.
        assert!(
            pins.contains(&PinRef::new("R1", "1")) || pins.contains(&PinRef::new("R1", "2")),
            "an end of the loop is named: {pins:?}"
        );
        assert!(why.to_string().contains("R1."), "{why}");
        assert_eq!(why.part(), None, "a loose net is not one part's fault");
    }

    #[test]
    fn a_rail_is_read_the_way_it_is_written_and_a_name_is_not_a_voltage() {
        assert_eq!(volts("3V3"), Some(3.3));
        assert_eq!(volts("3.3V"), Some(3.3));
        assert_eq!(volts("+5V"), Some(5.0));
        assert_eq!(volts("12"), Some(12.0));
        assert_eq!(volts("VCC"), None, "a name is not a number");
        assert_eq!(volts("VDD"), None);
        assert_eq!(volts(""), None);
    }
}
