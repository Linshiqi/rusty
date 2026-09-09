//! What a part looks like on the desk.
//!
//! The sheet draws real components — a 5 mm LED with its flat and its long
//! anode leg, a resistor with the colour bands of its own value, a tactile
//! switch with a cap that sinks — rather than schematic outlines. The board
//! beside them is a photograph of a devkit, and a KiCad line drawing next to
//! it read as two pictures of two different things; a part somebody is about
//! to wire up on their desk is the thing this panel is for.
//!
//! **The drawing decides where the pins are.** A symbol's own coordinates say
//! where a schematic would put them; here the wire attaches to the end of a
//! leg, because that is where a wire goes. [`layout`] answers that — cheaply,
//! no markup — and everything geometric reads it; [`markup`] draws the same
//! shapes from the same constants, so the leg a wire lands on and the leg
//! that is drawn cannot drift apart.
//!
//! An imported part rusty knows nothing about is drawn as a package with its
//! pins down the two long sides: a chip, which is what most of them are, and
//! an honest one — the outline says "some part", the pin names say the rest.

use rusty_embed::nets::{Behaviour, behaviour_of, ohms};
use rusty_embed::{Pin, Symbol};

use super::geometry::local;

/// One pin of the drawn part: which pin it is, what it is called, where its
/// wire attaches, and the direction the lead runs away from the body.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Spot {
    pub number: String,
    pub name: String,
    pub at: (f64, f64),
    pub out: (f64, f64),
}

/// The part's drawing, in sheet pixels around its anchor, y down.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Layout {
    pub spots: Vec<Spot>,
    /// `(x0, y0, x1, y1)` — the box the drawing occupies, legs included.
    pub bounds: (f64, f64, f64, f64),
    /// The lit part of a lamp, or a button's cap: centre and radius. The
    /// view paints it, because its colour is what the firmware is doing.
    pub lens: Option<(f64, f64, f64)>,
    /// A screen's face: `(x, y, width, height)`. The view draws what is on
    /// it, upright however the part is turned.
    pub face: Option<(f64, f64, f64, f64)>,
}

impl Layout {
    pub fn spot(&self, number: &str) -> Option<&Spot> {
        self.spots.iter().find(|s| s.number == number)
    }
}

/// How a part is drawn. Read off its behaviour, so an LED imported from LCSC
/// is drawn as an LED and a part rusty knows nothing about is a package.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Look {
    /// The devkit, drawn by `kit_art`; its pins are the header's rows.
    Kit,
    Led,
    Rgb,
    Seven,
    Resistor,
    Capacitor,
    Switch,
    Pot,
    Analog,
    Display,
    Motor,
    /// A rail: the ground symbol, or the supply arrow.
    Ground,
    Supply,
    /// A name for a net, drawn as the tag a schematic uses.
    Label,
    Buzzer,
    Servo,
    Sensor,
    /// Two leads and a body: an unknown two-pin part.
    Axial,
    /// Pins down two sides.
    Package,
}

fn look(symbol: &Symbol) -> Look {
    if symbol.library == "rusty" && symbol.name == "kit" {
        return Look::Kit;
    }
    match behaviour_of(symbol) {
        Behaviour::Led => Look::Led,
        Behaviour::Rgb => Look::Rgb,
        Behaviour::Seven => Look::Seven,
        Behaviour::Resistor => Look::Resistor,
        Behaviour::Capacitor => Look::Capacitor,
        Behaviour::Switch => Look::Switch,
        Behaviour::Pot => Look::Pot,
        Behaviour::Analog => Look::Analog,
        Behaviour::Display => Look::Display,
        Behaviour::Motor => Look::Motor,
        Behaviour::Power if symbol.name == "GND" => Look::Ground,
        Behaviour::Power => Look::Supply,
        Behaviour::Label => Look::Label,
        Behaviour::Buzzer => Look::Buzzer,
        Behaviour::Servo => Look::Servo,
        Behaviour::Sensor => Look::Sensor,
        Behaviour::Other if visible(symbol).len() == 2 => Look::Axial,
        Behaviour::Other => Look::Package,
    }
}

fn visible(symbol: &Symbol) -> Vec<&Pin> {
    symbol.pins.iter().filter(|p| !p.hidden).collect()
}

// ── the metal and the plastic ────────────────────────────────────────────
const LEAD: &str = "#9aa2ae";
const LEAD_DARK: &str = "#6d7480";
const PLASTIC: &str = "#20242b";
const PLASTIC_EDGE: &str = "#3c434e";
const RESISTOR_BODY: &str = "#d8c49f";
const RESISTOR_EDGE: &str = "#a9906a";
const CERAMIC: &str = "#d8a441";
const PCB: &str = "#15384f";
const SCREEN: &str = "#050d16";
const KNOB: &str = "#e3e7ec";
const CAN: &str = "#8b939f";

/// A lead: the wire that leaves the body and ends where a net attaches.
fn lead(out: &mut String, from: (f64, f64), to: (f64, f64)) {
    out.push_str(&format!(
        r##"<line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="{LEAD}" stroke-width="1.8" stroke-linecap="round"/>"##,
        from.0, from.1, to.0, to.1
    ));
}

/// Spots on legs that run straight down from `y0` to `y1`.
fn legs_down(pins: &[&Pin], xs: &[f64], y1: f64) -> Vec<Spot> {
    pins.iter()
        .zip(xs)
        .map(|(pin, x)| Spot {
            number: pin.number.clone(),
            name: pin.name.clone(),
            at: (*x, y1),
            out: (0.0, 1.0),
        })
        .collect()
}

/// Evenly spaced positions for `n` legs of pitch `pitch`, centred on zero.
fn spread(n: usize, pitch: f64) -> Vec<f64> {
    (0..n)
        .map(|i| (i as f64 - (n as f64 - 1.0) / 2.0) * pitch)
        .collect()
}

/// A lamp's two pins, anode first: by name where the library gives one,
/// and by KiCad's order — pin 1 is the cathode — where it does not.
fn anode_first<'a>(pins: &[&'a Pin]) -> Vec<&'a Pin> {
    let named = |name: &str| pins.iter().find(|p| p.name == name).copied();
    match (named("A"), named("K")) {
        (Some(a), Some(k)) => vec![a, k],
        _ => {
            let mut order: Vec<&Pin> = pins.to_vec();
            order.reverse();
            order
        }
    }
}

/// The drawing's geometry: where every pin's wire lands, how big the part
/// is, and where its light and its screen are.
pub(super) fn layout(symbol: &Symbol) -> Layout {
    let pins = visible(symbol);
    let none = Layout {
        spots: Vec::new(),
        bounds: (-16.0, -12.0, 16.0, 12.0),
        lens: None,
        face: None,
    };
    match look(symbol) {
        // The devkit keeps the header's own geometry: its pins are rows on
        // a board that is already drawn to scale.
        Look::Kit => {
            let spots = pins
                .iter()
                .map(|pin| Spot {
                    number: pin.number.clone(),
                    name: pin.name.clone(),
                    at: local(pin.at),
                    out: out_of(pin),
                })
                .collect();
            let (x0, y0, x1, y1) = symbol.bounds().unwrap_or((0.0, 0.0, 1.0, -1.0));
            let a = local((x0, y0));
            let b = local((x1, y1));
            Layout {
                spots,
                bounds: (a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1)),
                lens: None,
                face: None,
            }
        }

        // A 5 mm lamp: dome, flange, and two legs — the anode's longer, as
        // it is in the bag, so which way round it goes is visible without
        // reading anything.
        Look::Led => {
            let order = anode_first(&pins);
            let mut spots = Vec::new();
            for (index, pin) in order.iter().enumerate() {
                let anode = index == 0;
                spots.push(Spot {
                    number: pin.number.clone(),
                    name: pin.name.clone(),
                    at: (
                        if anode { 4.0 } else { -4.0 },
                        if anode { 24.0 } else { 18.0 },
                    ),
                    out: (0.0, 1.0),
                });
            }
            Layout {
                spots,
                bounds: (-10.0, -18.0, 10.0, 24.0),
                lens: Some((0.0, -7.0, 9.0)),
                face: None,
            }
        }

        // Four legs under one lens: the common leg is the long one.
        Look::Rgb => {
            let xs = spread(pins.len().max(1), 6.0);
            Layout {
                spots: legs_down(&pins, &xs, 26.0),
                bounds: (-13.0, -22.0, 13.0, 26.0),
                lens: Some((0.0, -9.0, 11.0)),
                face: None,
            }
        }

        // A digit in its own package, pins down both long sides.
        Look::Seven => {
            let half = pins.len().div_ceil(2).max(1);
            let ys = spread(half, 13.0);
            let mut spots = Vec::new();
            for (index, pin) in pins.iter().enumerate() {
                let left = index < half;
                let y = ys[if left { index } else { index - half }];
                spots.push(Spot {
                    number: pin.number.clone(),
                    name: pin.name.clone(),
                    at: (if left { -32.0 } else { 32.0 }, y),
                    out: (if left { -1.0 } else { 1.0 }, 0.0),
                });
            }
            Layout {
                spots,
                bounds: (-32.0, -32.0, 32.0, 32.0),
                lens: None,
                face: Some((-15.0, -26.0, 30.0, 52.0)),
            }
        }

        // Axial, leads left and right, as it lies on the bench.
        Look::Resistor | Look::Axial => {
            let mut spots = Vec::new();
            for (index, pin) in pins.iter().enumerate() {
                let left = index == 0;
                spots.push(Spot {
                    number: pin.number.clone(),
                    name: pin.name.clone(),
                    at: (if left { -30.0 } else { 30.0 }, 0.0),
                    out: (if left { -1.0 } else { 1.0 }, 0.0),
                });
            }
            Layout {
                spots,
                bounds: (-30.0, -7.0, 30.0, 7.0),
                lens: None,
                face: None,
            }
        }

        // A ceramic disc on two legs.
        Look::Capacitor => Layout {
            spots: legs_down(&pins, &spread(pins.len().max(1), 9.0), 20.0),
            bounds: (-10.0, -17.0, 10.0, 20.0),
            lens: None,
            face: None,
        },

        // A tactile switch: a square body with a cap, legs out of the sides
        // — two where the library gives two, four where it gives four.
        Look::Switch => {
            let mut spots = Vec::new();
            let four = pins.len() >= 4;
            for (index, pin) in pins.iter().enumerate() {
                let left = index % 2 == 0;
                let y = if four {
                    if index < 2 { -6.0 } else { 6.0 }
                } else {
                    0.0
                };
                spots.push(Spot {
                    number: pin.number.clone(),
                    name: pin.name.clone(),
                    at: (if left { -22.0 } else { 22.0 }, y),
                    out: (if left { -1.0 } else { 1.0 }, 0.0),
                });
            }
            Layout {
                spots,
                bounds: (-22.0, -13.0, 22.0, 13.0),
                lens: Some((0.0, 0.0, 6.5)),
                face: None,
            }
        }

        // A knob on a body, three legs under it.
        Look::Pot => Layout {
            spots: legs_down(&pins, &spread(pins.len().max(1), 9.0), 24.0),
            bounds: (-16.0, -16.0, 16.0, 24.0),
            lens: None,
            face: None,
        },

        // A source: a cell with two legs, and the count beside it.
        Look::Analog => Layout {
            spots: legs_down(&pins, &spread(pins.len().max(1), 10.0), 24.0),
            bounds: (-16.0, -14.0, 16.0, 24.0),
            lens: None,
            face: None,
        },

        // A screen on a carrier board, its header down one edge.
        Look::Display => {
            let ys = spread(pins.len().max(1), 8.0);
            let spots = pins
                .iter()
                .zip(&ys)
                .map(|(pin, y)| Spot {
                    number: pin.number.clone(),
                    name: pin.name.clone(),
                    at: (-52.0, *y),
                    out: (-1.0, 0.0),
                })
                .collect();
            Layout {
                spots,
                bounds: (-52.0, -24.0, 42.0, 24.0),
                lens: None,
                face: Some((-34.0, -17.0, 70.0, 30.0)),
            }
        }

        // A can with a shaft, its wires out of the back.
        Look::Motor => {
            let ys = spread(pins.len().max(1), 8.0);
            let spots = pins
                .iter()
                .zip(&ys)
                .map(|(pin, y)| Spot {
                    number: pin.number.clone(),
                    name: pin.name.clone(),
                    at: (-38.0, *y),
                    out: (-1.0, 0.0),
                })
                .collect();
            Layout {
                spots,
                bounds: (-38.0, -20.0, 32.0, 20.0),
                lens: None,
                face: None,
            }
        }

        // A rail. The wire comes down into ground and up out of a supply,
        // which is how every schematic draws them and which way round tells
        // one from the other at a glance.
        Look::Ground => Layout {
            spots: pins
                .iter()
                .map(|pin| Spot {
                    number: pin.number.clone(),
                    name: pin.name.clone(),
                    at: (0.0, -14.0),
                    out: (0.0, -1.0),
                })
                .collect(),
            bounds: (-11.0, -14.0, 11.0, 8.0),
            lens: None,
            face: None,
        },

        Look::Supply => Layout {
            spots: pins
                .iter()
                .map(|pin| Spot {
                    number: pin.number.clone(),
                    name: pin.name.clone(),
                    at: (0.0, 14.0),
                    out: (0.0, 1.0),
                })
                .collect(),
            bounds: (-11.0, -8.0, 11.0, 14.0),
            lens: None,
            face: None,
        },

        // A tag with the net's name in it, pointing back at the wire.
        Look::Label => Layout {
            spots: pins
                .iter()
                .map(|pin| Spot {
                    number: pin.number.clone(),
                    name: pin.name.clone(),
                    at: (-16.0, 0.0),
                    out: (-1.0, 0.0),
                })
                .collect(),
            bounds: (-16.0, -9.0, 46.0, 9.0),
            lens: None,
            face: Some((-8.0, -8.0, 54.0, 16.0)),
        },

        // A sounder: a can with two legs, and a ring the view lights while
        // it is being driven.
        Look::Buzzer => Layout {
            spots: legs_down(&pins, &spread(pins.len().max(1), 10.0), 26.0),
            bounds: (-14.0, -16.0, 14.0, 26.0),
            lens: Some((0.0, -3.0, 11.0)),
            face: None,
        },

        // A servo: the case, its three wires out of the left, and a horn
        // the view turns.
        Look::Servo => {
            let ys = spread(pins.len().max(1), 8.0);
            let spots = pins
                .iter()
                .zip(&ys)
                .map(|(pin, y)| Spot {
                    number: pin.number.clone(),
                    name: pin.name.clone(),
                    at: (-40.0, *y),
                    out: (-1.0, 0.0),
                })
                .collect();
            Layout {
                spots,
                bounds: (-40.0, -20.0, 30.0, 20.0),
                lens: None,
                face: Some((-16.0, -14.0, 28.0, 28.0)),
            }
        }

        // A sensor board: a small module with its header down one side and
        // the die in the middle, which is what most of them look like.
        Look::Sensor => {
            let ys = spread(pins.len().max(1), 8.0);
            let spots = pins
                .iter()
                .zip(&ys)
                .map(|(pin, y)| Spot {
                    number: pin.number.clone(),
                    name: pin.name.clone(),
                    at: (-34.0, *y),
                    out: (-1.0, 0.0),
                })
                .collect();
            Layout {
                spots,
                bounds: (-34.0, -18.0, 26.0, 18.0),
                lens: None,
                face: Some((-20.0, -12.0, 44.0, 24.0)),
            }
        }

        // Anything else: a package, pins down the two long sides in the
        // order the library lists them — a chip, which is what most parts
        // rusty has never heard of actually are.
        Look::Package => {
            if pins.is_empty() {
                return none;
            }
            let half = pins.len().div_ceil(2);
            let height = (half as f64 * 12.0 + 10.0).max(30.0);
            let ys: Vec<f64> = (0..half)
                .map(|i| -height / 2.0 + 11.0 + i as f64 * 12.0)
                .collect();
            let mut spots = Vec::new();
            for (index, pin) in pins.iter().enumerate() {
                // A DIP is numbered down one side and back up the other.
                let left = index < half;
                let y = if left {
                    ys[index]
                } else {
                    ys[(pins.len() - 1 - index).min(half - 1)]
                };
                spots.push(Spot {
                    number: pin.number.clone(),
                    name: pin.name.clone(),
                    at: (if left { -32.0 } else { 32.0 }, y),
                    out: (if left { -1.0 } else { 1.0 }, 0.0),
                });
            }
            Layout {
                spots,
                bounds: (-32.0, -height / 2.0, 32.0, height / 2.0),
                lens: None,
                face: None,
            }
        }
    }
}

/// The direction a wire leaves a symbol pin, in sheet units: the symbol's
/// own, flipped into y-down and pointing away from the body.
fn out_of(pin: &Pin) -> (f64, f64) {
    let (dx, dy) = pin.direction();
    let (x, y) = local((dx, dy));
    let len = x.hypot(y);
    if len < 1e-9 {
        return (0.0, 0.0);
    }
    (-x / len, -y / len)
}

/// The part as SVG, in the same frame as [`layout`]: everything that does
/// not change while the firmware runs. What does — a lit lens, a sunk cap,
/// a digit's segments, a screen's text — the view paints over it.
pub(super) fn markup(symbol: &Symbol, value: &str) -> String {
    let plan = layout(symbol);
    let mut out = String::new();
    let look = look(symbol);

    // Every leg, from the body's edge to the point a wire lands on.
    // Every leg, from where it leaves the body to the point a wire lands
    // on. `start` is where the body ends, not a length: legs of different
    // lengths — a lamp's, which is how its polarity is read — must still
    // come out of the same edge, and measuring back from each tip put the
    // short one's start inside the dome.
    let legs = |out: &mut String, start: (f64, f64)| {
        for spot in &plan.spots {
            let from = if spot.out.0.abs() > spot.out.1.abs() {
                (spot.out.0.signum() * start.0, spot.at.1)
            } else {
                (spot.at.0, spot.out.1.signum() * start.1)
            };
            lead(out, from, spot.at);
        }
    };

    match look {
        Look::Kit => {}

        Look::Led => {
            legs(&mut out, (0.0, 2.0));
            // The dome is the lens the view paints; this is its rim, the
            // flat on the cathode side, and the flange under it.
            out.push_str(&format!(
                r##"<circle cx="0" cy="-7" r="9" fill="#171a20" stroke="#4a515c" stroke-width="0.9"/>
<rect x="-10" y="1.5" width="20" height="4.5" rx="1.5" fill="#c6ccd5" stroke="{LEAD_DARK}" stroke-width="0.8"/>
<rect x="-10" y="1.5" width="4.5" height="4.5" rx="1.5" fill="#8f97a3"/>"##
            ));
        }

        Look::Rgb => {
            legs(&mut out, (0.0, 2.0));
            out.push_str(&format!(
                r##"<circle cx="0" cy="-9" r="11" fill="#171a20" stroke="#4a515c" stroke-width="0.9"/>
<rect x="-13" y="0" width="26" height="5" rx="2" fill="#c6ccd5" stroke="{LEAD_DARK}" stroke-width="0.8"/>"##
            ));
        }

        Look::Seven => {
            legs(&mut out, (22.0, 0.0));
            let (fx, fy, fw, fh) = plan.face.unwrap_or((0.0, 0.0, 0.0, 0.0));
            out.push_str(&format!(
                r##"<rect x="-22" y="-32" width="44" height="64" rx="3" fill="#1b1216" stroke="#2f2126" stroke-width="1"/>
<rect x="{fx}" y="{fy}" width="{fw}" height="{fh}" rx="2" fill="#150e11"/>"##
            ));
        }

        Look::Resistor => {
            legs(&mut out, (13.0, 0.0));
            out.push_str(&format!(
                r##"<rect x="-17" y="-6.5" width="34" height="13" rx="6" fill="{RESISTOR_BODY}" stroke="{RESISTOR_EDGE}" stroke-width="1"/>"##
            ));
            for (index, colour) in bands(value).iter().enumerate() {
                let x = -11.0 + index as f64 * 6.0 + if index == 3 { 4.0 } else { 0.0 };
                out.push_str(&format!(
                    r##"<rect x="{x:.1}" y="-6.5" width="3" height="13" fill="{colour}"/>"##
                ));
            }
        }

        Look::Axial => {
            legs(&mut out, (13.0, 0.0));
            out.push_str(&format!(
                r##"<rect x="-17" y="-6.5" width="34" height="13" rx="6" fill="{PLASTIC}" stroke="{PLASTIC_EDGE}" stroke-width="1"/>"##
            ));
        }

        Look::Capacitor => {
            legs(&mut out, (0.0, 2.0));
            out.push_str(&format!(
                r##"<ellipse cx="0" cy="-6" rx="10" ry="11" fill="{CERAMIC}" stroke="#a97c26" stroke-width="1"/>
<text x="0" y="-3" text-anchor="middle" font-family="ui-monospace" font-size="7" fill="#5c421a">{}</text>"##,
                escape(&short(value))
            ));
        }

        Look::Switch => {
            legs(&mut out, (11.0, 0.0));
            out.push_str(&format!(
                r##"<rect x="-13" y="-13" width="26" height="26" rx="2" fill="{PLASTIC}" stroke="{PLASTIC_EDGE}" stroke-width="1"/>
<rect x="-10" y="-10" width="20" height="20" rx="1.5" fill="#2b3038"/>"##
            ));
        }

        Look::Pot => {
            legs(&mut out, (0.0, 2.0));
            out.push_str(&format!(
                r##"<rect x="-16" y="-14" width="32" height="26" rx="2.5" fill="{PCB}" stroke="#0d2436" stroke-width="1"/>
<circle cx="0" cy="-2" r="9.5" fill="{KNOB}" stroke="#9aa2ae" stroke-width="1"/>"##
            ));
        }

        Look::Analog => {
            legs(&mut out, (0.0, 2.0));
            // A cell: the source this part stands for, and the one shape
            // that says "a voltage that is simply there".
            out.push_str(
                r##"<rect x="-16" y="-14" width="32" height="26" rx="2.5" fill="#b8452f" stroke="#7d2c1d" stroke-width="1"/>
<rect x="-16" y="-14" width="32" height="9" rx="2.5" fill="#2b3038"/>
<text x="0" y="7" text-anchor="middle" font-family="ui-monospace" font-size="8" fill="#f7e3d8">+</text>"##,
            );
        }

        Look::Display => {
            legs(&mut out, (40.0, 0.0));
            let (fx, fy, fw, fh) = plan.face.unwrap_or((0.0, 0.0, 0.0, 0.0));
            out.push_str(&format!(
                r##"<rect x="-40" y="-24" width="82" height="48" rx="2.5" fill="{PCB}" stroke="#0d2436" stroke-width="1"/>
<rect x="{fx}" y="{fy}" width="{fw}" height="{fh}" rx="1.5" fill="{SCREEN}" stroke="#0a1f2e" stroke-width="1"/>
<circle cx="-36" cy="-20" r="1.6" fill="#0d2436"/>
<circle cx="38" cy="-20" r="1.6" fill="#0d2436"/>
<circle cx="-36" cy="20" r="1.6" fill="#0d2436"/>
<circle cx="38" cy="20" r="1.6" fill="#0d2436"/>"##
            ));
        }

        Look::Motor => {
            legs(&mut out, (26.0, 0.0));
            out.push_str(&format!(
                r##"<rect x="-26" y="-19" width="46" height="38" rx="6" fill="{CAN}" stroke="#6d7480" stroke-width="1"/>
<rect x="-26" y="-19" width="8" height="38" rx="3" fill="#79818d"/>
<rect x="20" y="-3" width="12" height="6" rx="1.5" fill="{LEAD_DARK}"/>"##
            ));
        }

        Look::Ground => {
            legs(&mut out, (0.0, -8.0));
            out.push_str(&format!(
                r##"<polyline points="-9,-8 9,-8 0,2" fill="none" stroke="{LEAD}" stroke-width="1.6" stroke-linejoin="round"/>
<path d="M -5.5 -3.5 H 5.5" stroke="{LEAD}" stroke-width="1.2"/>"##
            ));
        }

        Look::Supply => {
            legs(&mut out, (0.0, 8.0));
            out.push_str(&format!(
                r##"<polyline points="-8,2 0,-6 8,2" fill="none" stroke="{LEAD}" stroke-width="1.6" stroke-linejoin="round"/>
<text x="0" y="-9" text-anchor="middle" font-family="ui-monospace" font-size="8" fill="{LEAD}">{}</text>"##,
                escape(&short(value))
            ));
        }

        Look::Label => {
            legs(&mut out, (-8.0, 0.0));
            out.push_str(&format!(
                r##"<polygon points="-8,0 -2,-8 46,-8 46,8 -2,8" fill="#1d2733" stroke="#5fd0c8" stroke-width="1"/>
<text x="4" y="3" font-family="ui-monospace" font-size="9" fill="#5fd0c8">{}</text>"##,
                escape(&short(value))
            ));
        }

        Look::Buzzer => {
            legs(&mut out, (0.0, 6.0));
            out.push_str(&format!(
                r##"<circle cx="0" cy="-3" r="13" fill="{PLASTIC}" stroke="{PLASTIC_EDGE}" stroke-width="1"/>
<circle cx="0" cy="-3" r="3" fill="#0d1014"/>
<text x="-9" y="-9" font-family="ui-monospace" font-size="7" fill="#98a1ae">+</text>"##
            ));
        }

        Look::Servo => {
            legs(&mut out, (24.0, 0.0));
            out.push_str(&format!(
                r##"<rect x="-24" y="-14" width="42" height="28" rx="2" fill="{PLASTIC}" stroke="{PLASTIC_EDGE}" stroke-width="1"/>
<rect x="-24" y="-18" width="10" height="36" rx="2" fill="#2b3038"/>
<rect x="8" y="-18" width="10" height="36" rx="2" fill="#2b3038"/>
<circle cx="18" cy="0" r="10" fill="#2b3038" stroke="{PLASTIC_EDGE}" stroke-width="1"/>
<rect x="18" y="-16" width="12" height="4" rx="2" fill="#4a515c"/>"##
            ));
        }

        Look::Sensor => {
            legs(&mut out, (22.0, 0.0));
            out.push_str(&format!(
                r##"<rect x="-22" y="-18" width="48" height="36" rx="2.5" fill="{PCB}" stroke="#0d2436" stroke-width="1"/>
<rect x="-8" y="-8" width="18" height="16" rx="1.5" fill="{PLASTIC}" stroke="{PLASTIC_EDGE}" stroke-width="1"/>
<circle cx="-16" cy="-12" r="1.6" fill="#0d2436"/>
<circle cx="20" cy="12" r="1.6" fill="#0d2436"/>"##
            ));
        }

        Look::Package => {
            legs(&mut out, (22.0, 0.0));
            let (_, y0, _, y1) = plan.bounds;
            let height = y1 - y0;
            out.push_str(&format!(
                r##"<rect x="-22" y="{y0:.1}" width="44" height="{height:.1}" rx="3" fill="{PLASTIC}" stroke="{PLASTIC_EDGE}" stroke-width="1"/>
<path d="M -5 {y0:.1} A 5 5 0 0 0 5 {y0:.1}" fill="#171a20"/>
<circle cx="-14" cy="{:.1}" r="2" fill="#171a20"/>
<text x="0" y="{:.1}" text-anchor="middle" font-family="ui-monospace" font-size="7.5" fill="#98a1ae">{}</text>"##,
                y0 + 8.0,
                3.0,
                escape(&short(&symbol.name))
            ));
        }
    }
    out
}

/// Whether the drawing already carries the part's value, so the sheet
/// does not print it underneath as well: a rail and a label are their
/// value, and a second copy of it under the symbol reads as a mistake.
pub(super) fn draws_own_value(symbol: &Symbol) -> bool {
    matches!(look(symbol), Look::Supply | Look::Label | Look::Ground)
}

/// A value trimmed to what fits on a small body.
fn short(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= 8 {
        return text.to_string();
    }
    text.chars().take(7).collect::<String>() + "…"
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The colour bands of a resistor's value: two significant digits, a
/// multiplier and gold for the tolerance, exactly as the part in the
/// drawer is printed. A value nothing can be read from wears three grey
/// bands — the honest "some resistor", rather than a made-up number.
pub(super) fn bands(value: &str) -> Vec<&'static str> {
    const COLOURS: [&str; 10] = [
        "#1b1b1b", // black
        "#7a4a1e", // brown
        "#c0392b", // red
        "#d97b26", // orange
        "#e3c53f", // yellow
        "#3f9d54", // green
        "#3468c0", // blue
        "#8e5bd0", // violet
        "#9aa2ae", // grey
        "#f2f4f7", // white
    ];
    let Some(ohms) = ohms(value) else {
        return vec!["#6f7885", "#6f7885", "#6f7885"];
    };
    if ohms <= 0.0 {
        return vec![COLOURS[0], COLOURS[0], COLOURS[0], "#c9a227"];
    }
    let mut exponent = 0i32;
    let mut scaled = ohms;
    while scaled >= 100.0 {
        scaled /= 10.0;
        exponent += 1;
    }
    while scaled < 10.0 {
        scaled *= 10.0;
        exponent -= 1;
    }
    let two = scaled.round() as usize;
    let first = (two / 10).min(9);
    let second = (two % 10).min(9);
    let multiplier = exponent.clamp(0, 9) as usize;
    vec![
        COLOURS[first],
        COLOURS[second],
        COLOURS[multiplier],
        "#c9a227",
    ]
}

#[cfg(test)]
mod tests {
    use super::super::geometry::{MM_PX, kit_symbol};
    use super::*;
    use rusty_embed::PinKind;

    /// The library's own symbols cannot be loaded here — `schematic` is
    /// backend-only and this crate compiles to wasm — so the fixtures are
    /// the shapes of them the drawing actually reads: the library, the
    /// name, the reference prefix and the pin names.
    fn sym(library: &str, name: &str, reference: &str, pins: &[(&str, &str)]) -> Symbol {
        Symbol {
            library: library.into(),
            name: name.into(),
            reference: reference.into(),
            value: name.into(),
            description: None,
            pins: pins
                .iter()
                .map(|(number, pin_name)| Pin {
                    number: (*number).into(),
                    name: (*pin_name).into(),
                    kind: PinKind::Passive,
                    at: (-3.81, 0.0),
                    length: 2.54,
                    angle: 0,
                    hidden: false,
                })
                .collect(),
            graphics: Vec::new(),
        }
    }

    fn every_part() -> Vec<Symbol> {
        vec![
            sym("Device", "LED", "D", &[("1", "K"), ("2", "A")]),
            sym("Device", "R", "R", &[("1", "~"), ("2", "~")]),
            sym("Device", "C", "C", &[("1", "~"), ("2", "~")]),
            sym("Device", "SW_Push", "SW", &[("1", "1"), ("2", "2")]),
            sym("rusty", "Pot", "RV", &[("1", "1"), ("2", "W"), ("3", "3")]),
            sym("rusty", "Analog", "V", &[("1", "OUT"), ("2", "GND")]),
            sym(
                "rusty",
                "Display",
                "DS",
                &[("1", "SDA"), ("2", "SCL"), ("3", "VCC"), ("4", "GND")],
            ),
            sym(
                "rusty",
                "RGB_LED",
                "D",
                &[("1", "R"), ("2", "G"), ("3", "B"), ("4", "COM")],
            ),
            sym(
                "rusty",
                "7SEG",
                "DS",
                &[
                    ("1", "a"),
                    ("2", "b"),
                    ("3", "c"),
                    ("4", "d"),
                    ("5", "e"),
                    ("6", "f"),
                    ("7", "g"),
                    ("8", "COM"),
                ],
            ),
            sym(
                "rusty",
                "Motor",
                "M",
                &[("1", "PWM"), ("2", "IN1"), ("3", "IN2")],
            ),
            sym("lcsc", "C2286", "LED", &[("1", "A"), ("2", "K")]),
            sym("rusty", "GND", "#PWR", &[("1", "GND")]),
            sym("rusty", "Supply", "#PWR", &[("1", "VCC")]),
            sym("rusty", "Label", "#LBL", &[("1", "~")]),
            sym("rusty", "Buzzer", "BZ", &[("1", "+"), ("2", "-")]),
            sym(
                "rusty",
                "Servo",
                "M",
                &[("1", "SIG"), ("2", "VCC"), ("3", "GND")],
            ),
            sym(
                "rusty",
                "Sensor",
                "U",
                &[("1", "SDA"), ("2", "SCL"), ("3", "VCC"), ("4", "GND")],
            ),
        ]
    }

    fn named(name: &str) -> Symbol {
        every_part()
            .into_iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("{name} is one of the fixtures"))
    }

    /// Every part the library ships gets a spot for every pin it shows, and
    /// no spot lies outside the box the sheet selects it by — a wire that
    /// landed outside the drawing would be a wire attached to nothing.
    #[test]
    fn every_pin_of_every_part_has_a_lead_inside_the_box() {
        for symbol in every_part() {
            let plan = layout(&symbol);
            let shown = visible(&symbol);
            assert_eq!(
                plan.spots.len(),
                shown.len(),
                "{}: a spot per visible pin",
                symbol.id()
            );
            for pin in &shown {
                assert!(
                    plan.spot(&pin.number).is_some(),
                    "{}: pin {} has nowhere to attach",
                    symbol.id(),
                    pin.number
                );
            }
            let (x0, y0, x1, y1) = plan.bounds;
            assert!(x1 > x0 && y1 > y0, "{}: an empty box", symbol.id());
            for spot in &plan.spots {
                assert!(
                    spot.at.0 >= x0 - 0.01
                        && spot.at.0 <= x1 + 0.01
                        && spot.at.1 >= y0 - 0.01
                        && spot.at.1 <= y1 + 0.01,
                    "{}: {} at {:?} is outside {:?}",
                    symbol.id(),
                    spot.number,
                    spot.at,
                    plan.bounds
                );
                let len = spot.out.0.hypot(spot.out.1);
                assert!(
                    (len - 1.0).abs() < 1e-9,
                    "{}: a lead with no direction",
                    symbol.id()
                );
            }
            let drawn = markup(&symbol, "220");
            assert!(!drawn.is_empty(), "{}: nothing is drawn", symbol.id());
            assert_eq!(
                drawn.matches("<line").count(),
                shown.len(),
                "{}: one lead drawn per pin",
                symbol.id()
            );
        }
    }

    /// An LCSC part whose prefix says what it is gets that part's drawing,
    /// not a package: the import is worth having because it comes out as
    /// the thing it is.
    #[test]
    fn an_imported_lamp_is_drawn_as_a_lamp() {
        let plan = layout(&named("C2286"));
        assert!(plan.lens.is_some());
        assert_eq!(plan.spots.len(), 2);
        assert!(
            plan.spots.iter().all(|s| s.out == (0.0, 1.0)),
            "legs, not schematic stubs"
        );
    }

    /// A picture of every part, written when `RUSTY_ART_SVG` names a file.
    /// The check a unit test cannot be — whether the drawing looks like the
    /// component it stands for — needs eyes, and this is how they get it
    /// without building the app. Skipped, and says so, without the variable.
    #[test]
    fn every_part_can_be_drawn_to_a_sheet_for_looking_at() {
        let Ok(path) = std::env::var("RUSTY_ART_SVG") else {
            eprintln!("set RUSTY_ART_SVG=<file.svg> to draw every part for a look");
            return;
        };
        let mut svg = String::from(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="1180" height="480" viewBox="0 0 1180 480"><rect width="1180" height="480" fill="#101216"/>"##,
        );
        for (index, symbol) in every_part().into_iter().enumerate() {
            let x = 70.0 + (index % 6) as f64 * 185.0;
            let y = 90.0 + (index / 6) as f64 * 160.0;
            let plan = layout(&symbol);
            svg.push_str(&format!(
                r##"<g transform="translate({x} {y})">{}"##,
                markup(&symbol, "220")
            ));
            if let Some((lx, ly, r)) = plan.lens {
                svg.push_str(&format!(
                    r##"<circle cx="{lx}" cy="{ly}" r="{r}" fill="#3ddc84" fill-opacity="0.9"/>"##
                ));
            }
            for spot in &plan.spots {
                svg.push_str(&format!(
                    r##"<circle cx="{}" cy="{}" r="3.4" fill="#c9a227"/>"##,
                    spot.at.0, spot.at.1
                ));
            }
            svg.push_str(&format!(
                r##"<text x="0" y="{}" text-anchor="middle" font-family="ui-monospace" font-size="10" fill="#98a1ae">{}</text></g>"##,
                plan.bounds.3 + 18.0,
                escape(&symbol.name)
            ));
        }
        svg.push_str("</svg>");
        std::fs::write(&path, svg).expect("the sheet is written");
        eprintln!("drew every part into {path}");
    }

    /// The lamp's long leg is its anode, as it is in the bag: which way
    /// round the part goes is visible without reading anything.
    #[test]
    fn the_lamps_anode_is_the_long_leg_and_both_point_down() {
        let led = named("LED");
        let plan = layout(&led);
        let anode = plan.spots.iter().find(|s| s.name == "A").expect("an anode");
        let cathode = plan
            .spots
            .iter()
            .find(|s| s.name == "K")
            .expect("a cathode");
        assert!(anode.at.1 > cathode.at.1, "{anode:?} {cathode:?}");
        assert_eq!(anode.out, (0.0, 1.0));
        assert_eq!(cathode.out, (0.0, 1.0));
        assert!(plan.lens.is_some(), "a lamp has a lens the view lights");

        // KiCad's own diode order — pin 1 the cathode — where the library
        // names neither pin.
        let mut plain = led.clone();
        plain.pins[0].name = "~".into();
        plain.pins[1].name = "~".into();
        let plan = layout(&plain);
        let one = plan.spot("1").expect("pin 1");
        let two = plan.spot("2").expect("pin 2");
        assert!(two.at.1 > one.at.1, "pin 2 is the anode, so the long leg");
    }

    #[test]
    fn a_resistor_lies_across_the_sheet_and_wears_the_bands_of_its_value() {
        let plan = layout(&named("R"));
        assert_eq!(plan.spots[0].out, (-1.0, 0.0));
        assert_eq!(plan.spots[1].out, (1.0, 0.0));
        assert_eq!(
            plan.spots[0].at.1, 0.0,
            "the leads are in line with the body"
        );

        // 220 Ω: red, red, brown — the part in the drawer.
        let red = "#c0392b";
        let brown = "#7a4a1e";
        assert_eq!(&bands("220")[..3], &[red, red, brown]);
        assert_eq!(&bands("220R")[..3], &[red, red, brown]);
        // 10k: brown, black, orange.
        assert_eq!(&bands("10k")[..3], &["#7a4a1e", "#1b1b1b", "#d97b26"]);
        assert_eq!(bands("10K"), bands("10000"));
        assert_eq!(bands("4k7"), bands("4.7k"));
        // 1M: brown, black, green.
        assert_eq!(bands("1M")[2], "#3f9d54");
    }

    #[test]
    fn an_unreadable_value_wears_grey_rather_than_a_number_nobody_gave() {
        let grey = vec!["#6f7885", "#6f7885", "#6f7885"];
        assert_eq!(bands(""), grey);
        assert_eq!(bands("big one"), grey);
        assert_eq!(bands("R"), grey);
    }

    /// A part rusty has never heard of is a package: pins down the two
    /// sides, the box growing with the pin count, its name on the body.
    #[test]
    fn an_unknown_part_is_drawn_as_a_package_with_its_pins_down_both_sides() {
        let mut chip = named("Display");
        chip.library = "lcsc".into();
        chip.name = "C82891".into();
        chip.reference = "U".into();
        chip.pins = (1..=8)
            .map(|n| Pin {
                number: n.to_string(),
                name: format!("IO{n}"),
                kind: rusty_embed::PinKind::Bidirectional,
                at: (0.0, 0.0),
                length: 2.54,
                angle: 0,
                hidden: false,
            })
            .collect();
        let plan = layout(&chip);
        let left = plan.spots.iter().filter(|s| s.out.0 < 0.0).count();
        assert_eq!(left, 4, "half the pins down each side");
        assert_eq!(
            plan.spots[0].at.1, plan.spots[7].at.1,
            "1 faces 8, as a DIP is numbered"
        );
        let (_, y0, _, y1) = plan.bounds;
        assert!(
            y1 - y0 >= 48.0,
            "the body grows with the pins: {:?}",
            plan.bounds
        );
        assert!(
            markup(&chip, "").contains("C82891"),
            "the package wears its name"
        );

        // Two pins and nothing known: an axial body, not a chip.
        let mut two = chip.clone();
        two.pins.truncate(2);
        let plan = layout(&two);
        assert_eq!(plan.spots[0].out, (-1.0, 0.0));
        assert_eq!(plan.spots[1].out, (1.0, 0.0));
    }

    /// The devkit keeps the header's own geometry — it is already a drawing
    /// to scale, and its pins are rows on it.
    #[test]
    fn the_devkit_keeps_its_row_geometry() {
        let rows = rusty_embed::nets::kit_rows("esp32c3", &[0, 1, 2, 3, 4, 5]);
        let kit = kit_symbol("esp32c3", &rows);
        let plan = layout(&kit);
        assert_eq!(plan.spots.len(), rows.len());
        let first = &plan.spots[0];
        assert_eq!(first.at, local(kit.pins[0].at));
        assert_eq!(
            first.out,
            (-1.0, 0.0),
            "the left column's wires leave leftward"
        );
        assert!(markup(&kit, "").is_empty(), "the board is drawn by kit_art");
        assert!(
            local((1.0, 0.0)).0 == MM_PX,
            "one millimetre is one step of the scale"
        );
    }
}
