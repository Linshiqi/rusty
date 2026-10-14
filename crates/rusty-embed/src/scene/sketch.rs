//! What a program drew with `rusty-draw` (`crate::draw`), as a scene the
//! view projects: the floor and the axes, every mark in its colour, the
//! little square where the numbers make a right angle, and the framing
//! that holds it all — the Draw tab's arithmetic, shared by both windows.

use crate::draw::{self, Angle, Flaw, Mark, Shape, Sketch};
use crate::spatial::{TINY, Vec3};

use super::{Ink, Item, model};

/// A row's colour, or a drawn mark's: eight that read apart on the dark and
/// the light theme alike, fixed for the reason the Git lanes are — a vector
/// that changed colour with the theme would read as another vector.
pub const PALETTE: [&str; 8] = [
    "#5b9df0", "#e8a33d", "#3fb68b", "#d9658a", "#9a7bf0", "#4fc1d1", "#c7b14a", "#e06c4f",
];

/// A grid spacing near `target` that reads: 1, 2 or 5 times a power of ten.
pub fn nice_step(target: f64) -> f64 {
    if !(target.is_finite() && target > 0.0) {
        return 1.0;
    }
    let power = 10f64.powf(target.log10().floor());
    let m = target / power;
    let nice = if m < 1.5 {
        1.0
    } else if m < 3.5 {
        2.0
    } else if m < 7.5 {
        5.0
    } else {
        10.0
    };
    nice * power
}

/// The scene a camera was framed for and the view it was framed in: a new
/// scene, this one grown or shrunk past a factor of two, or a view that
/// changed shape by a third is framed again; anything less keeps the zoom
/// somebody chose.
#[derive(Debug, Clone, PartialEq)]
pub struct Framed {
    pub title: String,
    /// How far the scene reached.
    pub reach: f64,
    /// The view, across and down, in pixels.
    pub size: (f64, f64),
}

impl Framed {
    /// Whether a scene and a view this far from what was framed want
    /// framing again.
    pub fn stale(&self, title: &str, reach: f64, size: (f64, f64)) -> bool {
        let moved = |now: f64, then: f64| (now - then).abs() > then * 0.33;
        self.title != title
            || reach > self.reach * 2.0
            || reach < self.reach / 2.0
            || moved(size.0, self.size.0)
            || moved(size.1, self.size.1)
    }
}

/// How far a sketch reaches, as a size to draw by: a unit when nothing
/// drawn has one.
pub fn scale(reach: f64) -> f64 {
    if reach > TINY && reach.is_finite() {
        reach
    } else {
        1.0
    }
}

/// Every point a sketch's marks put on the page, where their names are
/// written included; nothing broken.
pub fn mark_points(sketch: &Sketch) -> Vec<Vec3> {
    let mut points = Vec::new();
    for mark in &sketch.marks {
        if draw::flaw(mark) == Some(Flaw::NotFinite) {
            continue;
        }
        match mark.shape {
            Shape::Vector { at, v } => points.extend([at, at + v, at + v * 1.06]),
            Shape::Point { at } => points.push(at),
            Shape::Line { a, b } => points.extend([a, b]),
            Shape::Span { at, a, b } => points.extend([at, at + a, at + a + b, at + b]),
            Shape::Frame { q } => {
                if let Some(q) = q.normalized() {
                    points.extend([Vec3::X, Vec3::Y, Vec3::Z].map(|axis| q.rotate(axis) * 1.12));
                }
            }
        }
    }
    points
}

/// How far each axis is drawn: a little past the furthest the marks reach
/// along it, so its name is read beyond them, and never less than a
/// quarter of the scene — a scene lying flat still shows which way Z
/// points. One length for all three let the longest decide the page, and
/// the arrows the page was for came out small.
pub fn axis_lengths(sketch: &Sketch) -> [f64; 3] {
    let least = scale(sketch.reach()) * 0.25;
    let mut far = [least; 3];
    for p in mark_points(sketch) {
        far[0] = far[0].max(p.x);
        far[1] = far[1].max(p.y);
        far[2] = far[2].max(p.z);
    }
    far.map(|length| length * 1.15)
}

/// The world's axes from the origin, each as long as it is given and named
/// at its end.
pub fn axes(lengths: [f64; 3]) -> Vec<Item> {
    [
        (Vec3::X, Ink::AxisX, "X"),
        (Vec3::Y, Ink::AxisY, "Y"),
        (Vec3::Z, Ink::AxisZ, "Z"),
    ]
    .into_iter()
    .zip(lengths)
    .flat_map(|((dir, ink, name), length)| {
        [
            Item::Line {
                a: Vec3::ZERO,
                b: dir * length,
                ink,
                width: 1.2,
                dashed: false,
            },
            Item::Label {
                at: dir * (length * 1.06),
                text: name.to_string(),
                ink,
            },
        ]
    })
    .collect()
}

/// The floor and the axes for a sketch: a grid on the XY plane out beyond
/// everything, at a spacing that reads, and X, Y and Z named at their ends.
pub fn floor(sketch: &Sketch) -> Vec<Item> {
    let reach = scale(sketch.reach());
    let step = nice_step(reach / 2.5);
    let half = (reach * 1.1 / step).ceil() * step;
    let mut items = model::grid(half, step);
    items.extend(axes(axis_lengths(sketch)));
    items
}

/// Every point the view has to show: the marks, where their names are
/// written, and the axes' ends with theirs. The grid is left out — it is a
/// floor to stand the drawing on, and may run off the page.
pub fn frame_points(sketch: &Sketch) -> Vec<Vec3> {
    let [x, y, z] = axis_lengths(sketch);
    let mut points = vec![
        Vec3::ZERO,
        Vec3::X * (x * 1.06),
        Vec3::Y * (y * 1.06),
        Vec3::Z * (z * 1.06),
    ];
    points.extend(mark_points(sketch));
    points
}

/// How far in from the page's edges a framing keeps everything, across and
/// down: room for a name written beside a tip, which runs across.
pub const MARGIN: (f64, f64) = (30.0, 16.0);

/// The colour a mark is drawn in: the one its program chose, or the
/// palette's colour for its place in the list.
pub fn ink(index: usize, mark: &Mark) -> Ink {
    match mark.color {
        Some([r, g, b]) => Ink::Rgb(r, g, b),
        None => Ink::Row((index % PALETTE.len()) as u8),
    }
}

/// What one mark draws.
pub fn items_of(mark: &Mark, ink: Ink) -> Vec<Item> {
    let label = |at: Vec3| {
        (!mark.label.is_empty()).then(|| Item::Label {
            at,
            text: mark.label.clone(),
            ink,
        })
    };
    let dot = |at: Vec3| Item::Dot {
        at,
        ink,
        radius: 3.5,
    };
    match mark.shape {
        // No length and so no direction: a dot where it would start.
        Shape::Vector { at, v } if v.norm() <= TINY => {
            [Some(dot(at)), label(at)].into_iter().flatten().collect()
        }
        Shape::Vector { at, v } => [
            Some(Item::Arrow {
                from: at,
                to: at + v,
                ink,
                width: 2.2,
            }),
            label(at + v * 1.06),
        ]
        .into_iter()
        .flatten()
        .collect(),
        Shape::Point { at } => [Some(dot(at)), label(at)].into_iter().flatten().collect(),
        Shape::Line { a, b } => [
            Some(Item::Line {
                a,
                b,
                ink,
                width: 1.8,
                dashed: false,
            }),
            label((a + b) * 0.5),
        ]
        .into_iter()
        .flatten()
        .collect(),
        Shape::Span { at, a, b } => [
            Some(Item::Polygon {
                points: vec![at, at + a, at + a + b, at + b],
                ink,
                fill: 0.16,
            }),
            label(at + (a + b) * 0.5),
        ]
        .into_iter()
        .flatten()
        .collect(),
        Shape::Frame { q } => {
            let Some(q) = q.normalized() else {
                return Vec::new();
            };
            // A frame its program coloured is that colour throughout — two
            // attitudes in one scene told apart — and otherwise its axes
            // keep the red, green and blue of the axes they stand for.
            let chosen = mark.color.map(|_| ink);
            let mut items = model::triad(q, 1.0, false);
            if let Some(chosen) = chosen {
                for item in &mut items {
                    if let Item::Arrow { ink, .. } = item {
                        *ink = chosen;
                    }
                }
            }
            // Each of the body's axes named at its end — `body x`, `body y`,
            // `body z`. Named at its X alone, the other two arrows were
            // nobody's, and the name read as the label of whatever else ended
            // near the X axis's tip.
            for (axis, name, axis_ink) in [
                (Vec3::X, "x", Ink::AxisX),
                (Vec3::Y, "y", Ink::AxisY),
                (Vec3::Z, "z", Ink::AxisZ),
            ] {
                let ink = chosen.unwrap_or(axis_ink);
                let text = if mark.label.is_empty() {
                    name.to_string()
                } else {
                    format!("{} {name}", mark.label)
                };
                items.push(Item::Label {
                    at: q.rotate(axis) * 1.12,
                    text,
                    ink,
                });
            }
            items
        }
    }
}

/// The little square that marks a right angle between `v` and `w` at
/// their tail, a sixth of the shorter one on a side.
pub fn right_angle(at: Vec3, v: Vec3, w: Vec3) -> Option<Item> {
    let side = v.norm().min(w.norm()) / 6.0;
    let (u, x) = (v.normalized()? * side, w.normalized()? * side);
    Some(Item::Polyline {
        points: vec![at + u, at + u + x, at + x],
        ink: Ink::Guide,
        width: 1.3,
        dashed: false,
    })
}

/// Everything in the view of one sketch: the floor, every mark that can be
/// drawn in its colour, and a square where the numbers found a right
/// angle. With a mark under the pointer in the list, the rest are faint.
pub fn compose(sketch: &Sketch, angles: &[Angle], hovered: Option<usize>) -> Vec<(Item, f64)> {
    let strength = |marks: &[usize]| match hovered {
        Some(h) if !marks.contains(&h) => 0.25,
        _ => 1.0,
    };
    let mut out: Vec<(Item, f64)> = floor(sketch).into_iter().map(|item| (item, 1.0)).collect();
    for (i, mark) in sketch.marks.iter().enumerate() {
        if draw::flaw(mark) == Some(Flaw::NotFinite) {
            continue;
        }
        let o = strength(&[i]);
        out.extend(
            items_of(mark, ink(i, mark))
                .into_iter()
                .map(|item| (item, o)),
        );
    }
    for angle in angles.iter().filter(|angle| angle.square) {
        let shape = |i: usize| sketch.marks.get(i).map(|mark| mark.shape);
        let (Some(Shape::Vector { at, v }), Some(Shape::Vector { v: w, .. })) =
            (shape(angle.a), shape(angle.b))
        else {
            continue;
        };
        if let Some(item) = right_angle(at, v, w) {
            out.push((item, strength(&[angle.a, angle.b])));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::parse_draw;

    fn sketch(lines: &[&str]) -> Sketch {
        let mut book = draw::Sketchbook::default();
        lines
            .iter()
            .filter_map(|line| book.read(parse_draw(line).expect(line)))
            .last()
            .expect("a scene")
    }

    fn arrows(items: &[(Item, f64)]) -> Vec<(Vec3, Vec3)> {
        items
            .iter()
            .filter_map(|(item, _)| match item {
                Item::Arrow {
                    from,
                    to,
                    ink: Ink::Row(_),
                    ..
                } => Some((*from, *to)),
                _ => None,
            })
            .collect()
    }

    fn squares(items: &[(Item, f64)]) -> usize {
        items
            .iter()
            .filter(|(item, _)| {
                matches!(
                    item,
                    Item::Polyline {
                        ink: Ink::Guide,
                        ..
                    }
                )
            })
            .count()
    }

    /// The report: two vectors and their cross product are three arrows,
    /// and the product meets each of the others in a marked right angle —
    /// two squares, and none between the two that are not square.
    #[test]
    fn a_cross_product_is_three_arrows_and_two_right_angles() {
        let s = sketch(&[
            "[rusty:draw] scene cross product",
            "[rusty:draw] vector 1 0.4 0 a",
            "[rusty:draw] vector 0.3 1 0.5 b",
            "[rusty:draw] vector 0.2 -0.5 0.88 a × b",
            "[rusty:draw] end",
        ]);
        let angles = draw::angles(&s.marks);
        let items = compose(&s, &angles, None);
        let drawn = arrows(&items);
        assert_eq!(drawn.len(), 3);
        assert_eq!(drawn[2], (Vec3::ZERO, Vec3::new(0.2, -0.5, 0.88)));
        assert_eq!(squares(&items), 2);
        let wrong = sketch(&[
            "[rusty:draw] vector 1 0.4 0 a",
            "[rusty:draw] vector 0.3 1 0.5 b",
            "[rusty:draw] vector -0.5 0.2 0.88 not a × b",
            "[rusty:draw] end",
        ]);
        let items = compose(&wrong, &draw::angles(&wrong.marks), None);
        assert_eq!(squares(&items), 0, "no square the numbers do not make");
    }

    /// Each square lies in the plane of its two arrows, at their tail, its
    /// corner a sixth of the shorter arrow along each.
    #[test]
    fn a_right_angle_is_drawn_between_its_two_arrows() {
        let Some(Item::Polyline { points, .. }) = right_angle(
            Vec3::new(1.0, 1.0, 1.0),
            Vec3::new(3.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 6.0),
        ) else {
            panic!("a square");
        };
        assert_eq!(
            points,
            [
                Vec3::new(1.5, 1.0, 1.0),
                Vec3::new(1.5, 1.0, 1.5),
                Vec3::new(1.0, 1.0, 1.5)
            ]
        );
        assert!(right_angle(Vec3::ZERO, Vec3::ZERO, Vec3::X).is_none());
    }

    /// A number that is not one draws nothing; a vector with no length is
    /// a dot where it starts; a frame is its three axes, turned.
    #[test]
    fn what_cannot_be_an_arrow_is_something_else_or_nothing() {
        let s = sketch(&[
            "[rusty:draw] vector NaN 0 0 broken",
            "[rusty:draw] vector_at 1 2 3 0 0 0 still",
            "[rusty:draw] frame 0.7071067811865476 0 0 0.7071067811865476 q",
            "[rusty:draw] end",
        ]);
        let items = compose(&s, &draw::angles(&s.marks), None);
        assert!(
            items
                .iter()
                .all(|(item, _)| !matches!(item, Item::Label { text, .. } if text == "broken"))
        );
        assert!(items.iter().any(
            |(item, _)| matches!(item, Item::Dot { at, .. } if *at == Vec3::new(1.0, 2.0, 3.0))
        ));
        let turned_x = items.iter().find_map(|(item, _)| match item {
            Item::Arrow {
                to,
                ink: Ink::AxisX,
                ..
            } => Some(*to),
            _ => None,
        });
        assert!(
            (turned_x.unwrap() - Vec3::Y).norm() < 1e-12,
            "a quarter turn about Z takes X to Y"
        );
    }

    #[test]
    fn a_mark_under_the_pointer_is_drawn_and_the_rest_are_faint() {
        let s = sketch(&[
            "[rusty:draw] vector 1 0 0 a",
            "[rusty:draw] vector 0 1 0 b",
            "[rusty:draw] end",
        ]);
        let angles = draw::angles(&s.marks);
        let items = compose(&s, &angles, Some(1));
        let strength = |to: Vec3| {
            items
                .iter()
                .find_map(|(item, o)| match item {
                    Item::Arrow {
                        to: t,
                        ink: Ink::Row(_),
                        ..
                    } if *t == to => Some(*o),
                    _ => None,
                })
                .unwrap()
        };
        assert_eq!(strength(Vec3::Y), 1.0);
        assert!(strength(Vec3::X) < 1.0);
    }

    /// The floor reaches past everything drawn, on a spacing that reads, and
    /// an empty scene still has a floor to stand on.
    #[test]
    fn the_floor_reaches_past_everything() {
        let far = |s: &Sketch| model::extent(&floor(s));
        let wide = sketch(&["[rusty:draw] point 7.3 0 0", "[rusty:draw] end"]);
        assert!(far(&wide) > 7.3);
        assert!(far(&Sketch::default()) >= 1.0);
    }

    /// Each axis runs a little past the furthest the marks reach along it,
    /// and no shorter than a quarter of the scene: a flat scene still shows
    /// Z, and a tall one does not stretch X and Y to its height.
    #[test]
    fn each_axis_reaches_just_past_the_marks_along_it() {
        let s = sketch(&[
            "[rusty:draw] vector 2 0 0 long",
            "[rusty:draw] vector 0 1 0 short",
            "[rusty:draw] end",
        ]);
        let [x, y, z] = axis_lengths(&s);
        assert!(x > 2.0 * 1.06 && x < 2.6, "{x}");
        assert!(y > 1.0 && y < x, "{y}");
        assert!(
            z > 0.0 && z < y,
            "Z is drawn, though nothing reaches up: {z}"
        );
    }

    /// The framing holds every mark that can be drawn and the names at the
    /// axes' ends — and nothing broken, which would frame nothing at all.
    #[test]
    fn the_framing_holds_every_mark_and_the_axes_names() {
        let s = sketch(&[
            "[rusty:draw] vector 0 0 2 up",
            "[rusty:draw] vector NaN 0 0 broken",
            "[rusty:draw] end",
        ]);
        let points = frame_points(&s);
        assert!(points.contains(&Vec3::new(0.0, 0.0, 2.0)));
        assert!(points.iter().all(|p| p.is_finite()));
        let [x, _, z] = axis_lengths(&s);
        assert!(z > 2.0);
        assert!(points.contains(&(Vec3::X * (x * 1.06))));
        assert!(points.contains(&(Vec3::Z * (z * 1.06))));
    }

    #[test]
    fn grid_steps_are_one_two_or_five() {
        for (target, nice) in [
            (0.7, 0.5),
            (1.2, 1.0),
            (2.6, 2.0),
            (40.0, 50.0),
            (0.012, 0.01),
            (7.9, 10.0),
        ] {
            assert!((nice_step(target) - nice).abs() < 1e-12, "{target}");
        }
        assert_eq!(nice_step(0.0), 1.0);
    }
}
