//! KiCad's schematic file, `.kicad_sch` — read into a [`Sheet`].
//!
//! Stage 1 of `docs/kicad.md`. The syntax is `super::sexpr`, shared with the
//! symbol library; everything here is the vocabulary and the one thing that
//! is genuinely different — **connectivity is geometric**. KiCad's wires are
//! segments and what is joined to what is read off the coordinates: two
//! endpoints at the same point, a pin lying on a wire, a junction where
//! wires cross, labels that share a name. rusty's wires are pin to pin, so
//! this reads KiCad's rules and states the answer in rusty's terms.
//!
//! What that costs is the author's routing, and what it buys is that
//! nothing else has to change — the sheet, the canvas and `.rusty/sim.toml`
//! are all as they were. The routing is not lost from the *file*, because
//! the writer patches the tree this returns rather than rewriting it; it is
//! lost only from rusty's own picture of the sheet, which draws parts as
//! the components they are and could not have honoured it anyway.
//!
//! Read, never trusted. A node this reader does not know stays in `tree`
//! and is skipped, so the writer gives it back untouched: hierarchical
//! sheets, buses, text, images and every field rusty has no use for.

use std::collections::HashMap;

use super::kicad_sym;
use super::place::{MM_PX, Mirror, Placement};
use super::sexpr::{ParseError, Sx, read};
use crate::model::{Instance, PinRef, Sheet, Symbol, Wire};

/// The file as it was read, kept whole and opaque.
///
/// The only thing a caller does with one is hand it back to the writer,
/// which is the point: what rusty understood is in [`Schematic::sheet`],
/// and what it did not is in here, unexamined and undamaged.
///
/// It keeps the **text**, not only the parsed tree, and that is the whole
/// of how the round trip stays honest. Reserialising a tree cannot give
/// back what came in — KiCad writes tabs, puts small nodes inline and long
/// ones one per line, and writes `0` where a parser only knows `0.0` — so a
/// writer that reprinted everything would rewrite a two-hundred-part board
/// on the first save. Patching the bytes gives every untouched node back
/// exactly.
#[derive(Debug, Clone)]
pub struct Tree {
    pub(crate) text: String,
}

/// A schematic read from a file.
#[derive(Debug, Clone)]
pub struct Schematic {
    /// What rusty draws and simulates.
    pub sheet: Sheet,
    /// The tree the file was read as, whole. The writer patches this; a
    /// node nobody understood is a node nobody may delete.
    pub tree: Tree,
    /// What could not be brought across, in the user's terms.
    pub notes: Vec<String>,
}

/// A KiCad ground symbol by name. Everything else under `power:` is a
/// supply — the same reading `power_rail` does for rusty's own two, and for
/// the same reason: the rules are on and off, so which supply it is does not
/// change an answer, but ground against supply does.
const GROUNDS: &[&str] = &["GND", "GNDA", "GNDD", "GNDS", "GNDREF", "GNDPWR", "Earth"];

/// Is this KiCad power symbol a ground?
pub fn is_ground(name: &str) -> bool {
    GROUNDS.contains(&name)
}

/// A point on the sheet, rounded to something two floats can agree on.
///
/// Schematic coordinates are a grid of mils written as millimetres, so the
/// file's numbers are exact and their binary forms are not. A thousandth of
/// a millimetre is far below anything KiCad will place and far above the
/// error of parsing `113.03`.
type Grid = (i64, i64);

fn grid(at: (f64, f64)) -> Grid {
    (
        (at.0 * 1000.0).round() as i64,
        (at.1 * 1000.0).round() as i64,
    )
}

/// One wire segment as the file writes it: two points, and nothing else
/// this reader needs.
struct Segment {
    a: (f64, f64),
    b: (f64, f64),
}

impl Segment {
    /// Does `p` lie on this segment — either end, or anywhere along it?
    ///
    /// The interior matters: KiCad connects a pin that sits *on* a wire,
    /// not only one a wire ends at, and a reader that only matched
    /// endpoints would quietly drop every part somebody wired by running a
    /// line across its pins.
    fn holds(&self, p: (f64, f64)) -> bool {
        const CLOSE: f64 = 0.0025;
        let (dx, dy) = (self.b.0 - self.a.0, self.b.1 - self.a.1);
        let length = dx.hypot(dy);
        if length < CLOSE {
            return (p.0 - self.a.0).hypot(p.1 - self.a.1) < CLOSE;
        }
        // Distance from the line, then whether the foot is between the ends.
        let cross = ((p.0 - self.a.0) * dy - (p.1 - self.a.1) * dx) / length;
        if cross.abs() > CLOSE {
            return false;
        }
        let along = ((p.0 - self.a.0) * dx + (p.1 - self.a.1) * dy) / length;
        (-CLOSE..=length + CLOSE).contains(&along)
    }
}

/// The union-find every net derivation here runs on, over grid points.
#[derive(Default)]
struct Points {
    of: HashMap<Grid, usize>,
    parent: Vec<usize>,
}

impl Points {
    fn at(&mut self, p: (f64, f64)) -> usize {
        let key = grid(p);
        if let Some(index) = self.of.get(&key) {
            return *index;
        }
        let index = self.parent.len();
        self.parent.push(index);
        self.of.insert(key, index);
        index
    }

    fn find(&mut self, mut node: usize) -> usize {
        while self.parent[node] != node {
            self.parent[node] = self.parent[self.parent[node]];
            node = self.parent[node];
        }
        node
    }

    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.parent[a] = b;
        }
    }
}

/// Read a schematic.
///
/// `chip` is what the sheet is drawn for — the file says nothing about a
/// microcontroller rusty would simulate, and the project's own chip is the
/// only honest answer.
pub fn parse(text: &str, chip: &str) -> Result<Schematic, ParseError> {
    let tree = read(text)?;
    let root = tree
        .items()
        .iter()
        .find(|item| item.head() == Some("kicad_sch"))
        .ok_or_else(|| ParseError {
            detail: "not a KiCad schematic: no `kicad_sch` at the top".to_string(),
        })?;
    let mut notes = Vec::new();

    // ── the symbols the file carries ────────────────────────────────────
    // A schematic embeds a copy of every symbol it places, which is what
    // makes an imported board self-contained: no library has to be found.
    let mut library: HashMap<String, Vec<Symbol>> = HashMap::new();
    for node in root
        .child("lib_symbols")
        .iter()
        .flat_map(|n| n.children("symbol"))
    {
        let Some(full) = node.text(1) else { continue };
        let (lib, name) = full.split_once(':').unwrap_or(("", full));
        match kicad_sym::units(lib, name, node) {
            Ok(units) => {
                library.insert(full.to_string(), units);
            }
            Err(error) => notes.push(format!("{full} could not be read: {error}")),
        }
    }

    // ── what is drawn on the sheet ──────────────────────────────────────
    let segments: Vec<Segment> = root
        .children("wire")
        .filter_map(|w| {
            let pts = w.child("pts")?;
            let mut ends = pts.children("xy").filter_map(point);
            Some(Segment {
                a: ends.next()?,
                b: ends.next()?,
            })
        })
        .collect();

    let junctions: Vec<(f64, f64)> = root.children("junction").filter_map(at).collect();
    let labels: Vec<(String, (f64, f64))> = root
        .children("label")
        .chain(root.children("global_label"))
        .chain(root.children("hierarchical_label"))
        .filter_map(|node| Some((node.text(1)?.to_string(), at(node)?)))
        .collect();

    // ── the parts, and where each pin landed ────────────────────────────
    let mut sheet = Sheet::empty(chip);
    let mut placed: Vec<(PinRef, (f64, f64))> = Vec::new();
    let mut used: Vec<Symbol> = Vec::new();

    for node in root.children("symbol") {
        let Some(lib_id) = node.child("lib_id").and_then(|n| n.text(1)) else {
            continue;
        };
        let Some(at_node) = node.child("at") else {
            continue;
        };
        let place = Placement {
            at: (
                at_node.number(1).unwrap_or(0.0),
                at_node.number(2).unwrap_or(0.0),
            ),
            angle: at_node.number(3).unwrap_or(0.0),
            mirror: node
                .child("mirror")
                .and_then(|n| n.text(1))
                .map(Mirror::read)
                .unwrap_or_default(),
        };
        let unit = node
            .child("unit")
            .and_then(|n| n.number(1))
            .unwrap_or(1.0)
            .max(1.0) as usize;
        let Some(symbol) = library
            .get(lib_id)
            .and_then(|units| units.get(unit - 1).or_else(|| units.first()))
        else {
            notes.push(format!(
                "{lib_id} is placed but the file carries no symbol for it, so it \
                 is left off the sheet"
            ));
            continue;
        };

        let reference = property(node, "Reference").unwrap_or_else(|| symbol.reference.clone());
        // A power symbol's *value* is its net's name, which is the thing
        // that matters about it; everything else keeps whatever the sheet
        // wrote beside it.
        let value = property(node, "Value").unwrap_or_default();

        for (number, point) in place.pins(symbol) {
            placed.push((PinRef::new(&reference, number), point));
        }
        if !used.iter().any(|s| s.id() == symbol.id()) {
            used.push(symbol.clone());
        }
        sheet.parts.push(Instance {
            reference,
            symbol: symbol.id(),
            value,
            // Millimetres into the sheet's own units, through the one
            // scale both sides use. The anchor is the one point that means
            // the same thing in both spaces -- the pins do not, because the
            // canvas draws parts as the components they are -- so this is
            // the whole of the position that crosses, and it crosses back
            // by the same constant.
            x: place.at.0 * MM_PX,
            y: place.at.1 * MM_PX,
            rot: (((place.angle as i64 % 360) + 360) % 360) as u16,
            mirror: place.mirror != Mirror::None,
            props: Default::default(),
        });
    }

    // A label is not a symbol in KiCad, so it becomes one here — rusty
    // joins nets by `rusty:Label` and this is the same idea spelled the way
    // this sheet spells it.
    for (index, (name, point)) in labels.iter().enumerate() {
        let reference = format!("#LBL{}", index + 1);
        sheet.parts.push(Instance {
            reference: reference.clone(),
            symbol: "rusty:Label".to_string(),
            value: name.clone(),
            x: point.0 * MM_PX,
            y: point.1 * MM_PX,
            rot: 0,
            mirror: false,
            props: Default::default(),
        });
        placed.push((PinRef::new(&reference, "1"), *point));
    }

    sheet.symbols = used;

    // ── KiCad's connectivity, read off the coordinates ──────────────────
    let mut points = Points::default();
    // A segment joins its own two ends. Two segments that end at the same
    // point are the same node already, because the point is the key.
    for segment in &segments {
        let (a, b) = (points.at(segment.a), points.at(segment.b));
        points.union(a, b);
    }
    // A junction joins every segment through it — which is what makes two
    // crossing wires with a junction one net, and two without it two nets.
    for spot in &junctions {
        let here = points.at(*spot);
        for segment in &segments {
            if segment.holds(*spot) {
                let end = points.at(segment.a);
                points.union(here, end);
            }
        }
    }
    // A pin joins any wire it lies on, at the end or in the middle.
    for (_, point) in &placed {
        let here = points.at(*point);
        for segment in &segments {
            if segment.holds(*point) {
                let end = points.at(segment.a);
                points.union(here, end);
            }
        }
    }
    // And labels of one name are one net wherever they are drawn.
    let mut by_name: HashMap<&str, usize> = HashMap::new();
    for (name, point) in &labels {
        let here = points.at(*point);
        match by_name.get(name.as_str()) {
            Some(first) => {
                let first = *first;
                points.union(first, here);
            }
            None => {
                by_name.insert(name.as_str(), here);
            }
        }
    }

    // ── the nets, stated as rusty's pin-to-pin wires ────────────────────
    let mut nets: HashMap<usize, Vec<PinRef>> = HashMap::new();
    for (pin, point) in &placed {
        let node = points.at(*point);
        let root = points.find(node);
        nets.entry(root).or_default().push(pin.clone());
    }
    let mut roots: Vec<usize> = nets.keys().copied().collect();
    roots.sort_unstable();
    for root in roots {
        let members = &nets[&root];
        // A star from the first pin: the connectivity is what survives the
        // crossing, and any spanning shape states it. The geometry is the
        // canvas's to lay out, and the file's own is patched back untouched.
        for pin in members.iter().skip(1) {
            sheet.wires.push(Wire {
                from: members[0].clone(),
                to: pin.clone(),
                bends: Vec::new(),
            });
        }
    }

    for kind in ["sheet", "bus", "bus_entry", "text", "text_box", "image"] {
        let count = root.children(kind).count();
        if count > 0 {
            notes.push(format!(
                "{count} `{kind}` in the file are kept but not drawn here; they \
                 come back untouched on the way out"
            ));
        }
    }

    Ok(Schematic {
        sheet,
        tree: Tree {
            text: text.to_string(),
        },
        notes,
    })
}

/// A `(at x y ...)` child's point.
fn at(node: &Sx) -> Option<(f64, f64)> {
    let at = node.child("at")?;
    Some((at.number(1)?, at.number(2)?))
}

/// An `(xy x y)` node's point.
fn point(node: &Sx) -> Option<(f64, f64)> {
    Some((node.number(1)?, node.number(2)?))
}

/// A named property's value.
fn property(node: &Sx, key: &str) -> Option<String> {
    node.children("property")
        .find(|p| p.text(1) == Some(key))
        .and_then(|p| p.text(2))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A lamp turned 90° between a supply and a ground, joined by two
    /// wires — the shape that settled the rotation, written here rather
    /// than copied from the board it was measured against, because that
    /// board is somebody's and this repository is public.
    const LAMP: &str = include_str!("../../tests/fixtures/kicad/lamp.kicad_sch");

    fn joined(sheet: &Sheet, a: &str, b: &str) -> bool {
        let (a, b) = (PinRef::parse(a).unwrap(), PinRef::parse(b).unwrap());
        sheet
            .wires
            .iter()
            .any(|w| (w.from == a && w.to == b) || (w.from == b && w.to == a))
    }

    #[test]
    fn a_lamp_between_two_rails_comes_across_the_right_way_round() {
        let read = parse(LAMP, "esp32c3").expect("parsed");
        assert!(read.notes.is_empty(), "{:?}", read.notes);

        let references: Vec<&str> = read
            .sheet
            .parts
            .iter()
            .map(|p| p.reference.as_str())
            .collect();
        assert_eq!(references, vec!["#PWR01", "D1", "#PWR02"]);
        assert_eq!(read.sheet.part("D1").unwrap().symbol, "Device:LED");
        assert_eq!(read.sheet.part("D1").unwrap().rot, 90);

        // The whole of the crossing, in one assertion: the cathode is on
        // the ground side and the anode on the supply, which is what KiCad
        // draws and what the rotation constant exists to get right.
        assert!(
            joined(&read.sheet, "D1.1", "#PWR01.1"),
            "the cathode reaches ground: {:?}",
            read.sheet.wires
        );
        assert!(
            joined(&read.sheet, "D1.2", "#PWR02.1"),
            "and the anode the supply: {:?}",
            read.sheet.wires
        );
        assert!(
            !joined(&read.sheet, "D1.1", "#PWR02.1"),
            "and the two rails are not one net"
        );
    }

    #[test]
    fn a_pin_lying_on_a_wire_is_on_its_net_and_a_crossing_is_not() {
        let segment = Segment {
            a: (0.0, 0.0),
            b: (0.0, 10.0),
        };
        assert!(segment.holds((0.0, 0.0)), "an end");
        assert!(segment.holds((0.0, 10.0)), "the other end");
        assert!(segment.holds((0.0, 5.0)), "the middle, which KiCad joins");
        assert!(!segment.holds((0.0, 10.5)), "past the end");
        assert!(!segment.holds((1.0, 5.0)), "beside it");
    }

    #[test]
    fn a_file_that_is_not_a_schematic_is_refused_by_name() {
        let error = parse("(kicad_symbol_lib (version 20211014))", "esp32c3").expect_err("refused");
        assert!(error.detail.contains("kicad_sch"), "{}", error.detail);
    }

    #[test]
    fn kicads_grounds_are_told_from_its_supplies_by_name() {
        assert!(is_ground("GND"));
        assert!(is_ground("GNDA"));
        assert!(is_ground("Earth"));
        assert!(!is_ground("VCC"));
        assert!(!is_ground("+3V3"));
    }
}
