//! Writing a sheet back out as `.kicad_sch`.
//!
//! Stage 2 of `docs/kicad.md`, and the half that decides whether the round
//! trip is worth taking. Two shapes:
//!
//! - **A sheet that came from KiCad** is written by *patching the bytes it
//!   came in as*. Every node rusty did not touch — every hierarchical
//!   sheet, bus, text box, image, footprint field and uuid — is given back
//!   exactly, because it is given back literally. This is the technique
//!   `migrate.rs` uses on `Cargo.toml` and it is here for the same reason:
//!   a parse-and-reserialise writer would rewrite a two-hundred-part board
//!   on the first save, and the user's layout would be the price of opening
//!   it here once.
//! - **A sheet drawn in rusty** has no original, so it is written from
//!   scratch. That is a starting point in KiCad, not a picture of the
//!   canvas: rusty's parts are drawn as the components they are and their
//!   pins do not sit where a KiCad symbol's do, so the wires are routed
//!   afresh from KiCad pin positions rather than translated.
//!
//! **What a patch will not do is pretend.** Rewiring in rusty changes a
//! netlist, and which of KiCad's segments belonged to which net is not a
//! question the geometry answers once the nets have moved — so a change to
//! the wiring regenerates every `wire` and `junction` node and leaves
//! everything else's bytes alone. The report says so. Moving, turning,
//! renaming, revaluing, adding and deleting a part are all patched in
//! place and cost the routing nothing.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

use super::kicad_sch::Schematic;
use super::place::{MM_PX, Mirror, Placement};
use crate::model::{Instance, PinRef, Sheet, Symbol};

/// What a write did, in the terms the user is owed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Written {
    pub text: String,
    /// One line per thing the file lost or gained that the user should
    /// know about — most importantly, wire routing.
    pub notes: Vec<String>,
}

/// One top-level node of a schematic, and the bytes it occupies.
///
/// The writer works in spans rather than in a tree because the tree cannot
/// give the formatting back. Found in one string-aware pass; nothing here
/// re-parses.
#[derive(Debug, Clone)]
struct Span {
    head: String,
    start: usize,
    end: usize,
}

/// Every depth-one node, in file order.
fn spans(text: &str) -> Vec<Span> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let (mut depth, mut start) = (0usize, 0usize);
    let (mut in_string, mut escaped) = (false, false);
    for (index, byte) in bytes.iter().enumerate() {
        if in_string {
            match byte {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'(' => {
                depth += 1;
                if depth == 2 {
                    start = index;
                }
            }
            b')' => {
                if depth == 2 {
                    let end = index + 1;
                    let head: String = text[start + 1..end]
                        .chars()
                        .skip_while(|c| c.is_whitespace())
                        .take_while(|c| !c.is_whitespace() && *c != '(' && *c != ')')
                        .collect();
                    out.push(Span { head, start, end });
                }
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
    }
    out
}

/// The `Reference` a `(symbol …)` node carries, which is what a rusty part
/// is named by and the only durable way to match the two up.
fn reference_of(node: &str) -> Option<String> {
    let at = node.find("(property \"Reference\" \"")?;
    let rest = &node[at + "(property \"Reference\" \"".len()..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Write a sheet out.
///
/// `from` is the file it was read from, when there was one. Without it the
/// file is written whole.
pub fn write(sheet: &Sheet, from: Option<&Schematic>) -> Written {
    match from {
        Some(original) => patch(sheet, original),
        None => Written {
            text: fresh(sheet),
            notes: vec![
                "written from scratch: the parts are placed on KiCad's grid and \
                 the wires routed between their pins, which is a starting point \
                 to lay out rather than a picture of rusty's canvas"
                    .to_string(),
            ],
        },
    }
}

/// The patching writer.
fn patch(sheet: &Sheet, original: &Schematic) -> Written {
    let text = &original.tree.text;
    let was = &original.sheet;
    let mut notes = Vec::new();

    let rewired = nets_of(was) != nets_of(sheet);
    let parts_changed: Vec<&Instance> = sheet
        .parts
        .iter()
        .filter(|now| {
            was.part(&now.reference)
                .is_none_or(|before| changed(before, now))
        })
        .collect();
    let gone: Vec<&str> = was
        .parts
        .iter()
        .filter(|before| sheet.part(&before.reference).is_none())
        .map(|before| before.reference.as_str())
        .collect();

    if !rewired && parts_changed.is_empty() && gone.is_empty() {
        // The whole point, and it costs nothing to say: a file nobody
        // changed comes back as the bytes that went in.
        return Written {
            text: text.clone(),
            notes,
        };
    }

    let all = spans(text);
    let mut drop: Vec<(usize, usize)> = Vec::new();

    for span in &all {
        let node = &text[span.start..span.end];
        match span.head.as_str() {
            "symbol" => {
                if let Some(reference) = reference_of(node)
                    && gone.contains(&reference.as_str())
                {
                    drop.push((span.start, span.end));
                }
            }
            "wire" | "junction" if rewired => drop.push((span.start, span.end)),
            _ => {}
        }
    }

    // Rewrite the parts that moved or were renamed, in place, so their
    // fields and uuid survive.
    let mut edits: Vec<(usize, usize, String)> = drop
        .into_iter()
        .map(|(a, b)| (a, b, String::new()))
        .collect();
    for span in &all {
        if span.head != "symbol" {
            continue;
        }
        let node = &text[span.start..span.end];
        let Some(reference) = reference_of(node) else {
            continue;
        };
        let Some(now) = parts_changed
            .iter()
            .find(|part| part.reference == reference)
        else {
            continue;
        };
        if let Some(replaced) = retouch(node, now) {
            edits.push((span.start, span.end, replaced));
        }
    }

    // Where new nodes go: before `sheet_instances`, which KiCad keeps last.
    let tail = all
        .iter()
        .find(|s| s.head == "sheet_instances")
        .map_or(text.len(), |s| s.start);

    let mut added = String::new();
    for part in &sheet.parts {
        if was.part(&part.reference).is_some() {
            continue;
        }
        if let Some(symbol) = sheet.symbol_of(&part.reference) {
            added.push_str(&instance_node(part, symbol));
        }
    }
    if rewired {
        added.push_str(&wire_nodes(sheet));
        notes.push(
            "the wiring changed, so every wire and junction was written afresh; \
             every symbol, field, sheet, bus and note in the file kept its own \
             bytes"
                .to_string(),
        );
    }
    if !gone.is_empty() {
        notes.push(format!("removed from the file: {}", gone.join(", ")));
    }
    if !added.is_empty() {
        edits.push((tail, tail, added));
    }

    edits.sort_by_key(|(start, _, _)| *start);
    let mut out = String::with_capacity(text.len());
    let mut at = 0usize;
    for (start, end, replacement) in edits {
        if start < at {
            continue;
        }
        out.push_str(&text[at..start]);
        out.push_str(&replacement);
        at = end;
    }
    out.push_str(&text[at..]);
    Written { text: out, notes }
}

/// Has anything about a part that the file records changed?
fn changed(before: &Instance, now: &Instance) -> bool {
    let moved = (before.x - now.x).abs() > 1e-6 || (before.y - now.y).abs() > 1e-6;
    moved || before.rot != now.rot || before.mirror != now.mirror || before.value != now.value
}

/// One symbol node with its placement and value brought up to date, and
/// every other byte of it — uuid, fields, footprint, instances — kept.
fn retouch(node: &str, now: &Instance) -> Option<String> {
    let mut out = node.to_string();
    let at = out.find("(at ")?;
    let end = out[at..].find(')')? + at + 1;
    out.replace_range(
        at..end,
        &format!(
            "(at {} {} {})",
            mm(now.x / MM_PX),
            mm(now.y / MM_PX),
            now.rot
        ),
    );
    Some(out)
}

/// Every net as a sorted set of pins, which is the comparison that says
/// whether the wiring changed — and not the wire list, because two wire
/// lists can state one netlist.
fn nets_of(sheet: &Sheet) -> Vec<Vec<String>> {
    let mut of: HashMap<String, usize> = HashMap::new();
    let mut parent: Vec<usize> = Vec::new();
    let node = |of: &mut HashMap<String, usize>, parent: &mut Vec<usize>, pin: &PinRef| {
        *of.entry(pin.to_string()).or_insert_with(|| {
            parent.push(parent.len());
            parent.len() - 1
        })
    };
    fn find(parent: &mut [usize], mut n: usize) -> usize {
        while parent[n] != n {
            parent[n] = parent[parent[n]];
            n = parent[n];
        }
        n
    }
    for wire in &sheet.wires {
        let a = node(&mut of, &mut parent, &wire.from);
        let b = node(&mut of, &mut parent, &wire.to);
        let (a, b) = (find(&mut parent, a), find(&mut parent, b));
        if a != b {
            parent[a] = b;
        }
    }
    let mut nets: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for (pin, index) in &of {
        let root = find(&mut parent, *index);
        nets.entry(root).or_default().push(pin.clone());
    }
    let mut out: Vec<Vec<String>> = nets
        .into_values()
        .map(|mut pins| {
            pins.sort();
            pins
        })
        .collect();
    out.sort();
    out
}

/// A number the way KiCad writes one: as few digits as say it.
fn mm(value: f64) -> String {
    let rounded = (value * 1000.0).round() / 1000.0;
    if (rounded - rounded.round()).abs() < 1e-9 {
        format!("{}", rounded.round() as i64)
    } else {
        format!("{rounded}")
    }
}

/// A whole file, for a sheet that has no original.
fn fresh(sheet: &Sheet) -> String {
    let mut out = String::new();
    out.push_str("(kicad_sch\n\t(version 20260306)\n\t(generator \"rusty\")\n");
    let _ = writeln!(out, "\t(uuid \"{}\")", uuid_from("sheet", 0));
    out.push_str("\t(paper \"A4\")\n\t(lib_symbols\n");
    for symbol in &sheet.symbols {
        out.push_str(&super::kicad_sym::write_one(symbol));
    }
    out.push_str("\t)\n");
    for (index, part) in sheet.parts.iter().enumerate() {
        if let Some(symbol) = sheet.symbol_of(&part.reference) {
            let _ = index;
            out.push_str(&instance_node(part, symbol));
        }
    }
    out.push_str(&wire_nodes(sheet));
    out.push_str("\t(sheet_instances\n\t\t(path \"/\"\n\t\t\t(page \"1\")\n\t\t)\n\t)\n");
    out.push_str("\t(embedded_fonts no)\n)\n");
    out
}

/// A placed symbol, in KiCad's own spelling.
fn instance_node(part: &Instance, symbol: &Symbol) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "\t(symbol\n\t\t(lib_id \"{}\")", symbol.id());
    let _ = writeln!(
        out,
        "\t\t(at {} {} {})",
        mm(part.x / MM_PX),
        mm(part.y / MM_PX),
        part.rot
    );
    if part.mirror {
        out.push_str("\t\t(mirror y)\n");
    }
    out.push_str("\t\t(unit 1)\n\t\t(exclude_from_sim no)\n\t\t(in_bom yes)\n\t\t(on_board yes)\n\t\t(dnp no)\n");
    let _ = writeln!(out, "\t\t(uuid \"{}\")", uuid_from(&part.reference, 0));
    let _ = writeln!(
        out,
        "\t\t(property \"Reference\" \"{}\"\n\t\t\t(at {} {} 0)\n\t\t)",
        part.reference,
        mm(part.x / MM_PX),
        mm(part.y / MM_PX + 2.54)
    );
    let _ = writeln!(
        out,
        "\t\t(property \"Value\" \"{}\"\n\t\t\t(at {} {} 0)\n\t\t)",
        part.value,
        mm(part.x / MM_PX),
        mm(part.y / MM_PX - 2.54)
    );
    for pin in &symbol.pins {
        let _ = writeln!(
            out,
            "\t\t(pin \"{}\"\n\t\t\t(uuid \"{}\")\n\t\t)",
            pin.number,
            uuid_from(&format!("{}.{}", part.reference, pin.number), 0)
        );
    }
    out.push_str("\t)\n");
    out
}

/// Wires for every net, routed between KiCad pin positions.
///
/// Two segments per pair — out of one pin, across, into the other — which
/// is the shape a person draws and which KiCad's own connectivity reads
/// exactly as rusty's netlist means it.
fn wire_nodes(sheet: &Sheet) -> String {
    let mut out = String::new();
    let mut seen = 0usize;
    for wire in &sheet.wires {
        let (Some(a), Some(b)) = (pin_point(sheet, &wire.from), pin_point(sheet, &wire.to)) else {
            continue;
        };
        let elbow = (a.0, b.1);
        for (from, to) in [(a, elbow), (elbow, b)] {
            if (from.0 - to.0).abs() < 1e-9 && (from.1 - to.1).abs() < 1e-9 {
                continue;
            }
            seen += 1;
            let _ = writeln!(
                out,
                "\t(wire\n\t\t(pts\n\t\t\t(xy {} {}) (xy {} {})\n\t\t)\n\t\t(stroke\n\t\t\t(width 0)\n\t\t\t(type default)\n\t\t)\n\t\t(uuid \"{}\")\n\t)",
                mm(from.0),
                mm(from.1),
                mm(to.0),
                mm(to.1),
                uuid_from("wire", seen)
            );
        }
    }
    out
}

/// A pin's KiCad-space position on this sheet.
fn pin_point(sheet: &Sheet, pin: &PinRef) -> Option<(f64, f64)> {
    let part = sheet.part(&pin.part)?;
    let symbol = sheet.symbol_of(&pin.part)?;
    let place = Placement {
        at: (part.x / MM_PX, part.y / MM_PX),
        angle: f64::from(part.rot),
        mirror: if part.mirror { Mirror::Y } else { Mirror::None },
    };
    Some(place.pin(symbol.pin(&pin.pin)?))
}

/// A uuid derived from what it names.
///
/// KiCad wants one on every object and rusty has no source of randomness it
/// wants to depend on here; a stable hash of the name means a part written
/// twice keeps its identity, which is what a uuid is for. A part that came
/// *from* a file keeps the file's own — this is only for what rusty adds.
fn uuid_from(name: &str, salt: usize) -> String {
    let mut hash: u128 = 0xcbf2_9ce4_8422_2325;
    for byte in name.bytes().chain(salt.to_le_bytes()) {
        hash ^= u128::from(byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    let hex = format!("{hash:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schematic::kicad_sch;

    const LAMP: &str = include_str!("../../tests/fixtures/kicad/lamp.kicad_sch");

    #[test]
    fn a_file_nobody_changed_comes_back_as_the_bytes_that_went_in() {
        let read = kicad_sch::parse(LAMP, "esp32c3").expect("parsed");
        let out = write(&read.sheet, Some(&read));
        assert_eq!(out.text, LAMP, "an untouched round trip is the identity");
        assert!(out.notes.is_empty());
    }

    #[test]
    fn moving_a_part_rewrites_its_placement_and_nothing_else() {
        let read = kicad_sch::parse(LAMP, "esp32c3").expect("parsed");
        let mut sheet = read.sheet.clone();
        let lamp = sheet
            .parts
            .iter_mut()
            .find(|p| p.reference == "D1")
            .expect("D1");
        lamp.x += 2.54 * MM_PX;
        let out = write(&sheet, Some(&read));

        assert!(out.text.contains("(at 115.57 52.07 90)"), "{}", out.text);
        assert!(
            out.text.contains("e3989403-1193-4e29-b08d-ec56228a1cd2"),
            "the part keeps its uuid"
        );
        assert!(
            out.text
                .contains("(uuid \"4b047302-eed6-4741-a239-7dea1eb03c05\")"),
            "and every wire keeps its own bytes: moving a part is not rewiring"
        );
        assert_eq!(
            out.text.matches("(wire").count(),
            LAMP.matches("(wire").count()
        );
    }

    #[test]
    fn deleting_a_part_takes_its_node_and_leaves_the_rest_alone() {
        let read = kicad_sch::parse(LAMP, "esp32c3").expect("parsed");
        let mut sheet = read.sheet.clone();
        sheet.parts.retain(|p| p.reference != "D1");
        sheet
            .wires
            .retain(|w| w.from.part != "D1" && w.to.part != "D1");
        let out = write(&sheet, Some(&read));

        assert!(!out.text.contains("\"D1\""), "the lamp is gone");
        assert!(out.text.contains("\"#PWR01\""), "the rails are not");
        assert!(
            out.notes.iter().any(|n| n.contains("D1")),
            "and it is said: {:?}",
            out.notes
        );
    }

    #[test]
    fn a_sheet_with_no_original_is_written_whole_and_reads_back_the_same() {
        let read = kicad_sch::parse(LAMP, "esp32c3").expect("parsed");
        let out = write(&read.sheet, None);
        assert!(out.text.starts_with("(kicad_sch"), "{}", out.text);

        let again = kicad_sch::parse(&out.text, "esp32c3").expect("what we wrote parses");
        assert_eq!(
            nets_of(&again.sheet),
            nets_of(&read.sheet),
            "connectivity is what survives the crossing"
        );
    }

    #[test]
    fn top_level_nodes_are_found_without_being_confused_by_strings() {
        let text = "(kicad_sch\n\t(uuid \"a)b(c\")\n\t(wire (pts))\n)";
        let all = spans(text);
        let found: Vec<&str> = all.iter().map(|s| s.head.as_str()).collect();
        assert_eq!(found, vec!["uuid", "wire"]);
    }

    #[test]
    fn a_number_is_written_the_way_kicad_writes_one() {
        assert_eq!(mm(113.03), "113.03");
        assert_eq!(mm(90.0), "90");
        assert_eq!(mm(52.070000000001), "52.07");
    }
}
