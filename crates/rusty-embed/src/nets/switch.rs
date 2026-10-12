//! What pressing a switch does: drive a GPIO to a rail's level, or join
//! two GPIOs.

use super::graph::Graph;
use super::{Rail, Row, gpio_of, power_rail};
use crate::model::{KIT_REFERENCE, Sheet};

/// What pressing a switch does: the GPIO it reaches on one side, and the
/// level the rail on its other side puts there. `None` when a press
/// changes nothing the firmware could read — no GPIO, no rail, or a switch
/// with one side unwired — which is a warning rather than a guess.
pub fn button_drives(sheet: &Sheet, rows: &[Row], part: &str) -> Option<(u8, bool)> {
    let (graph, mut dc) = Graph::conducting_of(sheet, rows);
    let mut sides: Vec<(Option<u8>, Option<Rail>)> = Vec::new();
    for root in graph.sides(&mut dc, part) {
        let mut gpio = None;
        let mut rail = None;
        for other in graph.members(&mut dc, root) {
            let other_pin = &graph.nodes[other];
            if other_pin.part == KIT_REFERENCE {
                if let Some(row) = graph.row_of(other) {
                    gpio = gpio.or(row.gpio);
                    rail = rail.or(row.rail);
                }
            } else {
                // A power symbol is a rail wherever it is drawn, as
                // `drivers` and `divider_at` read it: a button to a GND
                // symbol is a button to ground.
                rail = rail.or_else(|| sheet.symbol_of(&other_pin.part).and_then(power_rail));
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

/// The GPIOs one key of a matrix keypad joins: its row's and its column's.
///
/// `row` and `column` are zero-based, as the drawing numbers its keys. The
/// pins are named `R1`..`R4` and `C1`..`C4`, and either reaching no GPIO is
/// `None` — a keypad with three wires on it can be pressed and the press
/// reaches nothing, which the panel says rather than inventing a pin.
pub fn keypad_tie(
    sheet: &Sheet,
    rows: &[Row],
    part: &str,
    row: usize,
    column: usize,
) -> Option<(u8, u8)> {
    let a = gpio_of(sheet, rows, part, &format!("R{}", row + 1))?;
    let b = gpio_of(sheet, rows, part, &format!("C{}", column + 1))?;
    (a != b).then_some((a, b))
}

/// The two GPIOs a switch *joins*, when that is what it does.
///
/// A switch to a rail drives a level and `button_drives` says which; a
/// switch between two GPIOs drives nothing at all — it connects them, and
/// which way the level then flows is whichever of them the firmware is
/// driving at that instant. That is a matrix keypad, and reading it as a
/// drive is how a scanned row would look like every key in its column being
/// held down.
///
/// Both sides must reach a GPIO and they must be different ones; anything
/// else is `None` and the caller falls back to the rail reading.
pub fn switch_tie(sheet: &Sheet, rows: &[Row], part: &str) -> Option<(u8, u8)> {
    let (graph, mut dc) = Graph::conducting_of(sheet, rows);
    let mut sides: Vec<u8> = graph
        .sides(&mut dc, part)
        .into_iter()
        .filter_map(|root| graph.gpio_in(&mut dc, root))
        .collect();
    sides.sort_unstable();
    sides.dedup();
    (sides.len() == 2).then(|| (sides[0], sides[1]))
}
