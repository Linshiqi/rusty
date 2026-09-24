//! What the math toolbox draws, in world axes: the axes, a grid, a
//! quadcopter at an attitude, a body's three axes, and a step's working.

use rusty_embed::spatial::sheet::{Role, Shape};
use rusty_embed::spatial::{Frame, Quat, Vec3};

use super::{Ink, Item};

/// The world's axes from the origin, each named at its end.
pub fn axes(length: f64) -> Vec<Item> {
    [
        (Vec3::X, Ink::AxisX, "X"),
        (Vec3::Y, Ink::AxisY, "Y"),
        (Vec3::Z, Ink::AxisZ, "Z"),
    ]
    .into_iter()
    .flat_map(|(dir, ink, name)| {
        [
            Item::Line {
                a: Vec3::ZERO,
                b: dir * length,
                ink,
                width: 1.2,
                dashed: false,
            },
            Item::Label {
                at: dir * (length * 1.08),
                text: name.to_string(),
                ink,
            },
        ]
    })
    .collect()
}

/// Lines across the world's horizontal plane every `step`, out to `half`.
pub fn grid(half: f64, step: f64) -> Vec<Item> {
    let lines = (half / step).round() as i64;
    (-lines..=lines)
        .flat_map(|i| {
            let at = i as f64 * step;
            [
                Item::Line {
                    a: Vec3::new(at, -half, 0.0),
                    b: Vec3::new(at, half, 0.0),
                    ink: Ink::Grid,
                    width: 0.6,
                    dashed: false,
                },
                Item::Line {
                    a: Vec3::new(-half, at, 0.0),
                    b: Vec3::new(half, at, 0.0),
                    ink: Ink::Grid,
                    width: 0.6,
                    dashed: false,
                },
            ]
        })
        .collect()
}

/// A body's three axes at an attitude: solid for where it is, a dashed
/// ghost for where it was or is passing through.
pub fn triad(attitude: Quat, length: f64, ghost: bool) -> Vec<Item> {
    [
        (Vec3::X, Ink::AxisX),
        (Vec3::Y, Ink::AxisY),
        (Vec3::Z, Ink::AxisZ),
    ]
    .into_iter()
    .map(|(axis, ink)| {
        let tip = attitude.rotate(axis) * length;
        if ghost {
            Item::Line {
                a: Vec3::ZERO,
                b: tip,
                ink,
                width: 1.0,
                dashed: true,
            }
        } else {
            Item::Arrow {
                from: Vec3::ZERO,
                to: tip,
                ink,
                width: 2.0,
            }
        }
    })
    .collect()
}

/// How far each motor sits from the middle, along each body axis: an
/// aircraft most of the length of the world's axes, so its attitude reads
/// at a glance rather than as a knot of lines at the origin.
const ARM: f64 = 0.62;
const ROTOR: f64 = 0.3;

/// A quadcopter in the X configuration, the front arms in the front colour
/// and a nose pointing along the body's X — so which way it faces and
/// which way is up reads at a glance from any side. The rotors sit on the
/// body's up, which is +Z with Z up and −Z with Z down. A ghost is its
/// outline, for an attitude before a turn.
pub fn quad(attitude: Quat, frame: Frame, ghost: bool) -> Vec<Item> {
    let up = frame.up();
    let world = |x: f64, y: f64, z: f64| attitude.rotate(Vec3::new(x, y, 0.0) + up * z);
    let mut out = Vec::new();
    for (x, y) in [(ARM, ARM), (ARM, -ARM), (-ARM, -ARM), (-ARM, ARM)] {
        let front = x > 0.0;
        out.push(Item::Line {
            a: world(0.0, 0.0, 0.0),
            b: world(x, y, 0.0),
            ink: if ghost {
                Ink::Ghost
            } else if front {
                Ink::Front
            } else {
                Ink::Body
            },
            width: if ghost { 1.0 } else { 3.0 },
            dashed: ghost,
        });
        let rim: Vec<Vec3> = (0..20)
            .map(|k| {
                let a = std::f64::consts::TAU * k as f64 / 20.0;
                world(x + ROTOR * a.cos(), y + ROTOR * a.sin(), 0.06)
            })
            .collect();
        out.push(Item::Polygon {
            points: rim,
            ink: if ghost {
                Ink::Ghost
            } else if front {
                Ink::Front
            } else {
                Ink::Rotor
            },
            fill: if ghost { 0.04 } else { 0.16 },
        });
    }
    if !ghost {
        out.push(Item::Polygon {
            points: vec![
                world(0.22, 0.13, 0.0),
                world(0.22, -0.13, 0.0),
                world(-0.22, -0.13, 0.0),
                world(-0.22, 0.13, 0.0),
            ],
            ink: Ink::Body,
            fill: 0.45,
        });
    }
    out.push(Item::Polygon {
        points: vec![
            world(0.52, 0.0, 0.0),
            world(0.25, 0.11, 0.0),
            world(0.25, -0.11, 0.0),
        ],
        ink: if ghost { Ink::Ghost } else { Ink::Front },
        fill: if ghost { 0.1 } else { 0.9 },
    });
    out
}

fn ink_of(role: Role) -> Ink {
    match role {
        Role::First => Ink::First,
        Role::Second => Ink::Second,
        Role::Term => Ink::Term,
        Role::Result => Ink::Result,
        Role::Before | Role::Between => Ink::Ghost,
    }
}

/// How far out an axis of rotation is drawn, both ways.
const AXIS_REACH: f64 = 1.6;

/// One shape of a step's working.
pub fn working(shape: &Shape) -> Vec<Item> {
    match *shape {
        Shape::Arrow { from, to, role } => vec![Item::Arrow {
            from,
            to,
            ink: ink_of(role),
            width: if role == Role::Result { 2.4 } else { 1.8 },
        }],
        Shape::Guide { from, to } => vec![Item::Line {
            a: from,
            b: to,
            ink: Ink::Guide,
            width: 1.0,
            dashed: true,
        }],
        Shape::Axis { dir } => {
            let dir = dir.normalized().unwrap_or(Vec3::Z) * AXIS_REACH;
            vec![Item::Line {
                a: -dir,
                b: dir,
                ink: Ink::Guide,
                width: 1.2,
                dashed: true,
            }]
        }
        Shape::Arc {
            axis,
            from,
            angle,
            role,
        } => arc(axis, from, angle, ink_of(role)),
        Shape::Frame { attitude, role } => {
            triad(attitude, 1.0, matches!(role, Role::Before | Role::Between))
        }
        Shape::Span { a, b } => vec![Item::Polygon {
            points: vec![Vec3::ZERO, a, a + b, b],
            ink: Ink::Term,
            fill: 0.14,
        }],
    }
}

/// The tip of `from` turning `angle` about `axis`, with a head where it
/// ends.
fn arc(axis: Vec3, from: Vec3, angle: f64, ink: Ink) -> Vec<Item> {
    let Some(axis) = axis.normalized() else {
        return Vec::new();
    };
    if angle.abs() < 1e-9 || from.norm() < 1e-9 {
        return Vec::new();
    }
    let steps = ((angle.abs() * 24.0).ceil() as usize).clamp(6, 96);
    let points: Vec<Vec3> = (0..=steps)
        .map(|k| {
            let turn = Quat::from_axis_angle(axis, angle * k as f64 / steps as f64)
                .unwrap_or(Quat::IDENTITY);
            turn.rotate(from)
        })
        .collect();
    let last = points[points.len() - 1];
    let before = points[points.len() - 2];
    vec![
        Item::Polyline {
            points,
            ink,
            width: 1.4,
            dashed: false,
        },
        Item::Arrow {
            from: before,
            to: last,
            ink,
            width: 1.4,
        },
    ]
}

/// The furthest any item reaches from the origin, for fitting the camera.
pub fn extent(items: &[Item]) -> f64 {
    items
        .iter()
        .flat_map(|item| match item {
            Item::Line { a, b, .. } => vec![*a, *b],
            Item::Arrow { from, to, .. } => vec![*from, *to],
            Item::Polyline { points, .. } | Item::Polygon { points, .. } => points.clone(),
            Item::Label { at, .. } => vec![*at],
        })
        .map(Vec3::norm)
        .fold(0.0, f64::max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_embed::spatial::Euler;

    fn points(items: &[Item]) -> Vec<Vec3> {
        items
            .iter()
            .flat_map(|item| match item {
                Item::Line { a, b, .. } => vec![*a, *b],
                Item::Arrow { from, to, .. } => vec![*from, *to],
                Item::Polyline { points, .. } | Item::Polygon { points, .. } => points.clone(),
                Item::Label { at, .. } => vec![*at],
            })
            .collect()
    }

    /// The nose is where the body's X points, whatever the attitude: the
    /// one thing about the drawing that has to be right.
    #[test]
    fn the_nose_points_along_the_bodys_x() {
        let q = Quat::from_euler(Euler::new(0.3, -0.6, 2.0));
        let items = quad(q, Frame::ZUp, false);
        let Item::Polygon { points: nose, .. } = items.last().unwrap() else {
            panic!("the nose last");
        };
        let tip = nose[0];
        let forward = q.rotate(Vec3::X);
        assert!(
            (tip.normalized().unwrap() - forward).norm() < 1e-9,
            "{tip:?}"
        );
    }

    /// The rotors sit on the body's up: above the arms with Z up, below
    /// them in Z-down numbers — the same place on the desk.
    #[test]
    fn the_rotors_sit_on_the_bodys_up_in_either_frame() {
        for frame in [Frame::ZUp, Frame::ZDown] {
            let items = quad(Quat::IDENTITY, frame, false);
            let Item::Polygon { points: rim, .. } = &items[1] else {
                panic!("a rotor second");
            };
            assert!(
                rim.iter().all(|p| (p.dot(frame.up()) - 0.06).abs() < 1e-12),
                "{frame:?}"
            );
        }
    }

    #[test]
    fn an_arc_ends_where_the_turn_takes_it() {
        let items = arc(Vec3::Z, Vec3::X, std::f64::consts::FRAC_PI_2, Ink::Term);
        let Item::Polyline { points, .. } = &items[0] else {
            panic!("a polyline");
        };
        assert!((points[0] - Vec3::X).norm() < 1e-12);
        assert!((points[points.len() - 1] - Vec3::Y).norm() < 1e-12);
        // Every point stays on the circle.
        assert!(points.iter().all(|p| (p.norm() - 1.0).abs() < 1e-12));
        assert!(arc(Vec3::ZERO, Vec3::X, 1.0, Ink::Term).is_empty());
        assert!(arc(Vec3::Z, Vec3::X, 0.0, Ink::Term).is_empty());
    }

    #[test]
    fn a_triad_is_the_attitudes_columns() {
        let q = Quat::about_z(std::f64::consts::FRAC_PI_2);
        let items = triad(q, 1.0, false);
        let Item::Arrow { to, .. } = items[0] else {
            panic!("an arrow");
        };
        assert!((to - Vec3::Y).norm() < 1e-12, "X turned to Y");
        assert!(matches!(
            triad(q, 1.0, true)[0],
            Item::Line { dashed: true, .. }
        ));
    }

    #[test]
    fn the_extent_is_the_furthest_point() {
        let items = working(&Shape::Arrow {
            from: Vec3::ZERO,
            to: Vec3::new(3.0, 4.0, 0.0),
            role: Role::Result,
        });
        assert!((extent(&items) - 5.0).abs() < 1e-12);
        assert_eq!(points(&grid(1.0, 0.5)).len(), 20);
    }
}
