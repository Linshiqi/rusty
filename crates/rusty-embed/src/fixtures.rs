//! The sheet a test draws: a two-pin passive part, a placed instance, a
//! wire. One copy, where six test modules — the circuit, the period, the
//! live loop, the generator, the nets and the board's editor — each kept
//! their own `pin`, `symbol`, `place` and `wire`, byte for byte or with one
//! number changed. A fixture copied six times is a fixture one copy drifts
//! from, and a sheet two tests draw slightly differently is two tests that
//! cannot be compared.
//!
//! Where the copies differed is a [`Shape`]: how far from the origin the
//! pins sit, whether they alternate sides or only the first sits left,
//! whether the right-hand pins point into the body, and what body is drawn.
//! Every test keeps the geometry it had by naming its shape.

use std::collections::BTreeMap;

use crate::model::{Fill, Graphic, Instance, Pin, PinKind, PinRef, Sheet, Symbol, Wire};

/// How a test symbol's pins and body are laid out.
pub(crate) struct Shape {
    /// How far from the origin a pin sits, along the horizontal.
    pub offset: f64,
    /// Pins alternate left and right (`true`), or only the first sits left.
    pub alternate: bool,
    /// A right-hand pin points into the body (angle 180), as KiCad's do.
    pub angled: bool,
    /// The body drawn, or none.
    pub body: Option<Graphic>,
}

impl Default for Shape {
    fn default() -> Shape {
        Shape {
            offset: 2.54,
            alternate: false,
            angled: true,
            body: Some(rect((-1.0, 1.0), (1.0, -1.0), 0.0)),
        }
    }
}

/// An unfilled rectangle body.
pub(crate) fn rect(start: (f64, f64), end: (f64, f64), width: f64) -> Graphic {
    Graphic::Rectangle {
        start,
        end,
        width,
        fill: Fill::None,
    }
}

/// A passive pin at `x` on the horizontal, pointing into the body: angle 0
/// left of the origin, 180 right of it.
pub(crate) fn pin(number: &str, name: &str, x: f64) -> Pin {
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

/// A symbol with the default [`Shape`].
pub(crate) fn symbol(library: &str, name: &str, reference: &str, pins: &[(&str, &str)]) -> Symbol {
    symbol_shaped(library, name, reference, pins, &Shape::default())
}

/// A symbol whose pins are `(number, name)` pairs laid out by `shape`; its
/// value is its name.
pub(crate) fn symbol_shaped(
    library: &str,
    name: &str,
    reference: &str,
    pins: &[(&str, &str)],
    shape: &Shape,
) -> Symbol {
    Symbol {
        library: library.into(),
        name: name.into(),
        reference: reference.into(),
        value: name.into(),
        description: None,
        pins: pins
            .iter()
            .enumerate()
            .map(|(i, (number, pin_name))| {
                let left = if shape.alternate { i % 2 == 0 } else { i == 0 };
                let x = if left { -shape.offset } else { shape.offset };
                let mut pin = pin(number, pin_name, x);
                if !shape.angled {
                    pin.angle = 0;
                }
                pin
            })
            .collect(),
        graphics: shape.body.clone().into_iter().collect(),
    }
}

/// An instance at the origin with `props`.
pub(crate) fn place_with(
    sheet: &mut Sheet,
    reference: &str,
    symbol: &str,
    value: &str,
    props: &[(&str, &str)],
) {
    sheet.parts.push(Instance {
        reference: reference.into(),
        symbol: symbol.into(),
        value: value.into(),
        x: 0.0,
        y: 0.0,
        rot: 0,
        mirror: false,
        props: props
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<BTreeMap<String, String>>(),
    });
}

/// An instance at the origin with no props.
pub(crate) fn place(sheet: &mut Sheet, reference: &str, symbol: &str, value: &str) {
    place_with(sheet, reference, symbol, value, &[]);
}

/// A wire between two pins named `R1.1`-style, with no bends.
pub(crate) fn wire(sheet: &mut Sheet, from: &str, to: &str) {
    sheet.wires.push(Wire {
        from: PinRef::parse(from).unwrap(),
        to: PinRef::parse(to).unwrap(),
        bends: Vec::new(),
    });
}
