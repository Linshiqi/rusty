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

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::model::{KIT_REFERENCE, PinRef, Sheet, Symbol};

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

/// One row of the devkit's header: what is printed beside it, the pin
/// name a wire uses, and what it carries.
///
/// The name is `GPIO<n>` for a GPIO row whatever the label says — the
/// ESP32 devkit prints `RX` beside GPIO3 — and the label itself for a rail.
/// Nothing here is geometry: where a row is drawn is the frontend's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub label: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpio: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rail: Option<Rail>,
}

impl Row {
    fn gpio(n: u8, label: &str) -> Self {
        Row {
            label: label.to_string(),
            name: format!("GPIO{n}"),
            gpio: Some(n),
            rail: None,
        }
    }

    fn rail(label: &str, rail: Rail) -> Self {
        Row {
            label: label.to_string(),
            name: label.to_string(),
            gpio: None,
            rail: Some(rail),
        }
    }

    fn plain(label: &str) -> Self {
        Row {
            label: label.to_string(),
            name: label.to_string(),
            gpio: None,
            rail: None,
        }
    }
}

/// The classic 30-pin ESP32 devkit header, top to bottom, left then right.
///
/// The one module whose *header order* rusty knows. That order is a
/// property of the board, not the die, so it cannot be derived — and it
/// used to be drawn for every chip, which is why an ESP32-C3 board showed
/// GPIO36, 39, 34 and 35, none of which the part has.
const ESP32_DEVKIT: [(&str, Option<u8>); 30] = [
    ("EN", None),
    ("36", Some(36)),
    ("39", Some(39)),
    ("34", Some(34)),
    ("35", Some(35)),
    ("32", Some(32)),
    ("33", Some(33)),
    ("25", Some(25)),
    ("26", Some(26)),
    ("27", Some(27)),
    ("14", Some(14)),
    ("12", Some(12)),
    ("13", Some(13)),
    ("GND", None),
    ("VIN", None),
    ("3V3", None),
    ("GND", None),
    ("15", Some(15)),
    ("2", Some(2)),
    ("4", Some(4)),
    ("16", Some(16)),
    ("17", Some(17)),
    ("5", Some(5)),
    ("18", Some(18)),
    ("19", Some(19)),
    ("21", Some(21)),
    ("RX", Some(3)),
    ("TX", Some(1)),
    ("22", Some(22)),
    ("23", Some(23)),
];

/// The devkit's rows for a chip, given the GPIOs it actually has.
///
/// Two different drawings, and the difference is honest rather than
/// cosmetic. For the ESP32 the answer is a *module*: a real 30-pin devkit
/// whose header order somebody can match against the board on their desk.
/// For everything else rusty knows the die's pins and not any module's
/// header, so it draws a *chip* — the pins in numeric order, split down the
/// middle, with the rails around them. Every row is then a pin that exists.
///
/// An empty `gpio` means the catalogue does not say, and the part is drawn
/// with rails only rather than with somebody else's pins.
pub fn kit_rows(chip: &str, gpio: &[u32]) -> Vec<Row> {
    let row = |label: &str, pin: Option<u8>| match (label, pin) {
        (_, Some(n)) => Row::gpio(n, label),
        ("GND", None) => Row::rail("GND", Rail::Ground),
        ("3V3" | "VIN" | "5V", None) => Row::rail(label, Rail::Supply),
        (other, None) => Row::plain(other),
    };
    if chip == "esp32" {
        return ESP32_DEVKIT
            .iter()
            .map(|(label, pin)| row(label, *pin))
            .collect();
    }
    let half = gpio.len().div_ceil(2);
    let mut rows: Vec<Row> = Vec::with_capacity(gpio.len() + 4);
    rows.push(row("EN", None));
    rows.extend(
        gpio[..half]
            .iter()
            .map(|p| row(&p.to_string(), Some(*p as u8))),
    );
    rows.push(row("GND", None));
    // The right column starts here, so the rails sit at the top of each side
    // the way they do on a module.
    rows.push(row("3V3", None));
    rows.extend(
        gpio[half..]
            .iter()
            .map(|p| row(&p.to_string(), Some(*p as u8))),
    );
    rows.push(row("GND", None));
    rows
}

/// Which row a wire to `U1.<key>` lands on: by number (the row's 1-based
/// position, the spelling a file uses when a name repeats) and then by
/// name — `GPIO2`, `GND`, `3V3`. The first `GND` for a bare `GND`.
pub fn kit_pin(rows: &[Row], key: &str) -> Option<usize> {
    key.parse::<usize>()
        .ok()
        .filter(|n| (1..=rows.len()).contains(n))
        .map(|n| n - 1)
        .or_else(|| rows.iter().position(|r| r.name == key))
}

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
        }
    }
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
        let bytes = bytes.trim();
        if bytes.len() % 2 != 0 {
            return None;
        }
        let mut out = Vec::with_capacity(bytes.len() / 2);
        for pair in bytes.as_bytes().chunks(2) {
            let pair = std::str::from_utf8(pair).ok()?;
            out.push(u8::from_str_radix(pair, 16).ok()?);
        }
        runs.push((at, out));
    }
    Some(runs)
}

/// Every device the sheet puts on the bus, and what it wants said about the
/// ones it could not.
///
/// The wiring is checked, not assumed. The emulator's bus does not route
/// through the GPIO matrix, so a device with no wires at all would answer
/// there and be silent on the desk — the confident wrong answer this
/// workbench exists to avoid. A part with an address whose `SDA` or `SCL`
/// reaches no GPIO is named and left off.
pub fn bus_devices(sheet: &Sheet, rows: &[Row]) -> (Vec<BusDevice>, Vec<Warning>) {
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
        let Some(parsed) = u8::from_str_radix(address.trim_start_matches("0x"), 16)
            .ok()
            .filter(|a| *a <= 0x7f)
        else {
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
        let regs = match part.props.get("regs").map(String::as_str) {
            None => Vec::new(),
            Some(text) if text.trim().is_empty() => Vec::new(),
            Some(text) => match parse_regs(text) {
                Some(regs) => regs,
                None => {
                    warnings.push(Warning::BusRegistersUnreadable {
                        part: part.reference.clone(),
                        value: text.to_string(),
                    });
                    Vec::new()
                }
            },
        };
        devices.push(BusDevice {
            part: part.reference.clone(),
            address: parsed,
            regs,
        });
    }
    (devices, warnings)
}

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
    /// The devkit's rows, from [`kit_rows`].
    pub rows: &'a [Row],
    /// Pin levels the firmware has reported, by GPIO number.
    pub gpio: &'a HashMap<u8, bool>,
    /// The switches currently held, by reference.
    pub pressed: &'a HashSet<String>,
}

/// A node of the union-find: one pin of one part, or one kit row.
type Node = usize;

struct Graph<'a> {
    sheet: &'a Sheet,
    rows: &'a [Row],
    /// Every pin that a wire or a conducting part touches.
    nodes: Vec<PinRef>,
    index: HashMap<PinRef, Node>,
    behaviours: HashMap<&'a str, Behaviour>,
    warnings: Vec<Warning>,
}

struct UnionFind(Vec<usize>);

impl UnionFind {
    fn new(n: usize) -> Self {
        UnionFind((0..n).collect())
    }

    fn find(&mut self, i: usize) -> usize {
        let mut root = i;
        while self.0[root] != root {
            root = self.0[root];
        }
        let mut at = i;
        while self.0[at] != root {
            let next = self.0[at];
            self.0[at] = root;
            at = next;
        }
        root
    }

    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.0[a] = b;
        }
    }
}

impl<'a> Graph<'a> {
    fn new(sheet: &'a Sheet, rows: &'a [Row]) -> Self {
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
            graph.node(PinRef::new(KIT_REFERENCE, (row + 1).to_string()));
        }
        graph
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
            return self
                .index
                .get(&PinRef::new(KIT_REFERENCE, (row + 1).to_string()))
                .copied();
        }
        let symbol = self.sheet.symbol_of(&end.part)?;
        let pin = symbol.pin(&end.pin)?;
        self.index
            .get(&PinRef::new(&end.part, &pin.number))
            .copied()
    }

    /// The node of a part's pin by name or number.
    fn pin_node(&self, part: &str, key: &str) -> Option<Node> {
        self.resolve(&PinRef::new(part, key))
    }

    /// The two ends of a two-terminal part, in pin order.
    fn terminals(&self, part: &str) -> Option<(Node, Node)> {
        let symbol = self.sheet.symbol_of(part)?;
        let mut pins = symbol.pins.iter().filter(|p| !p.hidden);
        let a = pins.next()?;
        let b = pins.next()?;
        Some((
            self.pin_node(part, &a.number)?,
            self.pin_node(part, &b.number)?,
        ))
    }

    /// The nets the wires alone make.
    fn wired(&mut self) -> UnionFind {
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

    /// The wired nets plus every part that conducts: resistors always, and
    /// the switches in `pressed`. A four-pin tactile switch has its pairs
    /// joined always and all four while pressed.
    fn conducting(&mut self, wired: &UnionFind, pressed: &HashSet<String>) -> UnionFind {
        let mut uf = UnionFind(wired.0.clone());
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
            match self.behaviours.get(reference) {
                Some(Behaviour::Resistor) => {
                    if let Some((a, b)) = self.terminals(reference) {
                        uf.union(a, b);
                    }
                }
                Some(Behaviour::Switch) => {
                    let Some(symbol) = self.sheet.symbol_of(reference) else {
                        continue;
                    };
                    let nodes: Vec<Node> = symbol
                        .pins
                        .iter()
                        .filter_map(|p| self.pin_node(reference, &p.number))
                        .collect();
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
                _ => {}
            }
        }
        uf
    }

    /// What drives each net: the rails and the reported GPIOs in it.
    fn drivers(&self, uf: &mut UnionFind, gpio: &HashMap<u8, bool>) -> HashMap<Node, Drivers> {
        let mut out: HashMap<Node, Drivers> = HashMap::new();
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
        }
        for (node, pin) in self.nodes.iter().enumerate() {
            if pin.part != KIT_REFERENCE {
                continue;
            }
            let Some(row) = kit_pin(self.rows, &pin.pin) else {
                continue;
            };
            let root = uf.find(node);
            let drivers = out.entry(root).or_default();
            match (&self.rows[row].rail, self.rows[row].gpio) {
                (Some(rail), _) => drivers.rails.push((*rail, pin.clone())),
                (None, Some(n)) => {
                    drivers.gpios.push((n, gpio.get(&n).copied(), pin.clone()));
                }
                (None, None) => {}
            }
        }
        out
    }
}

#[derive(Default, Clone)]
struct Drivers {
    rails: Vec<(Rail, PinRef)>,
    gpios: Vec<(u8, Option<bool>, PinRef)>,
}

impl Drivers {
    /// The net's level, and what to say when the drivers disagree.
    fn level(&self) -> (Option<bool>, Option<Warning>) {
        let grounds = self
            .rails
            .iter()
            .filter(|(r, _)| *r == Rail::Ground)
            .count();
        let supplies = self.rails.len() - grounds;
        if grounds > 0 && supplies > 0 {
            return (
                None,
                Some(Warning::Short {
                    pins: self.rails.iter().map(|(_, p)| p.to_string()).collect(),
                }),
            );
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

/// Read the sheet: every pin's level, every lamp's state, and what is wrong.
pub fn evaluate(inputs: Inputs<'_>) -> Evaluation {
    let mut graph = Graph::new(inputs.sheet, inputs.rows);
    let mut wired = graph.wired();
    let mut dc = graph.conducting(&wired, inputs.pressed);
    let drivers = graph.drivers(&mut dc, inputs.gpio);

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

    Evaluation {
        lit,
        levels,
        nets,
        warnings,
    }
}

/// The GPIO a part's pin reaches through the wires and the resistors — what
/// a pot's wiper or a motor's duty pin is *on*, in the firmware's terms.
pub fn gpio_of(sheet: &Sheet, rows: &[Row], part: &str, pin: &str) -> Option<u8> {
    let mut graph = Graph::new(sheet, rows);
    let wired = graph.wired();
    let mut dc = graph.conducting(&wired, &HashSet::new());
    let node = graph.pin_node(part, pin)?;
    let root = dc.find(node);
    graph
        .nodes
        .iter()
        .enumerate()
        .find_map(|(other, other_pin)| {
            (other_pin.part == KIT_REFERENCE && dc.find(other) == root)
                .then(|| kit_pin(rows, &other_pin.pin))
                .flatten()
                .and_then(|row| rows[row].gpio)
        })
}

/// What pressing a switch does: the GPIO it reaches on one side, and the
/// level the rail on its other side puts there. `None` when a press
/// changes nothing the firmware could read — no GPIO, no rail, or a switch
/// with one side unwired — which is a warning rather than a guess.
pub fn button_drives(sheet: &Sheet, rows: &[Row], part: &str) -> Option<(u8, bool)> {
    let mut graph = Graph::new(sheet, rows);
    let wired = graph.wired();
    let mut dc = graph.conducting(&wired, &HashSet::new());
    let symbol = sheet.symbol_of(part)?;
    let mut sides: Vec<(Option<u8>, Option<Rail>)> = Vec::new();
    let mut seen_roots: Vec<Node> = Vec::new();
    for pin in &symbol.pins {
        let Some(node) = graph.pin_node(part, &pin.number) else {
            continue;
        };
        let root = dc.find(node);
        if seen_roots.contains(&root) {
            continue;
        }
        seen_roots.push(root);
        let mut gpio = None;
        let mut rail = None;
        for (other, other_pin) in graph.nodes.iter().enumerate() {
            if other_pin.part != KIT_REFERENCE || dc.find(other) != root {
                continue;
            }
            if let Some(row) = kit_pin(rows, &other_pin.pin) {
                gpio = gpio.or(rows[row].gpio);
                rail = rail.or(rows[row].rail);
            }
        }
        sides.push((gpio, rail));
    }
    let gpio_side = sides.iter().find(|(g, _)| g.is_some())?;
    let rail_side = sides
        .iter()
        .find(|(g, r)| r.is_some() && *g != gpio_side.0)?;
    Some((gpio_side.0?, rail_side.1? == Rail::Supply))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Fill, Graphic, Instance, Pin, PinKind, Wire};

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
                .map(|(i, (number, name))| pin(number, name, if i % 2 == 0 { -5.08 } else { 5.08 }))
                .collect(),
            graphics: vec![Graphic::Rectangle {
                start: (-2.54, -1.27),
                end: (2.54, 1.27),
                width: 0.254,
                fill: Fill::None,
            }],
        }
    }

    fn place(sheet: &mut Sheet, reference: &str, symbol: &str) {
        sheet.parts.push(Instance {
            reference: reference.into(),
            symbol: symbol.into(),
            value: String::new(),
            x: 0.0,
            y: 0.0,
            rot: 0,
            mirror: false,
            props: BTreeMap::new(),
        });
    }

    fn wire(sheet: &mut Sheet, from: &str, to: &str) {
        sheet.wires.push(Wire {
            from: PinRef::parse(from).unwrap(),
            to: PinRef::parse(to).unwrap(),
            bends: Vec::new(),
        });
    }

    fn sheet() -> Sheet {
        let mut sheet = Sheet::empty("esp32c3");
        sheet.symbols = vec![
            symbol("Device", "LED", "D", &[("1", "K"), ("2", "A")]),
            symbol("Device", "R", "R", &[("1", "~"), ("2", "~")]),
            symbol("Device", "C", "C", &[("1", "~"), ("2", "~")]),
            symbol("Device", "SW_Push", "SW", &[("1", "1"), ("2", "2")]),
            symbol("lcsc", "C2286", "LED", &[("1", "A"), ("2", "K")]),
            symbol(
                "rusty",
                "RGB_LED",
                "D",
                &[("1", "R"), ("2", "G"), ("3", "B"), ("4", "COM")],
            ),
            symbol("rusty", "Pot", "RV", &[("1", "1"), ("2", "W"), ("3", "3")]),
            symbol("rusty", "GND", "#PWR", &[("1", "GND")]),
            symbol("rusty", "Supply", "#PWR", &[("1", "VCC")]),
            symbol("rusty", "Label", "#LBL", &[("1", "~")]),
            symbol("rusty", "Buzzer", "BZ", &[("1", "+"), ("2", "-")]),
            symbol(
                "rusty",
                "Sensor",
                "U",
                &[("1", "SDA"), ("2", "SCL"), ("3", "VCC"), ("4", "GND")],
            ),
        ];
        sheet
    }

    /// A sensor wired to the bus, with whatever properties the test wants.
    fn place_on_bus(sheet: &mut Sheet, reference: &str, props: &[(&str, &str)]) {
        place(sheet, reference, "rusty:Sensor");
        wire(sheet, &format!("{reference}.1"), "U1.GPIO5");
        wire(sheet, &format!("{reference}.2"), "U1.GPIO6");
        if let Some(part) = sheet.parts.iter_mut().find(|p| p.reference == reference) {
            for (key, value) in props {
                part.props.insert((*key).into(), (*value).into());
            }
        }
    }

    /// Place a part carrying a value — a rail's name, a label's net.
    fn place_with(sheet: &mut Sheet, reference: &str, symbol: &str, value: &str) {
        place(sheet, reference, symbol);
        if let Some(part) = sheet.parts.iter_mut().find(|p| p.reference == reference) {
            part.value = value.to_string();
        }
    }

    fn rows() -> Vec<Row> {
        kit_rows("esp32c3", &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 20, 21])
    }

    fn eval(sheet: &Sheet, gpio: &[(u8, bool)], pressed: &[&str]) -> Evaluation {
        let rows = rows();
        evaluate(Inputs {
            sheet,
            rows: &rows,
            gpio: &gpio.iter().copied().collect(),
            pressed: &pressed.iter().map(|s| s.to_string()).collect(),
        })
    }

    #[test]
    fn behaviour_is_read_off_the_library_then_the_prefix_and_the_pins() {
        let s = sheet();
        let of = |name: &str| behaviour_of(s.symbols.iter().find(|x| x.name == name).unwrap());
        assert_eq!(of("LED"), Behaviour::Led);
        assert_eq!(of("R"), Behaviour::Resistor);
        assert_eq!(of("C"), Behaviour::Capacitor);
        assert_eq!(of("SW_Push"), Behaviour::Switch);
        assert_eq!(of("C2286"), Behaviour::Led, "LCSC's prefix is LED");
        assert_eq!(of("RGB_LED"), Behaviour::Rgb);
        assert_eq!(of("Pot"), Behaviour::Pot);
        let diode = symbol("Device", "D", "D", &[("1", "K"), ("2", "A")]);
        assert_eq!(
            behaviour_of(&diode),
            Behaviour::Led,
            "an A and a K is a lamp"
        );
        let zener = symbol("Device", "D_Zener", "D", &[("1", "1"), ("2", "2")]);
        assert_eq!(behaviour_of(&zener), Behaviour::Other);
    }

    #[test]
    fn the_kit_rows_name_gpios_and_rails_and_a_repeated_name_is_spelled_by_number() {
        let rows = rows();
        assert_eq!(rows[0].name, "EN");
        assert_eq!(rows[1].name, "GPIO0");
        assert_eq!(kit_pin(&rows, "GPIO2"), Some(3));
        assert_eq!(kit_pin(&rows, "4"), Some(3), "the fourth row, by number");
        assert_eq!(kit_pin(&rows, "GND"), Some(8), "the first GND");
        assert_eq!(kit_pin(&rows, "GPIO99"), None);
        assert_eq!(kit_pin(&rows, "0"), None);
        let esp32 = kit_rows("esp32", &[]);
        assert_eq!(esp32[26].name, "GPIO3", "RX is GPIO3 by name");
        assert_eq!(esp32[26].label, "RX");
        assert_eq!(esp32[15].rail, Some(Rail::Supply));
    }

    /// GPIO2 → R1 → D1 → GND: the everyday circuit. The lamp follows the
    /// pin, through the resistor, and the sheet has nothing to say.
    #[test]
    fn a_lamp_through_a_resistor_follows_its_gpio() {
        let mut s = sheet();
        place(&mut s, "R1", "Device:R");
        place(&mut s, "D1", "Device:LED");
        wire(&mut s, "U1.GPIO2", "R1.1");
        wire(&mut s, "R1.2", "D1.A");
        wire(&mut s, "D1.K", "U1.GND");
        let on = eval(&s, &[(2, true)], &[]);
        assert!(on.is_lit("D1"));
        assert!(on.is_pin_lit("D1", "2"), "keyed by the anode's number");
        assert_eq!(on.level("D1", "A"), Some(true));
        assert_eq!(on.level("D1", "K"), Some(false));
        assert!(on.warnings.is_empty(), "{:?}", on.warnings);
        let off = eval(&s, &[(2, false)], &[]);
        assert!(!off.is_lit("D1"));
        let unknown = eval(&s, &[], &[]);
        assert!(!unknown.is_lit("D1"), "a pin never reported is not high");
        assert_eq!(unknown.level("D1", "A"), None);
    }

    #[test]
    fn a_lamp_the_wrong_way_round_stays_dark_and_a_capacitor_passes_nothing() {
        let mut s = sheet();
        place(&mut s, "D1", "Device:LED");
        wire(&mut s, "U1.GPIO2", "D1.K");
        wire(&mut s, "D1.A", "U1.GND");
        assert!(!eval(&s, &[(2, true)], &[]).is_lit("D1"));

        let mut s = sheet();
        place(&mut s, "C1", "Device:C");
        place(&mut s, "D1", "Device:LED");
        wire(&mut s, "U1.GPIO2", "C1.1");
        wire(&mut s, "C1.2", "D1.A");
        wire(&mut s, "D1.K", "U1.GND");
        let e = eval(&s, &[(2, true)], &[]);
        assert!(!e.is_lit("D1"));
        assert_eq!(
            e.level("D1", "A"),
            None,
            "the anode floats behind the capacitor"
        );
    }

    #[test]
    fn a_lamp_straight_on_a_gpio_lights_and_is_warned_about() {
        let mut s = sheet();
        place(&mut s, "D1", "Device:LED");
        wire(&mut s, "U1.GPIO2", "D1.A");
        wire(&mut s, "D1.K", "U1.GND");
        let e = eval(&s, &[(2, true)], &[]);
        assert!(e.is_lit("D1"));
        assert_eq!(
            e.warnings,
            vec![Warning::LedWithoutResistor { part: "D1".into() }]
        );
        // Active-low wiring: anode on 3V3, the GPIO sinking through the
        // cathode. Lit when the pin is low, and still no resistor.
        let mut s = sheet();
        place(&mut s, "D2", "lcsc:C2286");
        wire(&mut s, "U1.3V3", "D2.A");
        wire(&mut s, "D2.K", "U1.GPIO8");
        assert!(eval(&s, &[(8, false)], &[]).is_lit("D2"));
        assert!(!eval(&s, &[(8, true)], &[]).is_lit("D2"));
        assert_eq!(eval(&s, &[], &[]).warnings.len(), 1);
    }

    /// A pull-up button: GPIO9 — SW1 — GND. Pressing it puts ground on the
    /// pin, which is what the emulator must be driven to, and what
    /// `Input::is_low()` then reads.
    #[test]
    fn a_pressed_button_puts_its_rail_on_its_gpio() {
        let mut s = sheet();
        place(&mut s, "SW1", "Device:SW_Push");
        wire(&mut s, "U1.GPIO9", "SW1.1");
        wire(&mut s, "SW1.2", "U1.GND");
        let rows = rows();
        assert_eq!(button_drives(&s, &rows, "SW1"), Some((9, false)));
        let held = eval(&s, &[(9, true)], &["SW1"]);
        assert_eq!(
            held.level("SW1", "1"),
            Some(false),
            "ground wins over the pin's own report"
        );
        assert_eq!(held.level("U1", "GPIO9"), Some(false));
        let released = eval(&s, &[(9, true)], &[]);
        assert_eq!(released.level("SW1", "2"), Some(false));
        assert_eq!(
            released.level("SW1", "1"),
            Some(true),
            "the pin's own level again"
        );
        assert!(released.warnings.is_empty(), "{:?}", released.warnings);

        // To 3V3 instead: a press drives high.
        let mut s = sheet();
        place(&mut s, "SW2", "Device:SW_Push");
        wire(&mut s, "U1.GPIO4", "SW2.2");
        wire(&mut s, "SW2.1", "U1.3V3");
        assert_eq!(button_drives(&s, &rows, "SW2"), Some((4, true)));

        // One side unwired: nothing to drive, and the sheet says so.
        let mut s = sheet();
        place(&mut s, "SW3", "Device:SW_Push");
        wire(&mut s, "U1.GPIO4", "SW3.1");
        assert_eq!(button_drives(&s, &rows, "SW3"), None);
        assert_eq!(
            eval(&s, &[], &[]).warnings,
            vec![Warning::SwitchDrivesNothing { part: "SW3".into() }]
        );
    }

    #[test]
    fn rails_shorted_and_gpios_fighting_are_named_and_a_dangling_wire_too() {
        let mut s = sheet();
        place(&mut s, "R1", "Device:R");
        wire(&mut s, "U1.GND", "R1.1");
        wire(&mut s, "R1.2", "U1.3V3");
        let e = eval(&s, &[], &[]);
        assert!(
            matches!(&e.warnings[..], [Warning::Short { pins }] if pins.len() == 2),
            "{:?}",
            e.warnings
        );

        let mut s = sheet();
        wire(&mut s, "U1.GPIO2", "U1.GPIO3");
        let e = eval(&s, &[(2, true), (3, false)], &[]);
        assert!(
            matches!(&e.warnings[..], [Warning::Conflict { .. }]),
            "{:?}",
            e.warnings
        );
        assert!(eval(&s, &[(2, true), (3, true)], &[]).warnings.is_empty());

        let mut s = sheet();
        wire(&mut s, "U1.GPIO2", "D9.A");
        let e = eval(&s, &[], &[]);
        assert_eq!(
            e.warnings,
            vec![Warning::DanglingWire {
                from: "U1.GPIO2".into(),
                to: "D9.A".into()
            }]
        );
    }

    /// A rail drawn on the sheet is the rail: a lamp whose cathode goes to
    /// a ground symbol lights exactly as one wired all the way back to the
    /// devkit's own GND pin does. That is what the symbol is for — a wire
    /// across the whole sheet is what everybody draws it to avoid.
    #[test]
    fn a_power_symbol_is_a_rail_wherever_it_is_drawn() {
        let mut s = sheet();
        place(&mut s, "R1", "Device:R");
        place(&mut s, "D1", "Device:LED");
        place_with(&mut s, "GND1", "rusty:GND", "GND");
        wire(&mut s, "U1.GPIO2", "R1.1");
        wire(&mut s, "R1.2", "D1.A");
        wire(&mut s, "D1.K", "GND1.GND");

        let on = eval(&s, &[(2, true)], &[]);
        assert!(on.is_lit("D1"), "the ground symbol grounds the cathode");
        assert_eq!(on.level("D1", "K"), Some(false));
        assert!(on.warnings.is_empty(), "{:?}", on.warnings);
        assert!(!eval(&s, &[(2, false)], &[]).is_lit("D1"));

        // And the other way up: the anode on a supply symbol, the GPIO
        // sinking the cathode — a devkit's onboard lamp, drawn in full.
        let mut s = sheet();
        place(&mut s, "D2", "Device:LED");
        place_with(&mut s, "PWR1", "rusty:Supply", "3V3");
        wire(&mut s, "PWR1.VCC", "D2.A");
        wire(&mut s, "D2.K", "U1.GPIO8");
        assert!(eval(&s, &[(8, false)], &[]).is_lit("D2"));
        assert!(!eval(&s, &[(8, true)], &[]).is_lit("D2"));

        // A supply and a ground on one net is the short it always was,
        // whether the rails come from the devkit or from symbols.
        let mut s = sheet();
        place(&mut s, "R2", "Device:R");
        place_with(&mut s, "GND2", "rusty:GND", "GND");
        place_with(&mut s, "PWR2", "rusty:Supply", "5V");
        wire(&mut s, "GND2.GND", "R2.1");
        wire(&mut s, "R2.2", "PWR2.VCC");
        assert!(
            matches!(&eval(&s, &[], &[]).warnings[..], [Warning::Short { .. }]),
            "{:?}",
            eval(&s, &[], &[]).warnings
        );
    }

    /// Two labels carrying the same name are one net, however far apart
    /// they are drawn — and two different names are two nets, which is the
    /// half that makes the first mean anything.
    #[test]
    fn labels_of_the_same_name_are_one_net_and_other_names_are_not() {
        let mut s = sheet();
        place(&mut s, "R1", "Device:R");
        place(&mut s, "D1", "Device:LED");
        place_with(&mut s, "L1", "rusty:Label", "SIG");
        place_with(&mut s, "L2", "rusty:Label", "SIG");
        wire(&mut s, "U1.GPIO2", "L1.1");
        wire(&mut s, "L2.1", "R1.1");
        wire(&mut s, "R1.2", "D1.A");
        wire(&mut s, "D1.K", "U1.GND");

        let on = eval(&s, &[(2, true)], &[]);
        assert!(on.is_lit("D1"), "the name carries the pin across the sheet");
        assert_eq!(
            on.net_of(&PinRef::new("L1", "1")),
            on.net_of(&PinRef::new("L2", "1"))
        );

        // Rename one of them and the two halves are two nets again.
        if let Some(part) = s.parts.iter_mut().find(|p| p.reference == "L2") {
            part.value = "OTHER".to_string();
        }
        let split = eval(&s, &[(2, true)], &[]);
        assert!(!split.is_lit("D1"), "a different name is a different net");
        assert_ne!(
            split.net_of(&PinRef::new("L1", "1")),
            split.net_of(&PinRef::new("L2", "1"))
        );

        // A label with no name joins nothing — an empty tag is a tag
        // somebody has not written on yet, not a net called "".
        for part in s.parts.iter_mut() {
            if part.reference.starts_with('L') {
                part.value = String::new();
            }
        }
        assert!(!eval(&s, &[(2, true)], &[]).is_lit("D1"));
    }

    /// The nets a probe reads: every pin the current joins is one number,
    /// and what is not joined is not.
    /// A buzzer is on by the lamp's rule and off by it, and — unlike a
    /// lamp — wants no series resistor, so the sheet does not ask for one.
    #[test]
    fn a_buzzer_sounds_the_way_a_lamp_lights_and_needs_no_resistor() {
        let mut s = sheet();
        place(&mut s, "BZ1", "rusty:Buzzer");
        wire(&mut s, "U1.GPIO4", "BZ1.+");
        wire(&mut s, "BZ1.-", "U1.GND");

        let on = eval(&s, &[(4, true)], &[]);
        assert!(on.is_lit("BZ1"), "driven the right way round, it sounds");
        assert!(
            on.warnings.is_empty(),
            "a buzzer is not a lamp missing its resistor: {:?}",
            on.warnings
        );
        assert!(!eval(&s, &[(4, false)], &[]).is_lit("BZ1"));

        // The other way round it is silent, which is the finding.
        let mut s = sheet();
        place(&mut s, "BZ2", "rusty:Buzzer");
        wire(&mut s, "U1.GPIO4", "BZ2.-");
        wire(&mut s, "BZ2.+", "U1.GND");
        assert!(!eval(&s, &[(4, true)], &[]).is_lit("BZ2"));
    }

    #[test]
    fn a_net_holds_every_pin_the_current_reaches() {
        let mut s = sheet();
        place(&mut s, "R1", "Device:R");
        place(&mut s, "D1", "Device:LED");
        place(&mut s, "C1", "Device:C");
        wire(&mut s, "U1.GPIO2", "R1.1");
        wire(&mut s, "R1.2", "D1.A");
        wire(&mut s, "D1.K", "U1.GND");
        wire(&mut s, "C1.1", "D1.A");

        let e = eval(&s, &[(2, true)], &[]);
        let net = e
            .net_of(&PinRef::new("D1", "A"))
            .expect("the anode is in a net");
        let members: Vec<String> = e.members(net).iter().map(PinRef::to_string).collect();
        // Through the resistor, and through the wire to the capacitor's
        // near plate — but never through the capacitor itself.
        for wanted in ["D1.A", "R1.2", "R1.1", "C1.1", "U1.GPIO2"] {
            assert!(
                members.contains(&wanted.to_string()),
                "{wanted} in {members:?}"
            );
        }
        assert!(
            !members.contains(&"C1.2".to_string()),
            "a capacitor does not join its plates: {members:?}"
        );
        assert_ne!(
            e.net_of(&PinRef::new("D1", "K")),
            Some(net),
            "the cathode is the other side of the lamp"
        );
    }

    #[test]
    fn an_rgb_lens_lights_each_channel_against_its_common_and_a_pot_knows_its_gpio() {
        let mut s = sheet();
        place(&mut s, "D1", "rusty:RGB_LED");
        wire(&mut s, "U1.GPIO2", "D1.R");
        wire(&mut s, "U1.GPIO3", "D1.G");
        wire(&mut s, "D1.COM", "U1.3V3");
        let e = eval(&s, &[(2, false), (3, true)], &[]);
        assert!(
            e.is_pin_lit("D1", "1"),
            "common anode: a channel pulled low lights"
        );
        assert!(!e.is_pin_lit("D1", "2"));
        assert!(!e.is_pin_lit("D1", "3"), "unwired channel: dark");
        assert!(e.is_lit("D1"));

        let mut s = sheet();
        place(&mut s, "RV1", "rusty:Pot");
        place(&mut s, "R1", "Device:R");
        wire(&mut s, "RV1.W", "R1.1");
        wire(&mut s, "R1.2", "U1.GPIO4");
        let rows = rows();
        assert_eq!(
            gpio_of(&s, &rows, "RV1", "W"),
            Some(4),
            "through the resistor"
        );
        assert_eq!(gpio_of(&s, &rows, "RV1", "1"), None);
    }

    /// A part reaches the bus by carrying an address and by being wired to
    /// one, and by nothing else. Its kind does not decide: a sensor, a
    /// display and a breakout imported from LCSC all get there the same way.
    #[test]
    fn a_part_with_an_address_and_wires_is_a_device_on_the_bus() {
        let mut s = sheet();
        place_on_bus(&mut s, "U2", &[("addr", "68"), ("regs", "75=68,3b=0102")]);
        let (devices, warnings) = bus_devices(&s, &rows());
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].address, 0x68);
        assert_eq!(
            devices[0].regs,
            vec![(0x75, vec![0x68]), (0x3b, vec![0x01, 0x02])]
        );

        // 0x prefixed, and a device that answers zeros because nobody reads
        // from it — a display.
        let mut s = sheet();
        place_on_bus(&mut s, "U2", &[("addr", "0x3C")]);
        let (devices, warnings) = bus_devices(&s, &rows());
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(devices[0].address, 0x3c);
        assert!(devices[0].regs.is_empty());
    }

    /// No address is not an address of zero. A part without one is the part
    /// it always was and the bus never hears of it.
    #[test]
    fn a_part_without_an_address_is_not_on_the_bus() {
        let mut s = sheet();
        place_on_bus(&mut s, "U2", &[]);
        assert_eq!(bus_devices(&s, &rows()).0.len(), 0);

        let mut s = sheet();
        place_on_bus(&mut s, "U2", &[("addr", "  ")]);
        assert_eq!(bus_devices(&s, &rows()).0.len(), 0, "blank is absent");
    }

    /// Each refusal names the part, because the alternative is a device that
    /// silently is not there. The wiring one is the load-bearing case: the
    /// emulator's bus does not route through the GPIO matrix, so an
    /// unwired device would answer in the simulator and be dead on the desk.
    #[test]
    fn a_bus_device_that_cannot_be_read_is_named_rather_than_dropped() {
        let mut s = sheet();
        place_on_bus(&mut s, "U2", &[("addr", "zz")]);
        let (devices, warnings) = bus_devices(&s, &rows());
        assert!(devices.is_empty());
        assert!(
            matches!(&warnings[..], [Warning::BusAddressUnreadable { part, value }]
                     if part == "U2" && value == "zz"),
            "{warnings:?}"
        );

        let mut s = sheet();
        place_on_bus(&mut s, "U2", &[("addr", "90")]);
        assert!(
            matches!(
                &bus_devices(&s, &rows()).1[..],
                [Warning::BusAddressUnreadable { .. }]
            ),
            "0x90 is an eight-bit address written where a seven-bit one goes"
        );

        let mut s = sheet();
        place_on_bus(&mut s, "U2", &[("addr", "68"), ("regs", "75=6")]);
        let (devices, warnings) = bus_devices(&s, &rows());
        assert_eq!(devices.len(), 1, "it is still on the bus");
        assert!(devices[0].regs.is_empty(), "and answers zeros");
        assert!(
            matches!(&warnings[..], [Warning::BusRegistersUnreadable { .. }]),
            "half a byte is not a byte: {warnings:?}"
        );

        // Wired to nothing at all.
        let mut s = sheet();
        place(&mut s, "U2", "rusty:Sensor");
        if let Some(part) = s.parts.iter_mut().find(|p| p.reference == "U2") {
            part.props.insert("addr".into(), "68".into());
        }
        let (devices, warnings) = bus_devices(&s, &rows());
        assert!(devices.is_empty());
        assert!(
            matches!(&warnings[..], [Warning::BusNotWired { part }] if part == "U2"),
            "{warnings:?}"
        );

        // And wired on one side only, which is the mistake somebody actually
        // makes.
        let mut s = sheet();
        place(&mut s, "U2", "rusty:Sensor");
        wire(&mut s, "U2.1", "U1.GPIO5");
        if let Some(part) = s.parts.iter_mut().find(|p| p.reference == "U2") {
            part.props.insert("addr".into(), "68".into());
        }
        assert!(
            matches!(
                &bus_devices(&s, &rows()).1[..],
                [Warning::BusNotWired { .. }]
            ),
            "SDA alone is not a bus"
        );
    }
}
