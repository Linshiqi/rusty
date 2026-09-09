//! A schematic symbol: the drawing of a part with its pins, in the part's
//! own coordinates.
//!
//! One shape for every source — KiCad's `.kicad_sym` libraries, EasyEDA's
//! JSON for an LCSC part, the built-in library — so the sheet, the netlist
//! and the rules never ask where a symbol came from. Coordinates are KiCad's:
//! millimetres, y *up*, the origin at the symbol's anchor. The renderer flips
//! y once, at the edge, rather than every importer remembering to.
//!
//! Wire model, not file format: `docs/schematic.md` has the file.

use serde::{Deserialize, Serialize};

/// A symbol in a library — `Device:LED` is library `Device`, name `LED`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Symbol {
    pub library: String,
    pub name: String,
    /// The prefix a placed instance is numbered with: `R`, `C`, `D`, `SW`.
    pub reference: String,
    /// The default value shown beside a placed instance — `R`, `LED`, or a
    /// part's own value for an imported one (`1kΩ`).
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub pins: Vec<Pin>,
    pub graphics: Vec<Graphic>,
}

/// One pin: where a wire attaches, and what the pin is electrically.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pin {
    /// What a wire and a net remember — `1`, `2`, `A`, `K` as the library
    /// numbered them. Unique within the symbol.
    pub number: String,
    /// What the body shows — `~` in KiCad's libraries when there is none.
    pub name: String,
    pub kind: PinKind,
    /// The connection point, where a wire lands.
    pub at: (f64, f64),
    /// From the connection point toward the body.
    pub length: f64,
    /// Which way the pin points *into* the body from its connection point:
    /// KiCad's convention, degrees counter-clockwise, 0 = the body is to
    /// the right of the connection point.
    pub angle: u16,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
}

/// KiCad's electrical types, the ones the rules read. Anything else a
/// library says is `Unspecified` rather than a guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinKind {
    Passive,
    Input,
    Output,
    Bidirectional,
    PowerIn,
    PowerOut,
    OpenCollector,
    Tristate,
    NoConnect,
    Unspecified,
}

/// How a closed shape is painted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fill {
    None,
    /// The stroke colour.
    Outline,
    /// The body colour — KiCad's pale yellow.
    Background,
}

/// One KiCad millimetre in the sheet's own units.
///
/// A symbol is in KiCad's millimetres; a sheet is in pixels. This is the
/// one scale between them, and it is `ROW_PITCH / 2.54` — a devkit header's
/// row pitch is KiCad's 100 mil pin pitch — so a symbol's pins land on the
/// same grid as the devkit's, which is what lets a snapped wire meet both
/// ends. It sits here, beside the coordinates it is about and on the wasm
/// side, because both the canvas and the KiCad writer convert with it and a
/// second copy is how an imported part comes in at a different size from
/// the one beside it.
pub const MM_PX: f64 = 16.0 / 2.54;

/// One drawing primitive of a symbol's body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Graphic {
    Polyline {
        points: Vec<(f64, f64)>,
        width: f64,
        fill: Fill,
    },
    Rectangle {
        start: (f64, f64),
        end: (f64, f64),
        width: f64,
        fill: Fill,
    },
    Circle {
        center: (f64, f64),
        radius: f64,
        width: f64,
        fill: Fill,
    },
    /// Three points on the arc, as KiCad stores it.
    Arc {
        start: (f64, f64),
        mid: (f64, f64),
        end: (f64, f64),
        width: f64,
        fill: Fill,
    },
    Text {
        text: String,
        at: (f64, f64),
        size: f64,
        angle: f64,
    },
}

impl Symbol {
    /// `library:name`, the way a board file names a symbol.
    pub fn id(&self) -> String {
        format!("{}:{}", self.library, self.name)
    }

    /// A pin by its number, or by its name when the number does not match —
    /// `D1.K` reads better than `D1.1`, and both should land.
    pub fn pin(&self, key: &str) -> Option<&Pin> {
        self.pins
            .iter()
            .find(|p| p.number == key)
            .or_else(|| self.pins.iter().find(|p| p.name == key))
    }

    /// The box every graphic and pin fits in, `(min x, min y, max x, max y)`
    /// in the symbol's own units — what placement and hit-testing use.
    /// `None` for a symbol with nothing to draw.
    pub fn bounds(&self) -> Option<(f64, f64, f64, f64)> {
        let mut acc: Option<(f64, f64, f64, f64)> = None;
        let mut take = |x: f64, y: f64| {
            acc = Some(match acc {
                None => (x, y, x, y),
                Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
            });
        };
        for pin in &self.pins {
            take(pin.at.0, pin.at.1);
            let (dx, dy) = pin.direction();
            take(pin.at.0 + dx * pin.length, pin.at.1 + dy * pin.length);
        }
        for graphic in &self.graphics {
            match graphic {
                Graphic::Polyline { points, .. } => {
                    for (x, y) in points {
                        take(*x, *y);
                    }
                }
                Graphic::Rectangle { start, end, .. } => {
                    take(start.0, start.1);
                    take(end.0, end.1);
                }
                Graphic::Circle { center, radius, .. } => {
                    take(center.0 - radius, center.1 - radius);
                    take(center.0 + radius, center.1 + radius);
                }
                Graphic::Arc {
                    start, mid, end, ..
                } => {
                    take(start.0, start.1);
                    take(mid.0, mid.1);
                    take(end.0, end.1);
                }
                Graphic::Text { at, .. } => take(at.0, at.1),
            }
        }
        acc
    }
}

impl Pin {
    /// The unit vector from the connection point toward the body.
    pub fn direction(&self) -> (f64, f64) {
        match self.angle % 360 {
            0 => (1.0, 0.0),
            90 => (0.0, 1.0),
            180 => (-1.0, 0.0),
            270 => (0.0, -1.0),
            _ => {
                let radians = f64::from(self.angle).to_radians();
                (radians.cos(), radians.sin())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resistor() -> Symbol {
        Symbol {
            library: "Device".into(),
            name: "R".into(),
            reference: "R".into(),
            value: "R".into(),
            description: None,
            pins: vec![
                Pin {
                    number: "1".into(),
                    name: "~".into(),
                    kind: PinKind::Passive,
                    at: (0.0, 3.81),
                    length: 1.27,
                    angle: 270,
                    hidden: false,
                },
                Pin {
                    number: "2".into(),
                    name: "~".into(),
                    kind: PinKind::Passive,
                    at: (0.0, -3.81),
                    length: 1.27,
                    angle: 90,
                    hidden: false,
                },
            ],
            graphics: vec![Graphic::Rectangle {
                start: (-1.016, -2.54),
                end: (1.016, 2.54),
                width: 0.254,
                fill: Fill::None,
            }],
        }
    }

    #[test]
    fn a_pin_is_found_by_number_and_then_by_name() {
        let mut led = resistor();
        led.pins[0].name = "K".into();
        led.pins[1].name = "A".into();
        assert_eq!(led.pin("1").map(|p| p.name.as_str()), Some("K"));
        assert_eq!(led.pin("A").map(|p| p.number.as_str()), Some("2"));
        assert!(led.pin("Q").is_none());
    }

    /// Pins reach *out* of the body: the bounds include the connection
    /// points, so a placed symbol's box is where wires land, not just
    /// where the rectangle is.
    #[test]
    fn bounds_reach_the_pins_connection_points() {
        let (x0, y0, x1, y1) = resistor().bounds().unwrap();
        assert_eq!((x0, x1), (-1.016, 1.016));
        assert_eq!((y0, y1), (-3.81, 3.81));
        assert_eq!(
            resistor().pins[0].direction(),
            (0.0, -1.0),
            "270° points down"
        );
        let empty = Symbol {
            pins: Vec::new(),
            graphics: Vec::new(),
            ..resistor()
        };
        assert_eq!(empty.bounds(), None);
    }
}
