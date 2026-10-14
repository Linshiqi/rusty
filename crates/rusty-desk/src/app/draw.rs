//! The Draw tab: what a program drew with `rusty-draw` — the scene in a
//! space you can turn, every mark listed with its numbers, and the angles
//! between arrows that share a tail.
//!
//! The angles are the point. Perspective foreshortens every angle, so a
//! right angle cannot be judged by eye in a turned view; the list says it
//! in degrees, and the view draws the little square only where the numbers
//! the program printed make one (`rusty_embed::draw::angles`). What a scene
//! puts in space and how it is framed is `rusty_embed::scene::sketch`; the
//! camera and the projection are `rusty_embed::scene`.

use std::rc::Rc;

use rusty_embed::{
    draw::{self, DrawLine, Flaw, Hand, Mark, Shape, Sketch, Sketchbook},
    scene::{
        self, Camera, Ink, Look, Projector, Svg,
        sketch::{Framed, MARGIN, PALETTE, compose, frame_points, ink},
    },
    spatial::{
        Frame, Vec3,
        sheet::steps::{deg, num, quat, vec3},
    },
};
use rusty_i18n::t;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::App;
use crate::{DrawPiece, DrawRow, Shell};

/// What the tab holds between scenes.
pub(super) struct Drawings {
    book: Sketchbook,
    sketches: Vec<Sketch>,
    /// The newest scene's title, which the tab follows unless one is chosen.
    latest: Option<String>,
    chosen: Option<String>,
    /// The mark under the pointer in the list: drawn, the rest faint.
    hovered: Option<usize>,
    camera: Camera,
    fitted: Option<Framed>,
    /// This run has brought the tab forward already.
    fronted: bool,
    size: (f64, f64),
}

impl Default for Drawings {
    fn default() -> Drawings {
        Drawings {
            book: Sketchbook::default(),
            sketches: Vec::new(),
            latest: None,
            chosen: None,
            hovered: None,
            camera: Camera::look(Look::Iso),
            fitted: None,
            fronted: false,
            size: (800.0, 480.0),
        }
    }
}

impl Drawings {
    pub(super) fn is_empty(&self) -> bool {
        self.sketches.is_empty()
    }

    /// The scene the tab shows: the chosen one while it is still there,
    /// else the newest.
    fn shown(&self) -> Option<&Sketch> {
        let title = self
            .chosen
            .as_ref()
            .filter(|chosen| self.sketches.iter().any(|s| &s.title == *chosen))
            .or(self.latest.as_ref())?;
        self.sketches.iter().find(|s| &s.title == title)
    }

    /// Framed again for a new scene, one grown or shrunk past a factor of
    /// two, or a view reshaped by a third; anything less keeps the zoom
    /// somebody chose.
    fn frame(&mut self) {
        let (w, h) = self.size;
        let Some((title, reach, points)) = self
            .shown()
            .map(|s| (s.title.clone(), s.reach(), frame_points(s)))
        else {
            return;
        };
        let stale = self
            .fitted
            .as_ref()
            .is_none_or(|framed| framed.stale(&title, reach, (w, h)));
        if stale {
            self.fitted = Some(Framed {
                title,
                reach,
                size: (w, h),
            });
            self.camera = self.camera.framing(&points, Frame::ZUp, w, h, MARGIN);
        }
    }
}

/// What a mark is called: its label, or its kind and its place.
fn name_of(index: usize, mark: &Mark) -> String {
    if !mark.label.is_empty() {
        return mark.label.clone();
    }
    let kind = match mark.shape {
        Shape::Vector { .. } => t!("draw.kind-vector"),
        Shape::Point { .. } => t!("draw.kind-point"),
        Shape::Line { .. } => t!("draw.kind-line"),
        Shape::Span { .. } => t!("draw.kind-span"),
        Shape::Frame { .. } => t!("draw.kind-frame"),
    };
    format!("{kind} {}", index + 1)
}

/// A mark's numbers, and the one fact worth reading beside them.
fn describe(mark: &Mark) -> (String, String) {
    let from = |at: Vec3| (at != Vec3::ZERO).then(|| t!("draw.at", point = vec3(at)));
    match mark.shape {
        Shape::Vector { at, v } => {
            let length = t!("draw.length", length = num(v.norm()));
            let detail = match from(at) {
                Some(from) => format!("{length} · {from}"),
                None => length,
            };
            (vec3(v), detail)
        }
        Shape::Point { at } => (vec3(at), String::new()),
        Shape::Line { a, b } => (
            format!("{} → {}", vec3(a), vec3(b)),
            t!("draw.length", length = num((b - a).norm())),
        ),
        Shape::Span { at, a, b } => {
            let area = t!("draw.area", area = num(a.cross(b).norm()));
            let detail = match from(at) {
                Some(from) => format!("{area} · {from}"),
                None => area,
            };
            (format!("{}, {}", vec3(a), vec3(b)), detail)
        }
        Shape::Frame { q } => (quat(q), String::new()),
    }
}

fn flaw_text(flaw: Flaw) -> String {
    match flaw {
        Flaw::NotFinite => t!("draw.not-finite"),
        Flaw::Zero => t!("draw.zero"),
        Flaw::NotUnit(length) => t!("draw.not-unit", length = num(length)),
    }
}

/// An ink as the panel names it: a theme's colour by number, or the colour
/// itself.
/// Drawn pieces as the rows a view of space draws — the Draw tab's and the
/// math toolbox's.
pub(super) fn pieces_of(drawn: Vec<scene::Drawn>) -> Vec<DrawPiece> {
    let mut pieces = Vec::new();
    for drawn in drawn {
        let (ink, rgb) = ink_of(drawn.ink);
        let mut piece = DrawPiece {
            ink,
            rgb,
            width: drawn.width as f32,
            opacity: drawn.opacity as f32,
            fill: drawn.fill as f32,
            ..DrawPiece::default()
        };
        match drawn.svg {
            Svg::Path(path) => {
                piece.kind = 0;
                piece.commands = path.into();
            }
            Svg::Polygon(points) => {
                piece.kind = 1;
                piece.commands = polygon(&points).into();
            }
            Svg::Text { x, y, text } => {
                piece.kind = 2;
                piece.x = x as f32;
                piece.y = y as f32;
                piece.text = text.into();
            }
            Svg::Dot { x, y, r } => {
                piece.kind = 3;
                piece.x = x as f32;
                piece.y = y as f32;
                piece.r = r as f32;
            }
        }
        pieces.push(piece);
    }
    pieces
}

pub(super) fn ink_of(ink: Ink) -> (i32, slint::Color) {
    let rgb = |hex: &str| {
        let value = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0);
        slint::Color::from_rgb_u8((value >> 16) as u8, (value >> 8) as u8, value as u8)
    };
    match ink {
        Ink::AxisX => (10, rgb("#e5484d")),
        Ink::AxisY => (10, rgb("#30a46c")),
        Ink::AxisZ => (10, rgb("#3e63dd")),
        Ink::Grid => (3, rgb("#000000")),
        Ink::Body => (4, rgb("#000000")),
        Ink::Front | Ink::Result => (5, rgb("#000000")),
        Ink::Rotor | Ink::Ghost | Ink::Guide => (6, rgb("#000000")),
        Ink::First => (7, rgb("#000000")),
        Ink::Second => (8, rgb("#000000")),
        Ink::Term => (9, rgb("#000000")),
        Ink::Row(i) => (10, rgb(PALETTE[usize::from(i) % PALETTE.len()])),
        Ink::Rgb(r, g, b) => (10, slint::Color::from_rgb_u8(r, g, b)),
    }
}

/// `x,y x,y …` as path commands, closed.
pub(super) fn polygon(points: &str) -> String {
    let mut out = String::new();
    for (index, pair) in points.split_whitespace().enumerate() {
        out.push_str(if index == 0 { "M " } else { " L " });
        out.push_str(&pair.replace(',', " "));
    }
    out.push_str(" Z");
    out
}

impl App {
    /// A `[rusty:draw]` line, whatever stream it came on. A scene is shown
    /// when it ends, not mark by mark.
    pub(super) fn drawn(self: &Rc<Self>, line: DrawLine) {
        let sketch = self.state.borrow_mut().draw.book.read(line);
        if let Some(sketch) = sketch {
            self.file_sketch(sketch);
        }
    }

    /// A run ended: a scene it began and never ended is shown as far as it
    /// got — a test that panicked mid-scene drew what it drew.
    pub(super) fn drawing_ends(self: &Rc<Self>) {
        let sketch = self.state.borrow_mut().draw.book.close();
        if let Some(sketch) = sketch {
            self.file_sketch(sketch);
        }
        self.state.borrow_mut().draw.fronted = false;
    }

    fn file_sketch(self: &Rc<Self>, sketch: Sketch) {
        let fronted = {
            let mut state = self.state.borrow_mut();
            let draw = &mut state.draw;
            draw.latest = Some(sketch.title.clone());
            draw::file(&mut draw.sketches, sketch);
            draw.frame();
            std::mem::replace(&mut draw.fronted, true)
        };
        // The first scene of a run brings the tab forward — drawing is what
        // the program was run for — and every later one leaves the dock
        // alone.
        if fronted {
            self.sync_docks();
        } else {
            self.show_panel("bottom", "draw");
        }
        self.sync_draw();
    }

    pub(super) fn draw_resized(self: &Rc<Self>, width: f32, height: f32) {
        {
            let mut state = self.state.borrow_mut();
            state.draw.size = (f64::from(width).max(1.0), f64::from(height).max(1.0));
            state.draw.frame();
        }
        self.sync_draw();
    }

    pub(super) fn draw_orbit(self: &Rc<Self>, dx: f32, dy: f32) {
        {
            let mut state = self.state.borrow_mut();
            let camera = &mut state.draw.camera;
            *camera = camera.orbit(f64::from(dx), f64::from(dy), Frame::ZUp);
        }
        self.sync_draw();
    }

    pub(super) fn draw_zoom(self: &Rc<Self>, delta: f32) {
        {
            let mut state = self.state.borrow_mut();
            let camera = &mut state.draw.camera;
            *camera = camera.zoomed((f64::from(delta) * 0.0015).exp());
        }
        self.sync_draw();
    }

    /// A named view, framed afresh: from the top a scene can be twice as
    /// wide on the page as it was from the side. `None` frames the view
    /// there is.
    pub(super) fn draw_look(self: &Rc<Self>, look: Option<Look>) {
        {
            let mut state = self.state.borrow_mut();
            let draw = &mut state.draw;
            let (w, h) = draw.size;
            let points = draw.shown().map(frame_points).unwrap_or_default();
            let from = look.map(Camera::look).unwrap_or(draw.camera);
            draw.camera = from.framing(&points, Frame::ZUp, w, h, MARGIN);
        }
        self.sync_draw();
    }

    /// A scene from the list; the newest's own row follows the newest again.
    pub(super) fn draw_pick(self: &Rc<Self>, title: &str) {
        {
            let mut state = self.state.borrow_mut();
            let draw = &mut state.draw;
            draw.chosen = (draw.latest.as_deref() != Some(title)).then(|| title.to_string());
            draw.hovered = None;
            draw.frame();
        }
        self.sync_draw();
    }

    pub(super) fn draw_hover(self: &Rc<Self>, index: i32) {
        {
            let mut state = self.state.borrow_mut();
            let hovered = usize::try_from(index).ok();
            if state.draw.hovered == hovered {
                return;
            }
            state.draw.hovered = hovered;
        }
        self.sync_draw();
    }

    pub(super) fn draw_clear(self: &Rc<Self>) {
        {
            let mut state = self.state.borrow_mut();
            let draw = &mut state.draw;
            draw.sketches.clear();
            draw.latest = None;
            draw.chosen = None;
            draw.hovered = None;
            draw.fitted = None;
        }
        self.sync_docks();
        self.sync_draw();
    }

    pub(super) fn sync_draw(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let state = self.state.borrow();
        let draw = &state.draw;
        let shown = draw.shown();
        let mut rows = Vec::new();
        let row = |kind: i32, text: String| DrawRow {
            kind,
            text: text.into(),
            ..DrawRow::default()
        };
        // Every scene a run drew, one row each, the one on the page lit.
        if draw.sketches.len() > 1 {
            for sketch in &draw.sketches {
                let title = if sketch.title.is_empty() {
                    t!("draw.untitled")
                } else {
                    sketch.title.clone()
                };
                rows.push(DrawRow {
                    kind: 0,
                    text: title.into(),
                    key: sketch.title.as_str().into(),
                    on: shown.is_some_and(|s| s.title == sketch.title),
                    ..DrawRow::default()
                });
            }
        }
        let mut pieces = Vec::new();
        if let Some(sketch) = shown {
            let mut notes = Vec::new();
            if !sketch.finished {
                notes.push(t!("draw.unfinished"));
            }
            if sketch.truncated {
                notes.push(t!("draw.truncated", count = draw::MAX_MARKS.to_string()));
            }
            if !notes.is_empty() {
                rows.push(row(1, notes.join(" · ")));
            }
            for (index, mark) in sketch.marks.iter().enumerate() {
                let (numbers, detail) = describe(mark);
                let (_, colour) = ink_of(ink(index, mark));
                rows.push(DrawRow {
                    kind: 2,
                    text: name_of(index, mark).into(),
                    detail: detail.into(),
                    numbers: numbers.into(),
                    flaw: draw::flaw(mark).map(flaw_text).unwrap_or_default().into(),
                    colour,
                    index: index as i32,
                    ..DrawRow::default()
                });
            }
            let angles = draw::angles(&sketch.marks);
            let hand = draw::handedness(&sketch.marks);
            if !angles.is_empty() || hand.is_some() {
                rows.push(row(3, t!("draw.angles")));
                let name = |i: usize| {
                    sketch
                        .marks
                        .get(i)
                        .map(|m| name_of(i, m))
                        .unwrap_or_default()
                };
                for angle in &angles {
                    rows.push(DrawRow {
                        kind: 4,
                        text: format!("{} ∠ {}", name(angle.a), name(angle.b)).into(),
                        numbers: deg(angle.radians).into(),
                        on: angle.square,
                        ..DrawRow::default()
                    });
                }
                if let Some(triple) = hand {
                    let [a, b, c] = triple.marks.map(name);
                    let (text, odd) = match triple.hand {
                        Hand::Right => (t!("draw.hand-right", a = a, b = b, c = c), false),
                        Hand::Left => (t!("draw.hand-left", a = a, b = b, c = c), true),
                        Hand::Flat => (t!("draw.hand-flat", a = a, b = b, c = c), false),
                    };
                    rows.push(DrawRow {
                        kind: 5,
                        text: text.into(),
                        on: odd,
                        ..DrawRow::default()
                    });
                }
            }
            let (w, h) = draw.size;
            let items = compose(sketch, &angles, draw.hovered);
            pieces = pieces_of(scene::render(
                &items,
                &Projector::new(&draw.camera, Frame::ZUp, w, h),
            ));
        }
        let heading = match draw.sketches.as_slice() {
            [] => String::new(),
            [only] if only.title.is_empty() => t!("draw.untitled"),
            [only] => only.title.clone(),
            many => t!("draw.scenes", count = many.len().to_string()),
        };
        shell.set_draw_heading(SharedString::from(heading));
        shell.set_draw_title(SharedString::from(
            shown.map(|s| s.title.clone()).unwrap_or_default(),
        ));
        shell.set_draw_empty(draw.sketches.is_empty());
        shell.set_draw_rows(ModelRc::new(VecModel::from(rows)));
        shell.set_draw_pieces(ModelRc::new(VecModel::from(pieces)));
        shell.set_draw_width(draw.size.0 as f32);
        shell.set_draw_height(draw.size.1 as f32);
    }
}
