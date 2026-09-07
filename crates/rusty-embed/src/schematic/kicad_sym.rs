//! KiCad's symbol library format, `.kicad_sym` — the S-expression files of
//! KiCad 6, 7 and 8 — read into [`Symbol`]s.
//!
//! Read, never trusted: a node this reader does not know is skipped, a
//! number that does not parse is skipped with its node, and a pin without a
//! number is refused with the symbol's name, because a pin nothing can wire
//! to is a symbol that would look fine and never work. A derived symbol
//! (`extends`) is skipped whole rather than half-read.
//!
//! The coordinates come through as KiCad keeps them — millimetres, y up.

use crate::model::{Fill, Graphic, Pin, PinKind, Symbol};

/// Why a library could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub detail: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.detail)
    }
}

impl std::error::Error for ParseError {}

/// One S-expression node.
#[derive(Debug, Clone, PartialEq)]
enum Sx {
    List(Vec<Sx>),
    /// A bare token: a keyword, a number, `hide`.
    Atom(String),
    /// A double-quoted string, unescaped.
    Str(String),
}

impl Sx {
    fn head(&self) -> Option<&str> {
        match self {
            Sx::List(items) => match items.first() {
                Some(Sx::Atom(name)) => Some(name),
                _ => None,
            },
            _ => None,
        }
    }

    fn items(&self) -> &[Sx] {
        match self {
            Sx::List(items) => items,
            _ => &[],
        }
    }

    /// The `n`th element as text, whichever way it was written.
    fn text(&self, n: usize) -> Option<&str> {
        match self.items().get(n)? {
            Sx::Atom(s) | Sx::Str(s) => Some(s),
            Sx::List(_) => None,
        }
    }

    fn number(&self, n: usize) -> Option<f64> {
        self.text(n)?.parse().ok()
    }

    /// The first child list headed `name`.
    fn child(&self, name: &str) -> Option<&Sx> {
        self.items().iter().find(|item| item.head() == Some(name))
    }

    fn children<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Sx> + 'a {
        self.items()
            .iter()
            .filter(move |item| item.head() == Some(name))
    }
}

/// Tokenise and nest the whole text. Strings keep their escapes resolved;
/// everything else is an atom.
fn read(text: &str) -> Result<Sx, ParseError> {
    let mut stack: Vec<Vec<Sx>> = vec![Vec::new()];
    let mut chars = text.char_indices().peekable();
    let mut line = 1usize;
    while let Some((_, c)) = chars.next() {
        match c {
            '\n' => line += 1,
            c if c.is_whitespace() => {}
            '(' => stack.push(Vec::new()),
            ')' => {
                let list = stack.pop().ok_or_else(|| ParseError {
                    detail: format!("line {line}: a `)` with nothing open"),
                })?;
                match stack.last_mut() {
                    Some(parent) => parent.push(Sx::List(list)),
                    None => {
                        return Err(ParseError {
                            detail: format!("line {line}: a `)` closing the file itself"),
                        });
                    }
                }
            }
            '"' => {
                let mut s = String::new();
                loop {
                    match chars.next() {
                        Some((_, '"')) => break,
                        Some((_, '\\')) => match chars.next() {
                            Some((_, 'n')) => s.push('\n'),
                            Some((_, 't')) => s.push('\t'),
                            Some((_, other)) => s.push(other),
                            None => break,
                        },
                        Some((_, '\n')) => {
                            line += 1;
                            s.push('\n');
                        }
                        Some((_, other)) => s.push(other),
                        None => {
                            return Err(ParseError {
                                detail: format!("line {line}: an unterminated string"),
                            });
                        }
                    }
                }
                stack
                    .last_mut()
                    .expect("the root list is always open")
                    .push(Sx::Str(s));
            }
            first => {
                let mut atom = String::from(first);
                while let Some((_, next)) = chars.peek() {
                    if next.is_whitespace() || *next == '(' || *next == ')' {
                        break;
                    }
                    atom.push(*next);
                    chars.next();
                }
                stack
                    .last_mut()
                    .expect("the root list is always open")
                    .push(Sx::Atom(atom));
            }
        }
    }
    if stack.len() != 1 {
        return Err(ParseError {
            detail: format!("{} list(s) never closed", stack.len() - 1),
        });
    }
    Ok(Sx::List(stack.pop().unwrap_or_default()))
}

/// Every symbol in a library file, in file order, labelled with `library`.
pub fn parse(library: &str, text: &str) -> Result<Vec<Symbol>, ParseError> {
    let root = read(text)?;
    let lib = root
        .items()
        .iter()
        .find(|item| item.head() == Some("kicad_symbol_lib"))
        .ok_or_else(|| ParseError {
            detail: "not a KiCad symbol library: no `kicad_symbol_lib` at the top".to_string(),
        })?;
    let mut symbols = Vec::new();
    for node in lib.children("symbol") {
        let Some(name) = node.text(1) else {
            continue;
        };
        // A derived symbol carries only what differs from its parent; reading
        // it alone would produce a symbol with no body.
        if node.child("extends").is_some() {
            continue;
        }
        symbols.push(symbol(library, name, node)?);
    }
    Ok(symbols)
}

fn symbol(library: &str, name: &str, node: &Sx) -> Result<Symbol, ParseError> {
    let property = |key: &str| -> Option<String> {
        node.children("property")
            .find(|p| p.text(1) == Some(key))
            .and_then(|p| p.text(2))
            .map(str::to_string)
    };
    let mut pins = Vec::new();
    let mut graphics = Vec::new();
    // Bodies and pins live in unit sub-symbols (`R_0_1`, `R_1_1`), and KiCad
    // allows them directly in the symbol as well; both are read.
    let mut holders: Vec<&Sx> = vec![node];
    holders.extend(node.children("symbol"));
    for holder in holders {
        for item in holder.items() {
            match item.head() {
                Some("pin") => pins.push(pin(name, item)?),
                Some("rectangle") => {
                    if let (Some(start), Some(end)) = (point(item, "start"), point(item, "end")) {
                        graphics.push(Graphic::Rectangle {
                            start,
                            end,
                            width: stroke_width(item),
                            fill: fill(item),
                        });
                    }
                }
                Some("circle") => {
                    if let (Some(center), Some(radius)) = (
                        point(item, "center"),
                        item.child("radius").and_then(|r| r.number(1)),
                    ) {
                        graphics.push(Graphic::Circle {
                            center,
                            radius,
                            width: stroke_width(item),
                            fill: fill(item),
                        });
                    }
                }
                Some("arc") => {
                    if let (Some(start), Some(mid), Some(end)) =
                        (point(item, "start"), point(item, "mid"), point(item, "end"))
                    {
                        graphics.push(Graphic::Arc {
                            start,
                            mid,
                            end,
                            width: stroke_width(item),
                            fill: fill(item),
                        });
                    }
                }
                Some("polyline") => {
                    let points: Vec<(f64, f64)> = item
                        .child("pts")
                        .map(|pts| {
                            pts.children("xy")
                                .filter_map(|xy| Some((xy.number(1)?, xy.number(2)?)))
                                .collect()
                        })
                        .unwrap_or_default();
                    if points.len() >= 2 {
                        graphics.push(Graphic::Polyline {
                            points,
                            width: stroke_width(item),
                            fill: fill(item),
                        });
                    }
                }
                Some("text") => {
                    if let (Some(text), Some(at)) = (item.text(1), item.child("at")) {
                        graphics.push(Graphic::Text {
                            text: text.to_string(),
                            at: (at.number(1).unwrap_or(0.0), at.number(2).unwrap_or(0.0)),
                            size: item
                                .child("effects")
                                .and_then(|e| e.child("font"))
                                .and_then(|f| f.child("size"))
                                .and_then(|s| s.number(1))
                                .unwrap_or(1.27),
                            angle: at.number(3).unwrap_or(0.0),
                        });
                    }
                }
                _ => {}
            }
        }
    }
    let reference = property("Reference").unwrap_or_else(|| "U".to_string());
    Ok(Symbol {
        library: library.to_string(),
        name: name.to_string(),
        value: property("Value").unwrap_or_else(|| name.to_string()),
        description: property("Description").filter(|d| !d.is_empty()),
        reference,
        pins,
        graphics,
    })
}

fn pin(symbol: &str, node: &Sx) -> Result<Pin, ParseError> {
    let number = node
        .child("number")
        .and_then(|n| n.text(1))
        .map(str::to_string)
        .ok_or_else(|| ParseError {
            detail: format!("symbol {symbol}: a pin without a number — nothing could wire to it"),
        })?;
    let at = node.child("at").ok_or_else(|| ParseError {
        detail: format!("symbol {symbol}: pin {number} has no position"),
    })?;
    let kind = match node.text(1) {
        Some("passive") => PinKind::Passive,
        Some("input") => PinKind::Input,
        Some("output") => PinKind::Output,
        Some("bidirectional") => PinKind::Bidirectional,
        Some("power_in") => PinKind::PowerIn,
        Some("power_out") => PinKind::PowerOut,
        Some("open_collector") => PinKind::OpenCollector,
        Some("tri_state") => PinKind::Tristate,
        Some("no_connect") => PinKind::NoConnect,
        _ => PinKind::Unspecified,
    };
    // KiCad 8 writes `(hide yes)`; 6 and 7 a bare `hide` atom.
    let hidden = node
        .items()
        .iter()
        .any(|i| matches!(i, Sx::Atom(a) if a == "hide"))
        || node
            .child("hide")
            .and_then(|h| h.text(1))
            .is_some_and(|v| v == "yes");
    Ok(Pin {
        name: node
            .child("name")
            .and_then(|n| n.text(1))
            .unwrap_or("~")
            .to_string(),
        number,
        kind,
        at: (at.number(1).unwrap_or(0.0), at.number(2).unwrap_or(0.0)),
        length: node
            .child("length")
            .and_then(|l| l.number(1))
            .unwrap_or(2.54),
        angle: at.number(3).unwrap_or(0.0).round().rem_euclid(360.0) as u16,
        hidden,
    })
}

fn point(node: &Sx, name: &str) -> Option<(f64, f64)> {
    let p = node.child(name)?;
    Some((p.number(1)?, p.number(2)?))
}

fn stroke_width(node: &Sx) -> f64 {
    node.child("stroke")
        .and_then(|s| s.child("width"))
        .and_then(|w| w.number(1))
        .unwrap_or(0.0)
}

fn fill(node: &Sx) -> Fill {
    match node
        .child("fill")
        .and_then(|f| f.child("type"))
        .and_then(|t| t.text(1))
    {
        Some("outline") => Fill::Outline,
        Some("background") => Fill::Background,
        _ => Fill::None,
    }
}

/// A library file holding `symbols`, in the shape KiCad 8 writes and every
/// KiCad since 6 reads — so an imported part can be opened in KiCad's own
/// editor, and so the cache is a library like any other. `parse` reads it
/// back to the same symbols, which is what the round-trip test holds it to.
pub fn write(symbols: &[Symbol]) -> String {
    let mut out = String::from("(kicad_symbol_lib (version 20231120) (generator \"rusty\")\n");
    for symbol in symbols {
        out.push_str(&format!(
            "  (symbol {} (pin_names (offset 1.016)) (exclude_from_sim no) (in_bom yes) (on_board yes)\n",
            quote(&symbol.name)
        ));
        property(&mut out, "Reference", &symbol.reference, (0.0, 2.54), false);
        property(&mut out, "Value", &symbol.value, (0.0, -2.54), false);
        property(&mut out, "Footprint", "", (0.0, 0.0), true);
        property(&mut out, "Datasheet", "", (0.0, 0.0), true);
        property(
            &mut out,
            "Description",
            symbol.description.as_deref().unwrap_or(""),
            (0.0, 0.0),
            true,
        );
        out.push_str(&format!(
            "    (symbol {}\n",
            quote(&format!("{}_0_1", symbol.name))
        ));
        for graphic in &symbol.graphics {
            out.push_str("      ");
            graphic_line(&mut out, graphic);
            out.push('\n');
        }
        out.push_str("    )\n");
        out.push_str(&format!(
            "    (symbol {}\n",
            quote(&format!("{}_1_1", symbol.name))
        ));
        for pin in &symbol.pins {
            let kind = match pin.kind {
                PinKind::Passive => "passive",
                PinKind::Input => "input",
                PinKind::Output => "output",
                PinKind::Bidirectional => "bidirectional",
                PinKind::PowerIn => "power_in",
                PinKind::PowerOut => "power_out",
                PinKind::OpenCollector => "open_collector",
                PinKind::Tristate => "tri_state",
                PinKind::NoConnect => "no_connect",
                PinKind::Unspecified => "unspecified",
            };
            out.push_str(&format!(
                "      (pin {kind} line (at {} {} {}) (length {}) (name {} (effects (font (size 1.27 1.27)))) (number {} (effects (font (size 1.27 1.27)))){})\n",
                num(pin.at.0),
                num(pin.at.1),
                pin.angle,
                num(pin.length),
                quote(&pin.name),
                quote(&pin.number),
                if pin.hidden { " (hide yes)" } else { "" }
            ));
        }
        out.push_str("    )\n  )\n");
    }
    out.push(')');
    out.push('\n');
    out
}

fn property(out: &mut String, key: &str, value: &str, at: (f64, f64), hidden: bool) {
    out.push_str(&format!(
        "    (property {} {} (at {} {} 0) (effects (font (size 1.27 1.27)){}))\n",
        quote(key),
        quote(value),
        num(at.0),
        num(at.1),
        if hidden { " (hide yes)" } else { "" }
    ));
}

fn graphic_line(out: &mut String, graphic: &Graphic) {
    let paint = |width: f64, fill: Fill| {
        let fill = match fill {
            Fill::None => "none",
            Fill::Outline => "outline",
            Fill::Background => "background",
        };
        format!(
            "(stroke (width {}) (type default)) (fill (type {fill}))",
            num(width)
        )
    };
    let xy = |(x, y): (f64, f64)| format!("{} {}", num(x), num(y));
    match graphic {
        Graphic::Polyline {
            points,
            width,
            fill,
        } => {
            out.push_str("(polyline (pts");
            for point in points {
                out.push_str(&format!(" (xy {})", xy(*point)));
            }
            out.push_str(&format!(") {})", paint(*width, *fill)));
        }
        Graphic::Rectangle {
            start,
            end,
            width,
            fill,
        } => out.push_str(&format!(
            "(rectangle (start {}) (end {}) {})",
            xy(*start),
            xy(*end),
            paint(*width, *fill)
        )),
        Graphic::Circle {
            center,
            radius,
            width,
            fill,
        } => out.push_str(&format!(
            "(circle (center {}) (radius {}) {})",
            xy(*center),
            num(*radius),
            paint(*width, *fill)
        )),
        Graphic::Arc {
            start,
            mid,
            end,
            width,
            fill,
        } => out.push_str(&format!(
            "(arc (start {}) (mid {}) (end {}) {})",
            xy(*start),
            xy(*mid),
            xy(*end),
            paint(*width, *fill)
        )),
        Graphic::Text {
            text,
            at,
            size,
            angle,
        } => out.push_str(&format!(
            "(text {} (at {} {}) (effects (font (size {} {}))))",
            quote(text),
            xy(*at),
            num(*angle),
            num(*size),
            num(*size)
        )),
    }
}

/// A number as KiCad writes one: no exponent, no trailing zeros, and never
/// `-0`.
fn num(v: f64) -> String {
    if v == 0.0 {
        "0".to_string()
    } else {
        format!("{v}")
    }
}

fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The built-in library is the fixture: the four parts every first
    /// circuit has, in the shape KiCad's own Device library draws them.
    const BUILTIN: &str = include_str!("../../data/symbols/Device.kicad_sym");

    #[test]
    fn the_builtin_library_reads_as_four_symbols_with_their_pins() {
        let symbols = parse("Device", BUILTIN).expect("parses");
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["R", "C", "LED", "SW_Push"]);

        let r = &symbols[0];
        assert_eq!((r.reference.as_str(), r.value.as_str()), ("R", "R"));
        assert_eq!(r.description.as_deref(), Some("Resistor"));
        assert_eq!(r.pins.len(), 2);
        assert_eq!(r.pins[0].at, (0.0, 3.81));
        assert_eq!(r.pins[0].angle, 270);
        assert_eq!(r.pins[0].length, 1.27);
        assert_eq!(r.pins[0].kind, PinKind::Passive);
        assert!(matches!(r.graphics[0], Graphic::Rectangle { width, .. } if width == 0.254));

        let led = &symbols[2];
        assert_eq!(led.reference, "D");
        assert_eq!(led.pin("K").map(|p| p.number.as_str()), Some("1"));
        assert_eq!(led.pin("A").map(|p| p.at), Some((3.81, 0.0)));
        assert_eq!(
            led.graphics
                .iter()
                .filter(|g| matches!(g, Graphic::Polyline { .. }))
                .count(),
            5,
            "body, bar, and the two arrows"
        );

        let sw = &symbols[3];
        assert_eq!(
            sw.graphics
                .iter()
                .filter(|g| matches!(g, Graphic::Circle { radius, .. } if *radius == 0.508))
                .count(),
            2
        );
        assert_eq!(sw.pins[1].angle, 180);
    }

    #[test]
    fn strings_keep_their_escapes_and_unknown_nodes_are_skipped() {
        let text = r#"(kicad_symbol_lib (version 20231120) (generator "kicad_symbol_editor")
          (symbol "X" (property "Reference" "U" (at 0 0 0)) (property "Value" "say \"hi\"" (at 0 0 0))
            (something_new (with 1 2 3))
            (symbol "X_1_1"
              (text "label" (at 1 2 90) (effects (font (size 2 2))))
              (pin input line (at -5.08 0 0) (length 2.54) (name "IN") (number "1") (hide yes))
              (pin no_connect line (at 5.08 0 180) (length 2.54) (name "NC") (number "2") hide))))"#;
        let symbols = parse("test", text).expect("parses");
        let x = &symbols[0];
        assert_eq!(x.value, "say \"hi\"");
        assert_eq!(x.pins.len(), 2);
        assert!(
            x.pins[0].hidden && x.pins[1].hidden,
            "both spellings of hide"
        );
        assert_eq!(x.pins[0].kind, PinKind::Input);
        assert_eq!(x.pins[1].kind, PinKind::NoConnect);
        assert!(
            matches!(&x.graphics[0], Graphic::Text { text, size, angle, .. }
            if text == "label" && *size == 2.0 && *angle == 90.0)
        );
    }

    #[test]
    fn a_pin_without_a_number_is_refused_by_symbol_name() {
        let text = r#"(kicad_symbol_lib (symbol "Broken" (symbol "Broken_1_1"
            (pin passive line (at 0 0 0) (length 2.54) (name "A")))))"#;
        let error = parse("test", text).expect_err("refused");
        assert!(error.detail.contains("Broken"), "{error}");
        assert!(error.detail.contains("without a number"), "{error}");
    }

    #[test]
    fn a_derived_symbol_is_skipped_and_a_broken_file_names_the_line() {
        let text = r#"(kicad_symbol_lib
            (symbol "Base" (symbol "Base_1_1" (pin passive line (at 0 0 0) (length 1) (name "~") (number "1"))))
            (symbol "Derived" (extends "Base")))"#;
        let symbols = parse("test", text).expect("parses");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Base");

        let error = parse("test", "(kicad_symbol_lib\n(symbol \"A\"\n").expect_err("unclosed");
        assert!(error.detail.contains("never closed"), "{error}");
        let error = parse("test", "(not_a_library)").expect_err("not a library");
        assert!(error.detail.contains("kicad_symbol_lib"), "{error}");
    }
}

#[cfg(test)]
mod write_tests {
    use super::*;

    /// Written and read back, the built-in library is the same library —
    /// every field, because the fixture sets every field: a hidden pin, a
    /// description, an arc and a text are added to make sure.
    #[test]
    fn a_library_survives_the_round_trip_through_the_writer() {
        let mut symbols = parse(
            "Device",
            include_str!("../../data/symbols/Device.kicad_sym"),
        )
        .unwrap();
        symbols[0].pins[1].hidden = true;
        symbols[0].pins[1].kind = PinKind::PowerIn;
        symbols[0].value = "say \"10k\" \\ ohm".to_string();
        symbols[1].description = None;
        symbols[2].graphics.push(Graphic::Arc {
            start: (0.0, 1.0),
            mid: (0.75, 0.66),
            end: (1.0, 0.0),
            width: 0.1,
            fill: Fill::Outline,
        });
        symbols[3].graphics.push(Graphic::Text {
            text: "press".into(),
            at: (-1.5, 2.0),
            size: 0.8,
            angle: 90.0,
        });
        let text = write(&symbols);
        let again = parse("Device", &text).unwrap();
        assert_eq!(again, symbols);
        assert!(
            text.contains("(pin power_in line (at 0 -3.81 90)"),
            "{text}"
        );
        assert!(text.contains("(hide yes)"), "{text}");
        assert!(!text.contains("-0 "), "no negative zero: {text}");
    }
}
