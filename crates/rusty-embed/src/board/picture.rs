//! The board as one picture: the devkit and every part drawn where the
//! sheet puts them — what does not change while a run goes on. What does —
//! a wire's level, a lamp's light, a cap held down — and every word on the
//! sheet are drawn over it by whoever shows the board, so a pin moving
//! never redraws the picture, and the words are the window's own text.

use super::{
    art,
    geometry::{
        EditPart, KitStyle, UNKNOWN_BOX, bounds, kit_art, kit_height, orient, part_box, pin_labels,
        row_offset,
    },
};
use crate::nets::Row;

/// Room round the parts, so a lead's name is not cut at the edge.
const MARGIN: f64 = 24.0;

/// The picture, and where on the sheet it sits.
pub struct Picture {
    /// A whole SVG document whose user units are the sheet's.
    pub svg: String,
    /// The sheet coordinates of the picture's top-left corner.
    pub origin: (f64, f64),
    pub size: (f64, f64),
}

/// A word on the sheet: a lead's name, a reference, a value, a devkit row.
#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    /// Where its baseline starts, ends or is centred, in sheet units.
    pub x: f64,
    pub y: f64,
    /// `start`, `middle` or `end`, as SVG says it.
    pub anchor: &'static str,
    pub text: String,
    pub size: f64,
    /// What it is, which decides its colour: `pin`, `reference`, `value`.
    pub kind: &'static str,
}

/// The part's body: the devkit's board, or the symbol's drawing, turned
/// and mirrored as the part is.
fn body(part: &EditPart, rows: &[Row], kit: &KitStyle, chip_label: &str) -> String {
    let inst = &part.inst;
    let flip = if inst.mirror { -1 } else { 1 };
    let drawn = if part.is_kit() {
        kit_art(kit, kit_height(rows.len()), chip_label)
    } else {
        match &part.symbol {
            Some(symbol) => art::markup(symbol, &inst.value),
            // A part whose symbol no library has is kept, as a labelled box.
            None => {
                let (w, h) = UNKNOWN_BOX;
                format!(
                    r##"<rect x="{}" y="{}" width="{w}" height="{h}" rx="4" fill="#2c313a" stroke="#e05d38" stroke-dasharray="4 3"/>"##,
                    -w / 2.0,
                    -h / 2.0
                )
            }
        }
    };
    format!(
        r#"<g transform="translate({} {})"><g transform="rotate({}) scale({flip} 1)">{drawn}</g></g>"#,
        inst.x, inst.y, inst.rot
    )
}

/// The board drawn: every part where the sheet puts it.
pub fn picture(parts: &[EditPart], rows: &[Row], kit: &KitStyle, chip_label: &str) -> Picture {
    let ((x0, y0), (x1, y1)) = bounds(parts);
    let origin = (x0 - MARGIN, y0 - MARGIN);
    let size = (
        (x1 - x0 + 2.0 * MARGIN).max(1.0),
        (y1 - y0 + 2.0 * MARGIN).max(1.0),
    );
    let mut svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{} {} {} {}" width="{}" height="{}">"#,
        origin.0, origin.1, size.0, size.1, size.0, size.1
    );
    for part in parts {
        svg.push_str(&body(part, rows, kit, chip_label));
    }
    svg.push_str("</svg>");
    Picture { svg, origin, size }
}

/// Every word on the sheet, placed after each part's turn so none is drawn
/// sideways: the names of the leads, a devkit's rows, and each part's
/// reference above it and value below, KiCad's placement.
pub fn words(parts: &[EditPart], rows: &[Row]) -> Vec<Word> {
    let mut out = Vec::new();
    for part in parts {
        let inst = &part.inst;
        if part.is_kit() {
            let per_side = rows.len().div_ceil(2).max(1);
            for (index, row) in rows.iter().enumerate() {
                // Eight pixels in from its pin, inside the board's edge,
                // where a devkit's silkscreen puts it.
                let (px, py) = row_offset(rows.len(), index);
                let inward = if index < per_side { 1.0 } else { -1.0 };
                let (x, y) = orient((px + inward * 8.0, py), inst.rot, inst.mirror);
                let (ox, oy) = orient((inward, 0.0), inst.rot, inst.mirror);
                let (anchor, dy) = if ox.abs() > oy.abs() {
                    (if ox > 0.0 { "start" } else { "end" }, 3.0)
                } else if oy > 0.0 {
                    ("middle", 8.0)
                } else {
                    ("middle", -3.0)
                };
                out.push(Word {
                    x: inst.x + x,
                    y: inst.y + y + dy,
                    anchor,
                    text: row.label.clone(),
                    size: 7.5,
                    kind: "pin",
                });
            }
            continue;
        }
        // Already on the sheet: placed after the part's turn.
        for label in pin_labels(part) {
            out.push(Word {
                x: label.x,
                y: label.y,
                anchor: label.anchor,
                text: label.text,
                size: label.size,
                kind: "pin",
            });
        }
        // A rail and a label are their own value, printed in the drawing.
        if part.symbol.as_ref().is_some_and(art::draws_own_value) {
            continue;
        }
        let (bx0, by0, bx1, by1) = part_box(part);
        let cx = (bx0 + bx1) / 2.0;
        out.push(Word {
            x: cx,
            y: by0 - 5.0,
            anchor: "middle",
            text: inst.reference.clone(),
            size: 9.0,
            kind: "reference",
        });
        if !inst.value.is_empty() {
            out.push(Word {
                x: cx,
                y: by1 + 11.0,
                anchor: "middle",
                text: inst.value.clone(),
                size: 8.0,
                kind: "value",
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::geometry::{kit_style, parts_of};
    use crate::nets::kit_rows;
    use crate::{Instance, Sheet, Wire};

    fn sheet() -> Sheet {
        let mut sheet = crate::board::geometry::empty_sheet("esp32c3");
        sheet.parts.push(Instance {
            reference: "R1".into(),
            symbol: "Device:R".into(),
            value: "220".into(),
            x: 260.0,
            y: 80.0,
            rot: 90,
            mirror: true,
            props: Default::default(),
        });
        sheet.parts.push(Instance {
            reference: "D1".into(),
            symbol: "Device:LED".into(),
            value: "green".into(),
            x: 180.0,
            y: 80.0,
            rot: 0,
            mirror: false,
            props: Default::default(),
        });
        sheet.wires.push(Wire {
            from: crate::PinRef::new("U1", "GPIO0"),
            to: crate::PinRef::new("R1", "1"),
            bends: Vec::new(),
        });
        sheet
    }

    /// Every part is in the picture, turned and mirrored as it is, and the
    /// picture covers them all with room to spare.
    #[test]
    fn the_picture_holds_every_part_where_the_sheet_puts_it() {
        let rows = kit_rows(crate::nets::Pinout::numbered(&[0, 1, 2, 3, 4, 5]));
        let parts = parts_of(&sheet(), &rows);
        let drawn = picture(&parts, &rows, &kit_style(None, ""), "ESP32C3");
        assert!(drawn.svg.starts_with("<svg"));
        assert!(drawn.svg.contains(r#"translate(260 80)"#));
        assert!(drawn.svg.contains("rotate(90) scale(-1 1)"));
        let ((x0, y0), (x1, y1)) = bounds(&parts);
        assert!(drawn.origin.0 < x0 && drawn.origin.1 < y0);
        assert!(drawn.origin.0 + drawn.size.0 > x1 && drawn.origin.1 + drawn.size.1 > y1);
    }

    /// The resistor's reference is above it and its value below; every row
    /// of the devkit is named, and no word is the devkit's reference.
    #[test]
    fn the_words_are_the_references_values_and_rows() {
        let rows = kit_rows(crate::nets::Pinout::numbered(&[0, 1, 2, 3, 4, 5]));
        let parts = parts_of(&sheet(), &rows);
        let said = words(&parts, &rows);
        let reference = said.iter().find(|w| w.text == "R1").expect("R1 named");
        let value = said.iter().find(|w| w.text == "220").expect("its value");
        assert!(reference.y < value.y);
        for row in &rows {
            assert!(
                said.iter().any(|w| w.kind == "pin" && w.text == row.label),
                "row {} is named",
                row.label
            );
        }
        assert!(!said.iter().any(|w| w.text == "U1"));
    }

    /// A lead's name is beside its part, not a part's width away from it.
    /// The LED's symbol is the built-in library's, which only the backend
    /// reads.
    #[cfg(feature = "backend")]
    #[test]
    fn a_leads_name_is_beside_its_part() {
        let rows = kit_rows(crate::nets::Pinout::numbered(&[0, 1, 2, 3, 4, 5]));
        let mut with_led = sheet();
        with_led.symbols = crate::schematic::builtin().symbols;
        let parts = parts_of(&with_led, &rows);
        let led = parts.iter().find(|p| p.inst.reference == "D1").expect("D1");
        assert!(led.symbol.is_some(), "the LED's symbol resolved");
        let (x0, y0, x1, y1) = part_box(led);
        let names: Vec<Word> = pin_labels(led)
            .iter()
            .filter_map(|label| {
                words(&parts, &rows).into_iter().find(|w| {
                    w.text == label.text && w.kind == "pin" && (w.x - label.x).abs() < 0.01
                })
            })
            .collect();
        assert!(!names.is_empty(), "the LED's leads are named");
        for name in names {
            assert!(
                name.x > x0 - 30.0
                    && name.x < x1 + 30.0
                    && name.y > y0 - 30.0
                    && name.y < y1 + 30.0,
                "{} at {},{} is far from the LED's box {x0},{y0}..{x1},{y1}",
                name.text,
                name.x,
                name.y
            );
        }
    }
}
