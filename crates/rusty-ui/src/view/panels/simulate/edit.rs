//! What the board editor's commands do to the sheet, with no signals in them.
//!
//! The sibling of [`super::geometry`], and there for the same reason: the
//! canvas is a two-thousand-line component, and everything inside it that
//! is really arithmetic or bookkeeping was unreachable from a test. What
//! rotating, deleting, duplicating, wiring and undoing actually *do* to the
//! part list and the wire list is pinned here.
//!
//! Every function here takes the lists and mutates them. The component
//! keeps the signals, calls these, and sets `dirty` — so a command is one
//! line there and its behaviour is here.

use rusty_embed::{Instance, KIT_REFERENCE, PinRef, Symbol, Wire};

use super::geometry::{EditPart, GroupStart, Snapshot, pin_key};

/// How many steps of undo the editor keeps.
///
/// Bounded because each snapshot is a whole copy of the sheet and a long
/// session would otherwise grow without limit; sixty-four is more than a
/// hand undoes in one go and far less than a browser tab minds.
const HISTORY_CAP: usize = 64;

/// Push a snapshot, dropping the oldest once the cap is reached.
pub(super) fn remember(past: &mut Vec<Snapshot>, now: Snapshot) {
    if past.len() >= HISTORY_CAP {
        past.remove(0);
    }
    past.push(now);
}

/// The next free `prefix<n>`: `D3` when `D1` and `D2` are placed. A deleted
/// `D2` is reused, as KiCad's annotation does. Never the devkit's `U1`.
pub(super) fn next_reference(list: &[EditPart], prefix: &str) -> String {
    let prefix = prefix.trim_end_matches(['?', '_']);
    let prefix = if prefix.is_empty() { "U" } else { prefix };
    (1..)
        .map(|n| format!("{prefix}{n}"))
        .find(|candidate| {
            *candidate != KIT_REFERENCE && list.iter().all(|p| p.inst.reference != *candidate)
        })
        .expect("the integers do not run out")
}

/// Place a symbol at `(x, y)`, numbered after the parts already there. Its
/// value starts as the symbol's own unless that is just the symbol's name
/// — `LED` beside an LED says nothing, `10kΩ` beside an imported resistor
/// says everything. Returns the new part's index.
pub(super) fn add(list: &mut Vec<EditPart>, symbol: &Symbol, x: f64, y: f64) -> usize {
    let reference = next_reference(list, &symbol.reference);
    let value = if symbol.value == symbol.name {
        String::new()
    } else {
        symbol.value.clone()
    };
    list.push(EditPart {
        inst: Instance {
            reference,
            symbol: symbol.id(),
            value,
            x,
            y,
            rot: 0,
            mirror: false,
            props: Default::default(),
        },
        symbol: Some(symbol.clone()),
    });
    list.len() - 1
}

/// Rename a part, and every wire end that named it. Refused — `false` —
/// for a blank name, the devkit's, or one another part already wears;
/// a sheet with two `D1`s is a sheet whose wires no longer say which.
pub(super) fn rename(list: &mut [EditPart], wires: &mut [Wire], index: usize, name: &str) -> bool {
    let name = name.trim();
    if name.is_empty() || name == KIT_REFERENCE {
        return false;
    }
    if list
        .iter()
        .enumerate()
        .any(|(i, p)| i != index && p.inst.reference == name)
    {
        return false;
    }
    let Some(part) = list.get_mut(index) else {
        return false;
    };
    if part.is_kit() {
        return false;
    }
    let old = std::mem::replace(&mut part.inst.reference, name.to_string());
    for wire in wires {
        if wire.from.part == old {
            wire.from.part = name.to_string();
        }
        if wire.to.part == old {
            wire.to.part = name.to_string();
        }
    }
    true
}

pub(super) fn set_value(list: &mut [EditPart], index: usize, value: &str) {
    if let Some(part) = list.get_mut(index) {
        part.inst.value = value.trim().to_string();
    }
}

pub(super) fn set_prop(list: &mut [EditPart], index: usize, key: &str, value: &str) {
    if let Some(part) = list.get_mut(index) {
        if value.trim().is_empty() {
            part.inst.props.remove(key);
        } else {
            part.inst
                .props
                .insert(key.to_string(), value.trim().to_string());
        }
    }
}

/// A quarter turn clockwise. The devkit does not turn: its art is drawn
/// upright and its header reads that way.
pub(super) fn rotate(list: &mut [EditPart], index: usize) {
    if let Some(part) = list.get_mut(index).filter(|p| !p.is_kit()) {
        part.inst.rot = (part.inst.rot + 90) % 360;
    }
}

/// Mirror left-to-right — KiCad's X key. Not a second rotation: a part
/// on the chip's right wants its pins on the near edge *in the same
/// order*, and turning it 180° reverses them.
pub(super) fn mirror(list: &mut [EditPart], index: usize) {
    if let Some(part) = list.get_mut(index).filter(|p| !p.is_kit()) {
        part.inst.mirror = !part.inst.mirror;
    }
}

pub(super) fn nudge(list: &mut [EditPart], index: usize, dx: f64, dy: f64) {
    if let Some(part) = list.get_mut(index) {
        part.inst.x += dx;
        part.inst.y += dy;
    }
}

/// Move every part in `start` by one displacement from where it stood
/// when the drag began — never by a delta from the last frame, which
/// accumulates snapping into drift.
pub(super) fn translate(list: &mut [EditPart], start: &[GroupStart], dx: f64, dy: f64) {
    for (index, (x, y)) in start {
        if let Some(part) = list.get_mut(*index) {
            part.inst.x = x + dx;
            part.inst.y = y + dy;
        }
    }
}

/// Remove a part and every wire that touched it. The devkit stays.
pub(super) fn remove(list: &mut Vec<EditPart>, wires: &mut Vec<Wire>, index: usize) -> bool {
    if index >= list.len() || list[index].is_kit() {
        return false;
    }
    let reference = list.remove(index).inst.reference;
    wires.retain(|w| w.from.part != reference && w.to.part != reference);
    true
}

/// Remove several parts at once — Delete on a rubber-band selection.
/// Highest index first, so each removal leaves the rest where they were.
pub(super) fn remove_many(list: &mut Vec<EditPart>, wires: &mut Vec<Wire>, indices: &[usize]) {
    let mut order: Vec<usize> = indices.to_vec();
    order.sort_unstable();
    order.dedup();
    for index in order.into_iter().rev() {
        remove(list, wires, index);
    }
}

/// A copy beside the original, numbered afresh, with no wires: wires are
/// connections somebody made, and a copy of a connection is a short.
pub(super) fn duplicate(list: &mut Vec<EditPart>, index: usize) -> Option<usize> {
    let original = list.get(index).filter(|p| !p.is_kit())?.clone();
    let prefix: String = original
        .inst
        .reference
        .trim_end_matches(|c: char| c.is_ascii_digit())
        .to_string();
    let reference = next_reference(list, &prefix);
    list.push(EditPart {
        inst: Instance {
            reference,
            x: original.inst.x + 16.0,
            y: original.inst.y + 16.0,
            ..original.inst
        },
        symbol: original.symbol,
    });
    Some(list.len() - 1)
}

/// The pin's spelling in a wire, from its part's symbol.
fn key_of(part: &EditPart, number: &str) -> Option<PinRef> {
    let symbol = part.symbol.as_ref()?;
    let pin = symbol.pin(number)?;
    Some(PinRef::new(&part.inst.reference, pin_key(symbol, pin)))
}

/// Whether a wire end names this pin, whichever spelling it used.
pub(super) fn end_is(list: &[EditPart], end: &PinRef, index: usize, number: &str) -> bool {
    let Some(part) = list.get(index) else {
        return false;
    };
    if end.part != part.inst.reference {
        return false;
    }
    part.pin(&end.pin).is_some_and(|p| p.number == number)
}

/// Every wire touching a pin.
pub(super) fn wires_at(
    list: &[EditPart],
    wires: &[Wire],
    index: usize,
    number: &str,
) -> Vec<usize> {
    wires
        .iter()
        .enumerate()
        .filter(|(_, w)| end_is(list, &w.from, index, number) || end_is(list, &w.to, index, number))
        .map(|(i, _)| i)
        .collect()
}

/// Join two pins. Refused — `None` — for a pin to itself, and for a pair
/// already joined; both are wires that mean nothing. Returns the index of
/// the wire made.
pub(super) fn connect(
    list: &[EditPart],
    wires: &mut Vec<Wire>,
    from: (usize, &str),
    to: (usize, &str),
) -> Option<usize> {
    if from == to {
        return None;
    }
    let a = key_of(list.get(from.0)?, from.1)?;
    let b = key_of(list.get(to.0)?, to.1)?;
    if wires
        .iter()
        .any(|w| (w.from == a && w.to == b) || (w.from == b && w.to == a))
    {
        return None;
    }
    wires.push(Wire {
        from: a,
        to: b,
        bends: Vec::new(),
    });
    Some(wires.len() - 1)
}

/// Drop every wire at a pin.
pub(super) fn disconnect_pin(list: &[EditPart], wires: &mut Vec<Wire>, index: usize, number: &str) {
    let doomed = wires_at(list, wires, index, number);
    for at in doomed.into_iter().rev() {
        wires.remove(at);
    }
}

/// Drop every wire touching a part.
pub(super) fn disconnect_all(list: &[EditPart], wires: &mut Vec<Wire>, index: usize) {
    let Some(part) = list.get(index) else {
        return;
    };
    let reference = part.inst.reference.clone();
    wires.retain(|w| w.from.part != reference && w.to.part != reference);
}

pub(super) fn remove_wire(wires: &mut Vec<Wire>, index: usize) {
    if index < wires.len() {
        wires.remove(index);
    }
}

/// Forget a wire's bends, so it routes itself again.
pub(super) fn straighten(wires: &mut [Wire], index: usize) {
    if let Some(wire) = wires.get_mut(index) {
        wire.bends.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_embed::nets::kit_rows;
    use rusty_embed::{Fill, Graphic, Pin, PinKind};

    use crate::view::panels::simulate::geometry::{KIT_SYMBOL, kit_symbol};

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
                .map(|(i, (number, name))| Pin {
                    number: (*number).into(),
                    name: (*name).into(),
                    kind: PinKind::Passive,
                    at: (if i == 0 { -3.81 } else { 3.81 }, 0.0),
                    length: 2.54,
                    angle: if i == 0 { 0 } else { 180 },
                    hidden: false,
                })
                .collect(),
            graphics: vec![Graphic::Rectangle {
                start: (-1.0, -1.0),
                end: (1.0, 1.0),
                width: 0.254,
                fill: Fill::None,
            }],
        }
    }

    fn led() -> Symbol {
        symbol("Device", "LED", "D", &[("1", "K"), ("2", "A")])
    }

    fn sheet() -> Vec<EditPart> {
        let rows = kit_rows("esp32c3", &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 20, 21]);
        vec![EditPart {
            inst: Instance {
                reference: KIT_REFERENCE.into(),
                symbol: KIT_SYMBOL.into(),
                value: "ESP32C3".into(),
                x: 460.0,
                y: 40.0,
                rot: 0,
                mirror: false,
                props: Default::default(),
            },
            symbol: Some(kit_symbol("esp32c3", &rows)),
        }]
    }

    #[test]
    fn a_new_part_is_numbered_after_the_ones_there_and_a_gap_is_reused() {
        let mut list = sheet();
        let first = add(&mut list, &led(), 100.0, 100.0);
        let second = add(&mut list, &led(), 150.0, 100.0);
        assert_eq!(list[first].inst.reference, "D1");
        assert_eq!(list[second].inst.reference, "D2");
        assert_eq!(
            list[first].inst.value, "",
            "`LED` beside an LED says nothing"
        );
        let mut wires = Vec::new();
        assert!(remove(&mut list, &mut wires, first));
        assert_eq!(add(&mut list, &led(), 0.0, 0.0), 2);
        assert_eq!(list[2].inst.reference, "D1", "the gap is filled");
        let mut lcsc = symbol("lcsc", "C25804", "R", &[("1", "1"), ("2", "2")]);
        lcsc.value = "10kΩ".into();
        let r = add(&mut list, &lcsc, 0.0, 0.0);
        assert_eq!(list[r].inst.reference, "R1");
        assert_eq!(list[r].inst.value, "10kΩ", "a real value is kept");
        assert_eq!(next_reference(&list, "U"), "U2", "never the devkit's");
        assert_eq!(
            next_reference(&list, "LED?"),
            "LED1",
            "LCSC's question mark is not part of the name"
        );
    }

    #[test]
    fn wiring_joins_two_pins_by_their_spelling_and_refuses_what_means_nothing() {
        let mut list = sheet();
        let d = add(&mut list, &led(), 100.0, 100.0);
        let mut wires = Vec::new();
        let made = connect(&list, &mut wires, (0, "4"), (d, "2")).expect("wired");
        assert_eq!(
            wires[made].from,
            PinRef::new("U1", "GPIO2"),
            "the row's unique name"
        );
        assert_eq!(wires[made].to, PinRef::new("D1", "A"), "the pin's name");
        assert_eq!(
            connect(&list, &mut wires, (d, "2"), (0, "4")),
            None,
            "already joined, either way round"
        );
        assert_eq!(
            connect(&list, &mut wires, (d, "2"), (d, "2")),
            None,
            "a pin to itself"
        );
        connect(&list, &mut wires, (d, "1"), (0, "9")).expect("to ground");
        assert_eq!(
            wires[1].to,
            PinRef::new("U1", "9"),
            "GND repeats, so its number"
        );
        assert_eq!(wires_at(&list, &wires, d, "1"), vec![1]);
        assert!(
            end_is(&list, &PinRef::new("D1", "K"), d, "1"),
            "a name resolves to its number"
        );
        disconnect_pin(&list, &mut wires, d, "2");
        assert_eq!(wires.len(), 1);
        assert!(rename(&mut list, &mut wires, d, "D9"));
        assert_eq!(wires[0].from.part, "D9", "the wire follows the new name");
        assert!(!rename(&mut list, &mut wires, d, ""));
        assert!(!rename(&mut list, &mut wires, d, "U1"));
        assert!(
            !rename(&mut list, &mut wires, 0, "U2"),
            "the devkit keeps its name"
        );
        let other = add(&mut list, &led(), 0.0, 0.0);
        assert!(!rename(&mut list, &mut wires, other, "D9"), "taken");
        disconnect_all(&list, &mut wires, d);
        assert!(wires.is_empty());
    }

    #[test]
    fn removing_a_part_takes_its_wires_and_the_devkit_cannot_go() {
        let mut list = sheet();
        let d = add(&mut list, &led(), 100.0, 100.0);
        let r = add(
            &mut list,
            &symbol("Device", "R", "R", &[("1", "~"), ("2", "~")]),
            200.0,
            100.0,
        );
        let mut wires = Vec::new();
        connect(&list, &mut wires, (0, "4"), (r, "1"));
        connect(&list, &mut wires, (r, "2"), (d, "2"));
        assert!(!remove(&mut list, &mut wires, 0), "the devkit stays");
        assert!(remove(&mut list, &mut wires, r));
        assert_eq!(wires.len(), 0, "both wires touched the resistor");
        assert_eq!(list.len(), 2);
        remove_many(&mut list, &mut wires, &[1, 0, 1]);
        assert_eq!(list.len(), 1, "the lamp went, the devkit did not");
    }

    #[test]
    fn turning_mirroring_and_duplicating_leave_the_devkit_alone() {
        let mut list = sheet();
        let d = add(&mut list, &led(), 100.0, 100.0);
        for _ in 0..4 {
            rotate(&mut list, d);
        }
        assert_eq!(list[d].inst.rot, 0, "four turns come back upright");
        rotate(&mut list, d);
        mirror(&mut list, d);
        assert_eq!((list[d].inst.rot, list[d].inst.mirror), (90, true));
        rotate(&mut list, 0);
        mirror(&mut list, 0);
        assert_eq!((list[0].inst.rot, list[0].inst.mirror), (0, false));
        assert_eq!(duplicate(&mut list, 0), None);
        let copy = duplicate(&mut list, d).expect("copied");
        assert_eq!(list[copy].inst.reference, "D2");
        assert_eq!(
            (
                list[copy].inst.x,
                list[copy].inst.rot,
                list[copy].inst.mirror
            ),
            (116.0, 90, true)
        );
        nudge(&mut list, copy, -8.0, 0.0);
        assert_eq!(list[copy].inst.x, 108.0);
        translate(&mut list, &[(d, (100.0, 100.0))], 10.0, 20.0);
        assert_eq!((list[d].inst.x, list[d].inst.y), (110.0, 120.0));
        set_value(&mut list, d, " blue ");
        assert_eq!(list[d].inst.value, "blue");
        set_prop(&mut list, d, "max", "1023");
        assert_eq!(list[d].inst.prop::<u16>("max"), Some(1023));
        set_prop(&mut list, d, "max", "");
        assert!(list[d].inst.props.is_empty());
    }

    #[test]
    fn history_is_capped_and_keeps_the_newest() {
        let mut past: Vec<Snapshot> = Vec::new();
        for i in 0..(HISTORY_CAP + 5) {
            let mut list = sheet();
            list[0].inst.x = i as f64;
            remember(&mut past, (list, Vec::new()));
        }
        assert_eq!(past.len(), HISTORY_CAP);
        assert_eq!(past[0].0[0].inst.x, 5.0, "the oldest five were dropped");
        assert_eq!(past.last().unwrap().0[0].inst.x, (HISTORY_CAP + 4) as f64);
    }
}
