//! The wiring as a graph: every pin a node, and the three partitions the
//! rules read — what the wires join, what joins with no resistance in it,
//! and what conducts.

use std::collections::{BTreeMap, HashMap, HashSet};

use super::{Behaviour, Rail, Row, Warning, behaviour_of, kit_pin, power_rail};
use crate::model::{KIT_REFERENCE, PinRef, Sheet};
use crate::union_find::UnionFind;

/// Which pins are the same *node* — joined by wires, labels and closed
/// switches, with nothing resistive in between.
///
/// The partition a circuit is built on, and not the conducting one: a
/// resistor's two ends are two nodes with an element between them, which is
/// the whole of what makes an answer possible. `crate::circuit` is the one
/// caller; it is exposed here because this is where the wires are read.
pub fn solid_nets(
    sheet: &Sheet,
    rows: &[Row],
    pressed: &HashSet<String>,
) -> BTreeMap<PinRef, usize> {
    let (graph, mut solid) = Graph::solid_of(sheet, rows, pressed);
    let mut out = BTreeMap::new();
    for node in 0..graph.nodes.len() {
        out.insert(graph.nodes[node].clone(), solid.find(node));
    }
    // A pin found by name answers with the same node as by number, the way
    // every other reading here does.
    let mut aliases: Vec<(PinRef, usize)> = Vec::new();
    for part in &sheet.parts {
        if let Some(symbol) = sheet.symbol_of(&part.reference) {
            for pin in symbol.pins.iter().filter(|p| p.name != p.number) {
                let by_number = PinRef::new(&part.reference, &pin.number);
                if let Some(node) = out.get(&by_number) {
                    aliases.push((PinRef::new(&part.reference, &pin.name), *node));
                }
            }
        }
    }
    for (row, spec) in rows.iter().enumerate() {
        let by_name = PinRef::new(KIT_REFERENCE, &spec.name);
        if let Some(node) = out.get(&PinRef::kit(row))
            && !out.contains_key(&by_name)
        {
            aliases.push((by_name, *node));
        }
    }
    out.extend(aliases);
    out
}

/// A node of the union-find: one pin of one part, or one kit row.
pub(super) type Node = usize;

pub(super) struct Graph<'a> {
    sheet: &'a Sheet,
    pub(super) rows: &'a [Row],
    /// Every pin that a wire or a conducting part touches.
    pub(super) nodes: Vec<PinRef>,
    index: HashMap<PinRef, Node>,
    pub(super) behaviours: HashMap<&'a str, Behaviour>,
    pub(super) warnings: Vec<Warning>,
}

impl<'a> Graph<'a> {
    pub(super) fn new(sheet: &'a Sheet, rows: &'a [Row]) -> Self {
        let behaviours = sheet
            .parts
            .iter()
            .filter_map(|p| {
                let symbol = sheet.symbols.iter().find(|s| s.id() == p.symbol)?;
                Some((p.reference.as_str(), behaviour_of(symbol)))
            })
            .collect();
        let mut graph = Graph {
            sheet,
            rows,
            nodes: Vec::new(),
            index: HashMap::new(),
            behaviours,
            warnings: Vec::new(),
        };
        // Every pin of every part gets a node, so a level is answered for an
        // unwired pin too (as floating) and the indices are stable.
        for part in &sheet.parts {
            if let Some(symbol) = sheet.symbols.iter().find(|s| s.id() == part.symbol) {
                for pin in &symbol.pins {
                    graph.node(PinRef::new(&part.reference, &pin.number));
                }
            }
        }
        for row in 0..rows.len() {
            graph.node(PinRef::kit(row));
        }
        graph
    }

    /// The graph and its solid partition, with `pressed` held.
    pub(super) fn solid_of(
        sheet: &'a Sheet,
        rows: &'a [Row],
        pressed: &HashSet<String>,
    ) -> (Self, UnionFind) {
        let mut graph = Graph::new(sheet, rows);
        let wired = graph.wired();
        let solid = graph.solid(&wired, pressed);
        (graph, solid)
    }

    /// The graph and its conducting partition with nothing held: what a
    /// question about where a pin *reaches* is asked of.
    pub(super) fn conducting_of(sheet: &'a Sheet, rows: &'a [Row]) -> (Self, UnionFind) {
        let (mut graph, solid) = Graph::solid_of(sheet, rows, &HashSet::new());
        let conducting = graph.conducting(&solid);
        (graph, conducting)
    }

    fn node(&mut self, pin: PinRef) -> Node {
        if let Some(&node) = self.index.get(&pin) {
            return node;
        }
        let node = self.nodes.len();
        self.nodes.push(pin.clone());
        self.index.insert(pin, node);
        node
    }

    /// The node a wire end names, with the pin resolved to its number — a
    /// wire may say `D1.K` and the node is `D1.1`.
    fn resolve(&self, end: &PinRef) -> Option<Node> {
        if end.part == KIT_REFERENCE {
            let row = kit_pin(self.rows, &end.pin)?;
            return self.index.get(&PinRef::kit(row)).copied();
        }
        let symbol = self.sheet.symbol_of(&end.part)?;
        let pin = symbol.pin(&end.pin)?;
        self.index
            .get(&PinRef::new(&end.part, &pin.number))
            .copied()
    }

    /// The node of a part's pin by name or number.
    pub(super) fn pin_node(&self, part: &str, key: &str) -> Option<Node> {
        self.resolve(&PinRef::new(part, key))
    }

    /// The two ends of a two-terminal part, in pin order.
    pub(super) fn terminals(&self, part: &str) -> Option<(Node, Node)> {
        let (a, b) = self.sheet.symbol_of(part)?.two_terminals()?;
        Some((
            self.pin_node(part, &a.number)?,
            self.pin_node(part, &b.number)?,
        ))
    }

    /// The devkit row a node stands for, or `None` for a part's pin.
    pub(super) fn row_of(&self, node: Node) -> Option<&'a Row> {
        let pin = &self.nodes[node];
        if pin.part != KIT_REFERENCE {
            return None;
        }
        kit_pin(self.rows, &pin.pin).map(|row| &self.rows[row])
    }

    /// Every node in the net `root` heads, in node order.
    pub(super) fn members<'u>(
        &self,
        uf: &'u mut UnionFind,
        root: Node,
    ) -> impl Iterator<Item = Node> + 'u {
        (0..self.nodes.len()).filter(move |node| uf.find(*node) == root)
    }

    /// The first GPIO row in the net `root` heads.
    pub(super) fn gpio_in(&self, uf: &mut UnionFind, root: Node) -> Option<u8> {
        self.members(uf, root)
            .find_map(|node| self.row_of(node).and_then(|row| row.gpio))
    }

    /// The distinct nets a part's pins sit in, in pin order: one per side of
    /// a switch, however many pins that side has.
    pub(super) fn sides(&self, uf: &mut UnionFind, part: &str) -> Vec<Node> {
        let Some(symbol) = self.sheet.symbol_of(part) else {
            return Vec::new();
        };
        let mut roots: Vec<Node> = Vec::new();
        for pin in &symbol.pins {
            let Some(node) = self.pin_node(part, &pin.number) else {
                continue;
            };
            let root = uf.find(node);
            if !roots.contains(&root) {
                roots.push(root);
            }
        }
        roots
    }

    /// The nets the wires alone make.
    pub(super) fn wired(&mut self) -> UnionFind {
        let mut uf = UnionFind::new(self.nodes.len());
        for wire in &self.sheet.wires {
            match (self.resolve(&wire.from), self.resolve(&wire.to)) {
                (Some(a), Some(b)) => uf.union(a, b),
                _ => self.warnings.push(Warning::DanglingWire {
                    from: wire.from.to_string(),
                    to: wire.to.to_string(),
                }),
            }
        }
        uf
    }

    /// The wired nets plus every join that has *no resistance in it*: a
    /// label's name, which is a wire drawn in words, and a closed switch,
    /// which is a piece of wire while it is held.
    ///
    /// This partition exists to answer one question the conducting one
    /// cannot: whether two rails are actually shorted. A resistor between a
    /// supply and ground is a voltage divider — the commonest analog
    /// circuit there is — and joining its ends into one node made every one
    /// of them read as `ground and a supply share a net`, which is the
    /// confident wrong answer this file exists to avoid. Measured before it
    /// was fixed: a plain two-resistor divider with its midpoint on GPIO4
    /// reported exactly that.
    pub(super) fn solid(&mut self, wired: &UnionFind, pressed: &HashSet<String>) -> UnionFind {
        let mut uf = wired.clone();
        // Labels first: a name is a wire drawn in words, and everything
        // after this treats the joined pins as the one node they are.
        let mut by_name: HashMap<String, Node> = HashMap::new();
        for part in &self.sheet.parts {
            if self.behaviours.get(part.reference.as_str()) != Some(&Behaviour::Label) {
                continue;
            }
            let name = part.value.trim();
            if name.is_empty() {
                continue;
            }
            let Some(symbol) = self.sheet.symbol_of(&part.reference) else {
                continue;
            };
            let Some(first) = symbol.pins.first() else {
                continue;
            };
            let Some(node) = self.pin_node(&part.reference, &first.number) else {
                continue;
            };
            match by_name.get(name) {
                Some(other) => uf.union(*other, node),
                None => {
                    by_name.insert(name.to_string(), node);
                }
            }
        }
        for part in &self.sheet.parts {
            let reference = part.reference.as_str();
            if self.behaviours.get(reference) != Some(&Behaviour::Switch) {
                continue;
            }
            let Some(symbol) = self.sheet.symbol_of(reference) else {
                continue;
            };
            let nodes: Vec<Node> = symbol
                .pins
                .iter()
                .filter_map(|p| self.pin_node(reference, &p.number))
                .collect();
            // A four-pin tactile switch has its pairs joined always and all
            // four while pressed.
            if nodes.len() >= 4 {
                uf.union(nodes[0], nodes[1]);
                uf.union(nodes[2], nodes[3]);
            }
            if pressed.contains(reference) {
                for pair in nodes.windows(2) {
                    uf.union(pair[0], pair[1]);
                }
            }
        }
        uf
    }

    /// The solid nets plus every resistor.
    ///
    /// This is the partition that answers *what reaches what*: a GPIO
    /// through a series resistor still lights the lamp beyond it, and a
    /// probe on the sheet lists everything down the chain. It is not the
    /// partition to judge a short on — see [`Graph::solid`] — and it is not
    /// one a level can be read off point by point either, since a divider's
    /// two ends and its midpoint are all one node here. The midpoint's
    /// value comes from [`divider_at`](super::divider_at), which walks the resistors instead of
    /// merging them.
    pub(super) fn conducting(&mut self, solid: &UnionFind) -> UnionFind {
        let mut uf = solid.clone();
        for part in &self.sheet.parts {
            let reference = part.reference.as_str();
            if self.behaviours.get(reference) == Some(&Behaviour::Resistor)
                && let Some((a, b)) = self.terminals(reference)
            {
                uf.union(a, b);
            }
        }
        uf
    }

    /// What drives each net: the rails and the reported GPIOs in it, and
    /// whether two of those rails meet with nothing between them — which is
    /// a short, where the same two rails through a resistor are a divider.
    pub(super) fn drivers(
        &self,
        uf: &mut UnionFind,
        solid: &mut UnionFind,
        gpio: &HashMap<u8, bool>,
    ) -> HashMap<Node, Drivers> {
        let mut out: HashMap<Node, Drivers> = HashMap::new();
        // Where each rail sits in the *solid* partition, kept beside the
        // answer so the short can be judged without a second walk.
        let mut rail_nodes: HashMap<Node, Vec<(Rail, Node)>> = HashMap::new();
        // A power symbol is a rail wherever it is drawn: the same thing the
        // devkit's own GND pin is, without a wire across the whole sheet.
        for part in &self.sheet.parts {
            let Some(symbol) = self.sheet.symbol_of(&part.reference) else {
                continue;
            };
            let Some(rail) = power_rail(symbol) else {
                continue;
            };
            let Some(first) = symbol.pins.first() else {
                continue;
            };
            let Some(node) = self.pin_node(&part.reference, &first.number) else {
                continue;
            };
            let root = uf.find(node);
            out.entry(root)
                .or_default()
                .rails
                .push((rail, PinRef::new(&part.reference, &first.number)));
            rail_nodes.entry(root).or_default().push((rail, node));
        }
        for (node, pin) in self.nodes.iter().enumerate() {
            let Some(row) = self.row_of(node) else {
                continue;
            };
            let root = uf.find(node);
            let drivers = out.entry(root).or_default();
            match (&row.rail, row.gpio) {
                (Some(rail), _) => {
                    drivers.rails.push((*rail, pin.clone()));
                    rail_nodes.entry(root).or_default().push((*rail, node));
                }
                (None, Some(n)) => {
                    drivers.gpios.push((n, gpio.get(&n).copied(), pin.clone()));
                }
                (None, None) => {}
            }
        }
        // Two rails are shorted when they are the same node with no
        // resistance in between. Through a resistor they are a divider, and
        // saying "short" about one of those is what this now avoids.
        for (root, rails) in &rail_nodes {
            let shorted = rails.iter().enumerate().any(|(i, (rail, node))| {
                rails[i + 1..]
                    .iter()
                    .any(|(other, at)| other != rail && solid.find(*node) == solid.find(*at))
            });
            if shorted && let Some(drivers) = out.get_mut(root) {
                drivers.shorted = true;
            }
        }
        out
    }
}

#[derive(Default, Clone)]
pub(super) struct Drivers {
    rails: Vec<(Rail, PinRef)>,
    gpios: Vec<(u8, Option<bool>, PinRef)>,
    /// Two of `rails` disagree with nothing between them.
    shorted: bool,
}

impl Drivers {
    /// The net's level, and what to say when the drivers disagree.
    pub(super) fn level(&self) -> (Option<bool>, Option<Warning>) {
        let grounds = self
            .rails
            .iter()
            .filter(|(r, _)| *r == Rail::Ground)
            .count();
        let supplies = self.rails.len() - grounds;
        if grounds > 0 && supplies > 0 {
            if self.shorted {
                return (
                    None,
                    Some(Warning::Short {
                        pins: self.rails.iter().map(|(_, p)| p.to_string()).collect(),
                    }),
                );
            }
            // Both rails, but only through resistance: a divider, and not a
            // fault. What its midpoint sits at is a number, not a level, and
            // `divider_at` is where the number is read — this net has no one
            // level to give, so it gives none rather than one of the two it
            // is between.
            return (None, None);
        }
        if let Some((rail, _)) = self.rails.first() {
            return (Some(*rail == Rail::Supply), None);
        }
        let reported: Vec<&(u8, Option<bool>, PinRef)> =
            self.gpios.iter().filter(|(_, l, _)| l.is_some()).collect();
        let Some(first) = reported.first() else {
            return (None, None);
        };
        if reported.iter().any(|(_, l, _)| *l != first.1) {
            return (
                first.1,
                Some(Warning::Conflict {
                    pins: reported.iter().map(|(_, _, p)| p.to_string()).collect(),
                }),
            );
        }
        (first.1, None)
    }
}
