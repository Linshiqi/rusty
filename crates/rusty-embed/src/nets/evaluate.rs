//! One reading of the whole sheet: every pin's level, every lamp's state,
//! and what is wrong with the drawing.

use std::collections::{BTreeMap, HashMap, HashSet};

use super::graph::{Drivers, Graph, Node, UnionFind};
use super::{Behaviour, Row, Warning, button_drives, kit_pin};
use crate::model::{KIT_REFERENCE, Pin, PinKind, PinRef, Sheet};

/// The whole reading of one sheet at one moment.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Evaluation {
    /// Which lamp pins are lit: a plain LED under its anode, an RGB or a
    /// seven-segment under each channel.
    pub lit: BTreeMap<PinRef, bool>,
    /// The DC level at every wired pin — `None` for a floating net.
    pub levels: BTreeMap<PinRef, Option<bool>>,
    /// Which net each pin belongs to, as the conducting rules join them:
    /// through wires, through resistors, through a closed switch, and
    /// between labels that share a name. Two pins with the same number are
    /// the same node — which is what a probe on the sheet asks, and what
    /// tells a wire that goes somewhere from one that goes nowhere.
    pub nets: BTreeMap<PinRef, usize>,
    pub warnings: Vec<Warning>,
}

impl Evaluation {
    /// Whether any lamp of a part is lit.
    pub fn is_lit(&self, part: &str) -> bool {
        self.lit.iter().any(|(pin, lit)| pin.part == part && *lit)
    }

    pub fn is_pin_lit(&self, part: &str, pin: &str) -> bool {
        self.lit
            .get(&PinRef::new(part, pin))
            .copied()
            .unwrap_or(false)
    }

    pub fn level(&self, part: &str, pin: &str) -> Option<bool> {
        self.levels.get(&PinRef::new(part, pin)).copied().flatten()
    }

    /// The net a pin sits in.
    pub fn net_of(&self, pin: &PinRef) -> Option<usize> {
        self.nets.get(pin).copied()
    }

    /// Every pin in a net, in a stable order — what a probe lists.
    pub fn members(&self, net: usize) -> Vec<PinRef> {
        self.nets
            .iter()
            .filter(|(_, at)| **at == net)
            .map(|(pin, _)| pin.clone())
            .collect()
    }
}

/// The inputs to one evaluation.
pub struct Inputs<'a> {
    pub sheet: &'a Sheet,
    /// The devkit's rows, from [`kit_rows`](super::kit_rows).
    pub rows: &'a [Row],
    /// Pin levels the firmware has reported, by GPIO number.
    pub gpio: &'a HashMap<u8, bool>,
    /// The switches currently held, by reference.
    pub pressed: &'a HashSet<String>,
}

/// Read the sheet: every pin's level, every lamp's state, and what is wrong.
pub fn evaluate(inputs: Inputs<'_>) -> Evaluation {
    let mut graph = Graph::new(inputs.sheet, inputs.rows);
    let mut wired = graph.wired();
    let mut solid = graph.solid(&wired, inputs.pressed);
    let mut dc = graph.conducting(&solid);
    let drivers = graph.drivers(&mut dc, &mut solid, inputs.gpio);

    let mut nets: BTreeMap<PinRef, usize> = BTreeMap::new();
    let mut levels: BTreeMap<PinRef, Option<bool>> = BTreeMap::new();
    let mut level_warnings: HashMap<Node, Warning> = HashMap::new();
    for node in 0..graph.nodes.len() {
        let root = dc.find(node);
        let (level, warning) = drivers
            .get(&root)
            .map(Drivers::level)
            .unwrap_or((None, None));
        if let Some(warning) = warning {
            level_warnings.entry(root).or_insert(warning);
        }
        levels.insert(graph.nodes[node].clone(), level);
        nets.insert(graph.nodes[node].clone(), root);
    }
    let level_at = |pin: Option<Node>| -> Option<bool> {
        pin.and_then(|n| levels.get(&graph.nodes[n]).copied().flatten())
    };
    // The map is keyed by pin number; a caller asking by name (`D1.A`,
    // `U1.GPIO9`) gets the same answer. The first row of a repeated kit
    // name stands for it, as `kit_pin` resolves it.
    let mut aliases: Vec<(PinRef, Option<bool>)> = Vec::new();
    for part in &inputs.sheet.parts {
        if let Some(symbol) = inputs.sheet.symbol_of(&part.reference) {
            for pin in symbol
                .pins
                .iter()
                .filter(|p| p.name != p.number && p.name != "~")
            {
                let by_number = PinRef::new(&part.reference, &pin.number);
                if let Some(level) = levels.get(&by_number) {
                    aliases.push((PinRef::new(&part.reference, &pin.name), *level));
                }
            }
        }
    }
    for (row, spec) in inputs.rows.iter().enumerate() {
        let by_number = PinRef::new(KIT_REFERENCE, (row + 1).to_string());
        let by_name = PinRef::new(KIT_REFERENCE, &spec.name);
        if let Some(level) = levels.get(&by_number)
            && !levels.contains_key(&by_name)
            && !aliases.iter().any(|(p, _)| *p == by_name)
        {
            aliases.push((by_name, *level));
        }
    }

    let mut lit: BTreeMap<PinRef, bool> = BTreeMap::new();
    let mut warnings: Vec<Warning> = std::mem::take(&mut graph.warnings);
    warnings.extend(level_warnings.into_values());

    // A GPIO row in the *wired* net of a lamp pin is a GPIO with nothing
    // between it and the lamp.
    let gpio_directly_on = |uf: &mut UnionFind, node: Node| -> bool {
        let root = uf.find(node);
        graph.nodes.iter().enumerate().any(|(other, pin)| {
            pin.part == KIT_REFERENCE
                && uf.find(other) == root
                && kit_pin(graph.rows, &pin.pin).is_some_and(|row| graph.rows[row].gpio.is_some())
        })
    };

    for part in &inputs.sheet.parts {
        let reference = part.reference.as_str();
        match graph.behaviours.get(reference) {
            // A buzzer is a lamp that makes a noise: current one way
            // through it and it is on. Reading it by the same rule is what
            // lets the sheet say "sounding" without a second set of
            // opinions about which way round the part is.
            Some(Behaviour::Led | Behaviour::Buzzer) => {
                let Some(symbol) = inputs.sheet.symbol_of(reference) else {
                    continue;
                };
                // By name first; KiCad's convention — pin 1 is the cathode,
                // pin 2 the anode — for a diode that names neither.
                let anode = symbol
                    .pin("A")
                    .or_else(|| symbol.pin("+"))
                    .or_else(|| symbol.pins.get(1))
                    .map(|p| p.number.clone());
                let cathode = symbol
                    .pin("K")
                    .or_else(|| symbol.pin("-"))
                    .or_else(|| symbol.pins.first())
                    .map(|p| p.number.clone());
                let (Some(anode), Some(cathode)) = (anode, cathode) else {
                    continue;
                };
                let a = graph.pin_node(reference, &anode);
                let k = graph.pin_node(reference, &cathode);
                let on = level_at(a) == Some(true) && level_at(k) == Some(false);
                lit.insert(PinRef::new(reference, &anode), on);
                if graph.behaviours.get(reference) == Some(&Behaviour::Led)
                    && [a, k]
                        .into_iter()
                        .flatten()
                        .any(|n| gpio_directly_on(&mut wired, n))
                {
                    warnings.push(Warning::LedWithoutResistor {
                        part: reference.to_string(),
                    });
                }
            }
            Some(Behaviour::Rgb | Behaviour::Seven) => {
                let Some(symbol) = inputs.sheet.symbol_of(reference) else {
                    continue;
                };
                let common = level_at(graph.pin_node(reference, "COM"));
                for pin in symbol.pins.iter().filter(|p| p.name != "COM") {
                    let channel = level_at(graph.pin_node(reference, &pin.number));
                    // Common anode lights a channel pulled low; common
                    // cathode one driven high. No common wired: nothing lights.
                    let on = matches!(
                        (common, channel),
                        (Some(true), Some(false)) | (Some(false), Some(true))
                    );
                    lit.insert(PinRef::new(reference, &pin.number), on);
                }
            }
            Some(Behaviour::Switch)
                if button_drives(inputs.sheet, inputs.rows, reference).is_none() =>
            {
                warnings.push(Warning::SwitchDrivesNothing {
                    part: reference.to_string(),
                });
            }
            _ => {}
        }
    }

    // Lamps answer by the pin's name as well as its number.
    let mut lit_aliases: Vec<(PinRef, bool)> = Vec::new();
    for (pin, on) in &lit {
        if let Some(symbol) = inputs.sheet.symbol_of(&pin.part)
            && let Some(named) = symbol.pins.iter().find(|p| p.number == pin.pin)
            && named.name != named.number
        {
            lit_aliases.push((PinRef::new(&pin.part, &named.name), *on));
        }
    }
    levels.extend(aliases);
    lit.extend(lit_aliases);

    // The name half of the nets: a pin found by name answers with the same
    // net as the same pin found by number.
    let mut net_aliases: Vec<(PinRef, usize)> = Vec::new();
    for (pin, net) in &nets {
        if let Some(symbol) = inputs.sheet.symbol_of(&pin.part)
            && let Some(named) = symbol.pins.iter().find(|p| p.number == pin.pin)
            && named.name != named.number
        {
            net_aliases.push((PinRef::new(&pin.part, &named.name), *net));
        }
    }
    for (row, spec) in inputs.rows.iter().enumerate() {
        let by_number = PinRef::new(KIT_REFERENCE, (row + 1).to_string());
        let by_name = PinRef::new(KIT_REFERENCE, &spec.name);
        if let Some(net) = nets.get(&by_number)
            && !nets.contains_key(&by_name)
            && !net_aliases.iter().any(|(p, _)| *p == by_name)
        {
            net_aliases.push((by_name, *net));
        }
    }
    nets.extend(net_aliases);

    // ── the two checks that are about the drawing rather than the run ───
    //
    // A pin nobody joined, on a part somebody was joining. Both halves are
    // load-bearing: a part just dropped on the sheet has every pin loose and
    // does not want six findings, and a pin the author has marked as
    // deliberately unconnected has already answered the question.
    //
    // And it stands down where something more specific already names the
    // part: a switch with one side loose is `SwitchDrivesNothing`, and
    // saying "and a pin reaches nothing" beside it is the same fault twice.
    let already: Vec<String> = warnings
        .iter()
        .filter_map(|w| w.about_part().map(str::to_string))
        .collect();
    for part in &inputs.sheet.parts {
        if already.contains(&part.reference) {
            continue;
        }
        let Some(symbol) = inputs.sheet.symbol_of(&part.reference) else {
            continue;
        };
        let wired = |number: &str| {
            let by_number = PinRef::new(&part.reference, number);
            let named = symbol
                .pins
                .iter()
                .find(|p| p.number == number)
                .map(|p| PinRef::new(&part.reference, &p.name));
            inputs.sheet.wires.iter().any(|w| {
                w.from == by_number
                    || w.to == by_number
                    || named.as_ref().is_some_and(|n| w.from == *n || w.to == *n)
            })
        };
        let visible: Vec<&Pin> = symbol.pins.iter().filter(|p| !p.hidden).collect();
        if visible.is_empty() || !visible.iter().any(|p| wired(&p.number)) {
            continue;
        }
        for pin in visible {
            if wired(&pin.number)
                || inputs
                    .sheet
                    .is_no_connect(&PinRef::new(&part.reference, &pin.number))
                || inputs
                    .sheet
                    .is_no_connect(&PinRef::new(&part.reference, &pin.name))
            {
                continue;
            }
            // Named as a wire would name it: the name when it is one and
            // no other pin of the symbol shares it, the number otherwise —
            // the same rule the sheet spells a wire's ends by.
            let named = pin.name != "~"
                && !pin.name.is_empty()
                && symbol.pins.iter().filter(|p| p.name == pin.name).count() == 1;
            warnings.push(Warning::PinReachesNothing {
                part: part.reference.clone(),
                pin: if named {
                    pin.name.clone()
                } else {
                    pin.number.clone()
                },
            });
        }
    }

    // And two pins that both drive, joined. Not `Conflict`, which is two
    // GPIOs the firmware has driven apart while it runs: this one is true of
    // the drawing, before anything is built.
    let mut driving: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for part in &inputs.sheet.parts {
        let Some(symbol) = inputs.sheet.symbol_of(&part.reference) else {
            continue;
        };
        for pin in &symbol.pins {
            if !matches!(pin.kind, PinKind::Output | PinKind::PowerOut) {
                continue;
            }
            let at = PinRef::new(&part.reference, &pin.number);
            if let Some(net) = nets.get(&at) {
                driving.entry(*net).or_default().push(at.to_string());
            }
        }
    }
    for (_, mut pins) in driving {
        if pins.len() >= 2 {
            pins.sort();
            warnings.push(Warning::OutputsFighting { pins });
        }
    }

    Evaluation {
        lit,
        levels,
        nets,
        warnings,
    }
}
