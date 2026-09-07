//! EasyEDA's component service, read into a [`Symbol`] by LCSC part number.
//!
//! 嘉立创 (LCSC) sells the parts and EasyEDA (立创EDA) draws them; its
//! service answers `api/products/<C-number>/components` with the symbol in
//! EasyEDA's own schematic format. `dataStr.shape` is a list of records,
//! each a `~`-separated line — `P` a pin, `R` a rectangle, `PL`/`PG` a
//! polyline or polygon, `E` an ellipse, `A` an arc, `PT` an SVG path, `T`
//! text — in a coordinate system of 10 mil per unit, y pointing down, with
//! the symbol's anchor at `dataStr.head.x/y`. [`parse`] turns one answer
//! into a symbol in KiCad's millimetres, y up; [`fetch`] asks the service
//! over rusty's proxy ladder; [`import`] does both and keeps the result in
//! the data directory's `symbols/lcsc.kicad_sym`, one library the sheet
//! reads like any other and KiCad can open.
//!
//! What it never does is stand in for a part it could not read. An unknown
//! number is refused with the service's own words, and a record the reader
//! does not know is listed in `warnings` rather than dropped in silence —
//! a symbol missing part of its body would read as a bug in the sheet.

use serde_json::Value;

use super::kicad_sym::{self, ParseError};
use crate::error::{Error, Result};
use crate::model::{Fill, Graphic, Pin, PinKind, Symbol};

/// One EasyEDA unit in millimetres: 10 mil.
const UNIT_MM: f64 = 0.254;
/// One point of EasyEDA's font sizes, in millimetres.
const POINT_MM: f64 = 0.3528;
/// The library every imported part lands in: `lcsc:C2286`.
pub const LIBRARY: &str = "lcsc";
/// The two hosts that answer the same service; the second is the one
/// reachable from inside China when the first is not.
const HOSTS: &[&str] = &["https://easyeda.com", "https://lceda.cn"];

/// A symbol read from the service, and what the reader had to leave out.
#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    pub symbol: Symbol,
    /// One line per record that was skipped, naming it.
    pub warnings: Vec<String>,
}

/// `C2286` from `c2286`, ` C2286 `, or the full number as LCSC prints it;
/// refused for anything that is not a C-number, because the service
/// answers a bad number with an HTML page.
pub fn part_number(text: &str) -> Result<String> {
    let trimmed = text.trim();
    let digits = trimmed
        .strip_prefix(['C', 'c'])
        .filter(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()));
    match digits {
        Some(digits) => Ok(format!("C{digits}")),
        None => Err(Error::refused(format!(
            "`{trimmed}` is not an LCSC part number — one looks like C2286, the number beside every part on lcsc.com"
        ))),
    }
}

/// The service's URL for a part, on `host`.
pub fn url(host: &str, lcsc: &str) -> String {
    format!("{host}/api/products/{lcsc}/components?version=6.4.19.5")
}

/// The raw answer for `lcsc`, from the first host and route that delivers.
pub fn fetch(lcsc: &str) -> Result<String> {
    let mut last = "no route to the service".to_string();
    for route in crate::net::proxy_routes() {
        let agent = crate::net::agent(
            route.as_deref(),
            crate::net::Deadlines {
                connect: std::time::Duration::from_secs(10),
                headers: None,
                total: std::time::Duration::from_secs(30),
            },
        );
        for host in HOSTS {
            match agent
                .get(&url(host, lcsc))
                .header("User-Agent", "rusty-workbench")
                .header("Accept", "application/json")
                .call()
            {
                Ok(mut response) => match response.body_mut().read_to_string() {
                    Ok(body) => return Ok(body),
                    Err(error) => last = format!("{host}: could not read the answer: {error}"),
                },
                Err(error) => {
                    last = format!("{host}: {}", crate::net::error_chain(&error));
                }
            }
        }
    }
    Err(Error::Download {
        detail: format!("could not reach EasyEDA's component service for {lcsc} — {last}"),
    })
}

/// Fetch, read, and keep in the cache library. The symbol is returned even
/// when the cache could not be written; the failure travels in `warnings`.
pub fn import(lcsc: &str) -> Result<Imported> {
    let lcsc = part_number(lcsc)?;
    let body = fetch(&lcsc)?;
    let mut imported = parse(&lcsc, &body).map_err(|e| Error::Refused { detail: e.detail })?;
    match cache(&imported.symbol) {
        Ok(path) => imported.warnings.push(format!(
            "kept as {}:{} in {}",
            LIBRARY,
            lcsc,
            path.display()
        )),
        Err(error) => imported.warnings.push(format!("not cached: {error}")),
    }
    Ok(imported)
}

/// Write `symbol` into the cache library, replacing an earlier import of
/// the same part. A cache file that no longer parses is moved aside as
/// `.broken` rather than overwritten — a read that degrades to empty in
/// front of a read-modify-write is how a library of imports vanishes.
fn cache(symbol: &Symbol) -> Result<std::path::PathBuf> {
    let dir = super::cache_dir().ok_or_else(|| Error::Config {
        detail: "no data directory to keep the imported symbol in".to_string(),
    })?;
    std::fs::create_dir_all(&dir).map_err(|source| Error::Write {
        path: dir.display().to_string(),
        source,
    })?;
    let path = dir.join(format!("{LIBRARY}.kicad_sym"));
    let mut symbols = match std::fs::read_to_string(&path) {
        Ok(text) => match kicad_sym::parse(LIBRARY, &text) {
            Ok(symbols) => symbols,
            Err(error) => {
                let broken = path.with_extension("kicad_sym.broken");
                eprintln!(
                    "{} did not parse ({error}); moved to {}",
                    path.display(),
                    broken.display()
                );
                std::fs::rename(&path, &broken).map_err(|source| Error::Write {
                    path: broken.display().to_string(),
                    source,
                })?;
                Vec::new()
            }
        },
        Err(_) => Vec::new(),
    };
    match symbols.iter_mut().find(|s| s.name == symbol.name) {
        Some(slot) => *slot = symbol.clone(),
        None => symbols.push(symbol.clone()),
    }
    let tmp = dir.join(format!("{LIBRARY}.kicad_sym.{}.tmp", std::process::id()));
    std::fs::write(&tmp, kicad_sym::write(&symbols)).map_err(|source| Error::Write {
        path: tmp.display().to_string(),
        source,
    })?;
    std::fs::rename(&tmp, &path).map_err(|source| Error::Write {
        path: path.display().to_string(),
        source,
    })?;
    Ok(path)
}

/// The symbol in one answer of the service, for the part `lcsc`.
pub fn parse(lcsc: &str, body: &str) -> std::result::Result<Imported, ParseError> {
    let value: Value = serde_json::from_str(body).map_err(|e| ParseError {
        detail: format!("EasyEDA's answer for {lcsc} is not JSON ({e})"),
    })?;
    let result = match value.get("result") {
        Some(result) if !result.is_null() && result.is_object() => result,
        _ => {
            let said = value
                .get("message")
                .and_then(Value::as_str)
                .filter(|m| !m.is_empty())
                .map(|m| format!(" — the service said: {m}"))
                .unwrap_or_default();
            return Err(ParseError {
                detail: format!("EasyEDA has no symbol for {lcsc}{said}"),
            });
        }
    };
    // Older answers carry `dataStr` as a JSON string inside the JSON.
    let owned_data;
    let data = match result.get("dataStr") {
        Some(Value::String(text)) => {
            owned_data = serde_json::from_str::<Value>(text).map_err(|e| ParseError {
                detail: format!("{lcsc}: dataStr is not JSON ({e})"),
            })?;
            &owned_data
        }
        Some(data) if data.is_object() => data,
        _ => {
            return Err(ParseError {
                detail: format!("{lcsc}: the answer carries no `dataStr` — not a symbol"),
            });
        }
    };
    let head = data.get("head").unwrap_or(&Value::Null);
    let frame = Frame {
        x: number_field(head, "x").unwrap_or(0.0),
        y: number_field(head, "y").unwrap_or(0.0),
    };
    let para = |key: &str| -> Option<String> {
        head.get("c_para")
            .and_then(|p| p.get(key))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let title = result
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let reference = para("pre")
        .map(|p| p.trim_end_matches('?').to_string())
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| "U".to_string());
    // LCSC's own library puts the electrical value (`10kΩ`, `100nF`) in
    // `Value` and the manufacturer's part number in `name`; a symbol drawn
    // by somebody else may have only the name. The rest goes into the
    // description in the order a hover would want it.
    let value_text = para("Value")
        .or_else(|| para("name"))
        .or_else(|| title.clone())
        .unwrap_or_else(|| lcsc.to_string());
    let mut description: Vec<String> = Vec::new();
    if let Some(said) = result
        .get("description")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        description.push(said.to_string());
    }
    if let Some(title) = title.filter(|t| *t != value_text) {
        description.push(title);
    }
    if let Some(package) = para("package") {
        description.push(package);
    }
    if let Some(part) =
        para("Manufacturer Part").filter(|p| *p != value_text && !description.contains(p))
    {
        description.push(part);
    }

    let shapes: Vec<&str> = data
        .get("shape")
        .and_then(Value::as_array)
        .map(|records| records.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let mut pins = Vec::new();
    let mut graphics = Vec::new();
    let mut warnings = Vec::new();
    for record in shapes {
        let tag = record.split('~').next().unwrap_or("");
        match tag {
            "P" => match pin(record, &frame) {
                Ok(pin) => pins.push(pin),
                Err(detail) => {
                    return Err(ParseError {
                        detail: format!("{lcsc}: {detail}"),
                    });
                }
            },
            "R" | "E" | "PL" | "PG" | "A" | "PT" | "T" => match graphic(record, &frame) {
                Ok(mut read) => graphics.append(&mut read),
                Err(detail) => warnings.push(format!("skipped a {tag} record: {detail}")),
            },
            // Schematic-level things a component's drawing never needs.
            "W" | "B" | "F" | "J" | "N" | "I" | "O" | "BE" => {}
            other => warnings.push(format!(
                "skipped a `{other}` record the reader does not know: {}",
                record.chars().take(60).collect::<String>()
            )),
        }
    }
    if pins.is_empty() {
        return Err(ParseError {
            detail: format!("{lcsc}: the symbol has no pins — nothing could wire to it"),
        });
    }
    Ok(Imported {
        symbol: Symbol {
            library: LIBRARY.to_string(),
            name: lcsc.to_string(),
            reference,
            value: value_text,
            description: (!description.is_empty()).then(|| description.join(" · ")),
            pins,
            graphics,
        },
        warnings,
    })
}

/// A number the service may write as a JSON number or as a string.
fn number_field(node: &Value, key: &str) -> Option<f64> {
    match node.get(key)? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// The anchor: EasyEDA's units, y down, converted into the symbol's
/// millimetres, y up.
struct Frame {
    x: f64,
    y: f64,
}

impl Frame {
    fn point(&self, x: f64, y: f64) -> (f64, f64) {
        (
            round((x - self.x) * UNIT_MM),
            round(-(y - self.y) * UNIT_MM),
        )
    }

    fn len(&self, units: f64) -> f64 {
        round(units.abs() * UNIT_MM)
    }
}

/// Four decimals of a millimetre — a tenth of a micron — so a converted
/// coordinate reads as a number and not as `3.8099999999999996`.
fn round(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0 + 0.0
}

fn num(text: &str) -> Option<f64> {
    text.trim().parse().ok()
}

fn pin(record: &str, frame: &Frame) -> std::result::Result<Pin, String> {
    let sections: Vec<Vec<&str>> = record.split("^^").map(|s| s.split('~').collect()).collect();
    let field = |section: usize, index: usize| -> &str {
        sections
            .get(section)
            .and_then(|s| s.get(index))
            .copied()
            .unwrap_or("")
            .trim()
    };
    // The visible number, falling back to the SPICE pin number: the two
    // are the same for every part that is wired by number.
    let number = match field(4, 4) {
        "" => field(0, 3),
        shown => shown,
    }
    .to_string();
    if number.is_empty() {
        return Err("a pin without a number".to_string());
    }
    let dot_x = num(field(1, 0))
        .or_else(|| num(field(0, 4)))
        .ok_or_else(|| format!("pin {number} has no position"))?;
    let dot_y = num(field(1, 1))
        .or_else(|| num(field(0, 5)))
        .ok_or_else(|| format!("pin {number} has no position"))?;
    // The pin line joins the connection point to the body, and LCSC's
    // library writes it from either end — `M 40 0 h -10` starting at the
    // dot, `M 20 20 h 10` ending at it — so the body end is whichever end
    // is not the dot. Its direction is the pin's angle and its extent the
    // length. The rotation field stands in only when there is no line:
    // EasyEDA's 0 is a pin sticking out to the right of its body, which is
    // KiCad's 180 (the body lies to the left of the connection point).
    let is_dot = |(x, y): (f64, f64)| (x - dot_x).abs() < 1e-9 && (y - dot_y).abs() < 1e-9;
    let body_end = svg_path(field(2, 0)).ok().and_then(|segments| {
        let start = segments.iter().find_map(|s| match *s {
            Segment::Move(x, y) => Some((x, y)),
            _ => None,
        })?;
        let end = path_end(&segments)?;
        [end, start].into_iter().find(|p| !is_dot(*p))
    });
    let (angle, length) = match body_end.map(|(bx, by)| (bx - dot_x, by - dot_y)) {
        Some((dx, dy)) if dx.abs() >= dy.abs() => (if dx > 0.0 { 0 } else { 180 }, frame.len(dx)),
        Some((_, dy)) => (if dy > 0.0 { 270 } else { 90 }, frame.len(dy)),
        None => {
            let rotation = num(field(0, 6)).unwrap_or(0.0).round().rem_euclid(360.0) as u16;
            let angle = match rotation {
                0 => 180,
                180 => 0,
                other => other,
            };
            (angle, frame.len(10.0))
        }
    };
    let kind = match field(0, 2) {
        "1" => PinKind::Input,
        "2" => PinKind::Output,
        "3" => PinKind::Bidirectional,
        "4" => PinKind::PowerIn,
        _ => PinKind::Unspecified,
    };
    let name = match field(3, 4) {
        "" => "~".to_string(),
        shown => shown.to_string(),
    };
    Ok(Pin {
        number,
        name,
        kind,
        at: frame.point(dot_x, dot_y),
        length,
        angle,
        hidden: field(0, 1) == "hide",
    })
}

fn fill(text: &str) -> Fill {
    match text.trim() {
        "" | "none" | "transparent" => Fill::None,
        _ => Fill::Background,
    }
}

fn stroke(text: &str) -> f64 {
    round(num(text).unwrap_or(1.0) * UNIT_MM)
}

/// Zero or more graphics from one body record.
fn graphic(record: &str, frame: &Frame) -> std::result::Result<Vec<Graphic>, String> {
    let fields: Vec<&str> = record.split('~').collect();
    let field = |index: usize| fields.get(index).copied().unwrap_or("").trim();
    let want = |index: usize| -> std::result::Result<f64, String> {
        num(field(index)).ok_or_else(|| format!("field {index} is not a number"))
    };
    match field(0) {
        "R" => {
            let (x, y, w, h) = (want(1)?, want(2)?, want(5)?, want(6)?);
            Ok(vec![Graphic::Rectangle {
                start: frame.point(x, y),
                end: frame.point(x + w, y + h),
                width: stroke(field(8)),
                fill: fill(field(10)),
            }])
        }
        "E" => {
            let (cx, cy, rx, ry) = (want(1)?, want(2)?, want(3)?, want(4)?);
            let width = stroke(field(6));
            let fill = fill(field(8));
            if (rx - ry).abs() < 1e-9 {
                return Ok(vec![Graphic::Circle {
                    center: frame.point(cx, cy),
                    radius: frame.len(rx),
                    width,
                    fill,
                }]);
            }
            // KiCad has no ellipse; thirty-two points draw one closely.
            let points = (0..=32)
                .map(|i| {
                    let t = f64::from(i) / 32.0 * std::f64::consts::TAU;
                    frame.point(cx + rx * t.cos(), cy + ry * t.sin())
                })
                .collect();
            Ok(vec![Graphic::Polyline {
                points,
                width,
                fill,
            }])
        }
        tag @ ("PL" | "PG") => {
            let numbers: Vec<f64> = field(1)
                .split(|c: char| c.is_whitespace() || c == ',')
                .filter(|s| !s.is_empty())
                .map(|s| num(s).ok_or_else(|| format!("`{s}` in the point list")))
                .collect::<std::result::Result<_, _>>()?;
            let mut points: Vec<(f64, f64)> = numbers
                .as_chunks::<2>()
                .0
                .iter()
                .map(|[x, y]| frame.point(*x, *y))
                .collect();
            if points.len() < 2 {
                return Err("fewer than two points".to_string());
            }
            if tag == "PG" && points.first() != points.last() {
                points.push(points[0]);
            }
            Ok(vec![Graphic::Polyline {
                points,
                width: stroke(field(3)),
                fill: fill(field(5)),
            }])
        }
        "A" => Ok(path_graphics(
            &svg_path(field(1))?,
            frame,
            stroke(field(4)),
            fill(field(6)),
        )),
        "PT" => Ok(path_graphics(
            &svg_path(field(1))?,
            frame,
            stroke(field(3)),
            fill(field(5)),
        )),
        "T" => {
            // `L` is literal text; `N` and `P` are the value and reference
            // placeholders, which the sheet draws from the instance.
            if field(1) != "L" {
                return Ok(Vec::new());
            }
            let text = field(12);
            if text.is_empty() || field(13) == "0" {
                return Ok(Vec::new());
            }
            let points = field(7)
                .trim_end_matches("pt")
                .trim_end_matches("px")
                .parse::<f64>()
                .unwrap_or(7.0);
            Ok(vec![Graphic::Text {
                text: text.to_string(),
                at: frame.point(want(2)?, want(3)?),
                size: round(points * POINT_MM),
                angle: -num(field(4)).unwrap_or(0.0),
            }])
        }
        other => Err(format!("`{other}` is not a body record")),
    }
}

/// One absolute segment of an SVG path.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Segment {
    Move(f64, f64),
    Line(f64, f64),
    Arc {
        rx: f64,
        ry: f64,
        rotation: f64,
        large: bool,
        sweep: bool,
        x: f64,
        y: f64,
    },
    Cubic {
        c1: (f64, f64),
        c2: (f64, f64),
        end: (f64, f64),
    },
    Quadratic {
        c: (f64, f64),
        end: (f64, f64),
    },
    Close,
}

/// The path grammar EasyEDA emits — `M L H V A C Q Z` and their relative
/// forms, with implicit repeats — made absolute.
fn svg_path(text: &str) -> std::result::Result<Vec<Segment>, String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    for c in text.chars() {
        if c.is_ascii_alphabetic() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            tokens.push(c.to_string());
        } else if c.is_whitespace() || c == ',' {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        } else if c == '-' && !current.is_empty() && !current.ends_with('e') {
            tokens.push(std::mem::take(&mut current));
            current.push(c);
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }

    let mut segments = Vec::new();
    let mut command: Option<char> = None;
    let (mut x, mut y) = (0.0, 0.0);
    let (mut start_x, mut start_y) = (0.0, 0.0);
    let mut i = 0;
    let number = |tokens: &[String], i: &mut usize| -> std::result::Result<f64, String> {
        let token = tokens
            .get(*i)
            .ok_or_else(|| "the path ends mid-command".to_string())?;
        let value = token
            .parse::<f64>()
            .map_err(|_| format!("`{token}` where a number was expected"))?;
        *i += 1;
        Ok(value)
    };
    while i < tokens.len() {
        let token = &tokens[i];
        if token.len() == 1
            && token
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic())
        {
            command = token.chars().next();
            i += 1;
            if matches!(command, Some('Z' | 'z')) {
                segments.push(Segment::Close);
                x = start_x;
                y = start_y;
                command = None;
            }
            continue;
        }
        let Some(cmd) = command else {
            return Err(format!("`{token}` before any command"));
        };
        let relative = cmd.is_ascii_lowercase();
        let (ox, oy) = if relative { (x, y) } else { (0.0, 0.0) };
        match cmd.to_ascii_uppercase() {
            'M' => {
                x = ox + number(&tokens, &mut i)?;
                y = oy + number(&tokens, &mut i)?;
                start_x = x;
                start_y = y;
                segments.push(Segment::Move(x, y));
                // Further pairs after a move are implicit line-tos.
                command = Some(if relative { 'l' } else { 'L' });
            }
            'L' => {
                x = ox + number(&tokens, &mut i)?;
                y = oy + number(&tokens, &mut i)?;
                segments.push(Segment::Line(x, y));
            }
            'H' => {
                x = ox + number(&tokens, &mut i)?;
                segments.push(Segment::Line(x, y));
            }
            'V' => {
                y = oy + number(&tokens, &mut i)?;
                segments.push(Segment::Line(x, y));
            }
            'A' => {
                let rx = number(&tokens, &mut i)?;
                let ry = number(&tokens, &mut i)?;
                let rotation = number(&tokens, &mut i)?;
                let large = number(&tokens, &mut i)? != 0.0;
                let sweep = number(&tokens, &mut i)? != 0.0;
                x = ox + number(&tokens, &mut i)?;
                y = oy + number(&tokens, &mut i)?;
                segments.push(Segment::Arc {
                    rx,
                    ry,
                    rotation,
                    large,
                    sweep,
                    x,
                    y,
                });
            }
            'C' => {
                let c1 = (ox + number(&tokens, &mut i)?, oy + number(&tokens, &mut i)?);
                let c2 = (ox + number(&tokens, &mut i)?, oy + number(&tokens, &mut i)?);
                x = ox + number(&tokens, &mut i)?;
                y = oy + number(&tokens, &mut i)?;
                segments.push(Segment::Cubic {
                    c1,
                    c2,
                    end: (x, y),
                });
            }
            'Q' => {
                let c = (ox + number(&tokens, &mut i)?, oy + number(&tokens, &mut i)?);
                x = ox + number(&tokens, &mut i)?;
                y = oy + number(&tokens, &mut i)?;
                segments.push(Segment::Quadratic { c, end: (x, y) });
            }
            other => return Err(format!("`{other}` is not a path command the reader knows")),
        }
    }
    Ok(segments)
}

/// Where a path ends, in its own units.
fn path_end(segments: &[Segment]) -> Option<(f64, f64)> {
    let mut at = None;
    for segment in segments {
        at = match *segment {
            Segment::Move(x, y) | Segment::Line(x, y) => Some((x, y)),
            Segment::Arc { x, y, .. } => Some((x, y)),
            Segment::Cubic { end, .. } | Segment::Quadratic { end, .. } => Some(end),
            Segment::Close => at,
        };
    }
    at
}

/// Polylines and arcs from a path: straight runs become polylines, an
/// arc becomes KiCad's three-point arc, and a curve is sampled into the
/// run it sits in.
fn path_graphics(segments: &[Segment], frame: &Frame, width: f64, fill: Fill) -> Vec<Graphic> {
    let mut graphics = Vec::new();
    let mut run: Vec<(f64, f64)> = Vec::new();
    let mut subpath_start: Option<(f64, f64)> = None;
    let mut at: Option<(f64, f64)> = None;
    let flush = |run: &mut Vec<(f64, f64)>, graphics: &mut Vec<Graphic>| {
        if run.len() >= 2 {
            graphics.push(Graphic::Polyline {
                points: run.iter().map(|(x, y)| frame.point(*x, *y)).collect(),
                width,
                fill,
            });
        }
        run.clear();
    };
    for segment in segments {
        match *segment {
            Segment::Move(x, y) => {
                flush(&mut run, &mut graphics);
                run.push((x, y));
                subpath_start = Some((x, y));
                at = Some((x, y));
            }
            Segment::Line(x, y) => {
                if run.is_empty()
                    && let Some(from) = at
                {
                    run.push(from);
                }
                run.push((x, y));
                at = Some((x, y));
            }
            Segment::Close => {
                if let Some(start) = subpath_start
                    && run.len() >= 2
                    && run.last() != Some(&start)
                {
                    run.push(start);
                }
                flush(&mut run, &mut graphics);
                at = subpath_start;
            }
            Segment::Cubic { c1, c2, end } => {
                let from = at.unwrap_or(c1);
                if run.is_empty() {
                    run.push(from);
                }
                for step in 1..=8 {
                    let t = f64::from(step) / 8.0;
                    let u = 1.0 - t;
                    run.push((
                        u * u * u * from.0
                            + 3.0 * u * u * t * c1.0
                            + 3.0 * u * t * t * c2.0
                            + t * t * t * end.0,
                        u * u * u * from.1
                            + 3.0 * u * u * t * c1.1
                            + 3.0 * u * t * t * c2.1
                            + t * t * t * end.1,
                    ));
                }
                at = Some(end);
            }
            Segment::Quadratic { c, end } => {
                let from = at.unwrap_or(c);
                if run.is_empty() {
                    run.push(from);
                }
                for step in 1..=8 {
                    let t = f64::from(step) / 8.0;
                    let u = 1.0 - t;
                    run.push((
                        u * u * from.0 + 2.0 * u * t * c.0 + t * t * end.0,
                        u * u * from.1 + 2.0 * u * t * c.1 + t * t * end.1,
                    ));
                }
                at = Some(end);
            }
            Segment::Arc {
                rx,
                ry,
                rotation,
                large,
                sweep,
                x,
                y,
            } => {
                let Some(from) = at else {
                    at = Some((x, y));
                    continue;
                };
                flush(&mut run, &mut graphics);
                match arc_midpoint(from, (x, y), rx, ry, rotation, large, sweep) {
                    Some(mid) => graphics.push(Graphic::Arc {
                        start: frame.point(from.0, from.1),
                        mid: frame.point(mid.0, mid.1),
                        end: frame.point(x, y),
                        width,
                        fill,
                    }),
                    // A degenerate arc — zero radius, or start on end — is
                    // the straight line the SVG rules say it is.
                    None => graphics.push(Graphic::Polyline {
                        points: vec![frame.point(from.0, from.1), frame.point(x, y)],
                        width,
                        fill,
                    }),
                }
                run.push((x, y));
                at = Some((x, y));
            }
        }
    }
    flush(&mut run, &mut graphics);
    graphics
}

/// The point halfway along an SVG arc — the third point KiCad's arc
/// wants. The centre comes from the endpoint-to-centre conversion in the
/// SVG specification (implementation notes F.6.5); `None` when the arc is
/// a line.
fn arc_midpoint(
    from: (f64, f64),
    to: (f64, f64),
    rx: f64,
    ry: f64,
    rotation_degrees: f64,
    large: bool,
    sweep: bool,
) -> Option<(f64, f64)> {
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx < 1e-9 || ry < 1e-9 || (from.0 - to.0).abs() < 1e-9 && (from.1 - to.1).abs() < 1e-9 {
        return None;
    }
    let phi = rotation_degrees.to_radians();
    let (sin_phi, cos_phi) = phi.sin_cos();
    let dx = (from.0 - to.0) / 2.0;
    let dy = (from.1 - to.1) / 2.0;
    let x1 = cos_phi * dx + sin_phi * dy;
    let y1 = -sin_phi * dx + cos_phi * dy;
    let lambda = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if lambda > 1.0 {
        rx *= lambda.sqrt();
        ry *= lambda.sqrt();
    }
    let sign = if large != sweep { 1.0 } else { -1.0 };
    let numerator = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
    let denominator = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let coefficient = sign * (numerator / denominator).max(0.0).sqrt();
    let cx1 = coefficient * (rx * y1 / ry);
    let cy1 = coefficient * -(ry * x1 / rx);
    let cx = cos_phi * cx1 - sin_phi * cy1 + (from.0 + to.0) / 2.0;
    let cy = sin_phi * cx1 + cos_phi * cy1 + (from.1 + to.1) / 2.0;
    let angle = |ux: f64, uy: f64, vx: f64, vy: f64| -> f64 {
        let dot = ux * vx + uy * vy;
        let len = (ux * ux + uy * uy).sqrt() * (vx * vx + vy * vy).sqrt();
        let mut a = (dot / len).clamp(-1.0, 1.0).acos();
        if ux * vy - uy * vx < 0.0 {
            a = -a;
        }
        a
    };
    let theta1 = angle(1.0, 0.0, (x1 - cx1) / rx, (y1 - cy1) / ry);
    let mut delta = angle(
        (x1 - cx1) / rx,
        (y1 - cy1) / ry,
        (-x1 - cx1) / rx,
        (-y1 - cy1) / ry,
    );
    if !sweep && delta > 0.0 {
        delta -= std::f64::consts::TAU;
    } else if sweep && delta < 0.0 {
        delta += std::f64::consts::TAU;
    }
    let theta = theta1 + delta / 2.0;
    Some((
        cx + rx * cos_phi * theta.cos() - ry * sin_phi * theta.sin(),
        cy + rx * sin_phi * theta.cos() + ry * cos_phi * theta.sin(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured from the service on 2026-09-07 for four parts every first
    /// circuit has — a resistor, a capacitor, an LED and a tactile switch —
    /// so the tests read what LCSC's own library actually writes rather
    /// than what its documentation says. `examples/lcsc_probe.rs` captures
    /// another.
    const RESISTOR: &str = include_str!("../../tests/fixtures/easyeda/C25804.json");
    const CAPACITOR: &str = include_str!("../../tests/fixtures/easyeda/C1525.json");
    const LED: &str = include_str!("../../tests/fixtures/easyeda/C2286.json");
    const SWITCH: &str = include_str!("../../tests/fixtures/easyeda/C318884.json");

    #[test]
    fn a_resistor_answer_becomes_a_symbol_in_millimetres_y_up() {
        let imported = parse("C25804", RESISTOR).expect("parses");
        assert!(imported.warnings.is_empty(), "{:?}", imported.warnings);
        let s = &imported.symbol;
        assert_eq!(s.id(), "lcsc:C25804");
        assert_eq!((s.reference.as_str(), s.value.as_str()), ("R", "10kΩ"));
        assert_eq!(
            s.description.as_deref(),
            Some("10KΩ (1002) ±1% · 0603WAF1002T5E · R0603")
        );

        // The anchor is at (20, 0); pin 1 sits twenty units left of it and
        // its line runs ten units toward the body: 2.54 mm, pointing right.
        let p1 = s.pin("1").expect("pin 1");
        assert_eq!(p1.at, (-5.08, 0.0));
        assert_eq!((p1.angle, p1.length), (0, 2.54));
        let p2 = s.pin("2").expect("pin 2");
        assert_eq!((p2.at, p2.angle), ((5.08, 0.0), 180));
        // What the library says the pins are, kept as said: the rules read
        // the reference prefix, not this.
        assert_eq!(p1.kind, PinKind::Input);

        // `R~10~-4~~~20~8`: top-left (10, -4), twenty by eight, y down —
        // which is (-2.54, 1.016)…(2.54, -1.016) with y up.
        assert_eq!(s.graphics.len(), 1);
        assert!(matches!(
            s.graphics[0],
            Graphic::Rectangle { start, end, width, fill: Fill::None }
                if start == (-2.54, 1.016) && end == (2.54, -1.016) && width == 0.254
        ));
    }

    /// The capacitor's pin lines are written from the body end to the dot,
    /// the opposite way round from the resistor's; both must come out
    /// pointing at the body.
    #[test]
    fn a_pin_line_written_from_either_end_points_at_the_body() {
        let s = parse("C1525", CAPACITOR).expect("parses").symbol;
        assert_eq!((s.reference.as_str(), s.value.as_str()), ("C", "100nF"));
        let p1 = s.pin("1").unwrap();
        let p2 = s.pin("2").unwrap();
        assert_eq!((p1.at, p1.angle, p1.length), ((-5.08, 0.0), 0, 2.54));
        assert_eq!((p2.at, p2.angle, p2.length), ((5.08, 0.0), 180, 2.54));
        assert_eq!(p1.kind, PinKind::Unspecified);
        // Two plates and two leads.
        assert_eq!(
            s.graphics
                .iter()
                .filter(|g| matches!(g, Graphic::Polyline { .. }))
                .count(),
            4
        );
    }

    #[test]
    fn an_led_keeps_its_polarity_and_a_switch_its_four_pins() {
        let s = parse("C2286", LED).expect("parses").symbol;
        assert_eq!(s.reference, "LED");
        assert_eq!(
            s.value, "KT-0603R",
            "no `Value` parameter: the name stands in"
        );
        assert_eq!(
            s.description.as_deref(),
            Some("0603 · LED-SMD_L1.6-W0.8-R-RD")
        );
        let k = s.pin("K").unwrap();
        let a = s.pin("A").unwrap();
        assert_eq!(
            (k.number.as_str(), k.at, k.angle, k.length),
            ("2", (-5.08, 0.0), 0, 3.81)
        );
        assert_eq!((a.number.as_str(), a.at, a.angle), ("1", (5.08, 0.0), 180));
        // The body is a closed, filled triangle pointing at the cathode bar.
        assert!(s.graphics.iter().any(|g| matches!(
            g,
            Graphic::Polyline { points, fill: Fill::Background, .. }
                if points.len() == 4 && points[0] == points[3] && points[1] == (-1.27, 0.0)
        )));

        let s = parse("C318884", SWITCH).expect("parses").symbol;
        assert_eq!(s.reference, "SW");
        let numbers: Vec<&str> = s.pins.iter().map(|p| p.number.as_str()).collect();
        assert_eq!(
            numbers,
            vec!["2", "1", "4", "3"],
            "file order, as the library drew them"
        );
        assert_eq!(s.pin("C").map(|p| p.at), Some((-5.08, -5.08)));
        // `E~400~298~1.25~1.25`: a circle of radius 1.25 units.
        assert_eq!(
            s.graphics
                .iter()
                .filter(|g| matches!(g, Graphic::Circle { radius, .. } if *radius == 0.3175))
                .count(),
            2
        );
    }

    #[test]
    fn a_missing_part_is_refused_in_the_services_words() {
        let error = parse(
            "C1",
            r#"{"success":false,"code":404,"message":"no such product"}"#,
        )
        .expect_err("refused");
        assert!(error.detail.contains("C1"), "{error}");
        assert!(error.detail.contains("no such product"), "{error}");
        let error = parse("C2", "<html>").expect_err("not json");
        assert!(error.detail.contains("not JSON"), "{error}");
        // A drawing with nothing to wire to is not a part.
        let error = parse(
            "C3",
            r#"{"result":{"dataStr":{"head":{"x":0,"y":0},"shape":["R~0~0~~~10~4~#A00000~1~0~none~g~0"]}}}"#,
        )
        .expect_err("no pins");
        assert!(error.detail.contains("no pins"), "{error}");
    }

    #[test]
    fn data_str_may_arrive_as_a_string_and_a_pin_with_no_line_uses_its_rotation() {
        let body = r#"{"result":{"title":"LED","dataStr":"{\"head\":{\"x\":100,\"y\":100,\"c_para\":{\"pre\":\"LED?\"}},\"shape\":[\"P~show~4~1~100~110~90~g1~0^^100~110^^~#880000^^1~0~0~0~A~start~~^^1~0~0~0~1~start~~^^0~0~0^^0~\",\"P~hide~1~2~100~90~270~g2~0^^100~90^^~#880000^^1~0~0~0~K~start~~^^1~0~0~0~2~start~~^^0~0~0^^0~\"]}"}}"#;
        let imported = parse("C2293", body).expect("parses");
        let s = &imported.symbol;
        assert_eq!(s.reference, "LED");
        assert_eq!(s.value, "LED", "the title stands in for a missing name");
        assert_eq!(s.pins[0].name, "A");
        assert_eq!(s.pins[0].kind, PinKind::PowerIn);
        assert_eq!(
            s.pins[0].at,
            (0.0, -2.54),
            "below the anchor on a y-down sheet"
        );
        assert_eq!(
            s.pins[0].angle, 90,
            "rotation 90 sticks the pin out downward, so the body is above"
        );
        assert_eq!(s.pins[1].angle, 270);
        assert!(s.pins[1].hidden);
        assert_eq!(s.pins[1].kind, PinKind::Input);
    }

    #[test]
    fn paths_become_polylines_and_arcs_and_a_polygon_closes() {
        let frame = Frame { x: 0.0, y: 0.0 };
        // A triangle drawn as a path, closed by Z.
        let read = graphic("PT~M 0 0 L 10 5 L 0 10 Z~#880000~1~0~none~g~0", &frame).unwrap();
        assert_eq!(read.len(), 1);
        assert!(matches!(&read[0], Graphic::Polyline { points, .. }
            if points.len() == 4 && points[3] == points[0] && points[1] == (2.54, -1.27)));

        // A polygon closes itself; a polyline does not.
        let closed = graphic("PG~0 0 10 0 10 10~#880000~1~0~#FFFFFF~g~0", &frame).unwrap();
        assert!(
            matches!(&closed[0], Graphic::Polyline { points, fill: Fill::Background, .. } if points.len() == 4)
        );
        let open = graphic("PL~0 0 10 0 10 10~#880000~1~0~none~g~0", &frame).unwrap();
        assert!(matches!(&open[0], Graphic::Polyline { points, .. } if points.len() == 3));

        // A semicircle from (0,0) to (20,0) swept clockwise on a y-down
        // sheet passes over the top: its midpoint is (10,-10) there, which
        // is (2.54, 2.54) with y up.
        let read = graphic("A~M 0 0 A 10 10 0 0 1 20 0~~#880000~1~0~none~g~0", &frame).unwrap();
        assert!(
            matches!(read[0], Graphic::Arc { start, mid, end, .. }
            if start == (0.0, 0.0) && mid == (2.54, 2.54) && end == (5.08, 0.0)),
            "{read:?}"
        );
        // Swept the other way it passes underneath.
        let read = graphic("A~M 0 0 A 10 10 0 0 0 20 0~~#880000~1~0~none~g~0", &frame).unwrap();
        assert!(
            matches!(read[0], Graphic::Arc { mid, .. } if mid == (2.54, -2.54)),
            "{read:?}"
        );

        // A circle is a circle; an ellipse is sampled.
        let read = graphic("E~5~5~5~5~#880000~1~0~none~g~0", &frame).unwrap();
        assert!(
            matches!(read[0], Graphic::Circle { center, radius, .. } if center == (1.27, -1.27) && radius == 1.27)
        );
        let read = graphic("E~5~5~5~2~#880000~1~0~none~g~0", &frame).unwrap();
        assert!(matches!(&read[0], Graphic::Polyline { points, .. } if points.len() == 33));

        // A relative curve is sampled into the run it sits in.
        let read = graphic("PT~M 0 0 c 0 -10 20 -10 20 0~#880000~1~0~none~g~0", &frame).unwrap();
        assert!(matches!(&read[0], Graphic::Polyline { points, .. }
            if points.len() == 9 && points[8] == (5.08, 0.0) && points[4].1 > 0.0));

        assert!(graphic("PT~M 0 0 S 1 2 3 4~#880000~1~0~none~g~0", &frame).is_err());
    }

    #[test]
    fn part_numbers_are_normalised_and_anything_else_is_refused() {
        assert_eq!(part_number(" c2286 ").unwrap(), "C2286");
        assert_eq!(part_number("C25804").unwrap(), "C25804");
        for bad in ["2286", "C", "C12a", "R0603", ""] {
            let error = part_number(bad).expect_err(bad);
            assert!(error.to_string().contains("C2286"), "{error}");
        }
        assert_eq!(
            url("https://lceda.cn", "C2286"),
            "https://lceda.cn/api/products/C2286/components?version=6.4.19.5"
        );
    }
}
