//! What a row did, one operation at a time: the working with the numbers
//! in, and what to draw for it — the "how" beside every "what". The panel
//! lists the working, draws the shapes of the step being read, and plays
//! the turns in order.

use crate::spatial::{Euler, Mat3, Quat, Vec2, Vec3};

/// One operation, shown.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub what: What,
    /// The working, a line at a time, numbers in. Formulae, not prose, so
    /// they read the same in every language and the panel prints them as
    /// they are.
    pub lines: Vec<String>,
    /// What is worth knowing about the working, which the panel says in
    /// the reader's language.
    pub remarks: Vec<Remark>,
    pub shapes: Vec<Shape>,
    /// The turn this step makes, from one attitude to another, for playing
    /// it on the aircraft.
    pub turn: Option<(Quat, Quat)>,
}

/// A sentence about a step, named rather than written, so it can be said in
/// any language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Remark {
    /// `|a × b|` is the area of the parallelogram the two span.
    Area,
    /// The angle is the shorter way round, and `−q` names the same turn.
    ShortWay,
    /// No turn at all, so no axis.
    NoTurn,
    /// The dot product was negative, so the other end was negated: the same
    /// attitude, reached the short way.
    Flipped,
    /// A rotation matrix's columns are the body's axes seen from the world.
    Columns,
    /// The rate is in the body's axes, so the step multiplies on the right.
    BodyRate,
    /// To first order the quaternion leaves the unit sphere.
    OffSphere,
    /// `a* ⊗ b` is the turn from `a` to `b` in `a`'s own axes.
    OwnAxes,
    /// Gravity says nothing about heading.
    NoHeading,
    /// A rotation's transpose is its inverse.
    TransposeInverse,
    /// The determinant is 1 for a rotation and −1 for a mirror.
    DetSign,
    /// At a pole roll and yaw turn about one axis; roll is set to zero and
    /// the rest given to yaw.
    Pole,
    /// Opposite directions: any perpendicular axis turns one onto the other.
    Antiparallel,
    /// The reading is in the body's axes, in g.
    BodyAxes,
    /// The two share an attitude and differ only in sign.
    SameAttitude,
}

/// Which operation a step is: the panel's caption for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum What {
    Add,
    Sub,
    Scale,
    Negate,
    Dot,
    Cross,
    Norm,
    Normalize,
    Project,
    AngleBetween,
    /// A vector in the plane turned by an angle.
    Turn2,
    /// A quaternion written out: its axis, its angle, its length.
    Quaternion,
    AxisAngle,
    EulerYaw,
    EulerPitch,
    EulerRoll,
    ToEuler,
    /// Reading Euler angles at a pole, where roll is given to yaw.
    ToEulerPole,
    Matrix,
    FromMatrix,
    Compose,
    Conjugate,
    Inverse,
    /// `q ⊗ (0, v) ⊗ q*`, body to world.
    ToWorld,
    /// `q* ⊗ (0, v) ⊗ q`, world to body.
    ToBody,
    /// The same turn as three vectors: `v cos θ`, `(n × v) sin θ` and
    /// `n (n·v)(1 − cos θ)`, head to tail.
    Rodrigues,
    Slerp,
    FromTo,
    Integrate,
    IntegrateLinear,
    Delta,
    AttitudeError,
    ToRotvec,
    FromRotvec,
    AtRest,
    Gravity,
    Tilt,
    MatVec,
    MatMat,
    Transpose,
    Det,
    Check,
}

/// A thing to draw, in world coordinates. The plane's are its `z = 0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    Arrow {
        from: Vec3,
        to: Vec3,
        role: Role,
    },
    /// A dashed construction line.
    Guide {
        from: Vec3,
        to: Vec3,
    },
    /// A line through the origin, both ways: an axis something turns about.
    Axis {
        dir: Vec3,
    },
    /// The tip of `from` turning `angle` about `axis`, through the origin.
    Arc {
        axis: Vec3,
        from: Vec3,
        angle: f64,
        role: Role,
    },
    /// A body's three axes, at an attitude.
    Frame {
        attitude: Quat,
        role: Role,
    },
    /// The parallelogram two vectors span from the origin.
    Span {
        a: Vec3,
        b: Vec3,
    },
}

/// What a shape stands for, which decides its colour and weight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The first thing an operation was handed.
    First,
    /// The second.
    Second,
    /// A part of the working: a term, a component, a projection.
    Term,
    /// What the operation made.
    Result,
    /// An attitude before the step turned it.
    Before,
    /// An attitude partway through.
    Between,
}

pub fn lift(v: Vec2) -> Vec3 {
    Vec3::new(v.x, v.y, 0.0)
}

// ---- Numbers as the working prints them -------------------------------------

/// Four decimals with the trailing zeros taken off, and scientific notation
/// where four decimals would say nothing: a `dt` of `1e-5` is not zero.
pub fn num(v: f64) -> String {
    if !v.is_finite() {
        return if v.is_nan() {
            "NaN".into()
        } else if v > 0.0 {
            "inf".into()
        } else {
            "-inf".into()
        };
    }
    let a = v.abs();
    if a != 0.0 && !(1e-4..1e7).contains(&a) {
        let text = format!("{v:.3e}");
        let (mantissa, exponent) = text.split_once('e').unwrap_or((&text, "0"));
        return format!("{}e{exponent}", trim(mantissa));
    }
    let text = trim(&format!("{v:.4}"));
    if text == "-0" { "0".into() } else { text }
}

/// Nine significant figures: for a length that should be one and is not
/// quite, where four decimals would say `1`.
pub fn precise(v: f64) -> String {
    let text = format!("{v:.9}");
    trim(&text)
}

fn trim(text: &str) -> String {
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text.to_string()
    }
}

/// Degrees, two decimals at most: `30°`, `12.5°`.
pub fn deg(radians: f64) -> String {
    let d = radians.to_degrees();
    let text = trim(&format!("{d:.2}"));
    let text = if text == "-0" { "0".to_string() } else { text };
    format!("{text}°")
}

pub fn vec2(v: Vec2) -> String {
    format!("({}, {})", num(v.x), num(v.y))
}

pub fn vec3(v: Vec3) -> String {
    format!("({}, {}, {})", num(v.x), num(v.y), num(v.z))
}

pub fn quat(q: Quat) -> String {
    format!("({}, {}, {}, {})", num(q.w), num(q.x), num(q.y), num(q.z))
}

pub fn euler(e: Euler) -> String {
    format!(
        "roll {}, pitch {}, yaw {}",
        deg(e.roll),
        deg(e.pitch),
        deg(e.yaw)
    )
}

pub fn mat3(m: Mat3) -> String {
    let row = |r: [f64; 3]| format!("[{}, {}, {}]", num(r[0]), num(r[1]), num(r[2]));
    format!("[{}, {}, {}]", row(m.m[0]), row(m.m[1]), row(m.m[2]))
}

// ---- The steps each operation shows -----------------------------------------

fn step(what: What, lines: Vec<String>, shapes: Vec<Shape>) -> Step {
    Step {
        what,
        lines,
        remarks: Vec::new(),
        shapes,
        turn: None,
    }
}

impl Step {
    fn noting(mut self, remark: Remark) -> Step {
        self.remarks.push(remark);
        self
    }
}

pub fn add(a: Vec3, b: Vec3, flat: bool) -> Step {
    let show = |v: Vec3| show(v, flat);
    step(
        What::Add,
        vec![format!("{} + {} = {}", show(a), show(b), show(a + b))],
        vec![
            Shape::Arrow {
                from: Vec3::ZERO,
                to: a,
                role: Role::First,
            },
            Shape::Arrow {
                from: a,
                to: a + b,
                role: Role::Second,
            },
            Shape::Guide {
                from: Vec3::ZERO,
                to: b,
            },
            Shape::Guide { from: b, to: a + b },
            Shape::Arrow {
                from: Vec3::ZERO,
                to: a + b,
                role: Role::Result,
            },
        ],
    )
}

pub fn sub(a: Vec3, b: Vec3, flat: bool) -> Step {
    let show = |v: Vec3| show(v, flat);
    step(
        What::Sub,
        vec![format!("{} - {} = {}", show(a), show(b), show(a - b))],
        vec![
            Shape::Arrow {
                from: Vec3::ZERO,
                to: a,
                role: Role::First,
            },
            Shape::Arrow {
                from: Vec3::ZERO,
                to: b,
                role: Role::Second,
            },
            // The difference runs from b's tip to a's, and is drawn from the
            // origin as well, where it is the vector it is.
            Shape::Guide { from: b, to: a },
            Shape::Arrow {
                from: Vec3::ZERO,
                to: a - b,
                role: Role::Result,
            },
        ],
    )
}

pub fn scale(k: f64, v: Vec3, flat: bool) -> Step {
    step(
        What::Scale,
        vec![format!(
            "{} · {} = {}",
            num(k),
            show(v, flat),
            show(v * k, flat)
        )],
        vec![
            Shape::Arrow {
                from: Vec3::ZERO,
                to: v,
                role: Role::First,
            },
            Shape::Arrow {
                from: Vec3::ZERO,
                to: v * k,
                role: Role::Result,
            },
        ],
    )
}

pub fn dot(a: Vec3, b: Vec3, flat: bool) -> Step {
    let d = a.dot(b);
    let mut lines = vec![if flat {
        format!(
            "a·b = {}·{} + {}·{} = {}",
            num(a.x),
            num(b.x),
            num(a.y),
            num(b.y),
            num(d)
        )
    } else {
        format!(
            "a·b = {}·{} + {}·{} + {}·{} = {}",
            num(a.x),
            num(b.x),
            num(a.y),
            num(b.y),
            num(a.z),
            num(b.z),
            num(d)
        )
    }];
    let mut shapes = vec![
        Shape::Arrow {
            from: Vec3::ZERO,
            to: a,
            role: Role::First,
        },
        Shape::Arrow {
            from: Vec3::ZERO,
            to: b,
            role: Role::Second,
        },
    ];
    if let (Some(theta), Some(along)) = (a.angle_to(b), b.project_onto(a)) {
        lines.push(format!(
            "= |a| |b| cos θ = {} · {} · cos {}",
            num(a.norm()),
            num(b.norm()),
            deg(theta)
        ));
        lines.push(format!(
            "b along a = (a·b / |a|²) a = {}",
            show(along, flat)
        ));
        shapes.push(Shape::Guide { from: b, to: along });
        shapes.push(Shape::Arrow {
            from: Vec3::ZERO,
            to: along,
            role: Role::Term,
        });
        shapes.push(arc_between(a, b));
    }
    step(What::Dot, lines, shapes)
}

pub fn cross(a: Vec3, b: Vec3) -> Step {
    let c = a.cross(b);
    let mut lines = vec![
        format!(
            "a × b = (a_y b_z − a_z b_y, a_z b_x − a_x b_z, a_x b_y − a_y b_x) = {}",
            vec3(c)
        ),
        format!("|a × b| = {}", num(c.norm())),
    ];
    if let Some(theta) = a.angle_to(b) {
        lines.push(format!(
            "= |a| |b| sin θ = {} · {} · sin {}",
            num(a.norm()),
            num(b.norm()),
            deg(theta)
        ));
    }
    step(
        What::Cross,
        lines,
        vec![
            Shape::Span { a, b },
            Shape::Arrow {
                from: Vec3::ZERO,
                to: a,
                role: Role::First,
            },
            Shape::Arrow {
                from: Vec3::ZERO,
                to: b,
                role: Role::Second,
            },
            Shape::Arrow {
                from: Vec3::ZERO,
                to: c,
                role: Role::Result,
            },
        ],
    )
    .noting(Remark::Area)
}

/// The plane's cross product: a number, the signed area.
pub fn cross2(a: Vec2, b: Vec2) -> Step {
    let c = a.cross(b);
    step(
        What::Cross,
        vec![format!(
            "a × b = a_x b_y − a_y b_x = {}·{} − {}·{} = {}",
            num(a.x),
            num(b.y),
            num(a.y),
            num(b.x),
            num(c)
        )],
        vec![
            Shape::Span {
                a: lift(a),
                b: lift(b),
            },
            Shape::Arrow {
                from: Vec3::ZERO,
                to: lift(a),
                role: Role::First,
            },
            Shape::Arrow {
                from: Vec3::ZERO,
                to: lift(b),
                role: Role::Second,
            },
        ],
    )
}

pub fn norm(v: Vec3, flat: bool) -> Step {
    let lines = vec![if flat {
        format!("|v| = √({}² + {}²) = {}", num(v.x), num(v.y), num(v.norm()))
    } else {
        format!(
            "|v| = √({}² + {}² + {}²) = {}",
            num(v.x),
            num(v.y),
            num(v.z),
            num(v.norm())
        )
    }];
    step(
        What::Norm,
        lines,
        vec![Shape::Arrow {
            from: Vec3::ZERO,
            to: v,
            role: Role::First,
        }],
    )
}

pub fn normalize(v: Vec3, unit: Vec3, flat: bool) -> Step {
    step(
        What::Normalize,
        vec![
            format!("|v| = {}", num(v.norm())),
            format!("v / |v| = {}", show(unit, flat)),
        ],
        vec![
            Shape::Arrow {
                from: Vec3::ZERO,
                to: v,
                role: Role::First,
            },
            Shape::Arrow {
                from: Vec3::ZERO,
                to: unit,
                role: Role::Result,
            },
        ],
    )
}

pub fn normalize_quat(q: Quat, unit: Quat) -> Step {
    step(
        What::Normalize,
        vec![
            format!("|q| = {}", precise(q.norm())),
            format!("q / |q| = {}", quat(unit)),
        ],
        Vec::new(),
    )
}

pub fn project(a: Vec3, onto: Vec3, along: Vec3, flat: bool) -> Step {
    step(
        What::Project,
        vec![
            format!(
                "(a·b / |b|²) b = ({} / {}) {}",
                num(a.dot(onto)),
                num(onto.dot(onto)),
                show(onto, flat)
            ),
            format!("= {}", show(along, flat)),
        ],
        vec![
            Shape::Arrow {
                from: Vec3::ZERO,
                to: a,
                role: Role::First,
            },
            Shape::Arrow {
                from: Vec3::ZERO,
                to: onto,
                role: Role::Second,
            },
            Shape::Guide { from: a, to: along },
            Shape::Arrow {
                from: Vec3::ZERO,
                to: along,
                role: Role::Result,
            },
        ],
    )
}

pub fn angle_between(a: Vec3, b: Vec3, theta: f64, flat: bool) -> Step {
    let line = if flat {
        format!(
            "θ = atan2(a × b, a·b) = atan2({}, {}) = {}",
            num(a.x * b.y - a.y * b.x),
            num(a.dot(b)),
            deg(theta)
        )
    } else {
        format!(
            "θ = atan2(|a × b|, a·b) = atan2({}, {}) = {}",
            num(a.cross(b).norm()),
            num(a.dot(b)),
            deg(theta)
        )
    };
    step(
        What::AngleBetween,
        vec![line],
        vec![
            Shape::Arrow {
                from: Vec3::ZERO,
                to: a,
                role: Role::First,
            },
            Shape::Arrow {
                from: Vec3::ZERO,
                to: b,
                role: Role::Second,
            },
            arc_between(a, b),
        ],
    )
}

/// The arc from `a` round to `b`, drawn at the shorter of their lengths.
fn arc_between(a: Vec3, b: Vec3) -> Shape {
    let axis = a.cross(b).normalized().unwrap_or(Vec3::Z);
    let theta = a.angle_to(b).unwrap_or(0.0);
    let r = a.norm().min(b.norm()) * 0.4;
    let from = a.normalized().unwrap_or(Vec3::X) * r;
    Shape::Arc {
        axis,
        from,
        angle: theta,
        role: Role::Term,
    }
}

pub fn turn2(angle: f64, v: Vec2, turned: Vec2) -> Step {
    let (s, c) = angle.sin_cos();
    step(
        What::Turn2,
        vec![
            format!(
                "R({}) = [[cos, −sin], [sin, cos]] = [[{}, {}], [{}, {}]]",
                deg(angle),
                num(c),
                num(-s),
                num(s),
                num(c)
            ),
            format!("R v = {}", vec2(turned)),
        ],
        vec![
            Shape::Arrow {
                from: Vec3::ZERO,
                to: lift(v),
                role: Role::First,
            },
            Shape::Arc {
                axis: Vec3::Z,
                from: lift(v),
                angle,
                role: Role::Term,
            },
            Shape::Arrow {
                from: Vec3::ZERO,
                to: lift(turned),
                role: Role::Result,
            },
        ],
    )
}

/// A quaternion written out: what it turns about, how far, and whether it
/// is a rotation at all.
pub fn quaternion(q: Quat) -> Step {
    let mut lines = vec![format!("|q| = {}", precise(q.norm()))];
    let mut shapes = Vec::new();
    match q.axis_angle() {
        Some((axis, angle)) => {
            lines.push(format!("θ = 2·atan2(|(x, y, z)|, w) = {}", deg(angle)));
            lines.push(format!("n = (x, y, z) / sin(θ/2) = {}", vec3(axis)));
            shapes.push(Shape::Axis { dir: axis });
            shapes.push(Shape::Frame {
                attitude: Quat::IDENTITY,
                role: Role::Before,
            });
        }
        None => lines.push("θ = 0".into()),
    }
    let remarks = if q.axis_angle().is_some() {
        vec![Remark::ShortWay]
    } else {
        vec![Remark::NoTurn]
    };
    shapes.push(Shape::Frame {
        attitude: q.normalized().unwrap_or(Quat::IDENTITY),
        role: Role::Result,
    });
    Step {
        what: What::Quaternion,
        lines,
        remarks,
        shapes,
        turn: q.normalized().map(|unit| (Quat::IDENTITY, positive(unit))),
    }
}

/// The same attitude with `w ≥ 0`, so a turn played to it goes the short
/// way.
fn positive(q: Quat) -> Quat {
    if q.w < 0.0 { -q } else { q }
}

pub fn axis_angle(axis: Vec3, angle: f64, q: Quat) -> Step {
    let unit = axis.normalized().unwrap_or(Vec3::Z);
    Step {
        what: What::AxisAngle,
        lines: vec![
            format!("n = {} / {} = {}", vec3(axis), num(axis.norm()), vec3(unit)),
            format!(
                "q = (cos(θ/2), n sin(θ/2)) = (cos {}, n sin {})",
                deg(angle / 2.0),
                deg(angle / 2.0)
            ),
            format!("= {}", quat(q)),
        ],
        remarks: Vec::new(),
        shapes: vec![
            Shape::Axis { dir: unit },
            Shape::Frame {
                attitude: Quat::IDENTITY,
                role: Role::Before,
            },
            Shape::Frame {
                attitude: q,
                role: Role::Result,
            },
        ],
        turn: Some((Quat::IDENTITY, q)),
    }
}

/// `euler(roll, pitch, yaw)` as the three turns it is: yaw about Z, pitch
/// about the Y the yaw left, roll about the X the pitch left.
pub fn euler_turns(e: Euler) -> Vec<Step> {
    let q_yaw = Quat::about_z(e.yaw);
    let q_pitch = Quat::about_y(e.pitch);
    let q_roll = Quat::about_x(e.roll);
    let after_yaw = q_yaw;
    let after_pitch = q_yaw * q_pitch;
    let q = after_pitch * q_roll;
    vec![
        Step {
            what: What::EulerYaw,
            lines: vec![
                format!("q_ψ = (cos(ψ/2), 0, 0, sin(ψ/2)), ψ = {}", deg(e.yaw)),
                format!("= {}", quat(q_yaw)),
            ],
            remarks: Vec::new(),
            shapes: vec![
                Shape::Axis { dir: Vec3::Z },
                Shape::Frame {
                    attitude: Quat::IDENTITY,
                    role: Role::Before,
                },
                Shape::Arc {
                    axis: Vec3::Z,
                    from: Vec3::X * 0.8,
                    angle: e.yaw,
                    role: Role::Term,
                },
                Shape::Frame {
                    attitude: after_yaw,
                    role: Role::Result,
                },
            ],
            turn: Some((Quat::IDENTITY, after_yaw)),
        },
        Step {
            what: What::EulerPitch,
            lines: vec![
                format!("q_θ = (cos(θ/2), 0, sin(θ/2), 0), θ = {}", deg(e.pitch)),
                format!("q_ψ ⊗ q_θ = {}", quat(after_pitch)),
            ],
            remarks: Vec::new(),
            shapes: vec![
                Shape::Axis {
                    dir: after_yaw.rotate(Vec3::Y),
                },
                Shape::Frame {
                    attitude: after_yaw,
                    role: Role::Before,
                },
                Shape::Arc {
                    axis: after_yaw.rotate(Vec3::Y),
                    from: after_yaw.rotate(Vec3::X) * 0.8,
                    angle: e.pitch,
                    role: Role::Term,
                },
                Shape::Frame {
                    attitude: after_pitch,
                    role: Role::Result,
                },
            ],
            turn: Some((after_yaw, after_pitch)),
        },
        Step {
            what: What::EulerRoll,
            lines: vec![
                format!("q_φ = (cos(φ/2), sin(φ/2), 0, 0), φ = {}", deg(e.roll)),
                format!("q = q_ψ ⊗ q_θ ⊗ q_φ = {}", quat(q)),
            ],
            remarks: Vec::new(),
            shapes: vec![
                Shape::Axis {
                    dir: after_pitch.rotate(Vec3::X),
                },
                Shape::Frame {
                    attitude: after_pitch,
                    role: Role::Before,
                },
                Shape::Arc {
                    axis: after_pitch.rotate(Vec3::X),
                    from: after_pitch.rotate(Vec3::Y) * 0.8,
                    angle: e.roll,
                    role: Role::Term,
                },
                Shape::Frame {
                    attitude: q,
                    role: Role::Result,
                },
            ],
            turn: Some((after_pitch, q)),
        },
    ]
}

pub fn to_euler(q: Quat, e: Euler) -> Step {
    let Quat { w, x, y, z } = q;
    let n2 = q.norm_squared();
    let sin_pitch = 2.0 * (w * y - x * z) / n2;
    let (what, lines) = if e.at_pole() {
        (
            What::ToEulerPole,
            vec![
                format!("sin θ = 2(wy − xz) / |q|² = {}", precise(sin_pitch)),
                format!("|sin θ| ≥ {} ⇒ φ := 0", crate::spatial::POLE),
                if sin_pitch > 0.0 {
                    format!("ψ = 2·atan2(z, y) = {}", deg(e.yaw))
                } else {
                    format!("ψ = 2·atan2(x, w) = {}", deg(e.yaw))
                },
            ],
        )
    } else {
        (
            What::ToEuler,
            vec![
                format!(
                    "φ = atan2(2(wx + yz), w² − x² − y² + z²) = atan2({}, {}) = {}",
                    num(2.0 * (w * x + y * z)),
                    num(w * w - x * x - y * y + z * z),
                    deg(e.roll)
                ),
                format!(
                    "θ = asin(2(wy − xz) / |q|²) = asin({}) = {}",
                    num(sin_pitch),
                    deg(e.pitch)
                ),
                format!(
                    "ψ = atan2(2(wz + xy), w² + x² − y² − z²) = atan2({}, {}) = {}",
                    num(2.0 * (w * z + x * y)),
                    num(w * w + x * x - y * y - z * z),
                    deg(e.yaw)
                ),
            ],
        )
    };
    let unit = q.normalized().unwrap_or(Quat::IDENTITY);
    let remarks = if what == What::ToEulerPole {
        vec![Remark::Pole]
    } else {
        Vec::new()
    };
    Step {
        what,
        lines,
        remarks,
        shapes: vec![Shape::Frame {
            attitude: unit,
            role: Role::Result,
        }],
        turn: None,
    }
}

pub fn matrix(q: Quat, m: Mat3) -> Step {
    step(
        What::Matrix,
        vec![
            "R = [[w²+x²−y²−z², 2(xy−wz), 2(xz+wy)], [2(xy+wz), w²−x²+y²−z², 2(yz−wx)], \
             [2(xz−wy), 2(yz+wx), w²−x²−y²+z²]]"
                .into(),
            format!("= {}", mat3(m)),
            format!(
                "R x̂ = {}, R ŷ = {}, R ẑ = {}",
                vec3(m.col(0)),
                vec3(m.col(1)),
                vec3(m.col(2))
            ),
        ],
        vec![Shape::Frame {
            attitude: q.normalized().unwrap_or(Quat::IDENTITY),
            role: Role::Result,
        }],
    )
    .noting(Remark::Columns)
}

pub fn from_matrix(m: Mat3, q: Quat) -> Step {
    let trace = m.m[0][0] + m.m[1][1] + m.m[2][2];
    step(
        What::FromMatrix,
        vec![
            format!("trace = {}", num(trace)),
            format!(
                "det = {}, |RᵀR − I| = {}",
                num(m.det()),
                num(m.orthonormal_error())
            ),
            format!("q = {}", quat(q)),
        ],
        vec![Shape::Frame {
            attitude: q,
            role: Role::Result,
        }],
    )
}

pub fn compose(a: Quat, b: Quat, ab: Quat) -> Step {
    let (va, vb) = (a.vector(), b.vector());
    Step {
        what: What::Compose,
        lines: vec![
            "a ⊗ b = (a_w b_w − a_v·b_v, a_w b_v + b_w a_v + a_v × b_v)".into(),
            format!(
                "a_w b_w − a_v·b_v = {}·{} − {} = {}",
                num(a.w),
                num(b.w),
                num(va.dot(vb)),
                num(ab.w)
            ),
            format!(
                "a_w b_v + b_w a_v + a_v × b_v = {} + {} + {} = {}",
                vec3(vb * a.w),
                vec3(va * b.w),
                vec3(va.cross(vb)),
                vec3(ab.vector())
            ),
        ],
        remarks: Vec::new(),
        shapes: vec![
            Shape::Frame {
                attitude: Quat::IDENTITY,
                role: Role::Before,
            },
            Shape::Frame {
                attitude: unit(a),
                role: Role::Between,
            },
            Shape::Frame {
                attitude: unit(ab),
                role: Role::Result,
            },
        ],
        // Read in the body's axes: `a`, then `b` about the axes `a` left.
        turn: Some((Quat::IDENTITY, unit(a))),
    }
}

/// The second half of a composition, played after [`compose`]'s: `b` about
/// the axes `a` left behind.
pub fn compose_second(a: Quat, ab: Quat) -> Step {
    Step {
        what: What::Compose,
        lines: Vec::new(),
        remarks: Vec::new(),
        shapes: Vec::new(),
        turn: Some((unit(a), unit(ab))),
    }
}

fn unit(q: Quat) -> Quat {
    q.normalized().map(positive).unwrap_or(Quat::IDENTITY)
}

pub fn conjugate(q: Quat, c: Quat, inverse: bool) -> Step {
    let mut lines = vec![format!("q* = (w, −x, −y, −z) = {}", quat(q.conj()))];
    if inverse {
        lines.push(format!(
            "q⁻¹ = q* / |q|² = {} / {} = {}",
            quat(q.conj()),
            precise(q.norm_squared()),
            quat(c)
        ));
    }
    Step {
        what: if inverse {
            What::Inverse
        } else {
            What::Conjugate
        },
        lines,
        remarks: Vec::new(),
        shapes: vec![
            Shape::Frame {
                attitude: unit(q),
                role: Role::Before,
            },
            Shape::Frame {
                attitude: unit(c),
                role: Role::Result,
            },
        ],
        turn: Some((Quat::IDENTITY, unit(c))),
    }
}

/// `q ⊗ (0, v) ⊗ q*`, and the same turn drawn as Rodrigues' three vectors.
///
/// `to_body` is the same with `q*` in `q`'s place, which is the whole
/// difference between the two, so one function draws both.
pub fn sandwich(q: Quat, v: Vec3, turned: Vec3, to_body: bool) -> Vec<Step> {
    let p = Quat::pure(v);
    let (left, right) = if to_body {
        (q.conj(), q)
    } else {
        (q, q.conj())
    };
    let half = left * p;
    let sandwich = Step {
        what: if to_body { What::ToBody } else { What::ToWorld },
        lines: vec![
            format!("p = (0, v) = {}", quat(p)),
            if to_body {
                format!("q* ⊗ p = {}", quat(half))
            } else {
                format!("q ⊗ p = {}", quat(half))
            },
            if to_body {
                format!("(q* ⊗ p) ⊗ q = {}", quat(half * right))
            } else {
                format!("(q ⊗ p) ⊗ q* = {}", quat(half * right))
            },
            format!("v' = {}", vec3(turned)),
        ],
        remarks: Vec::new(),
        shapes: vec![
            Shape::Arrow {
                from: Vec3::ZERO,
                to: v,
                role: Role::First,
            },
            Shape::Arrow {
                from: Vec3::ZERO,
                to: turned,
                role: Role::Result,
            },
        ],
        turn: None,
    };
    let mut steps = vec![sandwich];
    let unit = left.normalized().unwrap_or(Quat::IDENTITY);
    if let Some((n, theta)) = unit.axis_angle() {
        let (s, c) = theta.sin_cos();
        let t1 = v * c;
        let t2 = n.cross(v) * s;
        let t3 = n * (n.dot(v) * (1.0 - c));
        let scale = left.norm_squared();
        steps.push(Step {
            what: What::Rodrigues,
            lines: vec![
                format!("n = {}, θ = {}", vec3(n), deg(theta)),
                format!("v cos θ = {}", vec3(t1)),
                format!("(n × v) sin θ = {}", vec3(t2)),
                format!("n (n·v)(1 − cos θ) = {}", vec3(t3)),
                if (scale - 1.0).abs() > 1e-9 {
                    format!(
                        "sum · |q|² = {} · {} = {}",
                        vec3(t1 + t2 + t3),
                        precise(scale),
                        vec3((t1 + t2 + t3) * scale)
                    )
                } else {
                    format!("sum = {}", vec3(t1 + t2 + t3))
                },
            ],
            remarks: Vec::new(),
            shapes: vec![
                Shape::Axis { dir: n },
                Shape::Arrow {
                    from: Vec3::ZERO,
                    to: v,
                    role: Role::First,
                },
                Shape::Arc {
                    axis: n,
                    from: v,
                    angle: theta,
                    role: Role::Term,
                },
                Shape::Arrow {
                    from: Vec3::ZERO,
                    to: t1,
                    role: Role::Term,
                },
                Shape::Arrow {
                    from: t1,
                    to: t1 + t2,
                    role: Role::Term,
                },
                Shape::Arrow {
                    from: t1 + t2,
                    to: t1 + t2 + t3,
                    role: Role::Term,
                },
                Shape::Arrow {
                    from: Vec3::ZERO,
                    to: turned,
                    role: Role::Result,
                },
            ],
            turn: None,
        });
    }
    steps
}

pub fn slerp(a: Quat, b: Quat, t: f64, q: Quat) -> Step {
    let d = a.dot(b);
    let flipped = d < 0.0;
    let d = d.abs();
    let omega = d.clamp(-1.0, 1.0).acos();
    let mut lines = vec![format!("a·b = {}", num(a.dot(b)))];
    if flipped {
        lines.push("a·b < 0 ⇒ b := −b".into());
    }
    lines.push(format!("Ω = acos|a·b| = {}", deg(omega)));
    if omega.sin() > 1e-6 {
        lines.push(format!(
            "q = a sin((1−t)Ω)/sin Ω + b sin(tΩ)/sin Ω, t = {}",
            num(t)
        ));
        lines.push(format!(
            "= a · {} + b · {}",
            num(((1.0 - t) * omega).sin() / omega.sin()),
            num((t * omega).sin() / omega.sin())
        ));
    }
    lines.push(format!("= {}", quat(q)));
    Step {
        what: What::Slerp,
        lines,
        remarks: if flipped {
            vec![Remark::Flipped]
        } else {
            Vec::new()
        },
        shapes: vec![
            Shape::Frame {
                attitude: unit(a),
                role: Role::Before,
            },
            Shape::Frame {
                attitude: unit(b),
                role: Role::Between,
            },
            Shape::Frame {
                attitude: unit(q),
                role: Role::Result,
            },
        ],
        turn: Some((unit(a), unit(q))),
    }
}

pub fn from_to(a: Vec3, b: Vec3, q: Quat) -> Step {
    let (an, bn) = (
        a.normalized().unwrap_or(Vec3::X),
        b.normalized().unwrap_or(Vec3::X),
    );
    let axis = an.cross(bn);
    let theta = an.angle_to(bn).unwrap_or(0.0);
    let mut lines = vec![
        format!("â = {}, b̂ = {}", vec3(an), vec3(bn)),
        format!("â × b̂ = {}, â·b̂ = {}", vec3(axis), num(an.dot(bn))),
        format!("θ = atan2(|â × b̂|, â·b̂) = {}", deg(theta)),
    ];
    let opposite = axis.norm() < 1e-9 && an.dot(bn) < 0.0;
    if opposite {
        lines.push("â = −b̂".into());
    }
    lines.push(format!("q = {}", quat(q)));
    let turn_axis = q.axis_angle().map(|(n, _)| n).unwrap_or(Vec3::Z);
    Step {
        what: What::FromTo,
        lines,
        remarks: if opposite {
            vec![Remark::Antiparallel]
        } else {
            Vec::new()
        },
        shapes: vec![
            Shape::Arrow {
                from: Vec3::ZERO,
                to: a,
                role: Role::First,
            },
            Shape::Arrow {
                from: Vec3::ZERO,
                to: b,
                role: Role::Second,
            },
            Shape::Axis { dir: turn_axis },
            Shape::Arc {
                axis: turn_axis,
                from: an,
                angle: theta,
                role: Role::Term,
            },
        ],
        turn: Some((Quat::IDENTITY, q)),
    }
}

pub fn integrate(q: Quat, rate: Vec3, dt: f64, next: Quat) -> Step {
    let angle = rate.norm() * dt;
    let dq = Quat::from_rotvec(rate * dt);
    Step {
        what: What::Integrate,
        lines: vec![
            format!("|ω| = {} rad/s, |ω|·dt = {}", num(rate.norm()), deg(angle)),
            format!("Δq = (cos(|ω|dt/2), ω̂ sin(|ω|dt/2)) = {}", quat(dq)),
            format!("q ⊗ Δq = {}", quat(next)),
            format!("|q ⊗ Δq| = {}", precise(next.norm())),
        ],
        remarks: vec![Remark::BodyRate],
        shapes: vec![
            Shape::Frame {
                attitude: unit(q),
                role: Role::Before,
            },
            Shape::Arrow {
                from: Vec3::ZERO,
                to: unit(q).rotate(rate.normalized().unwrap_or(Vec3::ZERO)),
                role: Role::Second,
            },
            Shape::Frame {
                attitude: unit(next),
                role: Role::Result,
            },
        ],
        turn: Some((unit(q), unit(next))),
    }
}

pub fn integrate_linear(q: Quat, rate: Vec3, dt: f64, next: Quat) -> Step {
    let q_dot = q * Quat::pure(rate) * 0.5;
    let exact = q.integrate(rate, dt);
    Step {
        what: What::IntegrateLinear,
        lines: vec![
            format!("q̇ = ½ q ⊗ (0, ω) = {}", quat(q_dot)),
            format!("q + q̇·dt = {}", quat(next)),
            format!(
                "|q + q̇·dt| = {}, |q + q̇·dt| − 1 = {}",
                precise(next.norm()),
                num(next.norm() - 1.0)
            ),
            format!(
                "∠(q + q̇·dt, q ⊗ Δq) = {}",
                deg(next.normalized().unwrap_or(next).angle_to(exact))
            ),
        ],
        remarks: vec![Remark::OffSphere],
        shapes: vec![
            Shape::Frame {
                attitude: unit(q),
                role: Role::Before,
            },
            Shape::Frame {
                attitude: unit(next),
                role: Role::Result,
            },
        ],
        turn: Some((unit(q), unit(next))),
    }
}

pub fn delta(a: Quat, b: Quat, d: Quat) -> Step {
    let angle = a.angle_to(b);
    Step {
        what: What::Delta,
        lines: vec![
            format!("a* ⊗ b = {}", quat(d)),
            format!("a ⊗ (a* ⊗ b) = b, θ = {}", deg(angle)),
        ],
        remarks: vec![Remark::OwnAxes],
        shapes: vec![
            Shape::Frame {
                attitude: unit(a),
                role: Role::Before,
            },
            Shape::Frame {
                attitude: unit(b),
                role: Role::Result,
            },
        ],
        turn: Some((unit(a), unit(b))),
    }
}

pub fn attitude_error(a: Quat, b: Quat, theta: f64) -> Step {
    let d = a.conj() * b;
    Step {
        what: What::AttitudeError,
        lines: vec![
            format!("Δ = a* ⊗ b = {}", quat(d)),
            format!(
                "θ = 2·atan2(|Δ_v|, |Δ_w|) = 2·atan2({}, {}) = {}",
                num(d.vector().norm()),
                num(d.w.abs()),
                deg(theta)
            ),
        ],
        remarks: Vec::new(),
        shapes: vec![
            Shape::Frame {
                attitude: unit(a),
                role: Role::Before,
            },
            Shape::Frame {
                attitude: unit(b),
                role: Role::Result,
            },
        ],
        turn: Some((unit(a), unit(b))),
    }
}

pub fn to_rotvec(q: Quat, v: Vec3) -> Step {
    step(
        What::ToRotvec,
        vec![
            format!(
                "θ = {}, n = {}",
                deg(v.norm()),
                vec3(v.normalized().unwrap_or(Vec3::ZERO))
            ),
            format!("n θ = {}", vec3(v)),
        ],
        vec![
            Shape::Frame {
                attitude: unit(q),
                role: Role::Result,
            },
            Shape::Arrow {
                from: Vec3::ZERO,
                to: v,
                role: Role::Result,
            },
        ],
    )
}

pub fn from_rotvec(v: Vec3, q: Quat) -> Step {
    Step {
        what: What::FromRotvec,
        lines: vec![
            format!("θ = |v| = {}", deg(v.norm())),
            format!("q = (cos(θ/2), v̂ sin(θ/2)) = {}", quat(q)),
        ],
        remarks: Vec::new(),
        shapes: vec![
            Shape::Arrow {
                from: Vec3::ZERO,
                to: v,
                role: Role::First,
            },
            Shape::Frame {
                attitude: q,
                role: Role::Result,
            },
        ],
        turn: Some((Quat::IDENTITY, q)),
    }
}

/// What an accelerometer reads at rest (or, `gravity`, the pull it feels
/// against), drawn on the body: the world's up, and its body components.
pub fn at_rest(q: Quat, up: Vec3, reading: Vec3, gravity: bool) -> Step {
    let body = unit(q);
    let mut shapes = vec![
        Shape::Frame {
            attitude: body,
            role: Role::Result,
        },
        Shape::Arrow {
            from: Vec3::ZERO,
            to: if gravity { -up } else { up },
            role: Role::Result,
        },
    ];
    // Each body component, along the body axis it is measured on.
    for (axis, value) in [
        (Vec3::X, reading.x),
        (Vec3::Y, reading.y),
        (Vec3::Z, reading.z),
    ] {
        let along = body.rotate(axis) * value;
        if along.norm() > 1e-9 {
            shapes.push(Shape::Arrow {
                from: Vec3::ZERO,
                to: along,
                role: Role::Term,
            });
            shapes.push(Shape::Guide {
                from: along,
                to: if gravity { -up } else { up },
            });
        }
    }
    step(
        if gravity { What::Gravity } else { What::AtRest },
        vec![
            format!("up = {}", vec3(up)),
            if gravity {
                format!("q* ⊗ (0, −up) ⊗ q = {}", vec3(reading))
            } else {
                format!("q* ⊗ (0, up) ⊗ q = {}", vec3(reading))
            },
        ],
        shapes,
    )
    .noting(Remark::BodyAxes)
}

pub fn tilt(acc: Vec3, sign: f64, e: Euler) -> Step {
    step(
        What::Tilt,
        vec![
            format!(
                "φ = atan2({s}a_y, {s}a_z) = atan2({}, {}) = {}",
                num(sign * acc.y),
                num(sign * acc.z),
                deg(e.roll),
                s = if sign < 0.0 { "−" } else { "" }
            ),
            format!(
                "θ = atan2({s}a_x, √(a_y² + a_z²)) = atan2({}, {}) = {}",
                num(-sign * acc.x),
                num(acc.y.hypot(acc.z)),
                deg(e.pitch),
                s = if sign < 0.0 { "" } else { "−" }
            ),
        ],
        vec![Shape::Frame {
            attitude: Quat::from_euler(e),
            role: Role::Result,
        }],
    )
    .noting(Remark::NoHeading)
}

pub fn mat_vec(m: Mat3, v: Vec3, r: Vec3) -> Step {
    step(
        What::MatVec,
        vec![
            format!(
                "R v = ({}, {}, {})",
                num(m.row(0).dot(v)),
                num(m.row(1).dot(v)),
                num(m.row(2).dot(v))
            ),
            format!("= {}", vec3(r)),
        ],
        vec![
            Shape::Arrow {
                from: Vec3::ZERO,
                to: v,
                role: Role::First,
            },
            Shape::Arrow {
                from: Vec3::ZERO,
                to: r,
                role: Role::Result,
            },
        ],
    )
}

pub fn mat_mat(r: Mat3) -> Step {
    step(What::MatMat, vec![format!("= {}", mat3(r))], Vec::new())
}

pub fn transpose(m: Mat3, t: Mat3) -> Step {
    step(
        What::Transpose,
        vec![
            format!("Rᵀ = {}", mat3(t)),
            format!("max |RᵀR − I| = {}", num(m.orthonormal_error())),
        ],
        Vec::new(),
    )
    .noting(Remark::TransposeInverse)
}

pub fn det(m: Mat3, d: f64) -> Step {
    step(
        What::Det,
        vec![format!("det R = r₀·(r₁ × r₂) = {}", num(d)), mat3(m)],
        Vec::new(),
    )
    .noting(Remark::DetSign)
}

/// Two attitudes that should have been one: the reference as a ghost and
/// the value checked against it solid, so what the verdict names can be
/// seen — the inverse turned the other way, the other frame upside down.
pub fn check_rotation(mine: Quat, reference: Quat, error: f64) -> Step {
    step(
        What::Check,
        vec![format!("∠(mine, reference) = {}", deg(error))],
        vec![
            Shape::Frame {
                attitude: unit(reference),
                role: Role::Before,
            },
            Shape::Frame {
                attitude: unit(mine),
                role: Role::Result,
            },
        ],
    )
}

/// Two vectors that should have been one, and the gap between them.
pub fn check_vector(mine: Vec3, reference: Vec3) -> Step {
    step(
        What::Check,
        vec![format!(
            "|mine − reference| = |{}| = {}",
            vec3(mine - reference),
            num((mine - reference).norm())
        )],
        vec![
            Shape::Arrow {
                from: Vec3::ZERO,
                to: reference,
                role: Role::Second,
            },
            Shape::Arrow {
                from: Vec3::ZERO,
                to: mine,
                role: Role::Result,
            },
            Shape::Guide {
                from: reference,
                to: mine,
            },
        ],
    )
}

/// Two numbers, or two angles, that should have been one.
pub fn check_number(mine: f64, reference: f64, angle: bool) -> Step {
    let line = if angle {
        format!(
            "mine − reference = {} − {} = {}",
            deg(mine),
            deg(reference),
            deg(mine - reference)
        )
    } else {
        format!(
            "mine − reference = {} − {} = {}",
            num(mine),
            num(reference),
            num(mine - reference)
        )
    };
    step(What::Check, vec![line], Vec::new())
}

fn show(v: Vec3, flat: bool) -> String {
    if flat {
        vec2(Vec2::new(v.x, v.y))
    } else {
        vec3(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_print_as_short_as_they_are_exact() {
        assert_eq!(num(0.5), "0.5");
        assert_eq!(num(1.0), "1");
        assert_eq!(num(-0.00001), "-1e-5");
        assert_eq!(num(std::f64::consts::FRAC_1_SQRT_2), "0.7071");
        assert_eq!(num(-0.00004), "-4e-5");
        assert_eq!(num(-0.0), "0");
        assert_eq!(num(12345678.0), "1.235e7");
        assert_eq!(num(f64::NAN), "NaN");
        assert_eq!(deg(std::f64::consts::FRAC_PI_6), "30°");
        assert_eq!(deg(0.2181661564992912), "12.5°");
        assert_eq!(precise(1.0000012345), "1.000001235");
    }

    /// The three turns end where the closed form does, and each begins
    /// where the one before it ended — the only way playing them in order
    /// shows one continuous motion.
    #[test]
    fn the_euler_turns_are_continuous_and_end_at_the_closed_form() {
        let e = Euler::new(0.4, -0.3, 1.2);
        let steps = euler_turns(e);
        let turns: Vec<(Quat, Quat)> = steps.iter().map(|s| s.turn.unwrap()).collect();
        assert_eq!(turns[0].0, Quat::IDENTITY);
        assert_eq!(turns[0].1, turns[1].0);
        assert_eq!(turns[1].1, turns[2].0);
        assert!(turns[2].1.angle_to(Quat::from_euler(e)) < 1e-12);
    }

    /// Rodrigues' three terms, head to tail, land where the sandwich does.
    #[test]
    fn rodrigues_terms_sum_to_the_turned_vector() {
        let q = Quat::from_euler(Euler::new(0.3, 0.7, -1.1));
        let v = Vec3::new(1.0, -2.0, 0.5);
        let turned = q.rotate(v);
        let steps = sandwich(q, v, turned, false);
        let Shape::Arrow { to: tip, .. } = steps[1].shapes[5] else {
            panic!("the third term");
        };
        assert!((tip - turned).norm() < 1e-12);
        // And to the body: q*'s turn.
        let back = sandwich(q, turned, v, true);
        let Shape::Arrow { to: tip, .. } = back[1].shapes[5] else {
            panic!("the third term");
        };
        assert!((tip - v).norm() < 1e-12);
    }

    /// The pieces of an addition meet: b drawn from a's tip ends at a + b.
    #[test]
    fn an_addition_is_drawn_head_to_tail() {
        let (a, b) = (Vec3::new(1.0, 2.0, 0.0), Vec3::new(-3.0, 0.5, 0.0));
        let s = add(a, b, true);
        assert_eq!(s.lines, vec!["(1, 2) + (-3, 0.5) = (-2, 2.5)".to_string()]);
        assert_eq!(
            s.shapes[1],
            Shape::Arrow {
                from: a,
                to: a + b,
                role: Role::Second
            }
        );
    }
}
