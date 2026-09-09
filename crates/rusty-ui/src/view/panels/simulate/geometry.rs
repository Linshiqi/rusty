//! The sheet's shapes and arithmetic: what a placed symbol is, where its
//! pins are once it is turned, which pin a point lands on, how a wire runs
//! from one pin to another.
//!
//! Pure functions and plain data, no view code — the canvas got its geometry
//! wrong three times in a row while none of it was testable, so this half
//! lives where a test can reach it.
//!
//! **Units.** A symbol is in KiCad's millimetres with y up; the sheet is in
//! pixels with y down. [`MM_PX`] is the one scale between them and it is
//! chosen so that KiCad's pin pitch, 100 mil, is one kit row pitch — a
//! symbol's pins then sit on the same grid as the devkit's header, which is
//! what lets a snapped wire meet both ends. Every conversion goes through
//! [`local`] and [`orient`]; a second copy of the arithmetic is how a pin
//! circle and the wire that leaves it come to disagree.

use rusty_embed::nets::Row;

use super::art;
use rusty_embed::{Fill, Graphic, Instance, KIT_REFERENCE, Pin, PinRef, Sheet, Symbol, Wire};

pub(super) const SNAP: f64 = 8.0;
pub(super) const KIT_W: f64 = 150.0;
/// Pin-row pitch on the kit. A multiple of the base grid on purpose —
/// KiCad's oldest rule is that pins live on the grid, because a snapped
/// segment can only ever meet an anchor that is itself snapped.
pub(super) const ROW_PITCH: f64 = 16.0;
/// One KiCad millimetre in sheet pixels: 2.54 mm is one row pitch.
pub(super) const MM_PX: f64 = ROW_PITCH / 2.54;
/// The devkit's symbol id. Generated from the chip's rows rather than read
/// from a library, and never written to a file — `U1` is drawn by the chip.
pub(super) const KIT_SYMBOL: &str = "rusty:kit";
/// What an unknown symbol is drawn as: a box this big, with the id in it.
pub(super) const UNKNOWN_BOX: (f64, f64) = (64.0, 32.0);

/// One thing on the sheet: a placed symbol, with the symbol beside it so
/// the view never looks one up. `symbol` is `None` for a part whose symbol
/// no library has — it is kept, drawn as a labelled box, and its wires
/// cannot land anywhere, which is the honest picture.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct EditPart {
    pub(super) inst: Instance,
    pub(super) symbol: Option<Symbol>,
}

impl EditPart {
    pub(super) fn is_kit(&self) -> bool {
        self.inst.reference == KIT_REFERENCE
    }

    /// A pin by number or name, as a wire names it.
    pub(super) fn pin(&self, key: &str) -> Option<&Pin> {
        self.symbol.as_ref()?.pin(key)
    }

    pub(super) fn pins(&self) -> &[Pin] {
        self.symbol
            .as_ref()
            .map(|s| s.pins.as_slice())
            .unwrap_or(&[])
    }
}

/// What undo restores: the parts and the wires, as they were.
pub(super) type Snapshot = (Vec<EditPart>, Vec<Wire>);

/// Where another marked part stood when a group drag began.
pub(super) type GroupStart = (usize, (f64, f64));

/// A wire touching a moving part, with the axis of the leg at each of its
/// ends — judged once when the drag starts, so it cannot flip as the part
/// crosses its own bend. `None` where the end is not on the moving part
/// or the wire has no planted bend.
pub(super) type WireStart = (usize, Option<bool>, Option<bool>);

/// Both ends of a wire on the sheet: the connection point and the direction
/// a wire leaves it, for the `from` end and the `to` end.
pub(super) type WireEnds = [((f64, f64), (f64, f64)); 2];

/// What the pointer is doing between a press and its release.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Drag {
    Pan {
        start_tx: f64,
        start_ty: f64,
        px: f64,
        py: f64,
    },
    /// A part (or the group it belongs to) in the hand.
    Part {
        index: usize,
        dx: f64,
        dy: f64,
        from: (f64, f64),
        legs: Vec<WireStart>,
    },
    /// A new wire being pulled from a pin toward another.
    Wire { from: (usize, String) },
    /// The rubber band.
    Box { start: (f64, f64) },
    /// One segment of a wire being pushed sideways.
    Segment {
        wire: usize,
        first: usize,
        second: usize,
        horizontal: bool,
        grab: f64,
        base: f64,
    },
}

/// What a right-click landed on. The menu is about this and nothing else —
/// that is the entire point of a context menu.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum MenuTarget {
    Wire(usize),
    Part(usize),
    Sheet,
}

/// The same rounding on a user-chosen grid — the sheet's corner control
/// offers 1/4/8/16px, because "the grid is too coarse to align" deserved a
/// dial, even after the real cause (off-grid anchors) was fixed.
pub(super) fn snap_to(value: f64, grid: f64) -> f64 {
    if grid <= 1.0 {
        return value.round();
    }
    (value / grid).round() * grid
}

/// The kit's header height for a row count — the rails and pins decide it,
/// rather than a constant that only ever suited one module.
pub(super) fn kit_height(rows: usize) -> f64 {
    32.0 + rows.div_ceil(2) as f64 * ROW_PITCH
}

/// Where a kit row's pin circle sits, relative to the kit's top-left.
pub(super) fn row_offset(rows: usize, row: usize) -> (f64, f64) {
    let left = rows.div_ceil(2);
    if row < left {
        (10.0, 16.0 + row as f64 * ROW_PITCH)
    } else {
        (KIT_W - 10.0, 16.0 + (row - left) as f64 * ROW_PITCH)
    }
}

/// The devkit as a symbol: one pin per header row, numbered by position
/// and named by the row (`GPIO2`, `GND`), so a wire to `U1.GPIO2` resolves
/// the way a wire to `D1.K` does. The body is a rectangle the size of the
/// drawing, for bounds and the rubber band; the art itself is `kit_art`.
pub(super) fn kit_symbol(chip: &str, rows: &[Row]) -> Symbol {
    let height = kit_height(rows.len());
    let left = rows.len().div_ceil(2);
    let pins = rows
        .iter()
        .enumerate()
        .map(|(row, spec)| {
            let (px, py) = row_offset(rows.len(), row);
            Pin {
                number: (row + 1).to_string(),
                name: spec.name.clone(),
                kind: if spec.rail.is_some() {
                    rusty_embed::PinKind::PowerIn
                } else if spec.gpio.is_some() {
                    rusty_embed::PinKind::Bidirectional
                } else {
                    rusty_embed::PinKind::Input
                },
                at: (px / MM_PX, -py / MM_PX),
                length: 0.0,
                angle: if row < left { 0 } else { 180 },
                hidden: false,
            }
        })
        .collect();
    Symbol {
        library: "rusty".to_string(),
        name: "kit".to_string(),
        reference: "U".to_string(),
        value: chip.to_uppercase(),
        description: None,
        pins,
        graphics: vec![Graphic::Rectangle {
            start: (0.0, 0.0),
            end: (KIT_W / MM_PX, -height / MM_PX),
            width: 0.0,
            fill: Fill::Background,
        }],
    }
}

/// The editor's parts from the wire model: the devkit first, always, then
/// every placed symbol with its symbol found among the resolved ones.
pub(super) fn parts_of(sheet: &Sheet, rows: &[Row]) -> Vec<EditPart> {
    let mut out = Vec::with_capacity(sheet.parts.len() + 1);
    out.push(EditPart {
        inst: Instance {
            reference: KIT_REFERENCE.to_string(),
            symbol: KIT_SYMBOL.to_string(),
            value: sheet.chip.to_uppercase(),
            x: sheet.kit_x.unwrap_or(460.0),
            y: sheet.kit_y.unwrap_or(40.0),
            rot: sheet.kit_rot,
            mirror: sheet.kit_mirror,
            props: Default::default(),
        },
        symbol: Some(kit_symbol(&sheet.chip, rows)),
    });
    for inst in &sheet.parts {
        out.push(EditPart {
            symbol: sheet
                .symbols
                .iter()
                .find(|s| s.id() == inst.symbol)
                .cloned(),
            inst: inst.clone(),
        });
    }
    out
}

/// Back to the wire model, for saving. The symbols are not sent: the
/// backend resolves them on load, and the file never carries them.
pub(super) fn sheet_of(chip: &str, parts: &[EditPart], wires: &[Wire]) -> Sheet {
    let mut sheet = Sheet::empty(chip);
    for part in parts {
        if part.is_kit() {
            sheet.kit_x = Some(part.inst.x);
            sheet.kit_y = Some(part.inst.y);
            sheet.kit_rot = part.inst.rot;
            sheet.kit_mirror = part.inst.mirror;
        } else {
            sheet.parts.push(part.inst.clone());
        }
    }
    sheet.wires = wires.to_vec();
    sheet
}

/// An empty sheet for the project's chip.
pub(super) fn empty_sheet(chip: &str) -> Sheet {
    Sheet::empty(chip)
}

/// A symbol-local point (millimetres, y up) as a sheet offset (pixels, y
/// down) before the part's own turn and mirror.
pub(super) fn local(point: (f64, f64)) -> (f64, f64) {
    (point.0 * MM_PX, -point.1 * MM_PX)
}

/// A sheet offset turned and mirrored the way a part is: the mirror first,
/// then quarter turns clockwise on the screen.
pub(super) fn orient(offset: (f64, f64), rot: u16, mirror: bool) -> (f64, f64) {
    let (x, y) = if mirror {
        (-offset.0, offset.1)
    } else {
        offset
    };
    match rot % 360 {
        90 => (-y, x),
        180 => (-x, -y),
        270 => (y, -x),
        _ => (x, y),
    }
}

/// A point turned about a centre, in quarter turns.
#[cfg(test)]
pub(super) fn rotate_about(point: (f64, f64), centre: (f64, f64), rot: u16) -> (f64, f64) {
    let (dx, dy) = (point.0 - centre.0, point.1 - centre.1);
    let (rx, ry) = orient((dx, dy), rot, false);
    (centre.0 + rx, centre.1 + ry)
}

/// The connector a devkit carries at its bottom edge.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Usb {
    /// A bare chip, drawn as one: no board around it.
    None,
    MicroB,
    TypeC,
    /// The S3 and C6 devkits: one for the USB-UART bridge, one native.
    DualTypeC,
}

/// What a devkit for a chip looks like beyond its pins: the module soldered
/// on it, its connector, the two buttons every Espressif devkit carries and
/// whether it has an RGB LED. Drawn from the family the catalogue names —
/// the pin rows stay data-driven; this is the board around them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct KitStyle {
    /// The module's printed name, or `None` for a part rusty knows only as
    /// a die, which is drawn as a chip rather than as somebody's devkit.
    pub module: Option<&'static str>,
    pub usb: Usb,
    /// The reset button's silkscreen — `EN` on the classic ESP32 devkit,
    /// `RST` on the rest — and the boot button's.
    pub buttons: (&'static str, &'static str),
    pub rgb: bool,
}

pub(super) fn kit_style(chip: &str) -> KitStyle {
    let devkit = |module, usb, reset, rgb| KitStyle {
        module: Some(module),
        usb,
        buttons: (reset, "BOOT"),
        rgb,
    };
    match chip {
        "esp32" => devkit("ESP-WROOM-32", Usb::MicroB, "EN", false),
        "esp32s2" => devkit("ESP32-S2-MINI-1", Usb::TypeC, "RST", true),
        "esp32s3" => devkit("ESP32-S3-WROOM-1", Usb::DualTypeC, "RST", true),
        "esp32c2" => devkit("ESP8684-MINI-1", Usb::MicroB, "RST", false),
        "esp32c3" => devkit("ESP32-C3-MINI-1", Usb::TypeC, "RST", true),
        "esp32c6" => devkit("ESP32-C6-WROOM-1", Usb::DualTypeC, "RST", true),
        "esp32h2" => devkit("ESP32-H2-MINI-1", Usb::TypeC, "RST", true),
        "esp32p4" => devkit("ESP32-P4", Usb::DualTypeC, "RST", false),
        _ => KitStyle {
            module: None,
            usb: Usb::None,
            buttons: ("", ""),
            rgb: false,
        },
    }
}

/// The board drawn around the pin rows, as SVG markup for the kit's own
/// `<svg>`: the PCB, the module with its antenna meander and shield can,
/// the bridge chip and regulator, the reset and boot buttons, the power LED,
/// the RGB LED where the devkit has one, and the connector — everything a
/// hand reaching for the board on the desk uses to orient itself. Pure text,
/// so a test can say which board it is; `height` follows the pin rows.
pub(super) fn kit_art(style: KitStyle, height: f64, label: &str) -> String {
    let w = KIT_W;
    let h = height;
    let mut svg = String::new();
    // The PCB: matte black, a hairline of silkscreen inside the edge.
    svg.push_str(&format!(
        r##"<rect x="4" y="2" width="{}" height="{}" rx="7" fill="#141920" stroke="#3a414b" stroke-width="1.5"/>"##,
        w - 8.0,
        h - 4.0,
    ));
    let Some(module) = style.module else {
        // A die, not a devkit: the chip outline the editor always drew.
        svg.push_str(&format!(
            r##"<rect x="42" y="12" width="{}" height="84" rx="4" fill="#2e333b" stroke="#4a515d"/>"##,
            w - 84.0,
        ));
        svg.push_str(&format!(
            r##"<text x="{}" y="58" text-anchor="middle" font-family="ui-monospace" font-size="12" fill="#aab3c0">{label}</text>"##,
            w / 2.0,
        ));
        return svg;
    };

    // The shield can's brushed-metal fill; only a devkit has one to paint.
    svg.push_str(concat!(
        r##"<defs><linearGradient id="kit-can" x1="0" y1="0" x2="1" y2="1">"##,
        r##"<stop offset="0" stop-color="#d3d8dd"/><stop offset="0.55" stop-color="#9aa1a9"/>"##,
        r##"<stop offset="1" stop-color="#676d74"/></linearGradient></defs>"##,
    ));
    // The module: its own substrate, the antenna meander in copper across the
    // top, the shield can below it with the names printed on it.
    svg.push_str(
        r##"<rect x="40" y="6" width="70" height="98" rx="2" fill="#0e1216" stroke="#2a3038"/>"##,
    );
    let mut meander = String::from("M44 11");
    let mut x = 44.0;
    let mut down = true;
    while x < 104.0 {
        x += 6.0;
        meander.push_str(&format!(" H{x:.0}"));
        meander.push_str(if down { " V21" } else { " V11" });
        down = !down;
    }
    svg.push_str(&format!(
        r##"<path d="{meander}" fill="none" stroke="#c8a24a" stroke-width="1.6" stroke-linejoin="round"/>"##
    ));
    svg.push_str(r##"<rect x="44" y="27" width="62" height="70" rx="3" fill="url(#kit-can)" stroke="#8d949c"/>"##);
    svg.push_str(&format!(
        r##"<text x="{cx}" y="50" text-anchor="middle" font-family="ui-monospace" font-size="9" font-weight="700" fill="#1f242a">{label}</text>"##,
        cx = w / 2.0,
    ));
    svg.push_str(&format!(
        r##"<text x="{cx}" y="62" text-anchor="middle" font-family="ui-monospace" font-size="5.5" fill="#2b3138">{module}</text>"##,
        cx = w / 2.0,
    ));
    svg.push_str(&format!(
        r##"<text x="{cx}" y="84" text-anchor="middle" font-family="ui-sans-serif, system-ui" font-size="6.5" font-style="italic" fill="#3d444c">espressif</text>"##,
        cx = w / 2.0,
    ));
    if style.rgb {
        // The addressable LED under the module, off: a dark square with the
        // four dice a WS2812 shows through its lens.
        svg.push_str(r##"<rect x="98" y="106" width="8" height="8" rx="1" fill="#1a1e24" stroke="#3a4149"/>"##);
        svg.push_str(r##"<circle cx="102" cy="110" r="1.6" fill="#f3f4f6" opacity="0.7"/>"##);
    }

    // Between the module and the connector, when the board is tall enough
    // to hold them: the USB-UART bridge, the regulator, a few passives.
    let free = h - 30.0 - 106.0;
    if free > 30.0 {
        let top = 108.0;
        svg.push_str(&format!(
            r##"<rect x="60" y="{y}" width="20" height="20" rx="1" fill="#0b0e12" stroke="#3a4149"/><circle cx="63" cy="{dot}" r="1" fill="#6b7280"/>"##,
            y = top,
            dot = top + 3.0,
        ));
        svg.push_str(&format!(
            r##"<rect x="86" y="{y}" width="12" height="8" rx="1" fill="#16191e" stroke="#3a4149"/>"##,
            y = top + 4.0,
        ));
        for (i, px) in [86.0, 91.0, 96.0].into_iter().enumerate() {
            svg.push_str(&format!(
                r##"<rect x="{px}" y="{y}" width="3" height="5" rx="0.5" fill="{fill}"/>"##,
                y = top + 16.0,
                fill = if i == 1 { "#4a3b2a" } else { "#3b4a3a" },
            ));
        }
    }

    // The two buttons, low on the board where every devkit has them, the
    // silkscreen above each.
    let (reset, boot) = style.buttons;
    for (bx, name) in [(42.0, reset), (94.0, boot)] {
        svg.push_str(&format!(
            r##"<text x="{tx}" y="{ty}" text-anchor="middle" font-family="ui-monospace" font-size="5.5" fill="#98a1ae">{name}</text>"##,
            tx = bx + 7.0,
            ty = h - 32.0,
        ));
        svg.push_str(&format!(
            r##"<rect x="{bx}" y="{by}" width="14" height="14" rx="2" fill="#2b3036" stroke="#4a515b"/><circle cx="{cx}" cy="{cy}" r="4" fill="#c9ced4"/>"##,
            by = h - 30.0,
            cx = bx + 7.0,
            cy = h - 23.0,
        ));
    }
    // The power LED beside the connector.
    svg.push_str(&format!(
        r##"<circle cx="60" cy="{cy}" r="1.8" fill="#e03a3a"/>"##,
        cy = h - 6.0,
    ));
    // The connector, on the bottom edge.
    let can = "url(#kit-can)";
    match style.usb {
        Usb::None => {}
        Usb::MicroB => svg.push_str(&format!(
            r##"<rect x="66" y="{y}" width="18" height="9" rx="2" fill="{can}" stroke="#8d949c"/>"##,
            y = h - 11.0,
        )),
        Usb::TypeC => svg.push_str(&format!(
            r##"<rect x="64" y="{y}" width="22" height="10" rx="5" fill="{can}" stroke="#8d949c"/>"##,
            y = h - 12.0,
        )),
        Usb::DualTypeC => {
            for x in [52.0, 80.0] {
                svg.push_str(&format!(
                    r##"<rect x="{x}" y="{y}" width="18" height="10" rx="5" fill="{can}" stroke="#8d949c"/>"##,
                    y = h - 12.0,
                ));
            }
        }
    }
    svg
}

/// The part's drawing, in its own frame — `None` for a part whose symbol
/// no library has, which is drawn as a labelled box instead.
pub(super) fn part_layout(part: &EditPart) -> Option<art::Layout> {
    part.symbol.as_ref().map(art::layout)
}

/// One of the drawing's leads on the sheet: where the wire attaches, and
/// the direction it runs away from the body, after the part's turn and
/// mirror. Takes the layout, so a caller looking at every pin builds it
/// once — this is the inner loop of a drag.
pub(super) fn spot_on_sheet(part: &EditPart, spot: &art::Spot) -> ((f64, f64), (f64, f64)) {
    let (dx, dy) = orient(spot.at, part.inst.rot, part.inst.mirror);
    let (ox, oy) = orient(spot.out, part.inst.rot, part.inst.mirror);
    ((part.inst.x + dx, part.inst.y + dy), (ox, oy))
}

/// Where a pin's wire lands on the sheet — the end of its lead.
pub(super) fn pin_point(part: &EditPart, pin: &Pin) -> (f64, f64) {
    lead_of(part, pin).map_or((part.inst.x, part.inst.y), |(at, _)| at)
}

/// The unit vector a wire leaves a pin along: away from the body.
pub(super) fn pin_out(part: &EditPart, pin: &Pin) -> (f64, f64) {
    lead_of(part, pin).map_or((0.0, 0.0), |(_, out)| out)
}

fn lead_of(part: &EditPart, pin: &Pin) -> Option<((f64, f64), (f64, f64))> {
    let plan = part_layout(part)?;
    let spot = plan.spot(&pin.number)?;
    Some(spot_on_sheet(part, spot))
}

/// The spelling a wire uses for a pin: its name when no other pin of the
/// symbol shares it and it is a name at all, its number otherwise — so a
/// file reads `D1.K` and `U1.GPIO2`, and `U1.9` only where `GND` repeats.
pub(super) fn pin_key(symbol: &Symbol, pin: &Pin) -> String {
    let named = pin.name != "~" && !pin.name.is_empty();
    let unique = symbol.pins.iter().filter(|p| p.name == pin.name).count() == 1;
    if named && unique {
        pin.name.clone()
    } else {
        pin.number.clone()
    }
}

/// The box a part occupies on the sheet, `(x0, y0, x1, y1)`: the drawing's
/// box, turned and mirrored, or the unknown-symbol box.
pub(super) fn part_box(part: &EditPart) -> (f64, f64, f64, f64) {
    let (x, y) = (part.inst.x, part.inst.y);
    let Some(plan) = part_layout(part) else {
        let (w, h) = UNKNOWN_BOX;
        return (x - w / 2.0, y - h / 2.0, x + w / 2.0, y + h / 2.0);
    };
    box_on_sheet(part, plan.bounds)
}

/// Where a part's anchor has to move so that turning it looks like a spin
/// in place rather than a swing.
///
/// A KiCad symbol is drawn about its own anchor, so its box is roughly
/// centred there and a turn moves nothing — which is why this is the
/// devkit's rule and nobody else's. The devkit's anchor is the *top-left
/// corner* of a board three hundred pixels tall ([`kit_symbol`] draws it
/// from `(0, 0)` downwards), and a quarter turn about a corner puts the
/// board a board's length away from where the user was looking. Correcting
/// every part instead would change where a turned lamp lands in files
/// people have already saved, for no gain: their anchors are already
/// centred.
pub(super) fn turned_anchor(part: &EditPart, rot: u16, mirror: bool) -> (f64, f64) {
    let here = (part.inst.x, part.inst.y);
    if !part.is_kit() {
        return here;
    }
    let Some(plan) = part_layout(part) else {
        return here;
    };
    let (x0, y0, x1, y1) = plan.bounds;
    let middle = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    // Keep the box's centre where it is: the anchor moves by however much
    // the turn moved the centre away from it.
    let before = orient(middle, part.inst.rot, part.inst.mirror);
    let after = orient(middle, rot, mirror);
    (here.0 + before.0 - after.0, here.1 + before.1 - after.1)
}

/// A box in the drawing's frame, turned and mirrored onto the sheet.
pub(super) fn box_on_sheet(part: &EditPart, rect: (f64, f64, f64, f64)) -> (f64, f64, f64, f64) {
    let (x0, y0, x1, y1) = rect;
    let mut acc: Option<(f64, f64, f64, f64)> = None;
    for corner in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
        let (dx, dy) = orient(corner, part.inst.rot, part.inst.mirror);
        let (px, py) = (part.inst.x + dx, part.inst.y + dy);
        acc = Some(match acc {
            None => (px, py, px, py),
            Some((a, b, c, d)) => (a.min(px), b.min(py), c.max(px), d.max(py)),
        });
    }
    acc.unwrap_or((part.inst.x, part.inst.y, part.inst.x, part.inst.y))
}

/// The pin within `radius` of `point`, nearest first: `(part, pin number)`.
/// Nothing when nothing is in reach, rather than the nearest: a wire that
/// landed on a pin forty pixels from the pointer would be a connection
/// nobody made. Hidden pins take no wires.
pub(super) fn pin_under(
    parts: &[EditPart],
    point: (f64, f64),
    radius: f64,
) -> Option<(usize, String)> {
    let mut best: Option<((usize, String), f64)> = None;
    for (index, part) in parts.iter().enumerate() {
        let Some(plan) = part_layout(part) else {
            continue;
        };
        for spot in &plan.spots {
            let ((px, py), _) = spot_on_sheet(part, spot);
            let distance = (px - point.0).hypot(py - point.1);
            if distance <= radius && best.as_ref().is_none_or(|(_, nearest)| distance < *nearest) {
                best = Some(((index, spot.number.clone()), distance));
            }
        }
    }
    best.map(|(hit, _)| hit)
}

/// Every part whose box touches the rectangle between two corners, in
/// sheet order — the rubber-band selection. Touching rather than enclosed:
/// the parts are small and the intent of a band that clips a corner is
/// never "not that one". The corners may come in either order.
pub(super) fn parts_in_box(parts: &[EditPart], a: (f64, f64), b: (f64, f64)) -> Vec<usize> {
    let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
    let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
    parts
        .iter()
        .enumerate()
        .filter(|(_, part)| {
            let (px0, py0, px1, py1) = part_box(part);
            px0 <= x1 && px1 >= x0 && py0 <= y1 && py1 >= y0
        })
        .map(|(index, _)| index)
        .collect()
}

/// The box around everything on the sheet — what fit-to-view frames.
pub(super) fn bounds(parts: &[EditPart]) -> ((f64, f64), (f64, f64)) {
    let mut min = (f64::MAX, f64::MAX);
    let mut max = (f64::MIN, f64::MIN);
    for part in parts {
        let (x0, y0, x1, y1) = part_box(part);
        min = (min.0.min(x0), min.1.min(y0));
        max = (max.0.max(x1), max.1.max(y1));
    }
    if parts.is_empty() {
        return ((0.0, 0.0), (0.0, 0.0));
    }
    (min, max)
}

/// The two ends of a wire on the sheet, each with the direction a wire
/// leaves its pin: `None` when either end names a part or a pin the sheet
/// does not have.
pub(super) fn wire_ends(parts: &[EditPart], wire: &Wire) -> Option<WireEnds> {
    let end = |at: &PinRef| {
        let part = parts.iter().find(|p| p.inst.reference == at.part)?;
        let pin = part.pin(&at.pin)?;
        Some((pin_point(part, pin), pin_out(part, pin)))
    };
    Some([end(&wire.from)?, end(&wire.to)?])
}

/// The whole drawn path of one wire: pin to pin through the bends the
/// author placed, orthogonal by construction as every schematic wire is. An
/// untouched wire steps out of each pin along the pin's own direction and
/// meets itself with one elbow.
pub(super) fn wire_path(ends: &WireEnds, bends: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let [(a, out_a), (b, out_b)] = *ends;
    let mut points = vec![a];
    if bends.is_empty() {
        let step = ROW_PITCH;
        let p1 = (a.0 + out_a.0 * step, a.1 + out_a.1 * step);
        let p2 = (b.0 + out_b.0 * step, b.1 + out_b.1 * step);
        points.push(p1);
        points.push(p2);
    } else {
        points.extend(bends.iter().copied());
    }
    points.push(b);
    simplify_route(orthogonalize(points))
}

/// The distance from a point to a line segment — the wire hit test's inner
/// step, and the one piece of arithmetic a schematic needs that pins do not.
fn point_to_segment(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length = dx * dx + dy * dy;
    if length <= f64::EPSILON {
        return (p.0 - a.0).hypot(p.1 - a.1);
    }
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / length).clamp(0.0, 1.0);
    (p.0 - (a.0 + t * dx)).hypot(p.1 - (a.1 + t * dy))
}

/// The wire whose drawn path passes within `radius` of `point`, nearest
/// first — where a branch lands.
///
/// [`pin_under`]'s rule, for the same reason: nothing when nothing is in
/// reach rather than the nearest, because a branch attached to a wire forty
/// pixels from the pointer is a connection nobody made. A pin beats a wire,
/// so the caller asks this only after `pin_under` has answered nothing.
pub(super) fn wire_under(
    parts: &[EditPart],
    wires: &[Wire],
    point: (f64, f64),
    radius: f64,
) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for (index, wire) in wires.iter().enumerate() {
        let Some(ends) = wire_ends(parts, wire) else {
            continue;
        };
        for pair in wire_path(&ends, &wire.bends).windows(2) {
            let distance = point_to_segment(point, pair[0], pair[1]);
            if distance <= radius && best.is_none_or(|(_, near)| distance < near) {
                best = Some((index, distance));
            }
        }
    }
    best.map(|(index, _)| index)
}

/// Where a branch dropped on a wire runs: which of the trunk's two pins it
/// joins, and the bends that lay it along the trunk from the drop point to
/// that pin.
///
/// **A T-junction needs no junction.** Three wires at one pin are already
/// one net — the union-find in `nets` has always joined them — so a branch
/// onto the middle of a wire is electrically a wire to *either* of that
/// wire's ends, and the model, the file and the rules need no new idea at
/// all. What is left is the picture, and the picture is what the bends are
/// for: the branch overlays the trunk from the drop point onward, so the
/// two are drawn as a T rather than as a second wire taking its own route.
/// The nearer end is chosen, which is the shorter overlay and the fewer
/// bends to keep.
///
/// Those bends are sheet coordinates like every other bend here, so moving
/// the trunk later slides the branch's tail off it — exactly what happens
/// to any hand-bent wire whose neighbour moves, and the same repair: drag
/// the segment back. A junction node in the file would be a second way to
/// say what a shared net already says.
pub(super) fn branch_route(
    parts: &[EditPart],
    trunk: &Wire,
    at: (f64, f64),
) -> Option<(PinRef, Vec<(f64, f64)>)> {
    let ends = wire_ends(parts, trunk)?;
    let path = wire_path(&ends, &trunk.bends);
    if path.len() < 2 {
        return None;
    }
    // The segment the drop landed on, and how far along the path it is.
    let mut hit = (0usize, f64::INFINITY);
    for (index, pair) in path.windows(2).enumerate() {
        let distance = point_to_segment(at, pair[0], pair[1]);
        if distance < hit.1 {
            hit = (index, distance);
        }
    }
    let (segment, _) = hit;
    let run = |points: &[(f64, f64)]| {
        let mut length = 0.0;
        for pair in points.windows(2) {
            length += (pair[1].0 - pair[0].0).hypot(pair[1].1 - pair[0].1);
        }
        length
    };
    // The walk from the drop to each of the trunk's pins, the drop point
    // first and the pin last, so the two are compared by how far the
    // overlay would actually run rather than by how many bends it has.
    let toward_to: Vec<(f64, f64)> = std::iter::once(at)
        .chain(path[segment + 1..].iter().copied())
        .collect();
    let toward_from: Vec<(f64, f64)> = std::iter::once(at)
        .chain(path[..=segment].iter().rev().copied())
        .collect();
    let (pin, walk) = if run(&toward_to) <= run(&toward_from) {
        (trunk.to.clone(), toward_to)
    } else {
        (trunk.from.clone(), toward_from)
    };
    // The last point is the pin the wire names, not a bend through it.
    Some((pin, simplify_route(walk[..walk.len() - 1].to_vec())))
}

/// Re-tidy one wire after its ends moved: the pins are put back on the
/// route, collinear bends fold away, and what is stored is again only the
/// bends between them.
pub(super) fn retidy(wire: &mut Wire, ends: &WireEnds) {
    if wire.bends.is_empty() {
        return;
    }
    let mut full = vec![ends[0].0];
    full.extend(wire.bends.iter().copied());
    full.push(ends[1].0);
    let tidy = simplify_route(full);
    wire.bends = tidy[1..tidy.len() - 1].to_vec();
}

/// Drop the points a route no longer needs: consecutive duplicates, and any
/// bend whose neighbours run straight through it. This is what merges two
/// segments the user has dragged into line — the KiCad behaviour: aligned
/// segments become one segment, and the next grab moves them as one.
pub(super) fn simplify_route(full: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    let mut out: Vec<(f64, f64)> = Vec::with_capacity(full.len());
    for point in full {
        if let Some(last) = out.last()
            && (last.0 - point.0).abs() < 0.01
            && (last.1 - point.1).abs() < 0.01
        {
            continue;
        }
        out.push(point);
        while out.len() >= 3 {
            let c = out[out.len() - 1];
            let b = out[out.len() - 2];
            let a = out[out.len() - 3];
            let collinear = ((a.0 - b.0).abs() < 0.01 && (b.0 - c.0).abs() < 0.01)
                || ((a.1 - b.1).abs() < 0.01 && (b.1 - c.1).abs() < 0.01);
            if collinear {
                out.remove(out.len() - 2);
            } else {
                break;
            }
        }
    }
    out
}

/// Insert elbows so every segment runs purely horizontally or vertically.
/// Idempotent, so a path that came back from a drag survives a round trip.
pub(super) fn orthogonalize(points: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    let mut out: Vec<(f64, f64)> = Vec::with_capacity(points.len() + 2);
    out.push(points[0]);
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if (a.0 - b.0).abs() > 0.01 && (a.1 - b.1).abs() > 0.01 {
            out.push((b.0, a.1));
        }
        out.push(b);
    }
    out
}

/// Which way a route's leg runs — pin against the planted bend beside it.
/// `Some(true)` is horizontal. Judged once, when the drag starts: judging
/// it live would flip as the part crosses the bend.
pub(super) fn leg_axis(pin: (f64, f64), bend: (f64, f64)) -> Option<bool> {
    if (pin.1 - bend.1).abs() < 0.01 {
        Some(true)
    } else if (pin.0 - bend.0).abs() < 0.01 {
        Some(false)
    } else {
        None
    }
}

/// KiCad's stretch, completed: dragging a part slides the bend beside its
/// pin along the leg's own axis, so the leg stays parallel to itself and
/// only changes length. Without this, a planted bend at the old pin height
/// makes the route run out, all the way back up to where the part used to
/// be, and down again — a wall of wire the user never drew.
pub(super) fn follow_bend(pin: (f64, f64), axis: Option<bool>, bend: &mut (f64, f64)) {
    match axis {
        Some(true) => bend.1 = pin.1,
        Some(false) => bend.0 = pin.0,
        None => {}
    }
}

/// The legs of every wire touching any of `moving`, judged as a drag
/// begins — the argument [`follow_bend`] wants on every frame after.
pub(super) fn wire_legs(parts: &[EditPart], wires: &[Wire], moving: &[usize]) -> Vec<WireStart> {
    let moved = |at: &PinRef| {
        moving
            .iter()
            .any(|i| parts.get(*i).is_some_and(|p| p.inst.reference == at.part))
    };
    wires
        .iter()
        .enumerate()
        .filter_map(|(index, wire)| {
            let ends = wire_ends(parts, wire)?;
            let from = (moved(&wire.from) && !wire.bends.is_empty())
                .then(|| leg_axis(ends[0].0, wire.bends[0]))
                .flatten();
            let to = (moved(&wire.to) && !wire.bends.is_empty())
                .then(|| leg_axis(ends[1].0, *wire.bends.last().expect("non-empty")))
                .flatten();
            (moved(&wire.from) || moved(&wire.to)).then_some((index, from, to))
        })
        .collect()
}

/// The lamp colours a value names: lit and dark.
pub(super) fn lamp_colors(color: &str) -> (&'static str, &'static str) {
    match color.trim().to_ascii_lowercase().as_str() {
        "green" | "绿" => ("#3ddc84", "#1d4a2f"),
        "blue" | "蓝" => ("#4aa8ff", "#1d3350"),
        "yellow" | "黄" => ("#ffd75c", "#4a3f1d"),
        "white" | "白" => ("#f4f4f4", "#3a3f48"),
        _ => ("#ff5c5c", "#4a1d1d"),
    }
}

/// Additive mix of the RGB lens from three channel levels; dark when none
/// is on.
pub(super) fn rgb_color(r: bool, g: bool, b: bool) -> &'static str {
    match (r, g, b) {
        (false, false, false) => "#2a2d33",
        (true, false, false) => "#ff5c5c",
        (false, true, false) => "#3ddc84",
        (false, false, true) => "#4aa8ff",
        (true, true, false) => "#ffd75c",
        (true, false, true) => "#d97cff",
        (false, true, true) => "#5ce8e8",
        (true, true, true) => "#f4f4f4",
    }
}

/// One piece of upright text beside a part: where, how anchored, what.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Label {
    pub x: f64,
    pub y: f64,
    /// `start`, `middle` or `end`.
    pub anchor: &'static str,
    pub text: String,
    pub size: f64,
}

/// The pin names of a part, placed just past the end of each lead. Names
/// only: a real part has no pin numbers printed on it, and the sheet is a
/// picture of the part. A pin the library did not name (`~`, or the number
/// again) is left alone, which is why a resistor and a capacitor carry no
/// writing at all. The text is placed after the part's turn, so it reads
/// upright however the part lies.
pub(super) fn pin_labels(part: &EditPart) -> Vec<Label> {
    let Some(plan) = part_layout(part) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for spot in &plan.spots {
        if spot.name == "~" || spot.name.is_empty() || spot.name == spot.number {
            continue;
        }
        let ((x, y), (ox, oy)) = spot_on_sheet(part, spot);
        // Beside the lead rather than beyond its tip, which is where the
        // wire goes: a label under a wire is a label nobody can read.
        let (dx, dy, anchor) = if ox.abs() > oy.abs() {
            (-ox * 5.0, -7.0, "middle")
        } else {
            (6.0, -oy * 4.0 + 2.5, "start")
        };
        out.push(Label {
            x: x + dx,
            y: y + dy,
            anchor,
            text: spot.name.clone(),
            size: 7.0,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_embed::nets::kit_rows;
    use rusty_embed::{Fill, PinKind};

    fn led_symbol() -> Symbol {
        let pin = |number: &str, name: &str, x: f64, angle: u16| Pin {
            number: number.into(),
            name: name.into(),
            kind: PinKind::Passive,
            at: (x, 0.0),
            length: 2.54,
            angle,
            hidden: false,
        };
        Symbol {
            library: "Device".into(),
            name: "LED".into(),
            reference: "D".into(),
            value: "LED".into(),
            description: None,
            pins: vec![pin("1", "K", -3.81, 0), pin("2", "A", 3.81, 180)],
            graphics: vec![
                Graphic::Polyline {
                    points: vec![(-1.27, -1.27), (-1.27, 1.27)],
                    width: 0.254,
                    fill: Fill::None,
                },
                Graphic::Arc {
                    start: (0.0, 1.0),
                    mid: (1.0, 0.0),
                    end: (0.0, -1.0),
                    width: 0.1,
                    fill: Fill::None,
                },
            ],
        }
    }

    fn led(x: f64, y: f64) -> EditPart {
        EditPart {
            inst: Instance {
                reference: "D1".into(),
                symbol: "Device:LED".into(),
                value: "red".into(),
                x,
                y,
                rot: 0,
                mirror: false,
                props: Default::default(),
            },
            symbol: Some(led_symbol()),
        }
    }

    fn rows() -> Vec<Row> {
        kit_rows("esp32c3", &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 20, 21])
    }

    fn kit(x: f64, y: f64) -> EditPart {
        EditPart {
            inst: Instance {
                reference: KIT_REFERENCE.into(),
                symbol: KIT_SYMBOL.into(),
                value: "ESP32C3".into(),
                x,
                y,
                rot: 0,
                mirror: false,
                props: Default::default(),
            },
            symbol: Some(kit_symbol("esp32c3", &rows())),
        }
    }

    /// A wire lands on the end of a leg — where it does on the desk — and
    /// the kit's pins are its header's rows, exactly.
    #[test]
    fn a_wire_lands_on_a_leg_and_the_kits_pins_are_its_row_points() {
        let part = led(200.0, 96.0);
        let k = part.pin("K").unwrap();
        let a = part.pin("A").unwrap();
        assert_eq!(pin_point(&part, a), (204.0, 120.0));
        assert_eq!(pin_point(&part, k), (196.0, 114.0));
        assert!(
            pin_point(&part, a).1 > pin_point(&part, k).1,
            "the anode is the long leg, as it is in the bag"
        );
        assert_eq!(pin_out(&part, a), (0.0, 1.0), "the legs point down");
        assert_eq!(pin_out(&part, k), (0.0, 1.0));

        let kit = kit(460.0, 40.0);
        let rows = rows();
        for (row, spec) in rows.iter().enumerate() {
            // By number: `GND` names two rows, and by name finds the first.
            let pin = kit.pin(&(row + 1).to_string()).unwrap();
            assert_eq!(pin.name, spec.name);
            let (ox, oy) = row_offset(rows.len(), row);
            let (px, py) = pin_point(&kit, pin);
            assert!(
                (px - (460.0 + ox)).abs() < 1e-9 && (py - (40.0 + oy)).abs() < 1e-9,
                "row {row}"
            );
        }
        assert_eq!(kit.pin("GPIO2").map(|p| p.number.as_str()), Some("4"));
        assert_eq!(
            pin_key(kit.symbol.as_ref().unwrap(), kit.pin("GPIO2").unwrap()),
            "GPIO2"
        );
        assert_eq!(
            pin_key(kit.symbol.as_ref().unwrap(), kit.pin("GND").unwrap()),
            "9",
            "GND repeats"
        );
        let (x0, y0, x1, y1) = part_box(&kit);
        assert_eq!((x0, y0), (460.0, 40.0));
        assert!((x1 - 610.0).abs() < 1e-9 && (y1 - (40.0 + kit_height(rows.len()))).abs() < 1e-9);
    }

    /// A turned part moves its pins with it, and the wire still leaves
    /// each pin away from the body. Mirroring swaps the sides without
    /// reversing the order of pins on one side.
    #[test]
    fn a_turned_or_mirrored_part_moves_its_pins_with_it() {
        let mut part = led(200.0, 96.0);
        part.inst.rot = 90;
        let k = part.pin("K").unwrap().clone();
        assert_eq!(
            pin_point(&part, &k),
            (182.0, 92.0),
            "a quarter turn lays the legs to the left"
        );
        assert_eq!(pin_out(&part, &k), (-1.0, 0.0));
        part.inst.rot = 0;
        part.inst.mirror = true;
        assert_eq!(
            pin_point(&part, &k),
            (204.0, 114.0),
            "mirrored: the cathode's leg is on the right"
        );
        assert_eq!(pin_out(&part, &k), (0.0, 1.0), "and still points down");
        assert_eq!(orient((1.0, 0.0), 180, false), (-1.0, 0.0));
        assert_eq!(orient((1.0, 0.0), 270, false), (0.0, -1.0));
        assert_eq!(rotate_about((10.0, 0.0), (0.0, 0.0), 90), (0.0, 10.0));
    }

    #[test]
    fn a_wire_runs_only_in_right_angles_and_ends_on_both_pins() {
        let parts = vec![kit(460.0, 40.0), led(200.0, 96.0)];
        let wire = Wire {
            from: PinRef::new("U1", "GPIO2"),
            to: PinRef::new("D1", "A"),
            bends: Vec::new(),
        };
        let ends = wire_ends(&parts, &wire).expect("both pins exist");
        let path = wire_path(&ends, &wire.bends);
        assert_eq!(path[0], ends[0].0);
        assert_eq!(*path.last().unwrap(), (204.0, 120.0));
        for pair in path.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            assert!(
                (a.0 - b.0).abs() < 0.01 || (a.1 - b.1).abs() < 0.01,
                "diagonal: {a:?}->{b:?}"
            );
        }
        // Bends are honoured and the elbows they need are added.
        let bent = wire_path(&ends, &[(300.0, 200.0)]);
        assert!(bent.contains(&(300.0, 200.0)));
        assert!(bent.len() >= 4);
        // A missing pin is no path at all.
        let bad = Wire {
            from: PinRef::new("U1", "GPIO99"),
            to: PinRef::new("D1", "A"),
            bends: Vec::new(),
        };
        assert!(wire_ends(&parts, &bad).is_none());
    }

    #[test]
    fn a_dragged_part_slides_the_leg_beside_it_and_a_tidy_drops_collinear_bends() {
        let parts = vec![kit(460.0, 40.0), led(200.0, 96.0)];
        let wires = vec![Wire {
            from: PinRef::new("D1", "A"),
            to: PinRef::new("U1", "GPIO2"),
            bends: vec![(300.0, 120.0), (300.0, 200.0)],
        }];
        let legs = wire_legs(&parts, &wires, &[1]);
        assert_eq!(
            legs,
            vec![(0, Some(true), None)],
            "the anode's leg runs across; the kit is not moving"
        );
        let mut bend = wires[0].bends[0];
        follow_bend((230.0, 140.0), Some(true), &mut bend);
        assert_eq!(bend, (300.0, 140.0), "slid along its own axis");

        let mut wire = wires[0].clone();
        wire.bends = vec![(240.0, 120.0), (300.0, 120.0), (300.0, 200.0)];
        let ends = wire_ends(&parts, &wire).unwrap();
        retidy(&mut wire, &ends);
        assert_eq!(
            wire.bends,
            vec![(300.0, 120.0), (300.0, 200.0)],
            "the bend on the straight run folds away"
        );
    }

    #[test]
    fn pins_and_parts_are_found_within_reach_and_nowhere_else() {
        let parts = vec![kit(460.0, 40.0), led(200.0, 96.0)];
        assert_eq!(
            pin_under(&parts, (206.0, 122.0), 10.0),
            Some((1, "2".to_string())),
            "the anode's leg end"
        );
        assert_eq!(pin_under(&parts, (222.0, 140.0), 10.0), None);
        let (kx, ky) = row_offset(rows().len(), 3);
        assert_eq!(
            pin_under(&parts, (460.0 + kx + 2.0, 40.0 + ky), 10.0),
            Some((0, "4".to_string()))
        );
        assert_eq!(
            parts_in_box(&parts, (150.0, 60.0), (190.0, 130.0)),
            vec![1],
            "touching the lamp's box"
        );
        assert_eq!(parts_in_box(&parts, (0.0, 0.0), (700.0, 400.0)), vec![0, 1]);
        assert!(parts_in_box(&parts, (0.0, 0.0), (100.0, 50.0)).is_empty());
        let (min, max) = bounds(&parts);
        assert!(min.0 <= 190.0 && max.0 >= 610.0, "{min:?} {max:?}");
    }

    /// The pin names sit beside the leads, and a pin the library never
    /// named — a resistor's, a capacitor's — carries no writing at all: a
    /// real part has nothing printed on its legs.
    #[test]
    fn the_pin_names_are_written_beside_the_leads_and_nothing_else_is() {
        let part = led(200.0, 96.0);
        let labels = pin_labels(&part);
        let names: Vec<&str> = labels.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(names, vec!["A", "K"], "names, and no pin numbers");
        let anode = &labels[0];
        assert_eq!(anode.anchor, "start");
        assert!(
            anode.x > 204.0 && (anode.y - 120.0).abs() < 6.0,
            "beside the leg, not under the wire: {anode:?}"
        );

        // A turned part's writing moves with it and stays the same words;
        // the view never rotates it.
        let mut turned = part.clone();
        turned.inst.rot = 90;
        let moved = pin_labels(&turned);
        assert_eq!(moved.len(), labels.len());
        assert_ne!(moved[0].x, labels[0].x);
        assert_eq!(moved[0].text, "A");

        let mut bare = part.clone();
        if let Some(symbol) = bare.symbol.as_mut() {
            for pin in &mut symbol.pins {
                pin.name = "~".into();
            }
        }
        assert!(pin_labels(&bare).is_empty(), "an unnamed pin says nothing");
    }

    /// The devkit is a part like any other, and turning it must not fling
    /// it off the sheet: its anchor is the top-left corner of a board three
    /// hundred pixels tall, so a turn about the anchor would move it by its
    /// own length. What is asserted is the property, not the numbers — the
    /// box's middle stays where it was, and its pins go with it.
    #[test]
    fn the_devkit_turns_about_its_middle_and_the_turn_survives_the_sheet() {
        let mut sheet = Sheet::empty("esp32c3");
        sheet.kit_x = Some(300.0);
        sheet.kit_y = Some(20.0);
        let mut parts = parts_of(&sheet, &rows());
        let middle = |part: &EditPart| {
            let (x0, y0, x1, y1) = part_box(part);
            ((x0 + x1) / 2.0, (y0 + y1) / 2.0)
        };
        let before = middle(&parts[0]);
        let pin_before = parts[0].pin("GPIO2").map(|p| pin_point(&parts[0], p));
        assert!(pin_before.is_some(), "the kit has a GPIO2 to wire to");

        crate::view::panels::simulate::edit::rotate(&mut parts, 0);
        assert_eq!(parts[0].inst.rot, 90);
        let after = middle(&parts[0]);
        assert!(
            (after.0 - before.0).abs() < 0.001 && (after.1 - before.1).abs() < 0.001,
            "the board spins in place, not away: {before:?} -> {after:?}"
        );
        assert_ne!(
            parts[0].pin("GPIO2").map(|p| pin_point(&parts[0], p)),
            pin_before,
            "and its header went with it"
        );

        crate::view::panels::simulate::edit::mirror(&mut parts, 0);
        let mirrored = middle(&parts[0]);
        assert!(
            (mirrored.0 - before.0).abs() < 0.001 && (mirrored.1 - before.1).abs() < 0.001,
            "a mirror stays put too"
        );

        let saved = sheet_of("esp32c3", &parts, &[]);
        assert_eq!((saved.kit_rot, saved.kit_mirror), (90, true));
        let reloaded = parts_of(&saved, &rows());
        assert_eq!(
            (reloaded[0].inst.rot, reloaded[0].inst.mirror),
            (90, true),
            "and comes back turned"
        );
    }

    #[test]
    fn the_sheet_round_trips_through_the_editors_parts() {
        let mut sheet = Sheet::empty("esp32c3");
        sheet.kit_x = Some(300.0);
        sheet.kit_y = Some(20.0);
        sheet.parts.push(led(200.0, 96.0).inst);
        sheet.symbols.push(led_symbol());
        sheet.wires.push(Wire {
            from: PinRef::new("U1", "GPIO2"),
            to: PinRef::new("D1", "A"),
            bends: vec![(300.0, 96.0)],
        });
        let parts = parts_of(&sheet, &rows());
        assert_eq!(parts.len(), 2);
        assert!(parts[0].is_kit() && parts[0].symbol.is_some());
        assert_eq!(
            parts[1].symbol.as_ref().map(|s| s.name.as_str()),
            Some("LED")
        );
        let again = sheet_of("esp32c3", &parts, &sheet.wires);
        assert_eq!(again.kit_x, Some(300.0));
        assert_eq!((again.kit_rot, again.kit_mirror), (0, false));
        assert_eq!(again.parts, sheet.parts);
        assert_eq!(again.wires, sheet.wires);
        assert!(again.symbols.is_empty(), "the backend resolves them");

        let mut orphan = sheet.clone();
        orphan.symbols.clear();
        let parts = parts_of(&orphan, &rows());
        assert!(parts[1].symbol.is_none());
        assert_eq!(
            part_box(&parts[1]),
            (168.0, 80.0, 232.0, 112.0),
            "the unknown box"
        );
    }

    #[test]
    fn the_snap_grid_rounds_both_ways() {
        assert_eq!(snap_to(11.0, 8.0), 8.0);
        assert_eq!(snap_to(13.0, 8.0), 16.0);
        assert_eq!(snap_to(12.3, 1.0), 12.0);
    }

    /// module, so a board can be told apart on screen and in a test.
    #[test]
    fn every_espressif_part_is_drawn_as_its_devkit() {
        for chip in [
            "esp32", "esp32s2", "esp32s3", "esp32c2", "esp32c3", "esp32c6", "esp32h2", "esp32p4",
        ] {
            let style = kit_style(chip);
            let module = style
                .module
                .unwrap_or_else(|| panic!("{chip} has no module"));
            let art = kit_art(style, kit_height(26), "ESP32");
            assert!(
                art.contains(module),
                "{chip}: the can is printed with {module}"
            );
            assert!(art.contains("kit-can"), "{chip}: a shield can");
            assert!(art.contains(style.buttons.1), "{chip}: a BOOT button");
            assert_ne!(style.usb, Usb::None, "{chip}: a connector");
        }
        assert_eq!(kit_style("esp32").buttons.0, "EN");
        assert_eq!(kit_style("esp32c3").buttons.0, "RST");
        assert_eq!(kit_style("esp32s3").usb, Usb::DualTypeC);

        let bare = kit_style("stm32f103");
        assert_eq!(bare.module, None);
        let art = kit_art(bare, kit_height(10), "STM32F103");
        assert!(art.contains("STM32F103"));
        assert!(!art.contains("kit-can"), "no module, no can");
    }

    /// The buttons and the connector sit at the bottom edge whatever the
    /// pin count made the board's height — they are placed from `height`,
    /// not from the top.
    #[test]
    fn the_connector_follows_the_boards_height() {
        let style = kit_style("esp32c3");
        let short = kit_art(style, 200.0, "ESP32-C3");
        let tall = kit_art(style, 300.0, "ESP32-C3");
        assert!(short.contains(r#"y="188""#), "connector at 200-12: {short}");
        assert!(tall.contains(r#"y="288""#), "connector at 300-12: {tall}");
    }
}
