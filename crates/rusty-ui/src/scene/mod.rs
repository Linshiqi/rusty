//! A 3-D view drawn as SVG: a camera orbiting a point, a projection, and
//! shapes turned into paths sorted back to front. Pure and tested; the math
//! toolbox's view only puts the paths on the page.
//!
//! No 3-D library: there is no npm here, and a WebGL context for a few
//! dozen lines is more machinery than the lines. SVG draws them crisply at
//! any zoom, takes the theme's colours through classes, and a path per
//! shape is few enough that painting back to front is the whole of depth.

pub mod instrument;
pub mod model;

use rusty_embed::spatial::{Frame, Vec3};

/// The field of view, top to bottom.
pub const FOV: f64 = 35.0 * std::f64::consts::PI / 180.0;

/// Nearer than this and a point is behind the lens, and whatever it belongs
/// to is not drawn.
const NEAR: f64 = 0.05;

/// Where the camera stands: round a target, at a distance, looking at it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// Round the world's up, radians from +X towards +Y.
    pub azimuth: f64,
    /// Above the horizontal plane, radians.
    pub elevation: f64,
    pub distance: f64,
    pub target: Vec3,
}

/// The views a toolbar offers by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    /// Behind the aircraft, to its right and a little above: where a pilot's
    /// chase camera sits, and the view every attitude reads best from.
    Chase,
    /// From above, the nose pointing up the screen.
    Top,
    /// From ahead, looking back at the nose.
    Front,
    /// From the right wing.
    Side,
}

const MAX_ELEVATION: f64 = 89.5 * std::f64::consts::PI / 180.0;

impl Camera {
    pub fn preset(preset: Preset, frame: Frame) -> Camera {
        // The body's right wing is −Y with Z up (forward-left-up) and +Y with
        // Z down (forward-right-down), so "to the right" is a different
        // azimuth in each.
        let wing = match frame {
            Frame::ZUp => -1.0,
            Frame::ZDown => 1.0,
        };
        let (azimuth, elevation) = match preset {
            Preset::Chase => (
                std::f64::consts::PI - wing * 35f64.to_radians(),
                32f64.to_radians(),
            ),
            Preset::Top => (std::f64::consts::PI, MAX_ELEVATION),
            Preset::Front => (0.0, 0.0),
            Preset::Side => (wing * std::f64::consts::FRAC_PI_2, 0.0),
        };
        Camera {
            azimuth,
            elevation,
            distance: 4.5,
            target: Vec3::ZERO,
        }
    }

    /// Where the camera is, in world axes.
    pub fn eye(&self, frame: Frame) -> Vec3 {
        let (se, ce) = self.elevation.sin_cos();
        let (sa, ca) = self.azimuth.sin_cos();
        self.target + (Vec3::new(ce * ca, ce * sa, 0.0) + frame.up() * se) * self.distance
    }

    /// Turned by a drag of `dx`, `dy` pixels: the scene follows the
    /// pointer, so dragging right brings the target's left side round to
    /// the front — in either frame, although one's azimuth runs the other
    /// way round the other's up.
    pub fn orbit(self, dx: f64, dy: f64, frame: Frame) -> Camera {
        let handed = frame.up().z;
        Camera {
            azimuth: self.azimuth - handed * dx * 0.008,
            elevation: (self.elevation + dy * 0.008).clamp(-MAX_ELEVATION, MAX_ELEVATION),
            ..self
        }
    }

    /// Closer (below one) or further (above), within reason.
    pub fn zoomed(self, factor: f64) -> Camera {
        Camera {
            distance: (self.distance * factor).clamp(0.3, 500.0),
            ..self
        }
    }

    /// Far enough that a sphere of `radius` about the target fills the view
    /// with a margin.
    pub fn fitting(self, radius: f64) -> Camera {
        let radius = radius.max(0.2);
        Camera {
            distance: radius / (FOV / 2.0).sin() * 1.15,
            ..self
        }
    }
}

/// A point on the page, and how far into it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
    pub depth: f64,
}

/// World to page: the camera's axes and the lens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Projector {
    eye: Vec3,
    right: Vec3,
    up: Vec3,
    forward: Vec3,
    /// Pixels per unit of `x/depth`, or per unit of length when flat.
    focal: f64,
    /// A plane seen square on: no perspective, a fixed depth.
    flat: bool,
    cx: f64,
    cy: f64,
}

impl Projector {
    pub fn new(camera: &Camera, frame: Frame, width: f64, height: f64) -> Projector {
        let eye = camera.eye(frame);
        let forward = (camera.target - eye).normalized().unwrap_or(Vec3::X);
        let up_world = frame.up();
        // Straight up or down the world's up there is no right; the
        // elevation is clamped short of that, and this is the fallback.
        let right = forward
            .cross(up_world)
            .normalized()
            .unwrap_or_else(|| forward.any_perpendicular());
        let up = right.cross(forward);
        Projector {
            eye,
            right,
            up,
            forward,
            focal: (height / 2.0) / (FOV / 2.0).tan(),
            flat: false,
            cx: width / 2.0,
            cy: height / 2.0,
        }
    }

    /// The plane, seen from above with X to the right and Y up the page:
    /// `scale` pixels to a unit, `centre` in the middle.
    pub fn plane(scale: f64, centre: (f64, f64), width: f64, height: f64) -> Projector {
        Projector {
            eye: Vec3::new(centre.0, centre.1, 10.0),
            right: Vec3::X,
            up: Vec3::Y,
            forward: -Vec3::Z,
            focal: scale,
            flat: true,
            cx: width / 2.0,
            cy: height / 2.0,
        }
    }

    pub fn project(&self, p: Vec3) -> Option<Point> {
        let v = p - self.eye;
        let depth = v.dot(self.forward);
        let (x, y) = (v.dot(self.right), v.dot(self.up));
        if self.flat {
            return Some(Point {
                x: self.cx + x * self.focal,
                y: self.cy - y * self.focal,
                depth,
            });
        }
        (depth > NEAR).then(|| Point {
            x: self.cx + self.focal * x / depth,
            y: self.cy - self.focal * y / depth,
            depth,
        })
    }
}

/// What something drawn stands for, which picks its colour and weight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Ink {
    AxisX,
    AxisY,
    AxisZ,
    Grid,
    /// The aircraft's frame and rear arms.
    Body,
    /// The front arms and the nose: which way it points, at a glance.
    Front,
    Rotor,
    /// An attitude before or partway through a turn.
    Ghost,
    First,
    Second,
    Term,
    Result,
    Guide,
    /// A row's own colour, from a small palette.
    Row(u8),
}

/// A thing to draw, in world axes.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Line {
        a: Vec3,
        b: Vec3,
        ink: Ink,
        width: f64,
        dashed: bool,
    },
    Arrow {
        from: Vec3,
        to: Vec3,
        ink: Ink,
        width: f64,
    },
    Polyline {
        points: Vec<Vec3>,
        ink: Ink,
        width: f64,
        dashed: bool,
    },
    /// A face, filled at `fill` opacity.
    Polygon {
        points: Vec<Vec3>,
        ink: Ink,
        fill: f64,
    },
    Label {
        at: Vec3,
        text: String,
        ink: Ink,
    },
}

/// One thing, ready for the page.
#[derive(Debug, Clone, PartialEq)]
pub struct Drawn {
    pub svg: Svg,
    pub ink: Ink,
    pub width: f64,
    pub dashed: bool,
    /// Fill opacity for a face; zero for a stroke.
    pub fill: f64,
    /// Opacity of the whole: dimmed when it belongs to a step not being
    /// read.
    pub opacity: f64,
    depth: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Svg {
    /// A path's `d`.
    Path(String),
    /// A polygon's `points`.
    Polygon(String),
    Text {
        x: f64,
        y: f64,
        text: String,
    },
}

/// Every item on the page, furthest first so the nearest paints last.
/// Each comes with an opacity; an item any of whose points are behind the
/// lens is left out whole.
pub fn render(items: &[(Item, f64)], p: &Projector) -> Vec<Drawn> {
    let mut out: Vec<Drawn> = items
        .iter()
        .flat_map(|(item, opacity)| draw(item, *opacity, p))
        .collect();
    out.sort_by(|a, b| b.depth.total_cmp(&a.depth));
    out
}

fn draw(item: &Item, opacity: f64, p: &Projector) -> Vec<Drawn> {
    let base = |svg: Svg, ink: Ink, width: f64, dashed: bool, fill: f64, depth: f64| Drawn {
        svg,
        ink,
        width,
        dashed,
        fill,
        opacity,
        depth,
    };
    match item {
        Item::Line {
            a,
            b,
            ink,
            width,
            dashed,
        } => {
            let (Some(a), Some(b)) = (p.project(*a), p.project(*b)) else {
                return Vec::new();
            };
            vec![base(
                Svg::Path(format!("M{} {}L{} {}", f(a.x), f(a.y), f(b.x), f(b.y))),
                *ink,
                *width,
                *dashed,
                0.0,
                (a.depth + b.depth) / 2.0,
            )]
        }
        Item::Polyline {
            points,
            ink,
            width,
            dashed,
        } => {
            let Some(projected) = points
                .iter()
                .map(|q| p.project(*q))
                .collect::<Option<Vec<_>>>()
            else {
                return Vec::new();
            };
            if projected.len() < 2 {
                return Vec::new();
            }
            vec![base(
                Svg::Path(path(&projected)),
                *ink,
                *width,
                *dashed,
                0.0,
                mean_depth(&projected),
            )]
        }
        Item::Polygon { points, ink, fill } => {
            let Some(projected) = points
                .iter()
                .map(|q| p.project(*q))
                .collect::<Option<Vec<_>>>()
            else {
                return Vec::new();
            };
            let text = projected
                .iter()
                .map(|q| format!("{},{}", f(q.x), f(q.y)))
                .collect::<Vec<_>>()
                .join(" ");
            vec![base(
                Svg::Polygon(text),
                *ink,
                1.0,
                false,
                *fill,
                mean_depth(&projected),
            )]
        }
        Item::Arrow {
            from,
            to,
            ink,
            width,
        } => arrow(*from, *to, *width, p)
            .into_iter()
            .map(|(svg, fill, depth)| base(svg, *ink, *width, false, fill, depth))
            .collect(),
        Item::Label { at, text, ink } => {
            let Some(q) = p.project(*at) else {
                return Vec::new();
            };
            vec![base(
                Svg::Text {
                    x: q.x,
                    y: q.y,
                    text: text.clone(),
                },
                *ink,
                0.0,
                false,
                0.0,
                // Labels last among their neighbours, so a line through one
                // does not cross its writing.
                q.depth - 0.01,
            )]
        }
    }
}

/// A shaft and a head: the head a triangle on the page, as long as a sixth
/// of the arrow but no longer than a few of its widths, so a short arrow is
/// still an arrow and a long one does not end in a tent.
fn arrow(from: Vec3, to: Vec3, width: f64, p: &Projector) -> Vec<(Svg, f64, f64)> {
    let (Some(a), Some(b)) = (p.project(from), p.project(to)) else {
        return Vec::new();
    };
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len = dx.hypot(dy);
    let depth = (a.depth + b.depth) / 2.0;
    if len < 0.5 {
        // Seen end on: a dot where it points at the viewer.
        let r = (width * 1.6).max(2.0);
        return vec![(
            Svg::Path(format!(
                "M{} {}m-{r} 0a{r} {r} 0 1 0 {d} 0a{r} {r} 0 1 0 -{d} 0",
                f(b.x),
                f(b.y),
                d = 2.0 * r
            )),
            1.0,
            depth,
        )];
    }
    let (ux, uy) = (dx / len, dy / len);
    let head = (len / 6.0).clamp(4.0, 8.0 + width * 3.0);
    let half = head * 0.42;
    let (bx, by) = (b.x - ux * head, b.y - uy * head);
    let shaft = format!("M{} {}L{} {}", f(a.x), f(a.y), f(bx), f(by));
    let tip = format!(
        "{},{} {},{} {},{}",
        f(b.x),
        f(b.y),
        f(bx - uy * half),
        f(by + ux * half),
        f(bx + uy * half),
        f(by - ux * half)
    );
    vec![
        (Svg::Path(shaft), 0.0, depth),
        (Svg::Polygon(tip), 1.0, depth - 1e-6),
    ]
}

fn path(points: &[Point]) -> String {
    let mut d = String::with_capacity(points.len() * 12);
    for (i, q) in points.iter().enumerate() {
        d.push(if i == 0 { 'M' } else { 'L' });
        d.push_str(&f(q.x));
        d.push(' ');
        d.push_str(&f(q.y));
    }
    d
}

fn mean_depth(points: &[Point]) -> f64 {
    points.iter().map(|q| q.depth).sum::<f64>() / points.len().max(1) as f64
}

/// A page coordinate to a tenth of a pixel: enough for any screen, and a
/// path a third the length of one written to the last digit.
fn f(v: f64) -> String {
    let rounded = (v * 10.0).round() / 10.0;
    if rounded == rounded.trunc() {
        format!("{}", rounded as i64)
    } else {
        format!("{rounded:.1}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn projector(frame: Frame, preset: Preset) -> Projector {
        Projector::new(&Camera::preset(preset, frame), frame, 800.0, 600.0)
    }

    #[test]
    fn the_target_is_the_middle_of_the_page() {
        for frame in [Frame::ZUp, Frame::ZDown] {
            let q = projector(frame, Preset::Chase).project(Vec3::ZERO).unwrap();
            assert!((q.x - 400.0).abs() < 1e-9 && (q.y - 300.0).abs() < 1e-9);
        }
    }

    /// Up is up the page in both frames: with Z up, +Z is above the
    /// middle; with Z down, +Z is below it.
    #[test]
    fn the_worlds_up_is_up_the_page_in_either_frame() {
        let up = projector(Frame::ZUp, Preset::Chase)
            .project(Vec3::Z)
            .unwrap();
        assert!(up.y < 300.0, "{up:?}");
        let down = projector(Frame::ZDown, Preset::Chase)
            .project(Vec3::Z)
            .unwrap();
        assert!(down.y > 300.0, "{down:?}");
    }

    /// The chase view sits behind and to the right: the nose points into
    /// the page, and the right wing towards the right of it.
    #[test]
    fn the_chase_view_is_behind_the_right_wing() {
        for (frame, right_wing) in [(Frame::ZUp, -Vec3::Y), (Frame::ZDown, Vec3::Y)] {
            let p = projector(frame, Preset::Chase);
            let origin = p.project(Vec3::ZERO).unwrap();
            let nose = p.project(Vec3::X).unwrap();
            let wing = p.project(right_wing).unwrap();
            assert!(nose.depth > origin.depth, "{frame:?}: nose into the page");
            assert!(wing.x > origin.x, "{frame:?}: the right wing on the right");
        }
    }

    /// From above, the nose points up the page and the right wing to the
    /// right, whichever way the frame's Z points.
    #[test]
    fn the_top_view_has_the_nose_up_the_page() {
        for (frame, right_wing) in [(Frame::ZUp, -Vec3::Y), (Frame::ZDown, Vec3::Y)] {
            let p = projector(frame, Preset::Top);
            let origin = p.project(Vec3::ZERO).unwrap();
            let nose = p.project(Vec3::X).unwrap();
            let wing = p.project(right_wing).unwrap();
            assert!(nose.y < origin.y, "{frame:?}: {nose:?}");
            assert!(wing.x > origin.x, "{frame:?}: {wing:?}");
        }
    }

    /// Dragging right brings the target's left side round: the camera moves
    /// towards its own left, in both frames.
    #[test]
    fn a_drag_moves_the_scene_with_the_pointer_in_either_frame() {
        for frame in [Frame::ZUp, Frame::ZDown] {
            let camera = Camera::preset(Preset::Chase, frame);
            let before = Projector::new(&camera, frame, 800.0, 600.0);
            let moved = camera.orbit(40.0, 0.0, frame);
            let step = moved.eye(frame) - camera.eye(frame);
            assert!(step.dot(before.right) < 0.0, "{frame:?}");
            assert!((moved.eye(frame).norm() - camera.eye(frame).norm()).abs() < 1e-9);
            let tilted = camera.orbit(0.0, 1e6, frame);
            assert!(tilted.elevation <= MAX_ELEVATION);
        }
    }

    #[test]
    fn nearer_things_are_bigger_and_drawn_after() {
        let p = projector(Frame::ZUp, Preset::Front);
        // The front view looks back along −X, so +X is nearer.
        let near = (p.project(Vec3::new(1.0, 0.5, 0.0)).unwrap().x - 400.0).abs();
        let far = (p.project(Vec3::new(-1.0, 0.5, 0.0)).unwrap().x - 400.0).abs();
        assert!(near > far);
        let drawn = render(
            &[
                (
                    Item::Line {
                        a: Vec3::new(1.0, -1.0, 0.0),
                        b: Vec3::new(1.0, 1.0, 0.0),
                        ink: Ink::First,
                        width: 1.0,
                        dashed: false,
                    },
                    1.0,
                ),
                (
                    Item::Line {
                        a: Vec3::new(-1.0, -1.0, 0.0),
                        b: Vec3::new(-1.0, 1.0, 0.0),
                        ink: Ink::Second,
                        width: 1.0,
                        dashed: false,
                    },
                    1.0,
                ),
            ],
            &p,
        );
        assert_eq!(drawn[0].ink, Ink::Second, "the far one first");
        assert_eq!(drawn[1].ink, Ink::First);
    }

    #[test]
    fn something_behind_the_lens_is_not_drawn() {
        let camera = Camera::preset(Preset::Front, Frame::ZUp);
        let p = Projector::new(&camera, Frame::ZUp, 800.0, 600.0);
        let behind = camera.eye(Frame::ZUp) + Vec3::X * 2.0;
        assert_eq!(p.project(behind), None);
        let drawn = render(
            &[(
                Item::Line {
                    a: Vec3::ZERO,
                    b: behind,
                    ink: Ink::First,
                    width: 1.0,
                    dashed: false,
                },
                1.0,
            )],
            &p,
        );
        assert!(drawn.is_empty());
    }

    /// An arrow's head is at its tip and points the way it does.
    #[test]
    fn an_arrowhead_sits_at_the_tip() {
        let p = Projector::plane(100.0, (0.0, 0.0), 400.0, 400.0);
        let drawn = render(
            &[(
                Item::Arrow {
                    from: Vec3::ZERO,
                    to: Vec3::new(1.0, 0.0, 0.0),
                    ink: Ink::Result,
                    width: 1.5,
                },
                1.0,
            )],
            &p,
        );
        let Svg::Polygon(points) = &drawn.iter().find(|d| d.fill > 0.0).unwrap().svg else {
            panic!("a head");
        };
        assert!(points.starts_with("300,200 "), "{points}");
        let Svg::Path(shaft) = &drawn.iter().find(|d| d.fill == 0.0).unwrap().svg else {
            panic!("a shaft");
        };
        assert!(shaft.starts_with("M200 200L"), "{shaft}");
    }

    #[test]
    fn the_plane_is_seen_square_on() {
        let p = Projector::plane(50.0, (1.0, 2.0), 400.0, 300.0);
        let q = p.project(Vec3::new(2.0, 3.0, 0.0)).unwrap();
        assert_eq!((q.x, q.y), (250.0, 100.0));
        // No perspective: far and near are the same size.
        let far = p.project(Vec3::new(2.0, 3.0, -50.0)).unwrap();
        assert_eq!((far.x, far.y), (q.x, q.y));
    }

    #[test]
    fn fitting_puts_a_sphere_inside_the_view() {
        let camera = Camera::preset(Preset::Chase, Frame::ZUp).fitting(3.0);
        let p = Projector::new(&camera, Frame::ZUp, 600.0, 600.0);
        for dir in [Vec3::X, Vec3::Y, Vec3::Z, -Vec3::X, -Vec3::Y, -Vec3::Z] {
            let q = p.project(dir * 3.0).unwrap();
            assert!(
                (0.0..600.0).contains(&q.x) && (0.0..600.0).contains(&q.y),
                "{dir:?} → {q:?}"
            );
        }
    }
}
